//! Live mode: microphone capture, turn detection, and playback.
//!
//! Flow: microphone (pw-record) -> streaming VAD -> when the phrase closes,
//! ASR -> MT -> TTS -> playback (pw-play).
//!
//! Decisions for this mode:
//!  - Native PipeWire audio (`pw-record`/`pw-play`) instead of ALSA/cpal:
//!    it is the same path Meet mode needs to route the browser audio, so we
//!    validate the tools now.
//!  - During processing and playback we **discard** microphone input. This
//!    avoids the echo loop (we would hear ourselves) and is consistent with
//!    turn-based mode. If you speak while playback is running, that part is
//!    lost: headphones are recommended.

use crate::asr::Asr;
use crate::control::{Dir, Hub, State, TurnEvent};
use crate::mt::Mt;
use crate::pwio::{Source, play_wav};
use crate::tts::Tts;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn hub_log(hub: Option<&Hub>, msg: impl Into<String>) {
    let msg = msg.into();
    if let Some(h) = hub {
        h.log(msg);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub struct LiveConfig {
    pub src_lang: String,
    pub dst_lang: String,
    pub mt_dir: String,
    pub pause_ms: u32,
    pub min_speech_ms: u32,
    pub max_turns: usize,
    pub play: bool,
    pub out_dir: PathBuf,
    pub mic_device: Option<String>,
    pub speaker_device: Option<String>,
    /// Test source: instead of the microphone, feed this WAV as if it were live
    /// audio. Useful for validating the streaming VAD reproducibly.
    pub source_wav: Option<PathBuf>,
    /// With `source_wav`, do not respect real time (run as fast as possible).
    pub simulate_fast: bool,
    /// Streaming VAD configuration.
    pub vad: crate::vad_stream::VadOpts,
    /// Probability required to *start* considering speech.
    pub vad_start: f32,
    /// Probability below which the input is considered silence.
    pub vad_end: f32,
    /// How long speech must be sustained to open a turn (filters isolated peaks).
    pub vad_onset_ms: u32,
    /// How much initial audio to discard (the capture-open pop).
    pub warmup_ms: u32,
    pub hub: Option<Arc<crate::control::Hub>>,
}

fn fmt_ms(ms: f64) -> String {
    format!("{ms:>7.0} ms")
}

pub fn run(cfg: &LiveConfig, asr: &Asr, engine: &mut Mt, voice: &mut Tts) -> Result<()> {
    let hub = cfg.hub.as_deref();
    println!("-- live mode ---------------------------------------------");
    hub_log(hub, "live mode: opening microphone...");
    println!(
        "  microphone -> {} -> {}   (cut pause: {} ms, Ctrl-C to quit)",
        cfg.src_lang, cfg.dst_lang, cfg.pause_ms
    );
    if cfg.play {
        println!("  use headphones: speaker output feeds back into the microphone");
    } else {
        println!("  (--no-play: no playback, print only)");
    }
    println!();

    let (source, speaker) = match &cfg.source_wav {
        Some(path) => {
            println!("  TEST SOURCE: {} (simulating microphone)", path.display());
            // With a simulated source we only resolve the output (if playback is needed).
            let spk = if cfg.play {
                Some(crate::devices::resolve_sink(
                    cfg.speaker_device.as_deref(),
                    Duration::from_secs(3),
                )?)
            } else {
                None
            };
            (Source::wav(path, !cfg.simulate_fast)?, spk)
        }
        None => {
            let src =
                crate::devices::resolve_source(cfg.mic_device.as_deref(), Duration::from_secs(5))?;
            println!("  microphone: {} ({})", src.desc, src.name);
            let spk = if cfg.play {
                let s = crate::devices::resolve_sink(
                    cfg.speaker_device.as_deref(),
                    Duration::from_secs(3),
                )?;
                println!("  speakers:   {} ({})", s.desc, s.name);
                Some(s)
            } else {
                None
            };
            (Source::mic(Some(&src.name))?, spk)
        }
    };
    let speaker_name = speaker.map(|s| s.name);
    if let Some(h) = hub {
        h.log(format!(
            "live mode ready: {} -> {} (output: {})",
            cfg.src_lang,
            cfg.dst_lang,
            speaker_name.as_deref().unwrap_or("default sink")
        ));
        h.set_state(State::Running);
    }

    // -- turn detection --
    let (mut det, vad_kind) = crate::turns::TurnDetector::new(&crate::turns::TurnOpts {
        vad: cfg.vad.clone(),
        vad_start: cfg.vad_start,
        vad_end: cfg.vad_end,
        vad_onset_ms: cfg.vad_onset_ms,
        pause_ms: cfg.pause_ms,
        min_speech_ms: cfg.min_speech_ms,
        warmup_ms: cfg.warmup_ms,
    })?;
    println!("  VAD: {vad_kind}");
    println!();

    let mut turns = 0usize;
    let mut totals = (0.0f64, 0.0f64, 0.0f64);
    let mut warned_play = false;

    while turns < cfg.max_turns {
        if hub.is_some_and(|h| h.stop_requested()) {
            hub_log(hub, "stop requested by the panel");
            break;
        }
        let turn = match source.rx.recv_timeout(Duration::from_millis(200)) {
            Ok(c) => match det.feed(&c) {
                Some(t) => t,
                None => continue,
            },
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                // The source ended: close the current phrase (if any) and, if
                // nothing remains, exit.
                match det.flush() {
                    Some(t) => t,
                    None => {
                        println!("audio source ended");
                        break;
                    }
                }
            }
        };

        turns += 1;
        let pcm = turn.pcm;
        let speech_s = turn.speech_s;
        let last_speech_at = turn.last_speech_at;

        let t_asr = Instant::now();
        let text = asr.transcribe(&pcm, &cfg.src_lang)?;
        let asr_ms = t_asr.elapsed().as_secs_f64() * 1e3;

        let t_mt = Instant::now();
        let translated = if text.is_empty() {
            String::new()
        } else {
            engine.translate(&cfg.mt_dir, &text)?
        };
        let mt_ms = t_mt.elapsed().as_secs_f64() * 1e3;

        let out_wav = cfg.out_dir.join(format!("live_turn_{turns}.wav"));
        let t_tts = Instant::now();
        let syn = if translated.is_empty() {
            None
        } else {
            Some(voice.synth(&cfg.dst_lang, &translated, &out_wav)?)
        };
        let tts_ms = t_tts.elapsed().as_secs_f64() * 1e3;

        // Playback: if it fails (Bluetooth disconnected mid-conversation, for example)
        // do not abort live mode; warn and keep listening.
        let mut play_start = None;
        if cfg.play && syn.is_some() {
            match play_wav(&out_wav, speaker_name.as_deref()) {
                Ok(start) => play_start = Some(start),
                Err(e) if !warned_play => {
                    warned_play = true;
                    eprintln!("   [warning] could not play: {e:#}");
                    eprintln!("   [warning] still listening; check that the destination exists.");
                }
                Err(_) => {}
            }
        }

        let e2e_ms = match (last_speech_at, play_start) {
            (Some(speech), Some(start)) => start.duration_since(speech).as_secs_f64() * 1e3,
            _ => asr_ms + mt_ms + tts_ms + cfg.pause_ms as f64,
        };

        totals.0 += asr_ms;
        totals.1 += mt_ms;
        totals.2 += tts_ms;

        println!("-- turn {turns} ({speech_s:.1}s of speech) --");
        println!("   [{}] {}", cfg.src_lang, text);
        println!("   [{}] {}", cfg.dst_lang, translated);
        println!(
            "   ASR {} | MT {} | TTS {} | audio {:.2}s",
            fmt_ms(asr_ms),
            fmt_ms(mt_ms),
            fmt_ms(tts_ms),
            syn.as_ref().map(|s| s.audio_s).unwrap_or(0.0)
        );
        println!("   end of speech -> starts playing: {}", fmt_ms(e2e_ms));
        if syn.is_some() {
            println!("   -> {}", out_wav.display());
        }

        if let Some(h) = hub {
            h.emit_turn(TurnEvent {
                kind: "turn",
                dir: Dir::UserToPeer,
                src_lang: cfg.src_lang.clone(),
                dst_lang: cfg.dst_lang.clone(),
                text_src: text.clone(),
                text_dst: translated.clone(),
                duration_ms: (asr_ms + mt_ms + tts_ms).round() as u64,
                ts: now_ms(),
            });
            if !text.is_empty() {
                h.log(format!(
                    "[{}] {} -> [{}] {}",
                    cfg.src_lang, text, cfg.dst_lang, translated
                ));
            }
        }

        // If we are playing, discard what was captured during processing and
        // playback: this prevents the microphone from re-hearing our own output
        // (echo loop). If we do not play, there is no echo to avoid and we keep the
        // audio exactly as it arrived.
        if cfg.play {
            source.drain();
            thread::sleep(Duration::from_millis(250));
            source.drain();
        }
        det.clear();
        println!();
    }

    let n = turns.max(1) as f64;
    if let Some(h) = hub {
        h.set_state(State::Stopping);
    }
    println!("-- summary ({turns} turns) --------------------------------");
    println!("   average ASR {}", fmt_ms(totals.0 / n));
    println!("   average MT  {}", fmt_ms(totals.1 / n));
    println!("   average TTS {}", fmt_ms(totals.2 / n));
    hub_log(hub, format!("live mode stopped ({turns} turns)"));
    Ok(())
}

/// `--meter`: show the live level (dBFS) and VAD speech probability.
/// It answers "is the microphone hearing me?".
pub fn meter(cfg: &LiveConfig) -> Result<()> {
    let src = match &cfg.source_wav {
        Some(_) => anyhow::bail!("--meter uses the microphone, not --live-source"),
        None => crate::devices::resolve_source(cfg.mic_device.as_deref(), Duration::from_secs(5))?,
    };
    let (mut vad, kind) = crate::vad_stream::StreamVad::new(&cfg.vad)?;
    println!("-- meter -------------------------------------------------");
    println!("  microphone: {} ({})", src.desc, src.name);
    println!("  VAD: {kind}   (Ctrl-C to quit)");
    println!("  speak normally and watch the bar and probability.");
    println!();
    let source = Source::mic(Some(&src.name))?;
    let mut warmup = 5;
    let mut peak_db = -120.0f32;
    let mut speech = 0usize;
    let mut total = 0usize;
    for chunk in source.rx.iter() {
        if warmup > 0 {
            warmup -= 1;
            continue;
        }
        let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len().max(1) as f32).sqrt();
        let db = 20.0 * (rms.max(1e-9)).log10();
        let prob = vad.prob(&chunk);
        peak_db = peak_db.max(db);
        total += 1;
        if prob >= cfg.vad_start {
            speech += 1;
        }
        let bars = (((db + 60.0) / 3.0).clamp(0.0, 20.0)) as usize;
        let marker = if prob >= cfg.vad_start {
            "SPEECH"
        } else if prob >= cfg.vad_end {
            "?"
        } else {
            ""
        };
        print!(
            "\r  {db:6.1} dBFS  [{}]  prob {prob:4.2}  {:<5}  speech {:>3}% ({speech}/{total})   ",
            "#".repeat(bars) + &" ".repeat(20 - bars),
            marker,
            if total > 0 { speech * 100 / total } else { 0 }
        );
        use std::io::Write;
        std::io::stdout().flush().ok();
    }
    println!();
    println!(
        "\n  summary: {total} chunks | peak {peak_db:.1} dBFS | speech {}% ({speech}/{total})",
        if total > 0 { speech * 100 / total } else { 0 }
    );
    Ok(())
}

