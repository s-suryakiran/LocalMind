//! Voice pipeline. Runs sherpa-onnx CLI binaries to produce a
//! speaker-diarized transcript from an arbitrary audio file.

use crate::config;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::{AppHandle, Emitter};
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceTurn {
    pub speaker: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceTranscript {
    pub id: String,
    pub source_path: Option<String>,
    pub created_at: u64,
    pub turns: Vec<VoiceTurn>,
}

/// Kick off a full transcribe-and-diarize pass on `path`. Emits one
/// `voice:turn` event per segment as it finishes. Returns the final
/// transcript value once all segments are processed.
pub async fn transcribe_file(app: &AppHandle, path: &Path) -> Result<VoiceTranscript> {
    let _ = crate::binaries::ensure_sherpa_onnx(app).await?;
    let models = crate::binaries::ensure_diarization_models(app).await?;

    // Ensure 16 kHz mono WAV; sherpa-onnx CLIs accept other rates but
    // we'd rather control the input shape than chase quirks.
    let pcm = crate::voice_audio::decode_to_16k_mono(path)?;
    let staging =
        std::env::temp_dir().join(format!("localmind-voice-{}.wav", uuid::Uuid::new_v4()));
    crate::voice_audio::write_wav_16k_mono(&pcm, &staging)?;

    // Pass 1: diarization. CLI prints lines like:
    //   0.000 1.234 0
    //   1.234 2.500 1
    // (start end speaker_id)
    let segments = run_diarization(&staging, &models).await?;
    let _ = app.emit(
        "voice:status",
        serde_json::json!({ "stage": "transcribing", "segments": segments.len() }),
    );

    let mut turns: Vec<VoiceTurn> = Vec::with_capacity(segments.len());
    for seg in &segments {
        // Slice the PCM for this segment; write a temp WAV; transcribe.
        let s = ((seg.start_s * 16_000.0) as usize).min(pcm.len());
        let e = ((seg.end_s * 16_000.0) as usize).min(pcm.len());
        if e <= s {
            continue;
        }
        let clip_path =
            std::env::temp_dir().join(format!("localmind-clip-{}.wav", uuid::Uuid::new_v4()));
        crate::voice_audio::write_wav_16k_mono(&pcm[s..e], &clip_path)?;
        let text = match run_asr(&clip_path, &models).await {
            Ok(t) => t,
            Err(e) => format!("[asr error: {e}]"),
        };
        let _ = std::fs::remove_file(&clip_path);
        let turn = VoiceTurn {
            speaker: seg.speaker,
            start_ms: (seg.start_s * 1000.0) as u64,
            end_ms: (seg.end_s * 1000.0) as u64,
            text,
        };
        let _ = app.emit("voice:turn", &turn);
        turns.push(turn);
    }

    let _ = std::fs::remove_file(&staging);

    let transcript = VoiceTranscript {
        id: uuid::Uuid::new_v4().to_string(),
        source_path: Some(path.display().to_string()),
        created_at: now_ms(),
        turns,
    };

    // Persist a copy to data_dir/transcripts/<id>.json so the user can
    // re-ingest later or back it up.
    let dst = config::transcripts_dir().join(format!("{}.json", transcript.id));
    let _ = std::fs::write(&dst, serde_json::to_vec_pretty(&transcript)?);

    Ok(transcript)
}

#[derive(Debug)]
struct Segment {
    start_s: f32,
    end_s: f32,
    speaker: u32,
}

async fn run_diarization(wav: &Path, models: &Path) -> Result<Vec<Segment>> {
    let bin = config::sherpa_diarization_bin_path();
    // Pyannote tarball extracts to a subdirectory containing model.onnx.
    let segmentation = models
        .join("sherpa-onnx-pyannote-segmentation-3-0")
        .join("model.onnx");
    let speaker = models.join("3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx");
    // The actual CLI flags use dotted namespacing
    // (--segmentation.pyannote-model, --embedding.model). Without
    // --clustering.num-clusters, the binary falls back to threshold
    // clustering at 0.5 (the default), which works for unknown
    // speaker counts.
    let out = Command::new(&bin)
        .arg(format!(
            "--segmentation.pyannote-model={}",
            segmentation.display()
        ))
        .arg(format!("--embedding.model={}", speaker.display()))
        .arg(wav)
        .output()
        .await
        .context("running sherpa-onnx diarization CLI")?;
    if !out.status.success() {
        return Err(anyhow!(
            "diarization failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    parse_diarization_stdout(&String::from_utf8_lossy(&out.stdout))
}

fn parse_diarization_stdout(stdout: &str) -> Result<Vec<Segment>> {
    let mut segs = Vec::new();
    for raw in stdout.lines() {
        let line = raw.trim();
        // Real format: "<start_s> -- <end_s> speaker_<NN>". Banner lines
        // (config dump, "Started", warnings) are skipped by the shape check.
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() != 4 || parts[1] != "--" {
            continue;
        }
        let start_s: f32 = match parts[0].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let end_s: f32 = match parts[2].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        // sherpa emits 1-indexed labels (speaker_01, speaker_02, …).
        // Store 0-indexed so the frontend's `Speaker N+1` display matches.
        let raw_id: u32 = match parts[3]
            .strip_prefix("speaker_")
            .and_then(|n| n.parse().ok())
        {
            Some(v) => v,
            None => continue,
        };
        if end_s > start_s {
            segs.push(Segment {
                start_s,
                end_s,
                speaker: raw_id.saturating_sub(1),
            });
        }
    }
    Ok(segs)
}

async fn run_asr(wav: &Path, models: &Path) -> Result<String> {
    let bin = config::sherpa_bin_path();
    // sherpa-onnx-whisper-tiny.en tarball extracts into a subdir with
    // both fp32 and int8-quantized variants. Prefer the int8 ones —
    // they're ~3x smaller and run on CPU just fine for tiny model size.
    let whisper_dir = models.join("sherpa-onnx-whisper-tiny.en");
    let encoder = whisper_dir.join("tiny.en-encoder.int8.onnx");
    let decoder = whisper_dir.join("tiny.en-decoder.int8.onnx");
    let tokens = whisper_dir.join("tiny.en-tokens.txt");
    let out = Command::new(&bin)
        .arg(format!("--whisper-encoder={}", encoder.display()))
        .arg(format!("--whisper-decoder={}", decoder.display()))
        .arg(format!("--tokens={}", tokens.display()))
        .arg(wav)
        .output()
        .await
        .context("running sherpa-onnx ASR CLI")?;
    if !out.status.success() {
        return Err(anyhow!(
            "asr failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(parse_asr_stdout(&String::from_utf8_lossy(&out.stdout)))
}

/// Sherpa-onnx-offline emits a banner + a final result line beginning
/// with "{". We extract the JSON object, take its `text` field.
fn parse_asr_stdout(stdout: &str) -> String {
    for line in stdout.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(t) = v["text"].as_str() {
                return t.trim().to_string();
            }
        }
    }
    // Fallback: last non-empty line, stripped.
    stdout
        .lines()
        .rfind(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_diarization_real_sherpa_output() {
        // Captured verbatim from sherpa-onnx-offline-speaker-diarization
        // running on a 3.3s recording. Format: "<start> -- <end> speaker_<NN>".
        let s = "OfflineSpeakerDiarizationConfig(segmentation=...)\n\
                 Started\n\
                 0.031 -- 2.495 speaker_01\n";
        let segs = parse_diarization_stdout(s).unwrap();
        assert_eq!(segs.len(), 1);
        assert!((segs[0].start_s - 0.031).abs() < 1e-3);
        assert!((segs[0].end_s - 2.495).abs() < 1e-3);
        // sherpa labels are 1-indexed; we normalize to 0-indexed.
        assert_eq!(segs[0].speaker, 0);
    }

    #[test]
    fn parses_diarization_multiple_speakers_zero_indexed() {
        let s = "0.000 -- 1.234 speaker_01\n\
                 1.234 -- 2.500 speaker_02\n\
                 2.500 -- 5.000 speaker_01\n";
        let segs = parse_diarization_stdout(s).unwrap();
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].speaker, 0);
        assert_eq!(segs[1].speaker, 1);
        assert_eq!(segs[2].speaker, 0);
    }

    #[test]
    fn parses_diarization_skips_garbage() {
        let s = "launching pipeline\n\
                 0.0 -- 1.0 speaker_01\n\
                 WARNING: low confidence\n\
                 1.0 -- 2.0 speaker_02\n";
        let segs = parse_diarization_stdout(s).unwrap();
        assert_eq!(segs.len(), 2);
    }

    #[test]
    fn parses_diarization_speaker_00_does_not_underflow() {
        // Defensive: if sherpa ever emits 0-indexed labels, store 0 not u32::MAX.
        let s = "0.0 -- 1.0 speaker_00\n";
        let segs = parse_diarization_stdout(s).unwrap();
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].speaker, 0);
    }

    #[test]
    fn parses_asr_json_text_field() {
        let s = r#"
sherpa-onnx ASR demo
{"text":"hello world","timestamps":[]}
"#;
        assert_eq!(parse_asr_stdout(s), "hello world");
    }

    #[test]
    fn parses_asr_falls_back_to_last_line() {
        let s = "
some banner
plain text result
";
        assert_eq!(parse_asr_stdout(s), "plain text result");
    }
}
