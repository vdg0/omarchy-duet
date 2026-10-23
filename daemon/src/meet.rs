//! Meet mode: **bidirectional** translation inside a video call.
//!
//! Architecture (only one person runs the app):
//!
//! ```text
//!   your voice (es) ──► VAD ──► ASR es ──► MT es_en ──► TTS en ──► FIFO
//!                                                                 │
//!                                                     duet_mic_src
//!                                                     (module-pipe-source)
//!                                                                 │
//!                                                         Meet sees it as MIC
//!
//!   Meet ──► duet_meet (sink) ──► parec -d duet_meet.monitor
//!                                          │
//!                                          ├──► VAD ──► ASR en ──► MT en_es
//!                                          └──► TTS es ──► your headphones
//! ```
//!
//! Two nodes, the minimum for Meet to see one input and one output:
//!  - `duet_mic_src` (`module-pipe-source`): Meet selects it as the microphone.
//!    We write the translation PCM into its FIFO; there is **no** intermediate
//!    sink, so no "Duet-Mic" appears in the outputs.
//!  - `duet_meet` (`module-null-sink`): Meet selects it as the speaker. We
//!    capture it with `parec` directly from the monitor; there is **no**
//!    intermediate source, so no "Duet-Meet" appears in the inputs.
//!
//! Why `parec` and not `pw-record` for the monitor: PipeWire does not expose
//! monitors as nodes (`pw-dump` does not list them) and `pw-record --target
//! foo.monitor` records silence. The PulseAudio compatibility layer resolves
//! them; `parec` delivers raw f32 PCM and also connects to the sink even if
//! Meet is not playing anything yet.
//!
//! Echo control: because we are in turn-based mode, playback to the headphones
//! is **synchronous** (`pw-play` blocks) and we drain the microphone during
//! output. Audio going to Meet is digital (FIFO), so it cannot reopen a turn
//! through acoustic echo; we still wait for its duration to keep turns in
//! order.

use crate::asr::Asr;
use crate::control::{Dir, Hub, State, TurnEvent};
use crate::mt::Mt;
use crate::pwio::{Source, play_wav, play_wav_to_pipe};
use crate::tts::Tts;
use crate::turns::{TurnDetector, TurnOpts};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
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

pub const VIRTUAL_MIC_SRC: &str = "duet_mic_src";
pub const PEER_SINK: &str = "duet_meet";
/// Monitor of the sink Meet uses as its speaker. It is not a PipeWire node:
/// the PulseAudio layer resolves it (that is why `parec`, not `pw-record`).
pub const PEER_MONITOR: &str = "duet_meet.monitor";

pub struct MeetConfig {
    pub mic_device: Option<String>,
    pub buds_device: Option<String>,
    pub user_lang: String,
    pub peer_lang: String,
    pub pause_ms: u32,
    pub min_speech_ms: u32,
    pub max_turns: usize,
    pub out_dir: PathBuf,
    pub vad: crate::vad_stream::VadOpts,
    pub vad_start: f32,
    pub vad_end: f32,
    pub vad_onset_ms: u32,
    pub warmup_ms: u32,
    /// WAV that simulates your voice (test without a microphone).
    pub sim_mic: Option<PathBuf>,
    /// WAV that simulates what arrives from Meet (test without a call).
    pub sim_meet: Option<PathBuf>,
    pub hub: Option<Arc<crate::control::Hub>>,
}

fn fmt_ms(ms: f64) -> String {
    format!("{ms:>7.0} ms")
}

// -- virtual node management through pactl -------------------------------

