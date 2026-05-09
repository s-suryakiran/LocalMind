import { describe, it, expect } from "vitest";
import { pickEngine } from "../voice-engine";

describe("pickEngine", () => {
  it("uses sherpa on Tauri when not forced", () => {
    expect(pickEngine({ isTauri: true, override: null })).toBe("sherpa");
  });

  it("uses web-speech in PWA", () => {
    expect(pickEngine({ isTauri: false, override: null })).toBe("web-speech");
  });

  it("respects user override over auto-detect", () => {
    expect(pickEngine({ isTauri: true, override: "web-speech" })).toBe("web-speech");
    expect(pickEngine({ isTauri: false, override: "sherpa" })).toBe("sherpa");
  });
});
