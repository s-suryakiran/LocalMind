import { describe, it, expect } from "vitest";
import { isOnline, OFFLINE_AFTER_MS } from "../online";

describe("isOnline", () => {
  it("returns true when last probe succeeded within the window", () => {
    const now = 1_000_000;
    const lastOk = now - 1_000;
    expect(isOnline({ lastOk, navigatorOnline: true }, now)).toBe(true);
  });

  it("returns false when last probe is older than the window", () => {
    const now = 1_000_000;
    const lastOk = now - (OFFLINE_AFTER_MS + 1);
    expect(isOnline({ lastOk, navigatorOnline: true }, now)).toBe(false);
  });

  it("returns false when navigator reports offline regardless of last probe", () => {
    const now = 1_000_000;
    expect(isOnline({ lastOk: now - 100, navigatorOnline: false }, now)).toBe(false);
  });

  it("returns false when no probe has ever succeeded", () => {
    expect(isOnline({ lastOk: null, navigatorOnline: true }, 1_000_000)).toBe(false);
  });
});

import { describe as describePoller, it as itPoller, expect as expectPoller, vi } from "vitest";
import { runReachabilityPoller } from "../online";

describePoller("runReachabilityPoller", () => {
  itPoller("calls onSuccess after a successful probe", async () => {
    const onSuccess = vi.fn();
    const onFailure = vi.fn();
    const fetchMock = vi.fn().mockResolvedValue({ ok: true });

    const stop = runReachabilityPoller({
      probe: () => fetchMock(),
      intervalMs: () => 10,
      onSuccess,
      onFailure,
    });

    await new Promise((r) => setTimeout(r, 30));
    stop();

    expectPoller(onSuccess).toHaveBeenCalled();
    expectPoller(onFailure).not.toHaveBeenCalled();
  });

  itPoller("calls onFailure when the probe rejects", async () => {
    const onSuccess = vi.fn();
    const onFailure = vi.fn();
    const fetchMock = vi.fn().mockRejectedValue(new Error("net down"));

    const stop = runReachabilityPoller({
      probe: () => fetchMock(),
      intervalMs: () => 10,
      onSuccess,
      onFailure,
    });

    await new Promise((r) => setTimeout(r, 30));
    stop();

    expectPoller(onFailure).toHaveBeenCalled();
    expectPoller(onSuccess).not.toHaveBeenCalled();
  });

  itPoller("stops firing after stop() is called", async () => {
    const probe = vi.fn().mockResolvedValue({ ok: true });
    const stop = runReachabilityPoller({
      probe,
      intervalMs: () => 5,
      onSuccess: () => {},
      onFailure: () => {},
    });
    await new Promise((r) => setTimeout(r, 12));
    stop();
    const callsAtStop = probe.mock.calls.length;
    await new Promise((r) => setTimeout(r, 20));
    expectPoller(probe.mock.calls.length).toBe(callsAtStop);
  });
});
