import { useEffect, useMemo, useRef, useState } from "react";
import {
  Network,
  Power,
  Terminal,
  Trash2,
  RefreshCw,
  Plus,
  Radio,
  Server,
  Search,
} from "lucide-react";
import { useApp } from "../lib/store";
import { api, listen } from "../lib/api";
import type { SynapsePeer } from "../lib/types";
import { isTauri } from "../lib/util";

// Live log buffer cap. ~400 lines is enough to scrollback through a model load
// without eating RAM during long sessions.
const MAX_LOG_LINES = 400;

type LogEntry = {
  source: "llama" | "synapse";
  stream: string;
  line: string;
  tag?: string;
};

// Pre-compiled regex for "interesting" log lines that confirm distributed
// inference is actually engaged. Mentioned RPC backends, buffer tables, layer
// offloads, and any errors qualify — those are the lines that prove a layer
// split actually happened.
// Lines we want to surface in the highlights view: anything that proves a
// distributed inference path is engaged (RPC, offload, layer split), anything
// about discovery (mDNS, advertise, peer), and any errors. The default-on
// "Highlights" toggle is meant to skip llama.cpp's chatty progress lines, not
// hide our own diagnostics — so this list has to cover both.
const HIGHLIGHT_RE =
  /\b(rpc|offload|buffer size|backend|n_layer|tensor split|mdns|advertise|peer|synapse|error|failed|timeout)\b/i;