/// `--test-speaker`: play a short tone through the selected destination.
///
/// It answers unambiguously "is the output going to the headphones?" before
/// starting a conversation, without loading ASR/MT/TTS.
pub fn test_speaker(cfg: &LiveConfig) -> Result<()> {
    let sink = crate::devices::resolve_sink(cfg.speaker_device.as_deref(), Duration::from_secs(5))?;
    println!("-- output test -------------------------------------------");
    println!("  destination: {} ({})", sink.desc, sink.name);

    // Two tones (440 and 660 Hz) with a pause in between.
    let sr = 22_050u32;
    let mut samples: Vec<f32> = Vec::new();
    for (freq, ms) in [(440.0f32, 350u32), (0.0, 120), (660.0, 350)] {
        let n = (ms as f32 / 1000.0 * sr as f32) as usize;
        for i in 0..n {
            if freq == 0.0 {
                samples.push(0.0);
                continue;
            }
            let t = i as f32 / sr as f32;
            let env = 1.0 - (i as f32 / n as f32); // decay to avoid a click
            let amp = 0.35 * env.clamp(0.0, 1.0);
            samples.push((2.0 * std::f32::consts::PI * freq * t).sin() * amp);
        }
    }
    let path = cfg.out_dir.join("_test_speaker.wav");
    crate::audio::write_wav(&path, &samples, sr)?;

    println!("  playing a tone through it...");
    match play_wav(&path, Some(&sink.name)) {
        Ok(_) => {
            println!("  Did you hear the tone at the destination? Then the output is correct.")
        }
        Err(e) => {
            println!("  ERROR playing audio: {e:#}");
            println!("  check that the device is still connected (--list-devices).");
        }
    }
    Ok(())
}
