//! Duet voice interpreter.
//!
//!  - Offline benchmark mode: process a WAV file and measure pipeline
//!    latency/RAM.
//!      duet --input conversation.wav --from es --to en
//!  - Live mode: listen to the microphone, detect turns, and play the
//!    translation through the speakers.
//!      duet --live --model models/ggml-base-q5_1.bin
//!  - Meet mode: bidirectional translation inside a video call.
//!      duet --meet --model models/ggml-base-q5_1.bin
//!  - Audio diagnostics (useful with Bluetooth):
//!      duet --list-devices

mod asr;
mod audio;
mod control;
mod devices;
mod live;
mod meet;
mod mt;
mod pwio;
mod tts;
mod turns;
mod vad;
mod vad_stream;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(
    name = "duet",
    version,
    about = "Local ES <-> EN voice interpreter for Meet and live conversations"
)]
struct Args {
    /// Input WAV with the conversation to process (not needed with --live).
    #[arg(long = "input", visible_alias = "in")]
    input: Option<PathBuf>,

    /// Source language (ISO-639-1 code, e.g. "es").
    #[arg(long = "from", visible_alias = "de", default_value = "es")]
    source_lang: String,

    /// Target language.
    #[arg(long = "to", visible_alias = "a", default_value = "en")]
    target_lang: String,

    /// Whisper ggml model.
    #[arg(long, default_value = "models/ggml-base-q5_1.bin")]
    model: PathBuf,

    /// Whisper backend: auto uses the GPU only for large models.
    #[arg(long, value_enum, default_value = "auto")]
    gpu: GpuArg,

    /// Python interpreter from the venv with ctranslate2/sentencepiece/piper.
    #[arg(long, default_value = ".venv-py/bin/python")]
    python: PathBuf,

    /// Translation sidecar script.
    #[arg(long, default_value = "daemon/scripts/mt_server.py")]
    mt_script: PathBuf,

    /// TTS sidecar script.
    #[arg(long, default_value = "daemon/scripts/tts_server.py")]
    tts_script: PathBuf,

    /// Number of threads for Whisper.
    #[arg(long, default_value_t = 4)]
    threads: i32,

    /// Number of threads for MT.
    #[arg(long, default_value_t = 2)]
    mt_threads: u32,

    /// Whisper encoder window in tokens (0 = automatic based on segment length).
    #[arg(long, default_value_t = 0)]
    audio_ctx: i32,

    /// Initial ASR prompt (helps with accents and punctuation).
    #[arg(long)]
    prompt: Option<String>,

    /// Temperature increment for Whisper's anti-loop fallback.
    /// 0 = a single pass (lower latency, higher risk of repetitions).
    #[arg(long, default_value_t = 0.2)]
    temp_inc: f32,

    /// Pause (ms) that closes a phrase in the VAD.
    #[arg(long, default_value_t = 700)]
    pause_ms: u32,

    /// Minimum speech duration (ms) required to consider a turn. Increase it
    /// (e.g. 500) to ignore short noises that would create spurious turns.
    #[arg(long, default_value_t = 300)]
    min_speech_ms: u32,

    /// Number of complete passes (the first one may include lazy initialization).
    #[arg(long, default_value_t = 2)]
    passes: usize,

    /// Directory for translated WAV files.
    #[arg(long, default_value = "/tmp/duet/out")]
    out_dir: PathBuf,

    /// Only load models and measure idle RAM (do not process audio).
    #[arg(long, default_value_t = false)]
    check_ram: bool,

    /// Do not silence whisper.cpp logs.
    #[arg(long, default_value_t = false)]
    verbose: bool,

    /// Live mode: listen to the microphone, detect turns, and play the translation.
    #[arg(long, default_value_t = false)]
    live: bool,

    /// Meet mode (bidirectional): create a virtual microphone and translate both directions.
    #[arg(long, default_value_t = false)]
    meet: bool,

    /// Live/Meet mode: stop after N turns (0 = unlimited).
    #[arg(long, default_value_t = 0)]
    max_turns: usize,

