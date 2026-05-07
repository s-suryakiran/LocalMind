import { useEffect } from "react";
import { Trash2, Boxes } from "lucide-react";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import { formatBytes } from "../lib/util";
import { SlotsPanel } from "../components/SlotsPanel";

export function Models() {
  const { installed, setInstalled, setLlama, setView } = useApp();

  useEffect(() => {
    api.listInstalledModels().then(setInstalled).catch(console.error);
    api.llamaStatus().then(setLlama).catch(() => {});
  }, [setInstalled, setLlama]);

  async function remove(id: string) {
    if (!confirm("Delete this model file?")) return;
    await api.deleteModel(id);
    const list = await api.listInstalledModels();
    setInstalled(list);
  }

  return (
    <div className="flex flex-col h-full">
      <header className="px-5 py-4 border-b border-[var(--color-border-soft)] flex items-center justify-between">
        <div>
          <h1 className="font-semibold text-[17px]">Active slots</h1>
          <p className="text-[var(--color-text-muted)] text-sm">
            Load chat, embedding, and vision models concurrently.
          </p>
        </div>
        <button
          onClick={() => setView("marketplace")}
          className="text-sm px-3 py-1.5 rounded-md bg-[var(--color-panel-2)] border border-[var(--color-border)] hover:border-[var(--color-accent)]/60 flex items-center gap-2"
        >
          <Boxes size={14} /> Get more
        </button>
      </header>

      <div className="flex-1 overflow-y-auto px-5 py-4">
        <SlotsPanel />

        <h2 className="text-sm font-medium mt-6 mb-2">Library</h2>
        {installed.length === 0 ? (
          <p className="text-xs text-[var(--color-text-muted)]">
            No models yet. Open the marketplace to download one.
          </p>
        ) : (
          <div className="flex flex-col gap-2">
            {installed.map((m) => (
              <div
                key={m.id}
                className="rounded-lg border border-[var(--color-border)] bg-[var(--color-panel)] p-3 flex items-center gap-3"
              >
                <div className="flex-1 min-w-0">
                  <div className="text-sm truncate">{m.filename}</div>
                  <div className="text-xs text-[var(--color-text-muted)] truncate">
                    {m.repo !== "local" ? m.repo : "Local file"} · {formatBytes(m.sizeBytes)} · {m.kind}
                  </div>
                </div>
                <button
                  onClick={() => remove(m.id)}
                  className="text-[var(--color-text-subtle)] hover:text-[var(--color-danger)] p-1"
                >
                  <Trash2 size={15} />
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
