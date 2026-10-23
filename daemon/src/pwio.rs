//! PipeWire audio input/output, shared by live mode and Meet mode.
//!
//! All audio goes through the native PipeWire tools (`pw-record` and `pw-play`)
//! at 16 kHz mono f32: it is the same path we need to route the browser, so we
//! do not add ALSA/cpal dependencies.

use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

pub const SAMPLE_RATE: u32 = 16_000;
/// Native sample rate of the Piper voices used by the project.
pub const TTS_RATE: u32 = 22_050;
/// Audio chunks delivered by the reader thread to the main loop.
pub const CHUNK_SAMPLES: usize = 1_600; // 100 ms

/// Streaming audio source: always exposes a channel of 16 kHz f32 chunks.
pub struct Source {
    child: Option<Child>,
    pub rx: Receiver<Vec<f32>>,
}

impl Source {
    /// Microphone (or any Source node): `pw-record` emitting raw f32 PCM.
    pub fn mic(device: Option<&str>) -> Result<Self> {
        let mut cmd = Command::new("pw-record");
        cmd.args([
            "--rate",
            &SAMPLE_RATE.to_string(),
            "--channels",
            "1",
            "--format",
            "f32",
            // Without `--raw`, pw-record chooses a container (AU by default when
            // writing to stdout) and the reader interprets the header as PCM.
            "--raw",
        ]);
        if let Some(d) = device {
            cmd.args(["--target", d]);
        }
        cmd.arg("-")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        let mut child = cmd.spawn().context("could not launch pw-record")?;
        let stdout = child.stdout.take().context("pw-record has no stdout")?;
        let (tx, rx) = channel();
        thread::spawn(move || read_pcm_stream(stdout, tx));
        Ok(Self {
            child: Some(child),
            rx,
        })
    }

    /// Sink monitor through the PulseAudio compatibility layer (`parec`). Native
    /// PipeWire does not expose monitors as nodes, and `pw-record --target
    /// foo.monitor` records silence; PulseAudio resolves them and delivers raw f32
    /// PCM without intermediate conversions.
    pub fn pulse_monitor(device: &str) -> Result<Self> {
        let mut cmd = Command::new("parec");
        cmd.arg(format!("--device={device}"))
            .arg(format!("--rate={SAMPLE_RATE}"))
            .arg("--channels=1")
            .arg("--format=float32le")
            .arg("--latency-msec=50")
            .arg("--raw")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        let mut child = cmd.spawn().context("could not launch parec")?;
        let stdout = child.stdout.take().context("parec has no stdout")?;
        let (tx, rx) = channel();
        thread::spawn(move || read_pcm_stream(stdout, tx));
        Ok(Self {
            child: Some(child),
            rx,
        })
    }

    /// File: split a WAV and deliver it at the same cadence as the microphone.
    pub fn wav(path: &PathBuf, realtime: bool) -> Result<Self> {
        let samples = crate::audio::read_wav_16k_mono(path)?;
        let (tx, rx) = channel();
        thread::spawn(move || {
            for chunk in samples.chunks(CHUNK_SAMPLES) {
                if tx.send(chunk.to_vec()).is_err() {
                    return;
                }
                if realtime {
                    thread::sleep(Duration::from_millis(
                        (chunk.len() as u64 * 1000) / SAMPLE_RATE as u64,
                    ));
                }
            }
        });
        Ok(Self { child: None, rx })
    }

    pub fn drain(&self) {
        while self.rx.try_recv().is_ok() {}
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Read raw f32 PCM from a stream and publish aligned chunks.
fn read_pcm_stream(mut stream: impl Read, tx: Sender<Vec<f32>>) {
    let mut pending: Vec<u8> = Vec::new();
    let mut samples: Vec<f32> = Vec::with_capacity(CHUNK_SAMPLES);
    let mut raw = [0u8; 8192];
    loop {
        let n = match stream.read(&mut raw) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        pending.extend_from_slice(&raw[..n]);
        let usable = pending.len() - (pending.len() % 4);
        for b in pending[..usable].chunks_exact(4) {
            samples.push(f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            if samples.len() == CHUNK_SAMPLES {
                if tx.send(std::mem::take(&mut samples)).is_err() {
                    return;
                }
                samples.reserve(CHUNK_SAMPLES);
            }
        }
        pending.drain(..usable);
    }
    if !samples.is_empty() {
        let _ = tx.send(samples);
    }
}

/// Play a WAV through the given destination (or the default one).
/// Return the start instant for latency measurements.
pub fn play_wav(path: &PathBuf, device: Option<&str>) -> Result<Instant> {
    let mut cmd = Command::new("pw-play");
    if let Some(d) = device {
        cmd.args(["--target", d]);
    }
    let started = Instant::now();
    let status = cmd
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("could not launch pw-play")?;
    if !status.success() {
        anyhow::bail!("pw-play failed with {status}");
    }
    Ok(started)
}

/// Write a WAV to the virtual microphone FIFO and wait for its duration.
///
/// `module-pipe-source` consumes it as mono s16le at `TTS_RATE`; if the WAV has
/// a different rate we resample it. Blocking is intentional: it keeps turn-based
/// mode while the translation "plays" on Meet's microphone.
pub fn play_wav_to_pipe(path: &PathBuf, fifo: &Path, audio_s: f64) -> Result<Instant> {
    let (samples, rate) = crate::audio::read_wav_mono(path)?;
    let samples = if rate == TTS_RATE {
        samples
    } else {
        crate::audio::resample_linear(&samples, rate, TTS_RATE)
    };

    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for &s in &samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        pcm.extend_from_slice(&v.to_le_bytes());
    }

    let started = Instant::now();
    let mut out = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(fifo)
        .with_context(|| format!("could not open FIFO {}", fifo.display()))?;

    // A FIFO with no active reader fills at 64 KB and `write` would block forever.
    // We shrink it and write with a timeout: if Meet is not capturing, discard the
    // rest instead of freezing the daemon.
    let fd = out.as_raw_fd();
    let _ = unsafe { libc::fcntl(fd, libc::F_SETPIPE_SZ, 4096) };

    let mut written = 0usize;
    while written < pcm.len() {
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLOUT,
            revents: 0,
        };
        let rc = unsafe { libc::poll(&mut pfd, 1, 200) };
        if rc == 0 {
            break; // nobody is consuming: Meet is not capturing the virtual microphone
        }
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if (pfd.revents & libc::POLLOUT) == 0 {
            break;
        }

        let chunk = (pcm.len() - written).min(4096);
        match out.write(&pcm[written..written + chunk]) {
            Ok(0) => break,
            Ok(n) => written += n,
            Err(e) if e.kind() == ErrorKind::WouldBlock => continue,
            Err(e) => {
                return Err(e).context("could not write audio to the microphone FIFO");
            }
        }
    }
    drop(out);

    if written < pcm.len() {
        let dropped_s = (pcm.len() - written) as f64 / (TTS_RATE as f64 * 2.0);
        println!("  (virtual microphone not captured: dropped {dropped_s:.1}s of audio)");
    }

    // The FIFO may swallow the whole clip immediately; wait for its duration anyway
    // so the next turn does not overlap. 100 ms of headroom covers the PipeWire
    // graph latency.
    let seconds = if audio_s > 0.0 {
        audio_s
    } else {
        samples.len() as f64 / TTS_RATE as f64
    };
    let target = Duration::from_secs_f64(seconds + 0.10);
    let elapsed = started.elapsed();
    if target > elapsed {
        thread::sleep(target - elapsed);
    }
    Ok(started)
}
