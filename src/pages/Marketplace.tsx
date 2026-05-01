import { useEffect, useMemo, useState } from "react";
import {
  Search,
  Download,
  Check,
  Loader2,
  HardDrive,
  Heart,
  ExternalLink,
  Network,
  TriangleAlert,
} from "lucide-react";
import { api, listen } from "../lib/api";
import { useApp } from "../lib/store";
import type { HardwareInfo, ModelDownloadProgress, ModelListing } from "../lib/types";
import { formatBytes, formatCompact } from "../lib/util";

const SUGGESTED = [
  { label: "Fast chat (small)", q: "gemma-2-2b-it GGUF" },
  { label: "General (7–8B)", q: "llama-3.1-8b-instruct GGUF" },
  { label: "Reasoning", q: "qwen2.5-7b-instruct GGUF" },
  { label: "Coding", q: "qwen2.5-coder-7b-instruct GGUF" },
  { label: "Vision (LLaVA)", q: "llava-1.6 GGUF" },
];

export function Marketplace() {
  const [query, setQuery] = useState("llama-3.1-8b-instruct GGUF");
  const [loading, setLoading] = useState(false);
  const [results, setResults] = useState<ModelListing[]>([]);
  const { installed, setInstalled, downloads, setDownload, clearDownload, hardware, synapse } = useApp();
  // peerCount = workers the user has added with a token (i.e. usable). We
  // don't include unverified beacon peers in the Synapse budget — those
  // can't actually receive layers until the user pairs.
  const peerCount = synapse.workers.filter((w) => !!w.token).length;
  const budget = useMemo(() => memoryBudgetGb(hardware, peerCount), [hardware, peerCount]);

  useEffect(() => {
    listen<ModelDownloadProgress>("model:progress", (p) => {
      setDownload(p.id, { percent: p.percent, stage: p.stage, downloaded: p.downloaded, total: p.total });
      if (p.stage === "ready") {
        setTimeout(() => clearDownload(p.id), 1500);
        api.listInstalledModels().then(setInstalled).catch(() => {});
      }
    });
    api.listInstalledModels().then(setInstalled).catch(() => {});
    search(query);
  }, []);

  async function search(q: string) {
    setLoading(true);
    try {
      const r = await api.searchModels(q, 24);
      setResults(r);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="flex flex-col h-full">
      <header className="px-5 py-4 border-b border-[var(--color-border-soft)]">
        <h1 className="font-semibold text-[17px] mb-1">Marketplace</h1>
        <p className="text-[var(--color-text-muted)] text-sm mb-3">
          Browse thousands of open-source models from Hugging Face.
        </p>
        <form
          onSubmit={(e) => { e.preventDefault(); search(query); }}
          className="flex items-center gap-2 rounded-lg border border-[var(--color-border)] bg-[var(--color-panel)] px-3 py-2 focus-within:border-[var(--color-accent)]/60"
        >
          <Search size={15} className="text-[var(--color-text-muted)]" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search models… (e.g. llama-3.1-8b GGUF)"
            className="flex-1 bg-transparent outline-none text-sm"
          />
          <button type="submit" className="text-xs px-2.5 py-1 rounded-md bg-[var(--color-panel-2)] hover:bg-[var(--color-border)]">
            Search
          </button>
        </form>
        <div className="flex flex-wrap gap-2 mt-3">
          {SUGGESTED.map((s) => (
            <button
              key={s.q}
              onClick={() => { setQuery(s.q); search(s.q); }}
              className="text-xs px-2.5 py-1 rounded-full border border-[var(--color-border)] hover:border-[var(--color-accent)]/60 text-[var(--color-text-muted)] hover:text-[var(--color-text)]"
            >
              {s.label}
            </button>
          ))}
        </div>
      </header>

      <div className="flex-1 overflow-y-auto px-5 py-4">
        {loading && results.length === 0 ? (
          <div className="flex items-center justify-center py-16 text-[var(--color-text-muted)]">
            <Loader2 size={18} className="animate-spin mr-2" /> Searching…
          </div>
        ) : (
          <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
            {results.map((m) => (
              <ModelCard
                key={m.id}
                listing={m}
                installed={installed}
                downloads={downloads}
                budget={budget}
                peerCount={peerCount}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function ModelCard({
  listing,
  installed,
  downloads,
  budget,
  peerCount,
}: {
  listing: ModelListing;
  installed: { filename: string }[];
  downloads: Record<string, { percent: number; stage: string; downloaded: number; total: number }>;
  budget: { hostGb: number; synapseGb: number };
  peerCount: number;
}) {
  const [expanded, setExpanded] = useState(false);

  const defaultFile = useMemo(() => pickDefault(listing), [listing]);
  const filesSorted = useMemo(
    () => [...listing.files].sort((a, b) => a.sizeBytes - b.sizeBytes),
    [listing.files],
  );

  const isInstalled = (f: { filename: string }) =>
    installed.some((i) => i.filename === f.filename);

  async function download(filename: string) {
    await api.downloadModel(listing.id, filename, inferKind(listing, filename));
  }

  return (
    <div className="rounded-xl border border-[var(--color-border)] bg-[var(--color-panel)] p-4 flex flex-col gap-3">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="text-[13px] text-[var(--color-text-subtle)] truncate">{listing.author}</div>
          <div className="font-semibold text-[15px] truncate">{listing.name}</div>
        </div>
        <a
          href={`https://huggingface.co/${listing.id}`}
          target="_blank"
          rel="noreferrer"
          className="text-[var(--color-text-subtle)] hover:text-[var(--color-text)]"
          title="Open on Hugging Face"
        >
          <ExternalLink size={14} />
        </a>
      </div>

      <div className="flex items-center gap-3 text-xs text-[var(--color-text-muted)]">
        <span className="flex items-center gap-1"><Download size={12} /> {formatCompact(listing.downloads)}</span>
        <span className="flex items-center gap-1"><Heart size={12} /> {formatCompact(listing.likes)}</span>
        <span className="flex items-center gap-1"><HardDrive size={12} /> {listing.files.length} files</span>
      </div>

      {listing.tags.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {listing.tags.slice(0, 4).map((t) => (
            <span key={t} className="text-[10px] px-1.5 py-0.5 rounded bg-[var(--color-panel-2)] text-[var(--color-text-muted)]">{t}</span>
          ))}
        </div>
      )}

      <div className="flex flex-col gap-1.5">
        {(expanded ? filesSorted : defaultFile ? [defaultFile] : filesSorted.slice(0, 1)).map((f) => {
          const progress = downloads[safeId(listing.id, f.filename)];
          const installedFlag = isInstalled(f);
          // Phase 3 chunk H fit calculation. Three buckets the user cares
          // about: comfortably fits on host, needs Synapse, doesn't fit even
          // with current Synapse cluster. We render the third as a hard
          // warning so users don't waste a download on a non-loadable model.
          const needGb = fitGb(f.sizeBytes);
          const fitsHost = budget.hostGb > 0 && needGb <= budget.hostGb;
          const fitsSynapse = budget.synapseGb > 0 && needGb <= budget.synapseGb;
          let badge: React.ReactNode = null;
          if (!fitsHost && fitsSynapse) {
            badge = (
              <span
                title={
                  peerCount > 0
                    ? `Needs ~${needGb.toFixed(1)} GB · host has ~${budget.hostGb} GB · Synapse extends to ~${budget.synapseGb} GB`
                    : `Needs ~${needGb.toFixed(1)} GB · host has ~${budget.hostGb} GB · pair a Synapse worker to load this`
                }
                className="text-[10px] uppercase tracking-wider text-[var(--color-accent)] flex items-center gap-1"
              >
                <Network size={10} /> needs synapse
              </span>
            );
          } else if (!fitsHost && !fitsSynapse && budget.hostGb > 0) {
            badge = (
              <span
                title={`Needs ~${needGb.toFixed(1)} GB · best you have (host + ${peerCount} peer${peerCount === 1 ? "" : "s"}) is ~${budget.synapseGb} GB`}
                className="text-[10px] uppercase tracking-wider text-[var(--color-danger)] flex items-center gap-1"
              >
                <TriangleAlert size={10} /> too large
              </span>
            );
          }
          return (
            <div key={f.filename} className="flex items-center justify-between gap-2 text-xs">
              <div className="min-w-0">
                <div className="truncate">{f.filename}</div>
                <div className="text-[var(--color-text-subtle)] flex items-center gap-2">
                  <span>
                    {f.quantization} · {formatBytes(f.sizeBytes)}
                  </span>
                  {badge}
                </div>
              </div>
              {installedFlag ? (
                <span className="text-[var(--color-success)] flex items-center gap-1"><Check size={12} /> installed</span>
              ) : progress ? (
                <div className="flex items-center gap-2 min-w-[100px]">
                  <div className="h-1 flex-1 rounded-full bg-[var(--color-border)] overflow-hidden">
                    <div className="h-full gradient-accent" style={{ width: `${Math.min(100, progress.percent)}%` }} />
                  </div>
                  <span className="text-[var(--color-text-subtle)]">{Math.floor(progress.percent)}%</span>
                </div>
              ) : (
                <button
                  onClick={() => download(f.filename)}
                  className="px-2 py-1 rounded-md bg-[var(--color-panel-2)] hover:bg-[var(--color-border)] flex items-center gap-1"
                >
                  <Download size={12} /> get
                </button>
              )}
            </div>
          );
        })}
      </div>

      {filesSorted.length > 1 && (
        <button
          onClick={() => setExpanded((v) => !v)}
          className="text-[11px] text-[var(--color-text-subtle)] hover:text-[var(--color-text)] text-left"
        >
          {expanded ? "Show less" : `Show ${filesSorted.length - 1} more variant${filesSorted.length - 1 === 1 ? "" : "s"}`}
        </button>
      )}
    </div>
  );
}

/** Phase 3 chunk H: compute the available memory budget for loading a model.
 *  Returns { hostGb, synapseGb } so the marketplace can show the user how
 *  Synapse extends their reach. We deliberately don't try to be exact about
 *  KV-cache or activations — the budget is a guidance, not a guarantee, and
 *  keeping the heuristic simple beats getting it precisely wrong.
 *
 *  Host budget = total RAM (Apple Silicon: unified memory) or VRAM (NVIDIA/AMD).
 *  Synapse budget = host + sum of detected peers, with 4 GB assumed per peer
 *  since beacon doesn't carry VRAM yet (chunk 2c, deferred). Conservative: a
 *  paired RTX 5070 Ti has 16 GB but we assume 4, so the badge under-promises.
 */
function memoryBudgetGb(hw: HardwareInfo | null, peerCount: number) {
  if (!hw) return { hostGb: 0, synapseGb: 0 };
  const acc = hw.accelerator;
  const hostGb = (() => {
    switch (acc.type) {
      case "appleSilicon":
        // Apple Silicon shares unified memory; usable budget is ~75% of
        // total before the OS pushes back hard.
        return Math.floor(acc.unifiedMemoryGb * 0.75);
      case "nvidia":
        return acc.vramGb;
      case "amd":
        return acc.vramGb;
      case "intelArc":
      case "cpu":
      default:
        // No GPU info — fall back to system RAM minus a 4 GB OS reserve.
        return Math.max(0, Math.floor(hw.totalMemoryGb - 4));
    }
  })();
  const synapseGb = hostGb + peerCount * 4;
  return { hostGb, synapseGb };
}

/** Estimated bytes-on-GPU needed to actually load a GGUF. The file size is
 *  the dominant term; KV cache adds 5–15% depending on context. We multiply
 *  by 1.15 to keep the badge from being aggressively optimistic. */
function fitGb(fileBytes: number) {
  return (fileBytes * 1.15) / 1024 ** 3;
}

function pickDefault(l: ModelListing) {
  const prefer = ["Q4_K_M", "Q4_K_S", "Q5_K_M", "Q4_0", "Q8_0"];
  for (const p of prefer) {
    const f = l.files.find((f) => f.quantization === p);
    if (f) return f;
  }
  return [...l.files].sort((a, b) => a.sizeBytes - b.sizeBytes)[0];
}

function safeId(repo: string, fname: string) {
  return `${repo.replace("/", "__")}__${fname}`;
}

function inferKind(l: ModelListing, fname: string): string {
  const low = (l.name + " " + fname + " " + l.tags.join(" ")).toLowerCase();
  if (low.includes("llava") || low.includes("vision")) return "vision";
  if (low.includes("whisper")) return "whisper";
  if (low.includes("embed") || low.includes("bge-") || low.includes("nomic")) return "embedding";
  if (low.includes("stable-diffusion") || low.includes("sdxl")) return "sd";
  return "llm";
}
