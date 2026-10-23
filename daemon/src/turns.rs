//! Streaming turn detection: collects audio chunks and decides when a person
//! has finished speaking.
//!
//! It is used the same way in live and Meet modes: feed 100 ms chunks and, when
//! the VAD sees a sufficiently long pause, it returns the complete phrase (with
//! a little *preroll* so the beginning is not cut off).
//!
//! The state machine is deliberately the same in both modes so that what we
//! validate with the meter and files still holds in Meet.

use crate::pwio::SAMPLE_RATE;
use crate::vad_stream::{StreamVad, VadOpts};
use anyhow::Result;
use std::time::Instant;

/// A closed phrase, ready for ASR.
pub struct Turn {
    pub pcm: Vec<f32>,
    pub speech_s: f64,
    /// When the last chunk with speech arrived (for measuring real latency).
    pub last_speech_at: Option<Instant>,
}

pub struct TurnDetector {
    vad: StreamVad,
    start: f32,
    end: f32,
    onset_chunks: usize,
    pause_ms: u32,
    min_speech_ms: u32,
    preroll_max: usize,
    warmup_chunks: usize,
    // -- state --
    preroll: Vec<f32>,
    utterance: Vec<f32>,
    in_speech: bool,
    silence_ms: u32,
    speech_ms: u32,
    onset_run: usize,
    last_speech_at: Option<Instant>,
}

/// Detector parameters (the same in both modes).
pub struct TurnOpts {
    pub vad: VadOpts,
    pub vad_start: f32,
    pub vad_end: f32,
    pub vad_onset_ms: u32,
    pub pause_ms: u32,
    pub min_speech_ms: u32,
    pub warmup_ms: u32,
}

impl TurnDetector {
    pub fn new(opts: &TurnOpts) -> Result<(Self, &'static str)> {
        let (vad, kind) = StreamVad::new(&opts.vad)?;
        Ok((
            Self {
                vad,
                start: opts.vad_start,
                end: opts.vad_end,
                onset_chunks: (opts.vad_onset_ms / 100).max(1) as usize,
                pause_ms: opts.pause_ms,
                min_speech_ms: opts.min_speech_ms,
                preroll_max: (0.3 * SAMPLE_RATE as f32) as usize,
                warmup_chunks: (opts.warmup_ms / 100) as usize,
                preroll: Vec::new(),
                utterance: Vec::new(),
                in_speech: false,
                silence_ms: 0,
                speech_ms: 0,
                onset_run: 0,
                last_speech_at: None,
            },
            kind,
        ))
    }

    /// Process a 100 ms chunk. Returns the phrase when a turn closes.
    pub fn feed(&mut self, chunk: &[f32]) -> Option<Turn> {
        if self.warmup_chunks > 0 {
            // The first half second contains the capture-open transient.
            self.warmup_chunks -= 1;
            return None;
        }
        let chunk_ms = (chunk.len() as u64 * 1000 / SAMPLE_RATE as u64) as u32;
        let prob = self.vad.prob(chunk);

        if !self.in_speech {
            // Always keep recent audio: it is the preroll and also contains the
            // chunks that confirm speech onset.
            self.preroll.extend_from_slice(chunk);
            if self.preroll.len() > self.preroll_max {
                let drop = self.preroll.len() - self.preroll_max;
                self.preroll.drain(..drop);
            }
            self.onset_run = if prob >= self.start {
                self.onset_run + 1
            } else {
                0
            };
            if self.onset_run >= self.onset_chunks {
                self.in_speech = true;
                self.utterance.clear();
                self.utterance.extend_from_slice(&self.preroll);
                self.preroll.clear();
                self.speech_ms = self.onset_run as u32 * chunk_ms;
                self.silence_ms = 0;
                self.onset_run = 0;
                self.last_speech_at = Some(Instant::now());
            }
            return None;
        }

        self.utterance.extend_from_slice(chunk);
        if prob < self.end {
            self.silence_ms += chunk_ms;
        } else {
            self.silence_ms = 0;
            self.speech_ms += chunk_ms;
            self.last_speech_at = Some(Instant::now());
        }

        // Cut on pause, or force it if the phrase becomes very long (Whisper
        // works in 30 s windows).
        let long = self.utterance.len() > (29 * SAMPLE_RATE as usize);
        if self.silence_ms < self.pause_ms && !long {
            return None;
        }

        // -- a turn closed --
        self.in_speech = false;
        self.silence_ms = 0;
        let speech_s = self.speech_ms as f64 / 1000.0;
        let enough = self.speech_ms >= self.min_speech_ms;
        self.speech_ms = 0;
        let mut pcm = std::mem::take(&mut self.utterance);

        if !enough {
            return None;
        }

        // Trim excess trailing silence (keep ~150 ms of tail).
        if !long {
            let keep = (0.15 * SAMPLE_RATE as f32) as usize;
            let tail = (self.pause_ms as usize * SAMPLE_RATE as usize / 1000).saturating_sub(keep);
            if pcm.len() > tail {
                pcm.truncate(pcm.len() - tail);
            }
        }

        Some(Turn {
            pcm,
            speech_s,
            last_speech_at: self.last_speech_at,
        })
    }

    /// Forget accumulated state. Called after playback so that the audio we hear
    /// (or echo) does not contaminate the next turn.
    pub fn clear(&mut self) {
        self.preroll.clear();
        self.utterance.clear();
        self.in_speech = false;
        self.silence_ms = 0;
        self.speech_ms = 0;
        self.onset_run = 0;
    }

    /// Force-close an in-progress phrase. Used when the audio source ends without
    /// the final pause (e.g. a test WAV that ends abruptly, or a `pw-record` that
    /// is cut off): we prefer to translate what we managed to hear rather than
    /// discard it.
    pub fn flush(&mut self) -> Option<Turn> {
        if !self.in_speech {
            return None;
        }
        self.in_speech = false;
        self.silence_ms = 0;
        self.onset_run = 0;
        let speech_s = self.speech_ms as f64 / 1000.0;
        let enough = self.speech_ms >= self.min_speech_ms;
        self.speech_ms = 0;
        let pcm = std::mem::take(&mut self.utterance);
        if !enough || pcm.is_empty() {
            return None;
        }
        Some(Turn {
            pcm,
            speech_s,
            last_speech_at: self.last_speech_at,
        })
    }
}