    /// Live mode: do not play the translated audio.
    #[arg(long, default_value_t = false)]
    no_play: bool,

    /// Live/Meet mode: PipeWire input node (microphone). Defaults to the system device.
    #[arg(long)]
    mic_device: Option<String>,

    /// Live/Meet mode: PipeWire output node (speakers/headphones). Defaults to the system device.
    #[arg(long)]
    speaker_device: Option<String>,

    /// Live mode: use this WAV as the source instead of the microphone (for testing).
    #[arg(long)]
    live_source: Option<PathBuf>,

    /// With --live-source, process as fast as possible instead of in real time.
    #[arg(long, default_value_t = false)]
    live_fast: bool,

    /// List available microphones and speakers (with their names) and exit.
    #[arg(long, default_value_t = false)]
    list_devices: bool,

    /// Same as --list-devices but as JSON (for the Omarchy panel).
    #[arg(long, default_value_t = false)]
    devices_json: bool,

    /// Measure the live level and VAD probability (microphone diagnostics).
    #[arg(long, default_value_t = false)]
    meter: bool,

    /// Play a short tone through the selected destination and exit.
    #[arg(long, default_value_t = false)]
    test_speaker: bool,

    /// Streaming VAD engine.
    #[arg(long, value_enum, default_value_t = vad_stream::VadKind::Auto)]
    vad: vad_stream::VadKind,

    /// Silero VAD model (only for `--vad silero`).
    #[arg(long, default_value = "models/ggml-silero-v5.1.2.bin")]
    vad_model: String,

    /// WebRTC VAD aggressiveness (0 = quality, 3 = very aggressive).
    #[arg(long, default_value_t = 3)]
    vad_mode: u8,

    /// VAD probability required to start considering speech.
    #[arg(long, default_value_t = 0.5)]
    vad_start: f32,

    /// VAD probability below which the input is considered silence.
    #[arg(long, default_value_t = 0.2)]
    vad_end: f32,

    /// How long speech must be sustained to open a turn (ms).
    #[arg(long, default_value_t = 200)]
    vad_onset_ms: u32,

    /// Absolute RMS threshold for the energy VAD fallback.
    #[arg(long, default_value_t = 0.008)]
    vad_threshold: f32,

    /// Audio window that Silero sees on each step (ms).
    #[arg(long, default_value_t = 1000)]
    vad_window_ms: u32,

    /// How far back Silero looks for the probability (ms).
    #[arg(long, default_value_t = 200)]
    vad_lookback_ms: u32,

    /// WAV file used to simulate the microphone (Meet mode).
    #[arg(long)]
    sim_mic: Option<PathBuf>,

    /// WAV file used to simulate Meet's output (Meet mode).
    #[arg(long)]
    sim_meet: Option<PathBuf>,

    /// Control socket path (JSON-lines). Defaults to $XDG_RUNTIME_DIR/duet.sock.
    #[arg(long)]
    control_socket: Option<PathBuf>,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum GpuArg {
    /// CPU for small models; GPU for medium/large models.
    Auto,
    /// Force the GPU (requires building with the `vulkan` feature).
    On,
    /// Force the CPU.
    Off,
}

fn peak_rss_kb(pid: u32) -> Option<u64> {
    read_status_field(pid, "VmHWM:")
}

/// Read the current RSS of a PID from /proc.
fn cur_rss_kb(pid: u32) -> Option<u64> {
    read_status_field(pid, "VmRSS:")
}

fn read_status_field(pid: u32, field: &str) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix(field) {
            return rest.split_whitespace().next().and_then(|v| v.parse().ok());
        }
    }
    None
}

fn fmt_ms(ms: f64) -> String {
    format!("{ms:>8.0} ms")
}

