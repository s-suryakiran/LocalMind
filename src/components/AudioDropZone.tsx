import { useState } from "react";
import { Loader2, FileAudio } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { VoiceTranscript } from "../lib/types";

export function AudioDropZone({ onTranscript }: { onTranscript: (t: VoiceTranscript) => void }) {
  const { addTranscript } = useApp();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function pickAndTranscribe() {
    setError(null);
    const picked = await open({
      multiple: false,
      filters: [{ name: "Audio", extensions: ["wav", "mp3", "m4a", "flac", "ogg"] }],
    });
    if (!picked || Array.isArray(picked)) return;
    setBusy(true);
    try {
      await api.ensureVoiceEngine();
      const t = await api.voiceTranscribeFile(picked as string);
      addTranscript(t);
      onTranscript(t);
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex flex-col gap-2">
      <button
        onClick={pickAndTranscribe}
        disabled={busy}
        className="flex items-center justify-center gap-2 rounded-lg border border-dashed border-[var(--color-border)] bg-[var(--color-panel-2)] px-4 py-6 text-sm hover:border-[var(--color-accent)]/60 disabled:opacity-50"
      >
        {busy ? <Loader2 size={16} className="animate-spin" /> : <FileAudio size={16} />}
        {busy ? "Transcribing…" : "Pick an audio file"}
      </button>
      {error && (
        <p className="text-xs text-[var(--color-danger)] bg-[var(--color-danger)]/10 rounded-md px-3 py-2">{error}</p>
      )}
    </div>
  );
}
