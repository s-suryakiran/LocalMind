import { useState } from "react";
import { Loader2, Play, Square, Cpu, Image as ImageIcon, Database } from "lucide-react";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import type { Role, InstalledModel } from "../lib/types";

const ROLE_LABEL: Record<Role, string> = {
  chat: "Chat",
  embed: "Embedding",
  vision: "Vision",
};
const ROLE_ICON: Record<Role, typeof Cpu> = {
  chat: Cpu,
  embed: Database,
  vision: ImageIcon,
};

function modelsForRole(installed: InstalledModel[], role: Role): InstalledModel[] {
  switch (role) {
    case "chat":
      return installed.filter((m) => m.kind === "llm");
    case "embed":
      return installed.filter((m) => m.kind === "embedding");
    case "vision":
      return installed.filter((m) => m.kind === "vision");
  }
}

export function SlotsPanel() {
  const { llama, installed, setLlama } = useApp();
  const [busy, setBusy] = useState<Role | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function load(role: Role, modelId: string) {
    setBusy(role);
    setError(null);
    try {
      let mmprojId: string | undefined;
      if (role === "vision") {
        const model = installed.find((m) => m.id === modelId);
        const mmprojs = installed.filter((m) => m.kind === "mmproj");
        mmprojId = (mmprojs.find((m) => m.repo === model?.repo) ?? mmprojs[0])?.id;
        if (!mmprojId) {
          throw new Error("vision model needs a matching mmproj projector — download one from the marketplace first");
        }
      }
      const s = await api.startSlot(role, modelId, mmprojId);
      setLlama(s);
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setBusy(null);
    }
  }

  async function unload(role: Role) {
    setBusy(role);
    setError(null);
    try {
      await api.stopSlot(role);
      const s = await api.llamaStatus();
      setLlama(s);
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="flex flex-col gap-3">
      {(["chat", "embed", "vision"] as Role[]).map((role) => {
        const slot = llama.slots?.find((s) => s.role === role);
        const Icon = ROLE_ICON[role];
        const candidates = modelsForRole(installed, role);
        return (
          <div
            key={role}
            className="rounded-lg border border-[var(--color-border)] bg-[var(--color-panel)] p-3"
          >
            <div className="flex items-center gap-3 mb-2">
              <div className="w-9 h-9 rounded-md bg-[var(--color-panel-2)] grid place-items-center text-[var(--color-text-muted)]">
                <Icon size={16} />
              </div>
              <div className="flex-1 min-w-0">
                <div className="text-sm font-medium">{ROLE_LABEL[role]} slot</div>
                <div className="text-xs text-[var(--color-text-muted)] truncate">
                  {slot?.running ? slot.modelId : "Not loaded"}
                </div>
              </div>
              {slot?.running ? (
                <button
                  onClick={() => unload(role)}
                  disabled={busy === role}
                  className="flex items-center gap-1 text-xs px-2.5 py-1 rounded-md bg-[var(--color-panel-2)] border border-[var(--color-border)]"
                >
                  {busy === role ? <Loader2 size={12} className="animate-spin" /> : <Square size={12} />} unload
                </button>
              ) : null}
            </div>
            {!slot?.running && (
              <div className="flex flex-wrap gap-1.5">
                {candidates.length === 0 && (
                  <span className="text-xs text-[var(--color-text-subtle)]">
                    No {role} models on this device — visit the marketplace.
                  </span>
                )}
                {candidates.map((m) => (
                  <button
                    key={m.id}
                    onClick={() => load(role, m.id)}
                    disabled={busy !== null}
                    className="flex items-center gap-1 text-xs px-2 py-0.5 rounded-md gradient-accent text-white disabled:opacity-50"
                  >
                    <Play size={11} /> {m.filename}
                  </button>
                ))}
              </div>
            )}
          </div>
        );
      })}
      {error && (
        <p className="text-xs text-[var(--color-danger)] bg-[var(--color-danger)]/10 rounded-md px-3 py-2">{error}</p>
      )}
    </div>
  );
}