/// Build the live/meter configuration from the command-line arguments.
fn make_live_config(
    args: &Args,
    mt_dir: &str,
    hub: Option<std::sync::Arc<control::Hub>>,
) -> live::LiveConfig {
    live::LiveConfig {
        src_lang: args.source_lang.clone(),
        dst_lang: args.target_lang.clone(),
        mt_dir: mt_dir.to_string(),
        pause_ms: args.pause_ms,
        min_speech_ms: args.min_speech_ms,
        max_turns: if args.max_turns == 0 {
            usize::MAX
        } else {
            args.max_turns
        },
        play: !args.no_play,
        out_dir: args.out_dir.clone(),
        mic_device: args.mic_device.clone(),
        speaker_device: args.speaker_device.clone(),
        source_wav: args.live_source.clone(),
        simulate_fast: args.live_fast,
        vad: vad_stream::VadOpts {
            kind: args.vad,
            model: if args.vad_model.is_empty() {
                None
            } else {
                Some(PathBuf::from(&args.vad_model))
            },
            threads: 2,
            window_ms: args.vad_window_ms,
            lookback_ms: args.vad_lookback_ms,
            energy_threshold: args.vad_threshold,
            webrtc_mode: args.vad_mode,
        },
        vad_start: args.vad_start,
        vad_end: args.vad_end,
        vad_onset_ms: args.vad_onset_ms,
        warmup_ms: 500,
        hub,
    }
}

struct TurnResult {
    asr_ms: f64,
    mt_ms: f64,
    tts_ms: f64,
    audio_s: f64,
}

/// Remove the socket and pidfile on exit, even if the process fails.
struct RuntimeFiles {
    socket: PathBuf,
    pid: PathBuf,
}

impl Drop for RuntimeFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
        let _ = std::fs::remove_file(&self.pid);
    }
}

/// Resolve a relative default from the current directory or from the installed
/// runtime (`<runtime>/bin/duet`) / development checkout
/// (`<repo>/target/release/duet`). This keeps direct CLI use working
/// from outside the repository while still allowing explicit paths.
fn resolve_runtime_path(value: &Path, subdirs: &[&str]) -> PathBuf {
    if value.is_absolute() || value.exists() {
        return value.to_path_buf();
    }
    let Some(file_name) = value.file_name() else {
        return value.to_path_buf();
    };

    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin) = exe.parent() {
            if let Some(root) = bin.parent() {
                roots.push(root.to_path_buf());
            }
            if let Some(root) = bin.parent().and_then(Path::parent) {
                roots.push(root.to_path_buf());
            }
        }
    }

    for root in roots {
        for subdir in subdirs {
            let candidate = root.join(subdir).join(file_name);
            if candidate.exists() {
                return candidate;
            }
        }
    }
    value.to_path_buf()
}

fn resolve_runtime_paths(args: &mut Args) {
    args.model = resolve_runtime_path(&args.model, &["models"]);
    args.python = resolve_runtime_path(&args.python, &[".venv-py/bin"]);
    args.mt_script = resolve_runtime_path(&args.mt_script, &["daemon/scripts", "scripts"]);
    args.tts_script = resolve_runtime_path(&args.tts_script, &["daemon/scripts", "scripts"]);
    args.vad_model = resolve_runtime_path(Path::new(&args.vad_model), &["models"])
        .to_string_lossy()
        .into_owned();
}