export function Synapse() {
  const remote = !isTauri();
  const {
    synapse,
    setSynapseWorkerEnabled,
    setSynapseWorkerPort,
    setSynapseWorkers,
  } = useApp();

  const [workerBusy, setWorkerBusy] = useState(false);
  const [workerError, setWorkerError] = useState<string | null>(null);
  // We edit workers as a single textarea string so users can type commas/newlines
  // freely; on commit we split into the canonical `string[]`.
  const [workersText, setWorkersText] = useState(synapse.workers.join(", "));

  // Discovered peers via mDNS. Keyed by peer.id (the mDNS instance name) so
  // duplicates from re-resolution don't double-render.
  const [peers, setPeers] = useState<Record<string, SynapsePeer>>({});
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [highlightsOnly, setHighlightsOnly] = useState(true);
  const [autoScroll, setAutoScroll] = useState(true);
  const logBoxRef = useRef<HTMLDivElement>(null);

  // Sync the worker toggle with backend reality on mount — the persisted store
  // can drift from the actual rpc-server child if the app crashed mid-run.
  useEffect(() => {
    if (remote) return;
    api.synapseWorkerStatus()
      .then((s) => {
        setSynapseWorkerEnabled(s.running);
        if (s.port) setSynapseWorkerPort(s.port);
      })
      .catch(() => {});
    api.synapseListPeers()
      .then((list) => {
        const next: Record<string, SynapsePeer> = {};
        for (const p of list) next[p.id] = p;
        setPeers(next);
      })
      .catch(() => {});
  }, [remote, setSynapseWorkerEnabled, setSynapseWorkerPort]);

  // Subscribe to mDNS peer events. The Rust side starts browsing at app boot,
  // so we just have to hook into the stream — no `start_discovery` to call.
  useEffect(() => {
    if (remote) return;
    let pending: Array<() => void> = [];
    let cancelled = false;

    listen<SynapsePeer>("synapse:peer-added", (p) => {
      setPeers((prev) => ({ ...prev, [p.id]: p }));
    }).then((un) => { if (cancelled) un(); else pending.push(un); });

    listen<{ id: string }>("synapse:peer-removed", (p) => {
      setPeers((prev) => {
        if (!(p.id in prev)) return prev;
        const next = { ...prev };
        delete next[p.id];
        return next;
      });
    }).then((un) => { if (cancelled) un(); else pending.push(un); });

    return () => {
      cancelled = true;
      pending.forEach((un) => un());
    };
  }, [remote]);

  // Subscribe to llama-server / rpc-server log streams. Same shape as the old
  // Settings panel — we kept the panel here because debugging Synapse is the
  // main reason anyone wants to see these logs.
  useEffect(() => {
    if (remote) return;
    let pending: Array<() => void> = [];
    let cancelled = false;

    const append = (entry: LogEntry) => {
      setLogs((prev) => {
        const next = prev.concat(entry);
        return next.length > MAX_LOG_LINES ? next.slice(next.length - MAX_LOG_LINES) : next;
      });
    };

    listen<{ stream: string; line: string; tag?: string }>("llama:log", (p) => {
      append({ source: "llama", stream: p.stream, line: p.line, tag: p.tag });
    }).then((un) => { if (cancelled) un(); else pending.push(un); });

    listen<{ stream: string; line: string }>("synapse:log", (p) => {
      append({ source: "synapse", stream: p.stream, line: p.line });
    }).then((un) => { if (cancelled) un(); else pending.push(un); });

    return () => {
      cancelled = true;
      pending.forEach((un) => un());
    };
  }, [remote]);

  useEffect(() => {
    if (autoScroll && logBoxRef.current) {
      logBoxRef.current.scrollTop = logBoxRef.current.scrollHeight;
    }
  }, [logs, autoScroll]);

  async function toggleWorker(next: boolean) {
    setWorkerBusy(true);
    setWorkerError(null);
    try {
      if (next) {
        const status = await api.startSynapseWorker(synapse.workerPort);
        setSynapseWorkerEnabled(status.running);
        setSynapseWorkerPort(status.port);
      } else {
        await api.stopSynapseWorker();
        setSynapseWorkerEnabled(false);
      }
    } catch (e: any) {
      setWorkerError(e?.message ?? String(e));
    } finally {
      setWorkerBusy(false);
    }
  }

  // Restart the rpc-server child. Useful when a previous host disconnected
  // mid-inference and llama.cpp left buffers allocated on the GPU — a clean
  // restart guarantees VRAM is freed.
  async function restartWorker() {
    setWorkerBusy(true);
    setWorkerError(null);
    try {
      const status = await api.restartSynapseWorker(synapse.workerPort);
      setSynapseWorkerEnabled(status.running);
      setSynapseWorkerPort(status.port);
    } catch (e: any) {
      setWorkerError(e?.message ?? String(e));
    } finally {
      setWorkerBusy(false);
    }
  }

  function commitWorkersText(text: string) {
    setWorkersText(text);
    const parsed = text
      .split(/[,\n]/)
      .map((s) => s.trim())
      .filter(Boolean);
    setSynapseWorkers(parsed);
  }

  // Add a discovered peer to the workers list with one click. Avoid dupes.
  function addPeer(p: SynapsePeer) {
    if (synapse.workers.includes(p.endpoint)) return;
    const next = [...synapse.workers, p.endpoint];
    setSynapseWorkers(next);
    setWorkersText(next.join(", "));
  }

  function removeWorker(endpoint: string) {
    const next = synapse.workers.filter((w) => w !== endpoint);
    setSynapseWorkers(next);
    setWorkersText(next.join(", "));
  }

  const peerList = useMemo(() => Object.values(peers), [peers]);
  const filteredLogs = useMemo(
    () => (highlightsOnly ? logs.filter((l) => HIGHLIGHT_RE.test(l.line)) : logs),
    [logs, highlightsOnly],
  );

  if (remote) {
    // Phone/PWA — Synapse is desktop-only since it spawns child processes.
    return (
      <div className="flex flex-col h-full items-center justify-center text-center px-6">
        <Network size={28} className="text-[var(--color-text-muted)] mb-3" />
        <p className="text-sm text-[var(--color-text-muted)] max-w-sm">
          Synapse runs on the desktop host — open LocalMind on the paired Mac/PC to
          manage workers and peers.
        </p>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      <header className="px-5 py-4 border-b border-[var(--color-border-soft)]">
        <h1 className="font-semibold text-[17px] flex items-center gap-2">
          <Network size={16} /> Synapse
        </h1>
        <p className="text-[var(--color-text-muted)] text-sm">
          Pool GPUs across LAN machines to run models that don't fit on one device.
          Wired Ethernet recommended for tokens/sec.
        </p>
      </header>

      <div className="flex-1 overflow-y-auto px-5 py-5 max-w-3xl">
        {/* Worker mode card */}
        <Section title="This machine" icon={<Server size={15} />}>
          <div className="flex items-start gap-3">
            <Power
              size={15}
              className={
                synapse.workerEnabled
                  ? "text-[var(--color-success)] mt-0.5"
                  : "text-[var(--color-text-muted)] mt-0.5"
              }
            />
            <div className="flex-1">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <div className="text-sm font-medium">Run as a Synapse worker</div>
                  <div className="text-xs text-[var(--color-text-muted)] mt-0.5">
                    Spawns <code>rpc-server</code> and announces this machine on the LAN
                    via mDNS so other LocalMind hosts can find it automatically.
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  {synapse.workerEnabled && (
                    <button
                      onClick={restartWorker}
                      disabled={workerBusy}
                      title="Restart worker — flushes VRAM"
                      className="text-sm px-2.5 py-1.5 rounded-md bg-[var(--color-panel-2)] border border-[var(--color-border)] hover:border-[var(--color-accent)]/60 disabled:opacity-50"
                    >
                      <RefreshCw size={13} />
                    </button>
                  )}
                  <button
                    onClick={() => toggleWorker(!synapse.workerEnabled)}
                    disabled={workerBusy}
                    className={`text-sm px-3 py-1.5 rounded-md border ${
                      synapse.workerEnabled
                        ? "bg-[var(--color-success)]/10 border-[var(--color-success)]/40 text-[var(--color-success)]"
                        : "bg-[var(--color-panel)] border-[var(--color-border)] hover:border-[var(--color-accent)]/60"
                    } disabled:opacity-50`}
                  >
                    {workerBusy ? "…" : synapse.workerEnabled ? "Stop worker" : "Start worker"}
                  </button>
                </div>
              </div>
              <div className="mt-3 flex items-center gap-2 text-xs text-[var(--color-text-muted)]">
                <span>Listen port:</span>
                <input
                  type="number"
                  value={synapse.workerPort}
                  disabled={synapse.workerEnabled}
                  onChange={(e) =>
                    setSynapseWorkerPort(parseInt(e.target.value, 10) || 50052)
                  }
                  className="w-20 bg-[var(--color-panel)] border border-[var(--color-border)] rounded px-2 py-0.5 text-[var(--color-text)] disabled:opacity-50"
                />
                <span className="text-[var(--color-text-subtle)]">(default 50052)</span>
              </div>
              {workerError && (
                <div className="mt-2 text-xs text-[var(--color-danger)]">{workerError}</div>
              )}
            </div>
          </div>
        </Section>

        {/* Discovered peers */}
        <Section
          title={`Discovered on LAN (${peerList.length})`}
          icon={<Radio size={15} />}
        >
          <p className="text-xs text-[var(--color-text-muted)] mb-3">
            LocalMind workers advertise themselves via mDNS (<code>_localmind-synapse._tcp</code>).
            Click <Plus size={11} className="inline -mt-0.5" /> to add one to your worker list.
          </p>
          {peerList.length === 0 ? (
            <div className="flex items-center gap-2 text-sm text-[var(--color-text-subtle)] py-3">
              <Search size={14} className="animate-pulse" />
              No peers yet — start worker mode on another machine on this network.
            </div>
          ) : (
            <ul className="flex flex-col gap-1.5">
              {peerList.map((p) => {
                const added = synapse.workers.includes(p.endpoint);
                return (
                  <li
                    key={p.id}
                    className="flex items-center gap-3 rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2"
                  >
                    <div className="w-1.5 h-1.5 rounded-full bg-[var(--color-success)]" />
                    <div className="flex-1 min-w-0">
                      <div className="text-sm font-medium truncate">{p.hostname}</div>
                      <div className="text-xs text-[var(--color-text-muted)] font-mono truncate">
                        {p.endpoint}
                      </div>
                    </div>
                    <button
                      onClick={() => addPeer(p)}
                      disabled={added}
                      className={`text-xs px-2.5 py-1 rounded-md border flex items-center gap-1 ${
                        added
                          ? "bg-[var(--color-panel)] border-[var(--color-border)] text-[var(--color-text-subtle)] cursor-default"
                          : "bg-[var(--color-panel)] border-[var(--color-border)] hover:border-[var(--color-accent)]/60"
                      }`}
                    >
                      {added ? "Added" : (<><Plus size={11} /> Use</>)}
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </Section>

        {/* Workers list (host side) */}
        <Section title="Workers (this machine is the host)" icon={<Network size={15} />}>
          <div className="text-xs text-[var(--color-text-muted)] mb-2">
            Comma- or newline-separated <code>host:port</code>. Applied on the next model
            load. Leave empty to run only on this machine.
          </div>
          <textarea
            value={workersText}
            onChange={(e) => commitWorkersText(e.target.value)}
            placeholder="192.168.1.50:50052, mac-mini.local:50052"
            rows={3}
            className="w-full text-sm font-mono bg-[var(--color-panel-2)] border border-[var(--color-border)] rounded-md px-3 py-2 placeholder:text-[var(--color-text-subtle)]"
          />
          {synapse.workers.length > 0 && (
            <div className="mt-3 flex flex-wrap gap-1.5">
              {synapse.workers.map((w) => (
                <span
                  key={w}
                  className="text-xs font-mono bg-[var(--color-panel-2)] border border-[var(--color-border)] rounded-full pl-2.5 pr-1 py-0.5 flex items-center gap-1"
                >
                  {w}
                  <button
                    onClick={() => removeWorker(w)}
                    className="text-[var(--color-text-subtle)] hover:text-[var(--color-danger)] rounded-full w-4 h-4 grid place-items-center"
                    title="Remove worker"
                  >
                    ×
                  </button>
                </span>
              ))}
            </div>
          )}
        </Section>

        {/* Live logs */}
        <Section title="Live logs" icon={<Terminal size={15} />}>
          <div className="flex items-center justify-between mb-2">
            <span className="text-xs text-[var(--color-text-subtle)]">
              {logs.length}/{MAX_LOG_LINES} lines · llama-server (host) + rpc-server (worker)
            </span>
            <div className="flex items-center gap-3 text-xs text-[var(--color-text-muted)]">
              <label className="flex items-center gap-1.5 cursor-pointer">
                <input
                  type="checkbox"
                  checked={highlightsOnly}
                  onChange={(e) => setHighlightsOnly(e.target.checked)}
                  className="accent-[var(--color-accent)]"
                />
                Highlights
              </label>
              <label className="flex items-center gap-1.5 cursor-pointer">
                <input
                  type="checkbox"
                  checked={autoScroll}
                  onChange={(e) => setAutoScroll(e.target.checked)}
                  className="accent-[var(--color-accent)]"
                />
                Auto-scroll
              </label>
              <button
                onClick={() => setLogs([])}
                className="flex items-center gap-1 hover:text-[var(--color-text)]"
                title="Clear logs"
              >
                <Trash2 size={12} /> Clear
              </button>
            </div>
          </div>
          <div
            ref={logBoxRef}
            className="font-mono text-[11px] leading-[1.45] bg-black/60 border border-[var(--color-border)] rounded-md p-2.5 h-[260px] overflow-y-auto"
          >
            {filteredLogs.length === 0 ? (
              <div className="text-[var(--color-text-subtle)] italic">
                Waiting for output… Load a model to see llama-server initialize, or
                start worker mode to see rpc-server output.
              </div>
            ) : (
              filteredLogs.map((l, i) => (
                <div
                  key={i}
                  className={
                    l.stream === "stderr"
                      ? "text-[var(--color-text-muted)]"
                      : "text-[var(--color-text)]"
                  }
                >
                  <span
                    className={
                      l.source === "synapse"
                        ? "text-[var(--color-accent)]"
                        : "text-[var(--color-success)]"
                    }
                  >
                    [{l.source === "synapse" ? "rpc-server" : `llama-${l.tag ?? "server"}`}]
                  </span>{" "}
                  {l.line}
                </div>
              ))
            )}
          </div>
          <div className="mt-1.5 text-[11px] text-[var(--color-text-subtle)]">
            Highlights: lines mentioning <code>RPC</code>, <code>offload</code>,
            <code> buffer size</code>, or errors — the signals that confirm a layer
            split actually happened.
          </div>
        </Section>
      </div>
    </div>
  );
}

function Section({
  title,
  icon,
  children,
}: {
  title: string;
  icon: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="mb-6 rounded-lg border border-[var(--color-border)] bg-[var(--color-panel)] p-4">
      <div className="flex items-center gap-2 mb-3">
        <span className="text-[var(--color-text-muted)]">{icon}</span>
        <h2 className="font-semibold text-sm">{title}</h2>
      </div>
      {children}
    </div>
  );
}
