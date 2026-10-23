//! Streaming VAD for live mode.
//!
//! History of this decision, because it was not straightforward:
//!
//! 1. **Energy** (RMS with an adaptive threshold): cheap, but on this laptop's
//!    real microphone the noise floor is around -30 dBFS. With a relative
//!    threshold (`floor x 3`) you have to speak much louder than the room to
//!    trigger, and if you also speak far from the microphone (wearing headphones)
//!    it detects nothing.
//! 2. **Silero** (whisper.cpp's VAD): accurate, but **not viable here**. Measured
//!    on this i5-5257U: 1600 samples -> 127 ms, 16000 -> 436 ms. whisper.cpp
//!    rebuilds the ggml graph on every call, so there is ~90 ms of fixed
//!    overhead. It runs slower than real time when called frequently. On a fast
//!    machine yes, here no. It remains available with `--vad silero`.
//! 3. **WebRTC VAD** (the one we use): a small GMM with **state** across frames,
//!    made for streaming. It processes 10/20/30 ms frames and is essentially
//!    negligible on CPU. Robust to background noise, which is exactly the
//!    problem here.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use webrtc_vad::{SampleRate, Vad, VadMode};
use whisper_rs::{WhisperVadContext, WhisperVadContextParams};

const SR: usize = 16_000;
/// Silero internal window at 16 kHz (32 ms).
const SILERO_WINDOW: usize = 512;
/// WebRTC VAD frame at 16 kHz (10 ms).
const WEBRTC_FRAME: usize = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum VadKind {
    /// WebRTC if compiled (always), otherwise energy.
    Auto,
    /// WebRTC VAD (recommended on this machine).
    Webrtc,
    /// Silero via whisper.cpp (more accurate, much heavier).
    Silero,
    /// Energy threshold (last resort).
    Energy,
}

pub struct WebrtcVad {
    vad: Vad,
}

impl WebrtcVad {
    pub fn new(mode: u8) -> Self {
        let mode = match mode {
            0 => VadMode::Quality,
            1 => VadMode::LowBitrate,
            2 => VadMode::Aggressive,
            _ => VadMode::VeryAggressive,
        };
        Self {
            vad: Vad::new_with_rate_and_mode(SampleRate::Rate16kHz, mode),
        }
    }

    /// Fraction of 10 ms frames with speech (0..1). WebRTC VAD is stateful, so it
    /// must be fed frame by frame and in order.
    fn prob(&mut self, samples: &[f32]) -> f32 {
        let mut voice = 0usize;
        let mut total = 0usize;
        let mut buf = [0i16; WEBRTC_FRAME];
        for chunk in samples.chunks(WEBRTC_FRAME) {
            if chunk.len() < WEBRTC_FRAME {
                break; // discard the partial remainder (< 10 ms)
            }
            for (i, &s) in chunk.iter().enumerate() {
                buf[i] = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
            }
            total += 1;
            if matches!(self.vad.is_voice_segment(&buf), Ok(true)) {
                voice += 1;
            }
        }
        if total == 0 {
            0.0
        } else {
            voice as f32 / total as f32
        }
    }
}

/// Fallback energy VAD. **Bounded** adaptive noise floor (without a cap the
/// threshold drifted upward and detected nothing).
pub struct EnergyVad {
    noise_floor: f32,
    min_threshold: f32,
    max_threshold: f32,
}

impl EnergyVad {
    pub fn new(min_threshold: f32) -> Self {
        Self {
            noise_floor: 0.01,
            min_threshold,
            max_threshold: 0.05,
        }
    }

    fn prob(&mut self, samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
        let threshold = (self.noise_floor * 3.0).clamp(self.min_threshold, self.max_threshold);
        if rms > threshold {
            1.0
        } else {
            self.noise_floor = 0.95 * self.noise_floor + 0.05 * rms;
            0.0
        }
    }
}

pub struct SileroVad {
    ctx: WhisperVadContext,
    window: Vec<f32>,
    window_samples: usize,
    lookback: usize,
    failed: bool,
}

impl SileroVad {
    pub fn load(model: &Path, threads: i32, window_ms: u32, lookback_ms: u32) -> Result<Self> {
        let mut cp = WhisperVadContextParams::default();
        cp.set_n_threads(threads);
        let path = model.to_string_lossy().to_string();
        let ctx = WhisperVadContext::new(&path, cp)
            .with_context(|| format!("could not load VAD {}", model.display()))?;
        Ok(Self {
            ctx,
            window: Vec::new(),
            window_samples: (window_ms as usize * SR) / 1000,
            lookback: ((lookback_ms as usize * SR) / 1000 / SILERO_WINDOW).max(1),
            failed: false,
        })
    }

    fn prob(&mut self, samples: &[f32]) -> f32 {
        self.window.extend_from_slice(samples);
        if self.window.len() > self.window_samples {
            let drop = self.window.len() - self.window_samples;
            self.window.drain(..drop);
        }
        if self.window.len() < SILERO_WINDOW {
            return 0.0;
        }
        if let Err(e) = self.ctx.detect_speech(&self.window) {
            if !self.failed {
                self.failed = true;
                eprintln!("[vad] Silero error: {e}");
            }
            return 0.0;
        }
        let probs = self.ctx.probabilities();
        if probs.is_empty() {
            return 0.0;
        }
        let start = probs.len().saturating_sub(self.lookback);
        probs[start..].iter().copied().fold(0.0f32, f32::max)
    }
}

#[derive(Clone, Debug)]
pub struct VadOpts {
    pub kind: VadKind,
    pub model: Option<PathBuf>,
    pub threads: i32,
    pub window_ms: u32,
    pub lookback_ms: u32,
    pub energy_threshold: f32,
    pub webrtc_mode: u8,
}

pub enum StreamVad {
    Webrtc(WebrtcVad),
    Silero(SileroVad),
    Energy(EnergyVad),
}

impl StreamVad {
    pub fn new(opts: &VadOpts) -> Result<(Self, &'static str)> {
        let silero_ok = opts.model.as_deref().is_some_and(|p| p.exists());
        match opts.kind {
            VadKind::Webrtc => Ok((Self::Webrtc(WebrtcVad::new(opts.webrtc_mode)), "WebRTC")),
            VadKind::Silero => {
                let path = opts
                    .model
                    .as_deref()
                    .context("--vad silero requires --vad-model pointing to the Silero model")?;
                let v = SileroVad::load(path, opts.threads, opts.window_ms, opts.lookback_ms)?;
                Ok((Self::Silero(v), "Silero"))
            }
            VadKind::Energy => Ok((
                Self::Energy(EnergyVad::new(opts.energy_threshold)),
                "energy",
            )),
            VadKind::Auto => {
                // WebRTC is always compiled; Silero is only selected manually because it is
                // so heavy, so "auto" means WebRTC.
                let _ = silero_ok;
                Ok((Self::Webrtc(WebrtcVad::new(opts.webrtc_mode)), "WebRTC"))
            }
        }
    }

    pub fn prob(&mut self, samples: &[f32]) -> f32 {
        match self {
            Self::Webrtc(v) => v.prob(samples),
            Self::Silero(v) => v.prob(samples),
            Self::Energy(v) => v.prob(samples),
        }
    }
}
