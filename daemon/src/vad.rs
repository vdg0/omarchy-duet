//! Energy-based VAD with hysteresis.
//!
//! This is more than enough for the offline benchmark path: transparent,
//! dependency-free, and it lets us explicitly control the parameter that
//! dominates latency in turn-based mode: how much pause we wait for before
//! "closing" the segment.
//!
//! In production it is better to replace it with Silero VAD (which whisper-rs
//! exposes through `WhisperVadContext`), but the segmentation logic is the same.

pub struct VadConfig {
    /// Frame size in ms.
    pub frame_ms: u32,
    /// Silence pause (ms) that closes a phrase.
    pub pause_ms: u32,
    /// Minimum speech duration (ms) so it is not discarded as noise.
    pub min_speech_ms: u32,
    /// Padding (ms) added on each side of the segment.
    pub pad_ms: u32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            frame_ms: 30,
            pause_ms: 700,
            min_speech_ms: 300,
            pad_ms: 120,
        }
    }
}

pub struct Segment {
    pub start: usize,
    pub end: usize,
}

const ABS_FLOOR: f32 = 0.006;

pub fn segment(samples: &[f32], sample_rate: u32, cfg: &VadConfig) -> Vec<Segment> {
    let frame = ((sample_rate as u64 * cfg.frame_ms as u64) / 1000) as usize;
    if frame == 0 || samples.len() < frame {
        return Vec::new();
    }

    let rms: Vec<f32> = samples
        .chunks(frame)
        .map(|c| (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt())
        .collect();

    // Adaptive threshold: 10th percentile (noise floor) * 3, with an absolute floor.
    let mut sorted = rms.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let noise_floor = sorted[sorted.len() / 10];
    let threshold = (noise_floor * 3.0).max(ABS_FLOOR);

    let frames_per_pause = (cfg.pause_ms / cfg.frame_ms).max(1) as usize;
    let frames_per_min_speech = (cfg.min_speech_ms / cfg.frame_ms).max(1) as usize;
    let pad = ((sample_rate as u64 * cfg.pad_ms as u64) / 1000) as usize;

    let mut segments = Vec::new();
    let mut in_speech = false;
    let mut start_frame = 0usize;
    let mut silence_run = 0usize;

    for (i, &energy) in rms.iter().enumerate() {
        let loud = energy > threshold;
        if !in_speech {
            if loud {
                in_speech = true;
                start_frame = i;
                silence_run = 0;
            }
        } else if loud {
            silence_run = 0;
        } else {
            silence_run += 1;
            if silence_run >= frames_per_pause {
                let end_frame = i.saturating_sub(silence_run) + 1;
                let speech_frames = end_frame.saturating_sub(start_frame);
                let start = start_frame * frame;
                let end = (end_frame * frame).min(samples.len());
                if speech_frames >= frames_per_min_speech {
                    segments.push(Segment {
                        start: start.saturating_sub(pad),
                        end: (end + pad).min(samples.len()),
                    });
                }
                in_speech = false;
                silence_run = 0;
            }
        }
    }

    if in_speech {
        let end = samples.len();
        let start = start_frame * frame;
        if (end - start) / frame >= frames_per_min_speech {
            segments.push(Segment {
                start: start.saturating_sub(pad),
                end,
            });
        }
    }

    segments
}
