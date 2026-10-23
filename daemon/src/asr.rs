//! ASR with whisper.cpp (through whisper-rs). The model is loaded once and the
//! state is recreated per segment (whisper-rs does not allow state reuse safely
//! across calls), but the weights and graph stay resident.
//!
//! Key note: Whisper ALWAYS processes a fixed 30 s window. For short segments
//! (2-5 s) that is ~6-15x wasted work. `audio_ctx` shrinks the encoder window
//! to the real segment length and is the big latency optimization in turn-based
//! mode.

use anyhow::{Context, Result};
use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct AsrConfig {
    pub threads: i32,
    /// Encoder window in tokens (0 = model default, ~1500 = 30 s).
    pub audio_ctx: i32,
    /// Initial prompt (helps with punctuation, accents, and jargon).
    pub initial_prompt: Option<String>,
    /// Temperature increment for the anti-loop fallback. 0 = no fallback
    /// (one pass, faster but with a risk of repetitions).
    pub temperature_inc: f32,
    /// Ask whisper.cpp to use the GPU (only effective if the binary was built
    /// with the `vulkan` feature).
    pub use_gpu: bool,
}

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            threads: 4,
            audio_ctx: 0,
            initial_prompt: None,
            temperature_inc: 0.2,
            use_gpu: false,
        }
    }
}

/// Models below this size run as fast or faster on the CPU.
/// On this machine: base q5_1 (57 MB) and base f16 (142 MB) stay on the CPU;
/// small/medium/large (182 MB+) benefit from the Vulkan backend.
const GPU_MIN_MODEL_BYTES: u64 = 150 * 1024 * 1024;

/// Heuristic for `--gpu auto`: use the GPU only when the model is large enough
/// to offset the backend initialization cost.
pub fn gpu_recommended(model_path: &Path) -> bool {
    std::fs::metadata(model_path)
        .map(|m| m.len() >= GPU_MIN_MODEL_BYTES)
        .unwrap_or(false)
}

/// Convert audio seconds to an encoder window with headroom.
///
/// The model `n_audio_ctx` corresponds to 30 s (1500 tokens at 50/s). We give it
/// 2x the segment length as headroom and clamp between 256 and the default.
pub fn audio_ctx_for(seg_seconds: f64, model_default: i32) -> i32 {
    let needed = (seg_seconds * 2.0 * 50.0).round() as i32;
    needed.clamp(256, model_default.max(256))
}

/// Silence the whisper.cpp/ggml log (which is noisy on stderr on every state
/// creation). It is process-global.
pub fn silence_logs() {
    unsafe extern "C" fn noop(
        _level: std::os::raw::c_uint,
        _text: *const std::os::raw::c_char,
        _user_data: *mut std::ffi::c_void,
    ) {
    }
    unsafe {
        whisper_rs::set_log_callback(Some(noop), std::ptr::null_mut());
    }
}

pub struct Asr {
    ctx: WhisperContext,
    cfg: AsrConfig,
}

impl Asr {
    pub fn load(model_path: &Path, cfg: AsrConfig) -> Result<Self> {
        let mut params = WhisperContextParameters::default();
        params.use_gpu(cfg.use_gpu);
        let ctx = WhisperContext::new_with_params(model_path, params)
            .with_context(|| format!("could not load model {}", model_path.display()))?;
        Ok(Self { ctx, cfg })
    }

    pub fn model_audio_ctx(&self) -> i32 {
        self.ctx.model_n_audio_ctx()
    }

    /// Transcribe one segment (mono f32 PCM at 16 kHz).
    pub fn transcribe(&self, pcm: &[f32], lang: &str) -> Result<String> {
        let mut state = self.ctx.create_state()?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(lang));
        params.set_translate(false);
        params.set_no_context(true);
        // One VAD segment = one phrase: do not look for multiple segments.
        params.set_single_segment(true);
        params.set_no_timestamps(true);
        params.set_n_threads(self.cfg.threads);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        // Anti-loop: without temperature fallback, Whisper (especially tiny) can
        // repeat the same phrase until the decoding window is exhausted.
        // Note: each fallback retry runs the decoder again (and sometimes the
        // encoder), which greatly increases tail latency: with 0.2 you can see
        // ~6 s turns on real audio. With 0 there is a single pass (faster) at the
        // cost of a higher risk of repetitions.
        params.set_temperature_inc(self.cfg.temperature_inc);
        params.set_entropy_thold(2.4);
        params.set_suppress_blank(true);
        // Token cap proportional to segment length (an extra safety net against
        // repetitions without truncating real phrases).
        let seg_s_guard = pcm.len() as f64 / 16_000.0;
        params.set_max_tokens(((seg_s_guard * 15.0) as i32 + 48).min(224));

        let model_default = self.ctx.model_n_audio_ctx();
        let audio_ctx = if self.cfg.audio_ctx > 0 {
            self.cfg.audio_ctx
        } else {
            let seg_s = pcm.len() as f64 / 16_000.0;
            audio_ctx_for(seg_s, model_default)
        };
        params.set_audio_ctx(audio_ctx);

        if let Some(prompt) = &self.cfg.initial_prompt {
            params.set_initial_prompt(prompt);
        }

        state.full(params, pcm)?;

        let mut out = String::new();
        for segment in state.as_iter() {
            out.push_str(&format!("{segment}"));
        }
        Ok(out.trim().to_string())
    }
}
