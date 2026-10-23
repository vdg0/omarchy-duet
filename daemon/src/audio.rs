//! WAV reading and audio utilities.

use anyhow::{Context, Result};
use hound::{SampleFormat, WavReader};
use std::path::Path;

/// Read a WAV file and return it as mono f32 PCM at its original sample rate.
pub fn read_wav_mono(path: &Path) -> Result<(Vec<f32>, u32)> {
    let mut reader =
        WavReader::open(path).with_context(|| format!("could not open {}", path.display()))?;
    let spec = reader.spec();

    let interleaved: Vec<f32> = match spec.sample_format {
        SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("corrupt float WAV")?,
        SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max))
                .collect::<std::result::Result<Vec<_>, _>>()
                .context("corrupt integer WAV")?
        }
    };

    // Downmix to mono.
    let mut mono = Vec::with_capacity(interleaved.len() / spec.channels as usize);
    if spec.channels <= 1 {
        mono = interleaved;
    } else {
        let ch = spec.channels as usize;
        for frame in interleaved.chunks_exact(ch) {
            mono.push(frame.iter().sum::<f32>() / ch as f32);
        }
    }

    Ok((mono, spec.sample_rate))
}

/// Read a WAV file and return it as mono f32 PCM at 16 kHz (what Whisper expects).
pub fn read_wav_16k_mono(path: &Path) -> Result<Vec<f32>> {
    let (mono, rate) = read_wav_mono(path)?;
    // Resample to 16 kHz (linear interpolation is enough for this pipeline).
    if rate == 16_000 {
        Ok(mono)
    } else {
        Ok(resample_linear(&mono, rate, 16_000))
    }
}

pub fn resample_linear(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = to as f64 / from as f64;
    let out_len = ((input.len() as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f64 / ratio;
        let i0 = src.floor() as usize;
        let i1 = (i0 + 1).min(input.len() - 1);
        let frac = (src - i0 as f64) as f32;
        out.push(input[i0] * (1.0 - frac) + input[i1] * frac);
    }
    out
}

/// Write mono f32 PCM as a 16-bit WAV (utility for dumping audio from Rust).
#[allow(dead_code)]
pub fn write_wav(path: &Path, samples: &[f32], sample_rate: u32) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer.write_sample(v)?;
    }
    writer.finalize()?;
    Ok(())
}