fn main() -> Result<()> {
    let mut args = Args::parse();
    resolve_runtime_paths(&mut args);

    // Diagnostic commands: do not open the socket or load models.
    if args.list_devices {
        return devices::list();
    }
    if args.devices_json {
        println!("{}", devices::snapshot_json()?);
        return Ok(());
    }
    if args.meter {
        return live::meter(&make_live_config(&args, "es_en", None));
    }
    if args.test_speaker {
        return live::test_speaker(&make_live_config(&args, "es_en", None));
    }

    let hub = std::sync::Arc::new(control::Hub::new());
    let sock_path = args
        .control_socket
        .clone()
        .unwrap_or_else(control::default_socket_path);
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<control::CmdReq>();
    hub.clone().start_server(sock_path.clone(), cmd_tx.clone());
    let _runtime_files = RuntimeFiles {
        socket: sock_path.clone(),
        pid: sock_path.with_extension("pid"),
    };
    let _ = std::fs::write(
        sock_path.with_extension("pid"),
        std::process::id().to_string(),
    );
    let cmd_hub = hub.clone();
    std::thread::spawn(move || control::run_commands(cmd_rx, cmd_hub));

    let mode = if args.meet {
        Some("meet".to_string())
    } else if args.live {
        Some("live".to_string())
    } else {
        None
    };
    hub.set_mode(mode);
    if args.live || args.meet {
        hub.set_state(control::State::Starting);
        hub.log("starting Duet: loading models...");
    }

    std::fs::create_dir_all(&args.out_dir).ok();
    if !args.verbose {
        asr::silence_logs();
    }

    let mt_dir = if args.source_lang == "es" {
        "es_en"
    } else {
        "en_es"
    };
    let mt_dir_rev = if args.source_lang == "es" {
        "en_es"
    } else {
        "es_en"
    };

    println!("-- loading models (once) ---------------------------------");
    let use_gpu = match args.gpu {
        GpuArg::On => {
            if !cfg!(feature = "vulkan") {
                anyhow::bail!(
                    "this binary was not built with Vulkan; rebuild with --features vulkan"
                );
            }
            true
        }
        GpuArg::Off => false,
        GpuArg::Auto => cfg!(feature = "vulkan") && asr::gpu_recommended(&args.model),
    };
    let cfg = asr::AsrConfig {
        threads: args.threads,
        audio_ctx: args.audio_ctx,
        initial_prompt: args.prompt.clone(),
        temperature_inc: args.temp_inc,
        use_gpu,
    };
    let t = Instant::now();
    let asr = asr::Asr::load(&args.model, cfg)
        .with_context(|| format!("loading ASR {}", args.model.display()))?;
    println!(
        "  ASR   ({})   {}  (model n_audio_ctx: {})  [{}]",
        args.model.display(),
        fmt_ms(t.elapsed().as_secs_f64() * 1e3),
        asr.model_audio_ctx(),
        if use_gpu { "GPU Vulkan" } else { "CPU" }
    );

    let t = Instant::now();
    let mut engine = mt::Mt::spawn(&args.python, &args.mt_script, args.mt_threads)?;
    let mt_ready_ms = t.elapsed().as_secs_f64() * 1e3;

    let t = Instant::now();
    let mut voice = tts::Tts::spawn(&args.python, &args.tts_script)?;
    let tts_ready_ms = t.elapsed().as_secs_f64() * 1e3;

    // Spawn returns immediately; wait for the sidecars to finish loading by
    // making a trivial request (this measures the real load time).
    let t = Instant::now();
    let _ = engine.translate(mt_dir, "hola")?;
    let mt_load_ms = mt_ready_ms + t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let _ = voice.synth(&args.source_lang, "ok", &args.out_dir.join("_warmup.wav"))?;
    let tts_load_ms = tts_ready_ms + t.elapsed().as_secs_f64() * 1e3;

    println!("  MT    (ctranslate2 sidecar)  {}", fmt_ms(mt_load_ms));
    println!("  TTS   (piper sidecar)        {}", fmt_ms(tts_load_ms));

    let self_rss = peak_rss_kb(std::process::id());
    println!(
        "  Peak idle RSS: daemon {:?} MB | MT {:?} MB | TTS {:?} MB",
        self_rss.map(|k| k / 1024),
        peak_rss_kb(engine.pid()).map(|k| k / 1024),
        peak_rss_kb(voice.pid()).map(|k| k / 1024)
    );

    if args.check_ram {
        println!("\n(--check-ram: not processing audio)");
        return Ok(());
    }

    // The panel may have requested stop while the models were loading (~11 s).
    if hub.stop_requested() {
        hub.log("startup cancelled before opening the devices");
        hub.set_state(control::State::Idle);
        return Ok(());
    }

    if args.live {
        let live_cfg = make_live_config(&args, mt_dir, Some(hub.clone()));
        println!();
        let result = live::run(&live_cfg, &asr, &mut engine, &mut voice);
        hub.set_state(control::State::Idle);
        return result;
    }

    if args.meet {
        let live_cfg = make_live_config(&args, mt_dir, Some(hub.clone()));
        let meet_cfg = meet::MeetConfig {
            mic_device: args.mic_device.clone(),
            buds_device: args.speaker_device.clone(),
            user_lang: args.source_lang.clone(),
            peer_lang: args.target_lang.clone(),
            pause_ms: args.pause_ms,
            min_speech_ms: live_cfg.min_speech_ms,
            max_turns: live_cfg.max_turns,
            out_dir: args.out_dir.clone(),
            vad: live_cfg.vad.clone(),
            vad_start: live_cfg.vad_start,
            vad_end: live_cfg.vad_end,
            vad_onset_ms: live_cfg.vad_onset_ms,
            warmup_ms: live_cfg.warmup_ms,
            sim_mic: args.sim_mic.clone(),
            sim_meet: args.sim_meet.clone(),
            hub: Some(hub.clone()),
        };
        println!();
        let result = meet::run(&meet_cfg, &asr, &mut engine, &mut voice);
        hub.set_state(control::State::Idle);
        return result;
    }

    let in_path = args
        .input
        .clone()
        .context("missing --input (or use --live for live mode)")?;
    let samples = audio::read_wav_16k_mono(&in_path)?;
    let duration_s = samples.len() as f64 / 16_000.0;
    let vad_cfg = vad::VadConfig {
        pause_ms: args.pause_ms,
        ..Default::default()
    };
    let segments = vad::segment(&samples, 16_000, &vad_cfg);

    println!(
        "\n-- input audio ------------------------------------------\n  {} ({:.2} s)",
        in_path.display(),
        duration_s
    );
    println!(
        "  VAD: {} turns -> {}",
        segments.len(),
        segments
            .iter()
            .map(|s| format!("{:.1}s", (s.end - s.start) as f64 / 16_000.0))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let mut per_pass: Vec<Vec<TurnResult>> = Vec::new();
    let mut transcripts: Vec<(String, String)> = Vec::new();

    for pass in 0..args.passes {
        let mut results = Vec::new();
        println!(
            "\n-- pass {} ----------------------------------------------",
            pass + 1
        );
        for (i, seg) in segments.iter().enumerate() {
            let chunk = &samples[seg.start..seg.end];
            let src_lang = &args.source_lang;
            let dst_lang = &args.target_lang;

            let t = Instant::now();
            let text = asr.transcribe(chunk, src_lang)?;
            let asr_ms = t.elapsed().as_secs_f64() * 1e3;

            if text.is_empty() {
                println!("  turn {}: (empty segment, skipped)", i + 1);
                continue;
            }

            let t = Instant::now();
            let translated = engine.translate(mt_dir, &text)?;
            let mt_ms = t.elapsed().as_secs_f64() * 1e3;

            let out_wav = args
                .out_dir
                .join(format!("tts_p{}_t{}.wav", pass + 1, i + 1));
            let t = Instant::now();
            let syn = voice.synth(dst_lang, &translated, &out_wav)?;
            let tts_ms = t.elapsed().as_secs_f64() * 1e3;

            // The original audio in the target language also has to be
            // generated (person A hears their own language when needed); here
            // we only record the main direction.
            if pass == 0 {
                transcripts.push((text.clone(), translated.clone()));
            }

            let audio_s = chunk.len() as f64 / 16_000.0;
            println!(
                "  turn {} ({:.1}s): ASR {} | MT {} | TTS {} (synthesis {:.0} ms) -> {:.2}s audio",
                i + 1,
                audio_s,
                fmt_ms(asr_ms),
                fmt_ms(mt_ms),
                fmt_ms(tts_ms),
                syn.ms,
                syn.audio_s
            );
            println!("      [{}] {}", src_lang, text);
            println!("      [{}] {}", dst_lang, translated);

            results.push(TurnResult {
                asr_ms,
                mt_ms,
                tts_ms,
                audio_s,
            });
        }

        let n = results.len().max(1) as f64;
        let sum = |f: fn(&TurnResult) -> f64| results.iter().map(f).sum::<f64>();
        println!(
            "  -> pass {}: average turn ASR {:.0} | MT {:.0} | TTS {:.0} | TOTAL {:.0} ms",
            pass + 1,
            sum(|r| r.asr_ms) / n,
            sum(|r| r.mt_ms) / n,
            sum(|r| r.tts_ms) / n,
            sum(|r| r.asr_ms + r.mt_ms + r.tts_ms) / n
        );
        per_pass.push(results);
    }

    // Reverse direction (the other person's audio -> MT -> TTS).
    let probe = if args.source_lang == "es" {
        "Sure, I will review those numbers and send you an update tomorrow."
    } else {
        "Claro, voy a revisar esos numeros y te mando una actualizacion manana."
    };
    let t = Instant::now();
    let back = engine.translate(mt_dir_rev, probe)?;
    let rev_mt_ms = t.elapsed().as_secs_f64() * 1e3;
    let out_wav = args.out_dir.join("tts_reverse.wav");
    let t = Instant::now();
    let syn = voice.synth(&args.source_lang, &back, &out_wav)?;
    let rev_tts_ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "\n-- reverse direction ------------------------------------\n  MT  {}  {}\n  TTS {}  -> {} ({:.2}s audio)",
        fmt_ms(rev_mt_ms),
        back,
        fmt_ms(rev_tts_ms),
        out_wav.display(),
        syn.audio_s
    );

    // Summary: use passes 2..N as the steady state.
    let steady: Vec<&TurnResult> = if per_pass.len() > 1 {
        per_pass[1..].iter().flatten().collect()
    } else {
        per_pass[0].iter().collect()
    };
    let n = steady.len().max(1) as f64;
    let asr_avg = steady.iter().map(|r| r.asr_ms).sum::<f64>() / n;
    let mt_avg = steady.iter().map(|r| r.mt_ms).sum::<f64>() / n;
    let tts_avg = steady.iter().map(|r| r.tts_ms).sum::<f64>() / n;
    let total_avg = asr_avg + mt_avg + tts_avg;
    let audio_avg = steady.iter().map(|r| r.audio_s).sum::<f64>() / n;

    println!("\n-- summary (steady state) -------------------------------");
    println!("  average audio per turn            {audio_avg:.2} s");
    println!("  average ASR                       {}", fmt_ms(asr_avg));
    println!("  average MT                        {}", fmt_ms(mt_avg));
    println!("  average TTS                       {}", fmt_ms(tts_avg));
    println!("  --------------------------------------------");
    println!(
        "  COMPUTE LATENCY per turn          {}   (+{:.0} ms VAD pause wait)",
        fmt_ms(total_avg),
        args.pause_ms as f64
    );
    println!(
        "  PERCEIVED LATENCY per turn        {}",
        fmt_ms(total_avg + args.pause_ms as f64)
    );
    println!(
        "  Compute RTF (time/audio)          {:.2}x",
        total_avg / 1000.0 / audio_avg.max(0.001)
    );
    println!(
        "  Peak RSS: daemon {:?} MB | MT {:?} MB | TTS {:?} MB",
        peak_rss_kb(std::process::id()).map(|k| k / 1024),
        peak_rss_kb(engine.pid()).map(|k| k / 1024),
        peak_rss_kb(voice.pid()).map(|k| k / 1024)
    );
    let (d, m, t) = (
        cur_rss_kb(std::process::id()).unwrap_or(0),
        cur_rss_kb(engine.pid()).unwrap_or(0),
        cur_rss_kb(voice.pid()).unwrap_or(0),
    );
    println!(
        "  Steady RSS (current): daemon {} MB | MT {} MB | TTS {} MB | TOTAL {} MB",
        d / 1024,
        m / 1024,
        t / 1024,
        (d + m + t) / 1024
    );

    Ok(())
}
