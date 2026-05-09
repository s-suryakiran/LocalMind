import { useState } from "react";
import { Pencil, Check } from "lucide-react";
import type { VoiceTranscript } from "../lib/types";

interface Props {
  transcript: VoiceTranscript;
  /** Map of speaker id → display name. Falls back to "Speaker N+1". */
  speakerNames?: Record<number, string>;
  onRenameSpeaker?: (speakerId: number, name: string) => void;
}

const COLORS = [
  "bg-emerald-500/15 text-emerald-200 border-emerald-500/30",
  "bg-sky-500/15 text-sky-200 border-sky-500/30",
  "bg-fuchsia-500/15 text-fuchsia-200 border-fuchsia-500/30",
  "bg-amber-500/15 text-amber-200 border-amber-500/30",
];

function nameFor(id: number, names: Record<number, string> | undefined): string {
  return names?.[id] ?? `Speaker ${id + 1}`;
}

function fmtMs(ms: number): string {
  const s = Math.floor(ms / 1000);
  return `${Math.floor(s / 60)}:${(s % 60).toString().padStart(2, "0")}`;
}

export function TranscriptView({ transcript, speakerNames, onRenameSpeaker }: Props) {
  const [editing, setEditing] = useState<number | null>(null);
  const [draft, setDraft] = useState("");

  return (
    <div className="flex flex-col gap-2">
      {transcript.turns.map((t, idx) => {
        const color = COLORS[t.speaker % COLORS.length];
        const display = nameFor(t.speaker, speakerNames);
        const isEditing = editing === t.speaker;
        return (
          <div key={idx} className={`rounded-lg border p-2 ${color}`}>
            <div className="flex items-center gap-2 text-[11px] mb-1">
              {isEditing ? (
                <>
                  <input
                    autoFocus
                    value={draft}
                    onChange={(e) => setDraft(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        onRenameSpeaker?.(t.speaker, draft.trim() || display);
                        setEditing(null);
                      } else if (e.key === "Escape") {
                        setEditing(null);
                      }
                    }}
                    className="bg-transparent border-b border-current outline-none px-1"
                  />
                  <button
                    aria-label="confirm rename"
                    onClick={() => {
                      onRenameSpeaker?.(t.speaker, draft.trim() || display);
                      setEditing(null);
                    }}
                    className="opacity-70 hover:opacity-100"
                  >
                    <Check size={12} />
                  </button>
                </>
              ) : (
                <>
                  <span className="font-semibold">{display}</span>
                  <button
                    aria-label={`rename ${display}`}
                    onClick={() => {
                      setEditing(t.speaker);
                      setDraft(display);
                    }}
                    className="opacity-50 hover:opacity-100"
                  >
                    <Pencil size={11} />
                  </button>
                </>
              )}
              <span className="opacity-60 ml-auto">
                {fmtMs(t.startMs)}–{fmtMs(t.endMs)}
              </span>
            </div>
            <div className="text-sm">{t.text}</div>
          </div>
        );
      })}
    </div>
  );
}
