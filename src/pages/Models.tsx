import { useEffect } from "react";
import { Trash2, Play, Square, Boxes } from "lucide-react";
import { api } from "../lib/api";
import { useApp } from "../lib/store";
import { formatBytes } from "../lib/util";

export function Models() {
  const { installed, setInstalled, activeModelId, setActiveModelId, llama, setLlama, setView, synapse } = useApp();

  useEffect(() => {
    api.listInstalledModels().then(setInstalled).catch(console.error);
    api.llamaStatus().then(setLlama).catch(() => {});
  }, [setInstalled, setLlama]);

  async function remove(id: string) {
    if (!confirm("Delete this model file?")) return;
    await api.deleteModel(id);
    const list = await api.listInstalledModels();
    setInstalled(list);
    if (activeModelId === id) setActiveModelId(null);
  }

  async function activate(id: string) {
    setActiveModelId(id);
    const model = installed.find((m) => m.id === id);
    const mmprojs = installed.filter((m) => m.kind === "mmproj" || /mmproj/i.test(m.id));
    const mmprojId = model?.kind === "vision"
      ? (mmprojs.find((m) => m.repo === model.repo) ?? mmprojs[0])?.id
      : undefined;
    const synapseWorkers = synapse.workers.length > 0 ? synapse.workers : undefined;
    const s = await api.startLlama({
      modelId: id,
      mmprojId,
      synapseWorkers,
      hostWeight: synapse.hostWeight,
    });
    setLlama(s);
  }

  async function stop() {
    await api.stopLlama();
    setLlama({ ...llama, running: false, modelId: null, pid: null });
  }

  return (
    <div className="flex flex-col h-full">
      <header className="px-5 py-4 border-b border-[var(--color-border-soft)] flex items-center justify-between">
        <div>
          <h1 className="font-semibold text-[17px]">My models</h1>
          <p className="text-[var(--color-text-muted)] text-sm">Manage downloaded models on this device.</p>
        </div>
        <button
          onClick={() => setView("marketplace")}
          className="text-sm px-3 py-1.5 rounded-md bg-[var(--color-panel-2)] border border-[var(--color-border)] hover:border-[var(--color-accent)]/60 flex items-center gap-2"
        >
          <Boxes size={14} /> Get more
        </button>
      </header>

      <div className="flex-1 overflow-y-auto px-5 py-4">
        {installed.length === 0 ? (
          <div className="text-center py-16 text-[var(--color-text-muted)]">
            <p className="mb-3">No models yet.</p>
            <button
              onClick={() => setView("marketplace")}
              className="px-4 py-2 rounded-md gradient-accent text-white text-sm font-medium"
            >
              Open marketplace →
            </button>
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            {installed.map((m) => {
              const running = llama.running && llama.modelId === m.id;
              return (
                <div
                  key={m.id}
                  className="rounded-lg border border-[var(--color-border)] bg-[var(--color-panel)] p-3 flex items-center gap-3"
                >
                  <div className="w-9 h-9 rounded-md bg-[var(--color-panel-2)] grid place-items-center text-[var(--color-text-muted)]">
                    <Boxes size={16} />
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium truncate">{m.filename}</div>
                    <div className="text-xs text-[var(--color-text-muted)] truncate">
                      {m.repo !== "local" ? m.repo : "Local file"} · {formatBytes(m.sizeBytes)} · {m.kind}
                    </div>
                  </div>
                  {running ? (
                    <button
                      onClick={stop}
                      className="flex items-center gap-1 text-xs px-2.5 py-1 rounded-md bg-[var(--color-panel-2)] border border-[var(--color-border)]"
                    >
                      <Square size={12} /> stop
                    </button>
                  ) : (
                    <button
                      onClick={() => activate(m.id)}
                      className="flex items-center gap-1 text-xs px-2.5 py-1 rounded-md gradient-accent text-white"
                    >
                      <Play size={12} /> load
                    </button>
                  )}
                  <button
                    onClick={() => remove(m.id)}
                    className="text-[var(--color-text-subtle)] hover:text-[var(--color-danger)] p-1"
                  >
                    <Trash2 size={15} />
                  </button>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
