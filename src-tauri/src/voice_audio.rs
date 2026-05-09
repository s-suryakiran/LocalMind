//! Audio decoding into 16 kHz mono f32 PCM. Sherpa-onnx CLIs accept
//! input WAVs at any sample rate but the diarization models we ship
//! are trained on 16 kHz; resampling here keeps the rest of the
//! pipeline simple.

use anyhow::{anyhow, Context, Result};
use std::fs::File;
use std::path::Path;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Decode `path` into 16 kHz mono f32 samples. Stereo collapses to
/// mono by averaging channels; non-16k sample rates are linearly
/// resampled (simple but adequate for ASR-grade audio).
pub fn decode_to_16k_mono(path: &Path) -> Result<Vec<f32>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| anyhow!("no default audio track in {}", path.display()))?;
    let track_id = track.id;
    let codec_params = track.codec_params.clone();
    let mut decoder =
        symphonia::default::get_codecs().make(&codec_params, &DecoderOptions::default())?;
    let src_rate = codec_params
        .sample_rate
        .ok_or_else(|| anyhow!("unknown sample rate"))?;

    let mut samples: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::ResetRequired) => break,
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder.decode(&packet)?;
        append_mono_f32(&mut samples, decoded);
    }

    Ok(if src_rate == TARGET_SAMPLE_RATE {
        samples
    } else {
        linear_resample(&samples, src_rate, TARGET_SAMPLE_RATE)
    })
}

fn append_mono_f32(out: &mut Vec<f32>, buf: AudioBufferRef<'_>) {
    match buf {
        AudioBufferRef::F32(b) => mix_to_mono(out, &b),
        AudioBufferRef::S16(b) => {
            let mut tmp = b.make_equivalent::<f32>();
            b.convert(&mut tmp);
            mix_to_mono(out, &tmp);
        }
        AudioBufferRef::S32(b) => {
            let mut tmp = b.make_equivalent::<f32>();
            b.convert(&mut tmp);
            mix_to_mono(out, &tmp);
        }
        AudioBufferRef::U8(b) => {
            let mut tmp = b.make_equivalent::<f32>();
            b.convert(&mut tmp);
            mix_to_mono(out, &tmp);
        }
        _ => {} // formats we don't bother converting (rare)
    }
}

fn mix_to_mono(out: &mut Vec<f32>, buf: &symphonia::core::audio::AudioBuffer<f32>) {
    let frames = buf.frames();
    let channels = buf.spec().channels.count();
    if channels == 1 {
        out.extend_from_slice(&buf.chan(0)[..frames]);
        return;
    }
    out.reserve(frames);
    for i in 0..frames {
        let mut acc = 0.0f32;
        for c in 0..channels {
            acc += buf.chan(c)[i];
        }
        out.push(acc / channels as f32);
    }
}

/// Cheap linear interpolation. Quality is fine for speech ASR — Whisper
/// is robust to mild aliasing — and avoids pulling a full DSP crate.
fn linear_resample(input: &[f32], src_rate: u32, dst_rate: u32) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }
    let ratio = src_rate as f64 / dst_rate as f64;
    let out_len = ((input.len() as f64) / ratio).ceil() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos.floor() as usize;
        let frac = (pos - idx as f64) as f32;
        let a = input.get(idx).copied().unwrap_or(0.0);
        let b = input.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

/// Write a 16 kHz mono f32 sample buffer to a 16-bit PCM WAV at `dst`.
/// Sherpa-onnx CLIs read WAV directly; this is the handoff format.
pub fn write_wav_16k_mono(samples: &[f32], dst: &Path) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TARGET_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(dst, spec)?;
    for &s in samples {
        let clamped = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        writer.write_sample(clamped)?;
    }
    writer.finalize()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    #[test]
    fn decodes_16k_mono_wav_to_expected_length() {
        let samples = decode_to_16k_mono(&fixture("short.wav")).unwrap();
        // 1-second 16 kHz fixture → 16,000 samples (±10 from edge cases).
        assert!(
            samples.len() >= 15_990 && samples.len() <= 16_010,
            "got {} samples",
            samples.len()
        );
    }

    #[test]
    fn linear_resample_preserves_amplitude_envelope() {
        let src: Vec<f32> = (0..1000).map(|i| i as f32 / 999.0).collect();
        let dst = linear_resample(&src, 16_000, 8_000);
        assert_eq!(dst.len(), 500);
        // Endpoints close.
        assert!((dst[0] - 0.0).abs() < 0.01);
        assert!((dst[dst.len() - 1] - 1.0).abs() < 0.05);
    }

    #[test]
    fn round_trip_through_wav_preserves_samples() {
        let src = vec![0.0, 0.5, -0.5, 1.0, -1.0, 0.25];
        let tmp = std::env::temp_dir().join("localmind-voice-roundtrip.wav");
        write_wav_16k_mono(&src, &tmp).unwrap();
        let decoded = decode_to_16k_mono(&tmp).unwrap();
        assert_eq!(decoded.len(), src.len());
        for (a, b) in src.iter().zip(decoded.iter()) {
            assert!((a - b).abs() < 0.001, "{a} vs {b}");
        }
    }
}
