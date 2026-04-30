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
  KeyRound,
  Copy,
  Check,
  Eye,
  EyeOff,
  Pencil,
} from "lucide-react";
import { useApp } from "../lib/store";
import { api, listen, remoteSynapse } from "../lib/api";
import type {
  SynapseClusterLayout,
  SynapseMetric,
  SynapsePeer,
  SynapseRtt,
  SynapseWorker,
} from "../lib/types";
import { isTauri } from "../lib/util";
import { AddPeerDialog } from "../components/AddPeerDialog";

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
    setSynapseHostWeight,
  } = useApp();

  const [workerBusy, setWorkerBusy] = useState(false);
  const [workerError, setWorkerError] = useState<string | null>(null);
  // Workers are edited as a single textarea string so users can paste freely.
  // Phase 3 format per line: `endpoint|token` (e.g. `192.168.1.50:50052|ABC…`).
  // Bare `endpoint` is also accepted (token defaults to "") so users with
  // pre-Phase-3 muscle memory aren't blocked, but the next model load will
  // fail until they paste the real token. Chunk D replaces this textarea
  // with a proper add-peer dialog.
  const [workersText, setWorkersText] = useState(
    synapse.workers
      .map((w) => (w.token ? `${w.endpoint}|${w.token}` : w.endpoint))
      .join("\n"),
  );

  // Discovered peers via mDNS. Keyed by peer.id (the mDNS instance name) so
  // duplicates from re-resolution don't double-render.
  const [peers, setPeers] = useState<Record<string, SynapsePeer>>({});
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [highlightsOnly, setHighlightsOnly] = useState(true);
  const [autoScroll, setAutoScroll] = useState(true);
  const logBoxRef = useRef<HTMLDivElement>(null);

  // Phase 3 chunk D: this machine's worker token, shown so the user can copy
  // it onto hosts. We fetch lazily — `get_synapse_token` generates+persists
  // on first call, so the act of opening this page creates the token even
  // before worker mode is started. That way hosts can pre-pair.
  const [myToken, setMyToken] = useState<string | null>(null);
  const [tokenRevealed, setTokenRevealed] = useState(false);
  const [tokenCopied, setTokenCopied] = useState(false);
  const [tokenBusy, setTokenBusy] = useState(false);

  // Add/edit peer dialog state. `target` is null when closed; otherwise it
  // describes the worker we're configuring. `mode` decides between adding a
  // new entry vs editing an existing chip's token.
  type DialogTarget = {
    endpoint: string;
    hostname?: string;
    /** Present when editing an existing worker; absent when adding new. */
    initialToken?: string;
    /** Phase 4 chunk N: pinned cert fingerprint (when adding from a
     *  verified beacon) or pre-populated when editing a worker that
     *  already has one stored. */
    certFingerprint?: string;
  };
  const [dialogTarget, setDialogTarget] = useState<DialogTarget | null>(null);

  // Phase 3 chunk F: rolling tok/s sparkline samples + per-endpoint RTT.
  // We bound the sparkline buffer at 30 points (~30 samples spread across
  // the lifetime of one inference) and keep RTT in a flat record so the
  // worker chips can lookup by endpoint in O(1).
  const SPARK_LEN = 30;
  const [tokPerSecHistory, setTokPerSecHistory] = useState<number[]>([]);
  const [rttByEndpoint, setRttByEndpoint] = useState<Record<string, SynapseRtt>>({});
  // Phase 4 chunk I: track which workers the heartbeat has evicted. A
  // recovered event clears the entry. Stored as a Set so chip-render is
  // an O(1) `.has()`.
  const [evictedEndpoints, setEvictedEndpoints] = useState<Set<string>>(new Set());
  // Phase 4 chunk O: live count of authenticated hosts connected to *this
  // machine's* worker. Updates on every accept/disconnect via
  // `synapse:sessions`. Only meaningful when worker mode is enabled.
  const [activeSessions, setActiveSessions] = useState(0);
  // Phase 4 chunk Q: layer-distribution snapshot from the most recent
  // model load. We only display the most recent — older snapshots are
  // stale once the model is reloaded with a different split.
  const [clusterLayout, setClusterLayout] = useState<SynapseClusterLayout | null>(null);

  // Sync the worker toggle with backend reality on mount — the persisted store
  // can drift from the actual rpc-server child if the app crashed mid-run.
  // Also fetch the local worker token so hosts can copy it. This generates
  // the token on first call (idempotent thereafter), so paging through to
  // Synapse is enough to give every machine a stable identity.
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
    api.getSynapseToken().then(setMyToken).catch(() => setMyToken(null));
    // Phase 4 chunk O: cold-read the session count on mount in case the
    // page (re-)opens with hosts already connected.
    api.synapseActiveSessions().then(setActiveSessions).catch(() => {});
  }, [remote, setSynapseWorkerEnabled, setSynapseWorkerPort]);

  // Phase 3 chunk E: keep the backend's token map in sync so beacon HMACs can
  // verify in real time. Pushed on every change to synapse.workers (which
  // covers add, edit, remove, bulk-paste). Empty tokens are filtered out so
  // we don't claim to know a token we don't.
  useEffect(() => {
    if (remote) return;
    const map: Record<string, string> = {};
    for (const w of synapse.workers) {
      if (w.token) map[w.endpoint] = w.token;
    }
    api.setKnownSynapseTokens(map).catch(() => {
      // Backend not ready yet (very early boot); harmless to drop, the
      // next workers-change will retry.
    });
  }, [remote, synapse.workers]);

  // Phase 3 chunk F: live tok/s + RTT subscriptions. tok/s lands when llama-
  // server prints an eval-time block (one per generation), RTT lands every
  // 5s per active host_proxy.
  useEffect(() => {
    if (remote) return;
    let pending: Array<() => void> = [];
    let cancelled = false;

    listen<SynapseMetric>("synapse:metrics", (m) => {
      if (m.kind !== "host-tok-s") return;
      setTokPerSecHistory((prev) => {
        const next = prev.concat(m.tokPerSec);
        return next.length > SPARK_LEN ? next.slice(next.length - SPARK_LEN) : next;
      });
    }).then((un) => { if (cancelled) un(); else pending.push(un); });

    listen<SynapseRtt>("synapse:rtt", (r) => {
      setRttByEndpoint((prev) => ({ ...prev, [r.endpoint]: r }));
    }).then((un) => { if (cancelled) un(); else pending.push(un); });

    listen<{ endpoint: string }>("synapse:proxy-evicted", (e) => {
      setEvictedEndpoints((prev) => {
        if (prev.has(e.endpoint)) return prev;
        const next = new Set(prev);
        next.add(e.endpoint);
        return next;
      });
    }).then((un) => { if (cancelled) un(); else pending.push(un); });

    listen<{ endpoint: string }>("synapse:proxy-recovered", (e) => {
      setEvictedEndpoints((prev) => {
        if (!prev.has(e.endpoint)) return prev;
        const next = new Set(prev);
        next.delete(e.endpoint);
        return next;
      });
    }).then((un) => { if (cancelled) un(); else pending.push(un); });

    listen<{ count: number }>("synapse:sessions", (e) => {
      setActiveSessions(e.count);
    }).then((un) => { if (cancelled) un(); else pending.push(un); });

    listen<SynapseClusterLayout>("synapse:cluster-layout", (l) => {
      setClusterLayout(l);
    }).then((un) => { if (cancelled) un(); else pending.push(un); });

    return () => {
      cancelled = true;
      pending.forEach((un) => un());
    };
  }, [remote]);

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
    const parsed: SynapseWorker[] = text
      .split(/[,\n]/)
      .map((s) => s.trim())
      .filter(Boolean)
      .map((line) => {
        // Format: `endpoint|token`. Bare `endpoint` accepted with empty
        // token so users hitting backspace mid-paste don't lose the row
        // entirely — host_proxy will fail-fast at load time with a clear
        // "no token configured for X" error.
        const sep = line.indexOf("|");
        if (sep === -1) return { endpoint: line, token: "" };
        return {
          endpoint: line.slice(0, sep).trim(),
          token: line.slice(sep + 1).trim(),
        };
      });
    setSynapseWorkers(parsed);
  }

  // Serialize the workers list back into the textarea representation.
  // Centralized so addPeer / removeWorker stay in sync with commitWorkersText.
  function workersToText(workers: SynapseWorker[]): string {
    return workers
      .map((w) => (w.token ? `${w.endpoint}|${w.token}` : w.endpoint))
      .join("\n");
  }

  // Click "Use" on a discovered peer → opens the dialog so the user can
  // paste the worker's token before we save the entry. Adding without a
  // token is allowed (the chip flags it red) so users hitting cancel still
  // get a record of the discovery, but loads will fail until they edit.
  //
  // Phase 4 chunk N: capture the cert fingerprint from the (HMAC-signed)
  // beacon at pair time, stored alongside the token. The host's TLS
  // verifier then pins this exact fingerprint on every connect — a peer
  // at the same IP can't impersonate the worker even if it learns the
  // token.
  function openAddDialogForPeer(p: SynapsePeer) {
    if (synapse.workers.some((w) => w.endpoint === p.endpoint)) return;
    setDialogTarget({
      endpoint: p.endpoint,
      hostname: p.hostname,
      certFingerprint: p.certFingerprint,
    });
  }

  // Click an existing chip → edit dialog, pre-populated with the current
  // token so the user can fix or rotate it.
  function openEditDialogForWorker(w: SynapseWorker) {
    setDialogTarget({ endpoint: w.endpoint, initialToken: w.token });
  }

  // Confirm callback for both add + edit. Differentiates by checking whether
  // an existing worker matches the endpoint — keeps the dialog dumb.
  function handleDialogConfirm(token: string) {
    if (!dialogTarget) return;
    const existing = synapse.workers.find((w) => w.endpoint === dialogTarget.endpoint);
    let next: SynapseWorker[];
    if (existing) {
      // Edit-mode preserves whatever fingerprint was stored before; the
      // dialog only edits the token. Re-pairing fully (re-running
      // openAddDialogForPeer) is the way to refresh the fingerprint.
      next = synapse.workers.map((w) =>
        w.endpoint === dialogTarget.endpoint ? { ...w, token } : w,
      );
    } else {
      next = [
        ...synapse.workers,
        {
          endpoint: dialogTarget.endpoint,
          token,
          certFingerprint: dialogTarget.certFingerprint,
        },
      ];
    }
    setSynapseWorkers(next);
    setWorkersText(workersToText(next));
    setDialogTarget(null);
  }

  function removeWorker(endpoint: string) {
    const next = synapse.workers.filter((w) => w.endpoint !== endpoint);
    setSynapseWorkers(next);
    setWorkersText(workersToText(next));
  }

  // Phase 3 chunk G: update one worker's weight in the layer split. Stored
  // as 0–1 in the SynapseWorker type but presented as 0–100 in the UI.
  function setWorkerWeight(endpoint: string, value01: number) {
    const next = synapse.workers.map((w) =>
      w.endpoint === endpoint ? { ...w, weight: value01 } : w,
    );
    setSynapseWorkers(next);
  }

  // Reset every weight, including host's, back to "use llama.cpp default".
  function clearAllWeights() {
    const next = synapse.workers.map((w) => ({ ...w, weight: undefined }));
    setSynapseWorkers(next);
    setSynapseHostWeight(undefined);
  }

  // Phase 4 chunk L: pre-fill the layer split using each device's
  // advertised VRAM. A real benchmark-tuned split would need to load the
  // model under several configurations and measure tok/s — costly and
  // tedious. VRAM-proportional is a strong heuristic for pipeline-parallel
  // inference with similar bandwidth on each link, which is the common
  // case on a home LAN.
  //
  // Host VRAM is read from `hardware`; peer VRAM from the latest beacon
  // (cached in `peers`). Devices we can't read fall back to a 4 GB equal
  // share so they aren't accidentally weighted to zero.
  function suggestSplitFromVram() {
    const hardware = useApp.getState().hardware;
    let hostGb = 4;
    if (hardware) {
      const acc = hardware.accelerator;
      if (acc.type === "appleSilicon") hostGb = acc.unifiedMemoryGb * 0.75;
      else if (acc.type === "nvidia") hostGb = acc.vramGb;
      else if (acc.type === "amd") hostGb = acc.vramGb;
      else hostGb = Math.max(1, hardware.totalMemoryGb - 4);
    }
    // peers is keyed by mDNS instance id, but we want to look up by
    // endpoint (host:port). Build a once-per-call index so each worker
    // lookup stays O(1).
    const peerByEndpoint: Record<string, SynapsePeer> = {};
    for (const p of Object.values(peers)) peerByEndpoint[p.endpoint] = p;
    const peerGbs = synapse.workers.map((w) => peerByEndpoint[w.endpoint]?.vramGb ?? 4);
    // Normalize to the 0–1 range the SplitSlider binds to. We rescale so
    // the largest device sits at 1.0; everything else is proportional.
    // The backend re-normalizes again at --tensor-split build time, so
    // only the *ratio* between values matters for inference — the
    // absolute scale is purely cosmetic.
    const max = Math.max(hostGb, ...peerGbs, 1);
    const nextWorkers = synapse.workers.map((w, i) => ({
      ...w,
      weight: peerGbs[i] / max,
    }));
    setSynapseWorkers(nextWorkers);
    setSynapseHostWeight(hostGb / max);
  }

  // Token actions — copy + rotate. Rotation is destructive; we confirm
  // because every paired host needs to re-paste the new value.
  async function copyToken() {
    if (!myToken) return;
    try {
      await navigator.clipboard.writeText(myToken);
      setTokenCopied(true);
      setTimeout(() => setTokenCopied(false), 1500);
    } catch {
      // Some sandboxed Tauri webviews block clipboard. Show a fallback
      // selection by revealing the token if it isn't already.
      setTokenRevealed(true);
    }
  }

  async function rotateToken() {
    if (!confirm(
      "Rotate this worker's token? Every host that was previously paired " +
      "with this machine will need to re-paste the new token before its " +
      "next model load.",
    )) {
      return;
    }
    setTokenBusy(true);
    try {
      const fresh = await api.rotateSynapseToken();
      setMyToken(fresh);
      setTokenRevealed(true);
    } catch (e) {
      // Don't blow up — the old token is still valid if rotation failed
      // (e.g. read-only data dir).
      console.error("rotate token", e);
    } finally {
      setTokenBusy(false);
    }
  }

  const peerList = useMemo(() => Object.values(peers), [peers]);
  const filteredLogs = useMemo(
    () => (highlightsOnly ? logs.filter((l) => HIGHLIGHT_RE.test(l.line)) : logs),
    [logs, highlightsOnly],
  );

  if (remote) {
    // Phone/PWA — Phase 4 chunk P: read-only Synapse viewer. Fetches the
    // desktop's worker status and discovered peers via the LAN API. We
    // can't drive worker mode or edit tokens from here (desktop-only),
    // but the diagnostic surface is identical.
    return <RemoteSynapseView />;
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
                  <div className="text-sm font-medium flex items-center gap-2">
                    Run as a Synapse worker
                    {/* Phase 4 chunk O: live "N hosts connected" pill.
                        Only shown when the worker is on AND at least one
                        host is actually paired-and-active — a 0-count
                        running worker is the steady-state idle case and
                        the badge would just be visual noise. */}
                    {synapse.workerEnabled && activeSessions > 0 && (
                      <span
                        title="Authenticated hosts currently connected"
                        className="text-[10px] uppercase tracking-wider text-[var(--color-success)] bg-[var(--color-success)]/10 border border-[var(--color-success)]/40 rounded-full px-2 py-0.5"
                      >
                        {activeSessions} host{activeSessions === 1 ? "" : "s"}
                      </span>
                    )}
                  </div>
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

          {/* Worker token — shown so the user can copy onto hosts. The token
              exists even before the worker is started; load_or_create spins
              one up on first read. We hide by default so screenshots /
              shoulder-surfers don't accidentally leak it. */}
          <div className="mt-4 pt-4 border-t border-[var(--color-border-soft)]">
            <div className="flex items-center justify-between mb-1.5">
              <div className="flex items-center gap-2 text-sm font-medium">
                <KeyRound size={13} className="text-[var(--color-accent)]" />
                Worker token
              </div>
              <span className="text-[11px] text-[var(--color-text-subtle)]">
                Hosts paste this once, per worker
              </span>
            </div>
            <div className="flex items-center gap-2 rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2">
              <code className="flex-1 text-[13px] font-mono truncate select-all">
                {myToken
                  ? tokenRevealed
                    ? myToken
                    : "•".repeat(Math.min(myToken.length, 52))
                  : "loading…"}
              </code>
              <button
                onClick={() => setTokenRevealed((v) => !v)}
                className="text-[var(--color-text-muted)] hover:text-[var(--color-text)]"
                title={tokenRevealed ? "Hide" : "Reveal"}
              >
                {tokenRevealed ? <EyeOff size={14} /> : <Eye size={14} />}
              </button>
              <button
                onClick={copyToken}
                className="text-[var(--color-text-muted)] hover:text-[var(--color-text)]"
                title="Copy"
                disabled={!myToken}
              >
                {tokenCopied ? (
                  <Check size={14} className="text-[var(--color-success)]" />
                ) : (
                  <Copy size={14} />
                )}
              </button>
              <button
                onClick={rotateToken}
                disabled={tokenBusy || !myToken}
                className="text-[var(--color-text-muted)] hover:text-[var(--color-danger)] disabled:opacity-50"
                title="Rotate (paired hosts will need the new token)"
              >
                <RefreshCw size={14} className={tokenBusy ? "animate-spin" : ""} />
              </button>
            </div>
            <div className="text-[11px] text-[var(--color-text-subtle)] mt-1.5 leading-snug">
              The token authenticates connections to this worker. Anyone who has it can
              run inference on your hardware — share carefully.
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
                const added = synapse.workers.some((w) => w.endpoint === p.endpoint);
                return (
                  <li
                    key={p.id}
                    className="flex items-center gap-3 rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2"
                  >
                    <div className="w-1.5 h-1.5 rounded-full bg-[var(--color-success)]" />
                    <div className="flex-1 min-w-0">
                      <div className="text-sm font-medium truncate flex items-center gap-1.5">
                        {p.hostname}
                        {/* HMAC-verified beacon: green check.
                            No token yet:           amber warning.
                            We use the icon-only treatment to keep the row tight. */}
                        {p.verified ? (
                          <span
                            title="Beacon verified — token matches"
                            className="text-[var(--color-success)] flex items-center"
                          >
                            <Check size={12} />
                          </span>
                        ) : (
                          <span
                            title="Unverified — add the worker token to authenticate"
                            className="text-[var(--color-text-subtle)] text-[10px] uppercase tracking-wider"
                          >
                            unverified
                          </span>
                        )}
                      </div>
                      <div className="text-xs text-[var(--color-text-muted)] font-mono truncate">
                        {p.endpoint}
                      </div>
                      {/* Phase 4 chunk J: hardware blurb. We show it when
                          available because users picking between two peers
                          on the same LAN almost always want to know which
                          has more VRAM. Hidden gracefully on pre-Phase-4
                          workers where the fields are undefined. */}
                      {(p.acceleratorName || p.vramGb) && (
                        <div className="text-[11px] text-[var(--color-text-subtle)] truncate">
                          {p.acceleratorName ?? "unknown"}
                          {p.vramGb ? ` · ${p.vramGb.toFixed(1)} GB` : ""}
                        </div>
                      )}
                    </div>
                    <button
                      onClick={() => openAddDialogForPeer(p)}
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
          <div className="flex items-start justify-between mb-2 gap-3">
            <div className="text-xs text-[var(--color-text-muted)] flex-1">
              Each entry needs the worker's token. Use <Plus size={11} className="inline -mt-0.5" /> on
              a discovered peer above, or <span className="text-[var(--color-text)]">Add manually</span> for
              workers that aren't broadcasting. Applied on the next model load.
            </div>
            <button
              onClick={() => {
                const endpoint = prompt(
                  "Worker endpoint (host:port)",
                  "192.168.1.50:50052",
                );
                if (!endpoint) return;
                const trimmed = endpoint.trim();
                if (!trimmed) return;
                if (synapse.workers.some((w) => w.endpoint === trimmed)) return;
                setDialogTarget({ endpoint: trimmed });
              }}
              className="text-xs px-2.5 py-1 rounded-md bg-[var(--color-panel-2)] border border-[var(--color-border)] hover:border-[var(--color-accent)]/60 flex items-center gap-1 shrink-0"
            >
              <Plus size={11} /> Add manually
            </button>
          </div>

          {synapse.workers.length === 0 ? (
            <div className="text-xs text-[var(--color-text-subtle)] py-3">
              No workers configured — this machine will run models on its own hardware.
            </div>
          ) : (
            <div className="flex flex-wrap gap-1.5">
              {synapse.workers.map((w) => {
                const missingToken = !w.token;
                const rtt = rttByEndpoint[w.endpoint];
                const evicted = evictedEndpoints.has(w.endpoint);
                // RTT colour bands match common networking expectations:
                //   <50 ms green (LAN healthy), 50–200 ms amber (Wi-Fi
                //   contention), >200 ms red (VPN / WAN / dropping).
                let rttClass = "text-[var(--color-text-subtle)]";
                if (rtt && rtt.ok) {
                  if (rtt.rttMs < 50) rttClass = "text-[var(--color-success)]";
                  else if (rtt.rttMs < 200) rttClass = "text-yellow-400";
                  else rttClass = "text-[var(--color-danger)]";
                } else if (rtt && !rtt.ok) {
                  rttClass = "text-[var(--color-danger)]";
                }
                return (
                  <span
                    key={w.endpoint}
                    title={
                      evicted
                        ? "Worker unreachable — heartbeat failed. Will auto-recover when the worker comes back."
                        : missingToken
                          ? "No token — model load will fail. Click ✎ to add it."
                          : "Click ✎ to update the token"
                    }
                    className={`text-xs font-mono border rounded-full pl-2.5 pr-1 py-0.5 flex items-center gap-1 ${
                      evicted
                        ? "bg-[var(--color-panel-2)] border-[var(--color-border)] opacity-60"
                        : missingToken
                          ? "bg-[var(--color-danger)]/10 border-[var(--color-danger)]/40"
                          : "bg-[var(--color-panel-2)] border-[var(--color-border)]"
                    }`}
                  >
                    {w.endpoint}
                    {evicted && (
                      <span className="text-[var(--color-danger)] text-[10px] uppercase tracking-wider">
                        offline
                      </span>
                    )}
                    {!evicted && rtt && (
                      <span className={`text-[10px] ${rttClass}`}>
                        {rtt.ok ? `${rtt.rttMs}ms` : "drop"}
                      </span>
                    )}
                    {missingToken && !evicted && (
                      <span className="text-[var(--color-danger)] text-[10px] uppercase tracking-wider">
                        no token
                      </span>
                    )}
                    <button
                      onClick={() => openEditDialogForWorker(w)}
                      className="text-[var(--color-text-subtle)] hover:text-[var(--color-text)] rounded-full w-4 h-4 grid place-items-center"
                      title="Edit token"
                    >
                      <Pencil size={10} />
                    </button>
                    <button
                      onClick={() => removeWorker(w.endpoint)}
                      className="text-[var(--color-text-subtle)] hover:text-[var(--color-danger)] rounded-full w-4 h-4 grid place-items-center"
                      title="Remove worker"
                    >
                      ×
                    </button>
                  </span>
                );
              })}
            </div>
          )}

          {/* Phase 4 chunk Q: cluster layout. Stacked-bar visualization of
              which device got how much model. Only renders after a model
              load completes (we get the snapshot from llama-server's
              "load_tensors: <DEVICE> model buffer size = …" lines). */}
          {clusterLayout && clusterLayout.devices.length > 0 && (
            <div className="mt-4">
              <div className="flex items-center justify-between text-xs text-[var(--color-text-muted)] mb-1.5">
                <span>Layer distribution (last load)</span>
                <span className="text-[var(--color-text-subtle)]">
                  {(clusterLayout.devices.reduce((a, b) => a + b.mb, 0) / 1024).toFixed(1)} GB total
                </span>
              </div>
              <ClusterLayoutBar layout={clusterLayout} />
            </div>
          )}

          {/* Inference throughput readout. Only visible after llama-server has
              actually completed at least one generation (we have data). The
              sparkline gives shape to the recent trend at a glance. */}
          {tokPerSecHistory.length > 0 && (
            <div className="mt-4 flex items-center gap-3 rounded-md bg-[var(--color-panel-2)] border border-[var(--color-border)] px-3 py-2">
              <div className="flex flex-col">
                <span className="text-[10px] uppercase tracking-wider text-[var(--color-text-subtle)]">
                  Throughput
                </span>
                <span className="text-sm font-mono">
                  {tokPerSecHistory[tokPerSecHistory.length - 1].toFixed(1)} tok/s
                </span>
              </div>
              <Sparkline values={tokPerSecHistory} />
              <span className="text-[11px] text-[var(--color-text-subtle)] ml-auto">
                last {tokPerSecHistory.length} generations
              </span>
            </div>
          )}

          {/* Layer split (--tensor-split). Lets users override llama.cpp's
              even-split default — important when host and worker have very
              different compute (e.g. M1 Pro host + 4090 worker → most layers
              should land on the 4090). Hidden in <details> so the common
              case (default split) doesn't get cluttered with sliders. */}
          {synapse.workers.length > 0 && (() => {
            // Compute display percentages from the explicit weights. Sliders
            // bind to raw 0–100 values; we do the normalization to a sum of
            // 1.0 in the backend when building --tensor-split.
            const hostW = synapse.hostWeight ?? 0;
            const workerWs = synapse.workers.map((w) => w.weight ?? 0);
            const total = hostW + workerWs.reduce((a, b) => a + b, 0);
            const pct = (v: number) =>
              total > 0 ? Math.round((v / total) * 100) : 0;
            const anyExplicit =
              synapse.hostWeight !== undefined ||
              synapse.workers.some((w) => w.weight !== undefined);
            return (
              <details className="mt-4 group" open={anyExplicit}>
                <summary className="text-xs text-[var(--color-text-muted)] cursor-pointer select-none hover:text-[var(--color-text)] flex items-center justify-between">
                  <span>
                    Layer split{" "}
                    <span className="text-[var(--color-text-subtle)]">
                      ({anyExplicit ? "custom" : "auto"})
                    </span>
                  </span>
                  <span className="flex items-center gap-3">
                    <button
                      onClick={(e) => {
                        e.preventDefault();
                        suggestSplitFromVram();
                      }}
                      className="text-[10px] uppercase tracking-wider text-[var(--color-accent)] hover:text-[var(--color-text)]"
                      title="Pre-fill weights from each device's VRAM"
                    >
                      suggest
                    </button>
                    {anyExplicit && (
                      <button
                        onClick={(e) => {
                          e.preventDefault();
                          clearAllWeights();
                        }}
                        className="text-[10px] uppercase tracking-wider text-[var(--color-text-subtle)] hover:text-[var(--color-text)]"
                      >
                        reset
                      </button>
                    )}
                  </span>
                </summary>
                <div className="mt-2 flex flex-col gap-2">
                  <SplitSlider
                    label="This machine (host)"
                    value01={synapse.hostWeight ?? 0.5}
                    pct={anyExplicit ? pct(hostW) : null}
                    onChange={(v) => setSynapseHostWeight(v)}
                  />
                  {synapse.workers.map((w, i) => (
                    <SplitSlider
                      key={w.endpoint}
                      label={w.endpoint}
                      value01={w.weight ?? 0.5}
                      pct={anyExplicit ? pct(workerWs[i]) : null}
                      onChange={(v) => setWorkerWeight(w.endpoint, v)}
                    />
                  ))}
                  <div className="text-[11px] text-[var(--color-text-subtle)] mt-1 leading-snug">
                    Higher = more layers on that device. Values are normalized
                    to a fraction; the actual layer count is rounded to the
                    nearest integer at load time. Reset to fall back to
                    llama.cpp's default heuristic.
                  </div>
                </div>
              </details>
            );
          })()}

          {/* Power-user textarea: bulk paste in `endpoint|token` format. Kept
              for discoverability of the format and for hosts that want to
              import many workers at once. The chips above are the primary
              UI; this is the escape hatch. */}
          <details className="mt-4 group">
            <summary className="text-xs text-[var(--color-text-muted)] cursor-pointer select-none hover:text-[var(--color-text)]">
              Bulk import (endpoint|token per line)
            </summary>
            <textarea
              value={workersText}
              onChange={(e) => commitWorkersText(e.target.value)}
              placeholder={"192.168.1.50:50052|ABCD…\nmac-mini.local:50052|EFGH…"}
              rows={3}
              className="mt-2 w-full text-sm font-mono bg-[var(--color-panel-2)] border border-[var(--color-border)] rounded-md px-3 py-2 placeholder:text-[var(--color-text-subtle)]"
            />
          </details>
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

      <AddPeerDialog
        open={dialogTarget !== null}
        endpoint={dialogTarget?.endpoint ?? ""}
        hostname={dialogTarget?.hostname}
        initialToken={dialogTarget?.initialToken}
        onCancel={() => setDialogTarget(null)}
        onConfirm={handleDialogConfirm}
      />
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

/// One row of the layer-split editor: label + slider + computed %. The slider
/// binds to a raw 0–1 weight; normalization to a sum of 1 happens in the
/// backend so the UI doesn't have to coordinate state across N rows.
function SplitSlider({
  label,
  value01,
  pct,
  onChange,
}: {
  label: string;
  value01: number;
  pct: number | null;
  onChange: (v: number) => void;
}) {
  return (
    <label className="grid grid-cols-[1fr_auto] items-center gap-2 text-xs">
      <div className="flex items-center justify-between gap-2 min-w-0">
        <span className="font-mono truncate">{label}</span>
        <span className="text-[var(--color-text-subtle)] tabular-nums shrink-0">
          {pct === null ? "auto" : `${pct}%`}
        </span>
      </div>
      <input
        type="range"
        min={0}
        max={1}
        step={0.01}
        value={value01}
        onChange={(e) => onChange(parseFloat(e.target.value))}
        className="w-[200px] accent-[var(--color-accent)]"
      />
    </label>
  );
}

/// Phase 4 chunk Q: stacked-bar visualization of model-layer distribution
/// across host + RPC workers. We bucket by exact device label so a layout
/// like `CUDA0 + RPC[127.0.0.1:54712] + CPU` shows three segments. Colour
/// rotates through a small palette — the goal is "you can see the relative
/// sizes" not "every device has its own brand colour."
function ClusterLayoutBar({ layout }: { layout: SynapseClusterLayout }) {
  const total = layout.devices.reduce((a, b) => a + b.mb, 0);
  if (total === 0) return null;
  // Tailwind-friendly palette. Order matches eye order in a typical
  // pipeline split — host first, then workers.
  const palette = [
    "bg-[var(--color-accent)]",
    "bg-[var(--color-success)]",
    "bg-yellow-400",
    "bg-purple-400",
    "bg-pink-400",
  ];
  return (
    <div>
      <div className="flex h-3 rounded overflow-hidden border border-[var(--color-border)]">
        {layout.devices.map((d, i) => (
          <div
            key={`${d.device}-${i}`}
            title={`${d.device}: ${(d.mb / 1024).toFixed(2)} GB`}
            className={palette[i % palette.length]}
            style={{ width: `${(d.mb / total) * 100}%` }}
          />
        ))}
      </div>
      <div className="mt-1.5 flex flex-wrap gap-x-3 gap-y-0.5 text-[10px] font-mono text-[var(--color-text-muted)]">
        {layout.devices.map((d, i) => (
          <span key={`${d.device}-${i}-legend`} className="flex items-center gap-1">
            <span
              aria-hidden
              className={`inline-block w-2 h-2 rounded-sm ${palette[i % palette.length]}`}
            />
            {d.device} · {(d.mb / 1024).toFixed(2)} GB ({Math.round((d.mb / total) * 100)}%)
          </span>
        ))}
      </div>
    </div>
  );
}

/// Tiny inline SVG sparkline. Avoids a charting dep — we only need a
/// 30-point line for the tok/s feed. Auto-scales y to the visible range.
function Sparkline({
  values,
  width = 120,
  height = 28,
}: {
  values: number[];
  width?: number;
  height?: number;
}) {
  if (values.length < 2) {
    return (
      <div
        style={{ width, height }}
        className="grid place-items-center text-[10px] text-[var(--color-text-subtle)]"
      >
        waiting…
      </div>
    );
  }
  const min = Math.min(...values);
  const max = Math.max(...values);
  // Pad so a flat line still has visible thickness.
  const pad = max === min ? 1 : (max - min) * 0.1;
  const lo = min - pad;
  const hi = max + pad;
  const span = hi - lo;
  const dx = width / (values.length - 1);
  const points = values
    .map((v, i) => {
      const x = i * dx;
      const y = height - ((v - lo) / span) * height;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  return (
    <svg width={width} height={height} className="block">
      <polyline
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
        points={points}
        className="text-[var(--color-accent)]"
      />
    </svg>
  );
}

/// Phase 4 chunk P: read-only Synapse viewer for the phone PWA. Polls the
/// desktop's `/api/synapse/*` endpoints every 5 s — no event subscription
/// because the LAN server doesn't currently expose SSE/WebSocket for
/// these and 5 s is plenty for the diagnostic use case.
function RemoteSynapseView() {
  const [workerStatus, setWorkerStatus] = useState<{
    running: boolean;
    port: number;
    pid: number | null;
  } | null>(null);
  const [peers, setPeers] = useState<SynapsePeer[]>([]);
  const [sessions, setSessions] = useState(0);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let stopped = false;
    async function refresh() {
      try {
        const [s, p, sess] = await Promise.all([
          remoteSynapse.status(),
          remoteSynapse.peers(),
          remoteSynapse.sessions(),
        ]);
        if (stopped) return;
        setWorkerStatus(s);
        setPeers(p);
        setSessions(sess.count);
        setError(null);
      } catch (e: unknown) {
        if (stopped) return;
        const msg = e instanceof Error ? e.message : String(e);
        setError(msg);
      }
    }
    refresh();
    const id = setInterval(refresh, 5000);
    return () => {
      stopped = true;
      clearInterval(id);
    };
  }, []);

  return (
    <div className="flex flex-col h-full">
      <header className="px-5 py-4 border-b border-[var(--color-border-soft)]">
        <h1 className="font-semibold text-[17px] flex items-center gap-2">
          <Network size={16} /> Synapse
          <span className="text-[10px] uppercase tracking-wider text-[var(--color-text-subtle)] bg-[var(--color-panel-2)] border border-[var(--color-border)] rounded-full px-2 py-0.5">
            view-only
          </span>
        </h1>
        <p className="text-[var(--color-text-muted)] text-sm">
          Live status of the paired desktop's worker and discovered peers.
          Editing happens on the desktop.
        </p>
      </header>

      <div className="flex-1 overflow-y-auto px-5 py-5 max-w-3xl">
        {error && (
          <div className="mb-4 text-xs text-[var(--color-danger)] rounded-md border border-[var(--color-danger)]/40 bg-[var(--color-danger)]/10 px-3 py-2">
            Couldn't reach desktop: {error}
          </div>
        )}

        <div className="mb-6 rounded-lg border border-[var(--color-border)] bg-[var(--color-panel)] p-4">
          <div className="flex items-center gap-2 mb-3">
            <Server size={15} className="text-[var(--color-text-muted)]" />
            <h2 className="font-semibold text-sm">Worker on desktop</h2>
          </div>
          {workerStatus ? (
            <div className="grid grid-cols-2 gap-y-2 text-sm">
              <div className="text-[var(--color-text-muted)]">Status</div>
              <div className="text-right">
                {workerStatus.running ? (
                  <span className="text-[var(--color-success)]">running on :{workerStatus.port}</span>
                ) : (
                  <span className="text-[var(--color-text-subtle)]">idle</span>
                )}
              </div>
              <div className="text-[var(--color-text-muted)]">PID</div>
              <div className="text-right font-mono">{workerStatus.pid ?? "—"}</div>
              <div className="text-[var(--color-text-muted)]">Active hosts</div>
              <div className="text-right">{sessions}</div>
            </div>
          ) : (
            <p className="text-sm text-[var(--color-text-muted)]">Loading…</p>
          )}
        </div>

        <div className="mb-6 rounded-lg border border-[var(--color-border)] bg-[var(--color-panel)] p-4">
          <div className="flex items-center gap-2 mb-3">
            <Radio size={15} className="text-[var(--color-text-muted)]" />
            <h2 className="font-semibold text-sm">Discovered on LAN ({peers.length})</h2>
          </div>
          {peers.length === 0 ? (
            <p className="text-sm text-[var(--color-text-subtle)]">No peers visible from the desktop.</p>
          ) : (
            <ul className="flex flex-col gap-1.5">
              {peers.map((p) => (
                <li
                  key={p.id}
                  className="flex items-center gap-3 rounded-md border border-[var(--color-border)] bg-[var(--color-panel-2)] px-3 py-2"
                >
                  <div className="w-1.5 h-1.5 rounded-full bg-[var(--color-success)]" />
                  <div className="flex-1 min-w-0">
                    <div className="text-sm font-medium truncate flex items-center gap-1.5">
                      {p.hostname}
                      {p.verified ? (
                        <span title="HMAC verified" className="text-[var(--color-success)]">
                          <Check size={12} />
                        </span>
                      ) : (
                        <span className="text-[10px] uppercase tracking-wider text-[var(--color-text-subtle)]">
                          unverified
                        </span>
                      )}
                    </div>
                    <div className="text-xs text-[var(--color-text-muted)] font-mono truncate">
                      {p.endpoint}
                    </div>
                    {(p.acceleratorName || p.vramGb) && (
                      <div className="text-[11px] text-[var(--color-text-subtle)] truncate">
                        {p.acceleratorName ?? "unknown"}
                        {p.vramGb ? ` · ${p.vramGb.toFixed(1)} GB` : ""}
                      </div>
                    )}
                  </div>
                </li>
              ))}
            </ul>
          )}
        </div>

        <p className="text-[11px] text-[var(--color-text-subtle)] text-center">
          Refreshes every 5 s · open the desktop app to manage peers
        </p>
      </div>
    </div>
  );
}
