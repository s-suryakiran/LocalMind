import { useRef, useState } from "react";
import { Loader2, FileAudio, Mic, Square } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { VoiceTranscript } from "../lib/types";

/** MediaRecorder mime types our backend can decode. Order = preference.
 *  symphonia decodes WAV+PCM, MP4+AAC, OGG+Vorbis but NOT Opus, so
 *  audio/webm (typically Opus on Chrome) is omitted deliberately. */
const RECORD_MIMES = [
  "audio/wav",
  "audio/mp4",
  "audio/mp4;codecs=mp4a.40.2",
  "audio/ogg;codecs=vorbis",
];

function pickRecordMime(): string | null {
  if (typeof MediaRecorder === "undefined") return null;
  for (const m of RECORD_MIMES) {
    if (MediaRecorder.isTypeSupported(m)) return m;
  }
  return null;
}

function extForMime(mime: string): string {
  if (mime.startsWith("audio/wav")) return "wav";
  if (mime.startsWith("audio/mp4")) return "m4a";
  if (mime.startsWith("audio/ogg")) return "ogg";
  return "bin";
}

export function AudioDropZone({ onTranscript }: { onTranscript: (t: VoiceTranscript) => void }) {
  const { addTranscript } = useApp();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [recording, setRecording] = useState(false);
  const recRef = useRef<MediaRecorder | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const mimeRef = useRef<string>("");

  async function pickAndTranscribe() {
    setError(null);
    const picked = await open({
      multiple: false,
      filters: [{ name: "Audio", extensions: ["wav", "mp3", "m4a", "flac", "ogg"] }],
    });
    if (!picked || Array.isArray(picked)) return;
    await runTranscription(picked as string);
  }

  async function runTranscription(path: string) {
    setBusy(true);
    try {
      await api.ensureVoiceEngine();
      const t = await api.voiceTranscribeFile(path);
      addTranscript(t);
      onTranscript(t);
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  }

  async function startRecording() {
    setError(null);
    const mime = pickRecordMime();
    if (!mime) {
      setError("No recordable audio format supported in this browser.");
      return;
    }
    let stream: MediaStream;
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    } catch (e: any) {
      setError(`microphone access denied: ${e?.message ?? e}`);
      return;
    }
    const rec = new MediaRecorder(stream, { mimeType: mime });
    chunksRef.current = [];
    mimeRef.current = mime;
    rec.ondataavailable = (e) => {
      if (e.data && e.data.size > 0) chunksRef.current.push(e.data);
    };
    rec.onstop = async () => {
      stream.getTracks().forEach((t) => t.stop());
      const blob = new Blob(chunksRef.current, { type: mimeRef.current });
      const buf = new Uint8Array(await blob.arrayBuffer());
      try {
        const fs = await import("@tauri-apps/plugin-fs");
        const pathApi = await import("@tauri-apps/api/path");
        const filename = `localmind-rec-${Date.now()}.${extForMime(mimeRef.current)}`;
        await fs.writeFile(filename, buf, { baseDir: fs.BaseDirectory.Temp });
        const tmpDir = await pathApi.tempDir();
        const fullPath = await pathApi.join(tmpDir, filename);
        await runTranscription(fullPath);
      } catch (e: any) {
        setError(`failed to save recording: ${e?.message ?? e}`);
      }
    };
    recRef.current = rec;
    rec.start();
    setRecording(true);
  }

  function stopRecording() {
    recRef.current?.stop();
    setRecording(false);
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="grid grid-cols-2 gap-2">
        <button
          onClick={pickAndTranscribe}
          disabled={busy || recording}
          className="flex items-center justify-center gap-2 rounded-lg border border-dashed border-[var(--color-border)] bg-[var(--color-panel-2)] px-4 py-4 text-sm hover:border-[var(--color-accent)]/60 disabled:opacity-50"
        >
          {busy ? <Loader2 size={16} className="animate-spin" /> : <FileAudio size={16} />}
          {busy ? "Transcribing…" : "Pick file"}
        </button>
        <button
          onClick={recording ? stopRecording : startRecording}
          disabled={busy}
          className="flex items-center justify-center gap-2 rounded-lg border border-[var(--color-border)] bg-[var(--color-panel-2)] px-4 py-4 text-sm hover:border-[var(--color-accent)]/60 disabled:opacity-50"
        >
          {recording ? <Square size={14} /> : <Mic size={14} />}
          {recording ? "Stop" : "Record"}
        </button>
      </div>
      {error && (
        <p className="text-xs text-[var(--color-danger)] bg-[var(--color-danger)]/10 rounded-md px-3 py-2">{error}</p>
      )}
    </div>
  );
}
