/**
 * Online-state machinery. We don't trust `navigator.onLine` alone —
 * iOS Safari leaves it `true` even on a captive-portal Wi-Fi where every
 * actual fetch fails, and Tauri's webview reports stale values during
 * suspend/resume. The truth source is "did a probe to the LocalMind
 * host succeed in the last N seconds."
 */

/** How long after a successful probe we still consider ourselves online. */
export const OFFLINE_AFTER_MS = 15_000;

/** How often the poller fires when we believe we're online. */
export const ONLINE_POLL_MS = 10_000;

/** How often the poller fires when we believe we're offline (back off). */
export const OFFLINE_POLL_MS = 5_000;

export interface OnlineSignal {
  /** Timestamp (ms since epoch) of the last successful probe, or null if never. */
  lastOk: number | null;
  /** Browser's hint via `navigator.onLine`. */
  navigatorOnline: boolean;
}

export function isOnline(s: OnlineSignal, now: number): boolean {
  if (!s.navigatorOnline) return false;
  if (s.lastOk == null) return false;
  return now - s.lastOk <= OFFLINE_AFTER_MS;
}

export interface PollerOptions {
  /** Async probe — should resolve when reachable, throw or return falsy `ok` when not. */
  probe: () => Promise<{ ok: boolean } | Response>;
  /** Returns the next delay in ms. Lets the caller back off when offline. */
  intervalMs: () => number;
  onSuccess: () => void;
  onFailure: (err: unknown) => void;
}

export function runReachabilityPoller(opts: PollerOptions): () => void {
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | null = null;

  const tick = async () => {
    if (stopped) return;
    try {
      const res = await opts.probe();
      if (stopped) return;
      if ((res as { ok: boolean }).ok) opts.onSuccess();
      else opts.onFailure(new Error(`probe responded not ok`));
    } catch (e) {
      if (!stopped) opts.onFailure(e);
    }
    if (stopped) return;
    timer = setTimeout(tick, opts.intervalMs());
  };

  timer = setTimeout(tick, opts.intervalMs());

  return () => {
    stopped = true;
    if (timer != null) {
      clearTimeout(timer);
      timer = null;
    }
  };
}