fn pactl(args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("pactl")
        .args(args)
        .output()
        .map_err(|e| anyhow::anyhow!("could not run pactl: {e}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "pactl {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// FIFO that feeds the virtual microphone `module-pipe-source`.
fn mic_fifo() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join("duet-mic.fifo")
}

/// If a previous run died with Ctrl-C it left the modules loaded; unload them
/// here so startup is idempotent.
fn cleanup_leftovers() {
    if let Ok(out) = pactl(&["list", "short", "modules"]) {
        for line in out.lines() {
            // "duet_mic" also covers the old sink from the previous scheme
            // (null-sink + remap) and its sources.
            if line.contains("duet_mic") || line.contains(PEER_SINK) {
                if let Some(id) = line.split_whitespace().next() {
                    let _ = pactl(&["unload-module", id]);
                }
            }
        }
    }
    let _ = std::fs::remove_file(mic_fifo());
}

struct Modules {
    mic_pipe: String,
    peer_null: String,
    fifo: PathBuf,
}

impl Modules {
    fn cleanup(&mut self) {
        let _ = pactl(&["unload-module", &self.peer_null]);
        let _ = pactl(&["unload-module", &self.mic_pipe]);
        let _ = std::fs::remove_file(&self.fifo);
    }
}

impl Drop for Modules {
    fn drop(&mut self) {
        self.cleanup();
    }
}

fn create_modules() -> anyhow::Result<Modules> {
    cleanup_leftovers();

    let fifo = mic_fifo();
    let _ = std::fs::remove_file(&fifo);
    let status = Command::new("mkfifo")
        .args(["-m", "600"])
        .arg(&fifo)
        .status()
        .map_err(|e| anyhow::anyhow!("could not run mkfifo: {e}"))?;
    if !status.success() {
        anyhow::bail!("mkfifo failed with {status}");
    }

    // The virtual microphone is a FIFO-fed source: it does not need a sink
    // behind it, which is why no "Duet-Mic" appears in the outputs.
    let mic_pipe = match pactl(&[
        "load-module",
        "module-pipe-source",
        &format!("source_name={VIRTUAL_MIC_SRC}"),
        &format!("file={}", fifo.display()),
        "format=s16le",
        &format!("rate={}", crate::pwio::TTS_RATE),
        "channels=1",
        "source_properties=device.description=Duet-Mic",
    ]) {
        Ok(id) => id,
        Err(e) => {
            let _ = std::fs::remove_file(&fifo);
            return Err(e);
        }
    };

    // Meet's speaker is a sink: it is the only thing Meet should see as an
    // output. Its monitor is captured with `parec` (see PEER_MONITOR).
    let peer_null = match pactl(&[
        "load-module",
        "module-null-sink",
        &format!("sink_name={PEER_SINK}"),
        "sink_properties=device.description=Duet-Meet",
    ]) {
        Ok(id) => id,
        Err(e) => {
            let _ = pactl(&["unload-module", &mic_pipe]);
            let _ = std::fs::remove_file(&fifo);
            return Err(e);
        }
    };

    // Volumes at 100% and unmuted: otherwise the translation is quiet or silent.
    let _ = pactl(&["set-source-volume", VIRTUAL_MIC_SRC, "100%"]);
    let _ = pactl(&["set-source-mute", VIRTUAL_MIC_SRC, "0"]);
    let _ = pactl(&["set-sink-volume", PEER_SINK, "100%"]);
    let _ = pactl(&["set-sink-mute", PEER_SINK, "0"]);

    Ok(Modules {
        mic_pipe,
        peer_null,
        fifo,
    })
}

// -- bidirectional loop --------------------------------------------------

/// Direction of a detected turn (who spoke and where it goes).
struct Leg {
    label: &'static str,
    dir: Dir,
    src: String,
    dst: String,
    mt_dir: String,
    /// Destination node for `pw-play`; empty if the audio goes to the mic FIFO.
    target: String,
    /// The virtual mic is fed through a FIFO, not a PipeWire sink.
    to_pipe: bool,
}

fn run_turn(
    turn: usize,
    leg: &Leg,
    pcm: &[f32],
    speech_s: f64,
    asr: &Asr,
    engine: &mut Mt,
    voice: &mut Tts,
    out_dir: &PathBuf,
    mic_fifo: &Path,
    hub: Option<&Hub>,
) -> anyhow::Result<(f64, f64, f64)> {
    let t_asr = Instant::now();
    let text = asr.transcribe(pcm, &leg.src)?;
    let asr_ms = t_asr.elapsed().as_secs_f64() * 1e3;

    let t_mt = Instant::now();
    let translated = if text.is_empty() {
        String::new()
    } else {
        engine.translate(&leg.mt_dir, &text)?
    };
    let mt_ms = t_mt.elapsed().as_secs_f64() * 1e3;

    let out_wav = out_dir.join(format!("meet_t{turn}.wav"));
    let t_tts = Instant::now();
    let syn = if translated.is_empty() {
        None
    } else {
        Some(voice.synth(&leg.dst, &translated, &out_wav)?)
    };
    let tts_ms = t_tts.elapsed().as_secs_f64() * 1e3;

    println!(
        "-- turn {turn} ({speech_s:.1}s of speech)   {} --",
        leg.label
    );
    println!("   [{:>2}] {}", leg.src, text);
    println!("   [{:>2}] {}", leg.dst, translated);
    println!(
        "   ASR {} | MT {} | TTS {} | audio {:.2}s",
        fmt_ms(asr_ms),
        fmt_ms(mt_ms),
        fmt_ms(tts_ms),
        syn.as_ref().map(|s| s.audio_s).unwrap_or(0.0)
    );

    if let Some(h) = hub {
        h.emit_turn(TurnEvent {
            kind: "turn",
            dir: leg.dir.clone(),
            src_lang: leg.src.clone(),
            dst_lang: leg.dst.clone(),
            text_src: text.clone(),
            text_dst: translated.clone(),
            duration_ms: (asr_ms + mt_ms + tts_ms).round() as u64,
            ts: now_ms(),
        });
        if !text.is_empty() {
            h.log(format!(
                "[{}] {} -> [{}] {}",
                leg.src, text, leg.dst, translated
            ));
        }
    }

    if let Some(s) = syn.as_ref() {
        // Synchronous playback: in turn-based mode we want the other direction
        // quiet while the translation plays.
        if leg.to_pipe {
            play_wav_to_pipe(&out_wav, mic_fifo, s.audio_s)?;
        } else {
            play_wav(&out_wav, Some(&leg.target))?;
        }
    }
    Ok((asr_ms, mt_ms, tts_ms))
}

/// Drain a source channel so audio captured while the translation played
/// does not reopen a spurious turn.
fn drain(rx: &Receiver<Vec<f32>>) {
    while rx.try_recv().is_ok() {}
}

fn loop_bidirectional(
    cfg: &MeetConfig,
    rx_user: &Receiver<Vec<f32>>,
    rx_peer: &Receiver<Vec<f32>>,
    asr: &Asr,
    engine: &mut Mt,
    voice: &mut Tts,
    mic_fifo: &Path,
) -> anyhow::Result<()> {
    let hub = cfg.hub.as_deref();
    let turn_opts = || TurnOpts {
        vad: cfg.vad.clone(),
        vad_start: cfg.vad_start,
        vad_end: cfg.vad_end,
        vad_onset_ms: cfg.vad_onset_ms,
        pause_ms: cfg.pause_ms,
        min_speech_ms: cfg.min_speech_ms,
        warmup_ms: cfg.warmup_ms,
    };

    let (mut det_user, kind_user) = TurnDetector::new(&turn_opts())?;
    let (mut det_peer, kind_peer) = TurnDetector::new(&turn_opts())?;
    println!("  VAD (you): {kind_user} | VAD (them): {kind_peer}");

    // destination for each direction
    let buds_name =
        crate::devices::resolve_sink(cfg.buds_device.as_deref(), Duration::from_secs(3))
            .map(|n| n.name)
            .unwrap_or_default();
    let user_leg = Leg {
        label: "you -> Meet",
        dir: Dir::UserToPeer,
        src: cfg.user_lang.clone(),
        dst: cfg.peer_lang.clone(),
        mt_dir: format!("{}_{}", cfg.user_lang, cfg.peer_lang),
        target: VIRTUAL_MIC_SRC.to_string(),
        to_pipe: true,
    };
    let peer_leg = Leg {
        label: "Meet -> you",
        dir: Dir::PeerToUser,
        src: cfg.peer_lang.clone(),
        dst: cfg.user_lang.clone(),
        mt_dir: format!("{}_{}", cfg.peer_lang, cfg.user_lang),
        target: buds_name,
        to_pipe: false,
    };

    println!("  Virtual mic (Meet): {VIRTUAL_MIC_SRC} (FIFO, no sink)");
    println!("  Audio from Meet: {PEER_MONITOR} (parec)");
    println!(
        "  Your output: {} ({})",
        if peer_leg.target.is_empty() {
            "default sink"
        } else {
            &peer_leg.target
        },
        peer_leg.dst
    );
    println!("  In Meet: Microphone = \"Duet-Mic\" | Speakers = \"Duet-Meet\"");
    println!();
    if let Some(h) = hub {
        h.log(format!(
            "Meet mode ready: mic={VIRTUAL_MIC_SRC} | output={} | {}<->{}",
            if peer_leg.target.is_empty() {
                "default sink"
            } else {
                &peer_leg.target
            },
            cfg.user_lang,
            cfg.peer_lang
        ));
        h.set_state(State::Running);
    }

    let mut turn = 0usize;
    let mut totals_user = (0.0f64, 0.0f64, 0.0f64);
    let mut totals_peer = (0.0f64, 0.0f64, 0.0f64);
    let mut user_done = false;
    let mut peer_done = false;

    while turn < cfg.max_turns {
        if hub.is_some_and(|h| h.stop_requested()) {
            hub_log(hub, "stop requested by the panel");
            break;
        }
        let mut did_work = false;

        // -- outbound direction: your voice -> Meet --
        if !user_done {
            let maybe = match rx_user.try_recv() {
                Ok(chunk) => {
                    did_work = true;
                    det_user.feed(&chunk)
                }
                Err(TryRecvError::Disconnected) => {
                    user_done = true;
                    det_user.flush()
                }
                Err(TryRecvError::Empty) => None,
            };
            if let Some(t) = maybe {
                turn += 1;
                let (a, m, s) = run_turn(
                    turn,
                    &user_leg,
                    &t.pcm,
                    t.speech_s,
                    asr,
                    engine,
                    voice,
                    &cfg.out_dir,
                    mic_fifo,
                    hub,
                )?;
                totals_user = (totals_user.0 + a, totals_user.1 + m, totals_user.2 + s);
                // The output went to the virtual mic through the FIFO: there is no acoustic
                // echo, so we do NOT touch the inbound direction (it could be accumulating a
                // turn from the other person).
                det_user.clear();
                println!();
            }
        }

        if turn >= cfg.max_turns {
            break;
        }

        // -- inbound direction: Meet -> your headphones --
        if !peer_done {
            let maybe = match rx_peer.try_recv() {
                Ok(chunk) => {
                    did_work = true;
                    det_peer.feed(&chunk)
                }
                Err(TryRecvError::Disconnected) => {
                    peer_done = true;
                    det_peer.flush()
                }
                Err(TryRecvError::Empty) => None,
            };
            if let Some(t) = maybe {
                turn += 1;
                let (a, m, s) = run_turn(
                    turn,
                    &peer_leg,
                    &t.pcm,
                    t.speech_s,
                    asr,
                    engine,
                    voice,
                    &cfg.out_dir,
                    mic_fifo,
                    hub,
                )?;
                totals_peer = (totals_peer.0 + a, totals_peer.1 + m, totals_peer.2 + s);
                // The translation already played in the headphones (pw-play blocked), so we
                // discard what the microphone captured in that interval — it could be distant
                // echo — and drop the partial turn so it does not mix with the next phrase.

                drain(rx_user);
                det_user.clear();
                det_peer.clear();
                println!();
            }
        }

        // With simulated sources both end: exit. With a live microphone the channel
        // never disconnects and we continue until Ctrl-C / --max-turns.
        if user_done && peer_done {
            println!(" (both sources ended)");
            break;
        }

        if !did_work {
            thread::sleep(Duration::from_millis(5));
        }
    }

    println!("-- summary -----------------------------------------------");
    let n_u = turn.max(1) as f64;
    println!(
        "   you -> Meet: avg ASR {} | avg MT {} | avg TTS {}",
        fmt_ms(totals_user.0 / n_u),
        fmt_ms(totals_user.1 / n_u),
        fmt_ms(totals_user.2 / n_u)
    );
    println!(
        "   Meet -> you: avg ASR {} | avg MT {} | avg TTS {}",
        fmt_ms(totals_peer.0 / n_u),
        fmt_ms(totals_peer.1 / n_u),
        fmt_ms(totals_peer.2 / n_u)
    );
    Ok(())
}

// -- entry point ---------------------------------------------------------

pub fn run(cfg: &MeetConfig, asr: &Asr, engine: &mut Mt, voice: &mut Tts) -> anyhow::Result<()> {
    println!("-- Meet mode (bidirectional) -----------------------------");
    let hub = cfg.hub.as_deref();
    hub_log(hub, "Meet mode: creating virtual nodes...");

    let mut mods = create_modules()?;
    println!("  virtual nodes created (cleaned up on exit)");
    println!("    mic  -> {VIRTUAL_MIC_SRC} (module-pipe-source)");
    println!("    Meet -> {PEER_SINK} (sink)");
    let mic_fifo = mods.fifo.clone();

    // -- source 1: your microphone --
    let user_src = if let Some(p) = &cfg.sim_mic {
        println!("  SIMULATED SOURCE (you): {}", p.display());
        Source::wav(p, true)?
    } else {
        let node =
            crate::devices::resolve_source(cfg.mic_device.as_deref(), Duration::from_secs(5))?;
        println!("  physical mic: {} ({})", node.desc, node.name);
        Source::mic(Some(&node.name))?
    };

    // -- source 2: what Meet plays --
    let peer_src = if let Some(p) = &cfg.sim_meet {
        println!("  SIMULATED SOURCE (them): {}", p.display());
        Source::wav(p, true)?
    } else {
        println!("  meet -> {PEER_MONITOR} (parec)");
        Source::pulse_monitor(PEER_MONITOR)?
    };
    println!();

    // Each Source already runs in its reader thread; forward them to a common
    // channel so we can alternate between the two directions.
    let (tx_user, rx_user) = std::sync::mpsc::channel::<Vec<f32>>();
    let (tx_peer, rx_peer) = std::sync::mpsc::channel::<Vec<f32>>();
    let fwd_user = user_src;
    let fwd_peer = peer_src;
    thread::spawn(move || {
        while let Ok(c) = fwd_user.rx.recv() {
            if tx_user.send(c).is_err() {
                break;
            }
        }
    });
    thread::spawn(move || {
        while let Ok(c) = fwd_peer.rx.recv() {
            if tx_peer.send(c).is_err() {
                break;
            }
        }
    });

    let result = loop_bidirectional(cfg, &rx_user, &rx_peer, asr, engine, voice, &mic_fifo);

    // Ctrl-C exits through here when the loop ends: unload manually (Drop would
    // also run, but this makes it explicit in the output).
    if let Some(h) = hub {
        h.set_state(State::Stopping);
    }
    mods.cleanup();
    println!("  virtual nodes removed");
    hub_log(hub, "virtual nodes removed");
    result
}
