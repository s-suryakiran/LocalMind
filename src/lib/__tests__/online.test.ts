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
