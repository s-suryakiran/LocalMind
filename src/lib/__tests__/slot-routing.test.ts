import { describe, it, expect } from "vitest";
import { pickSlotPort } from "../slot-routing";
import type { LlamaStatus, SlotStatus } from "../types";

function status(slots: Partial<SlotStatus>[]): LlamaStatus {
  return {
    running: false,
    port: 8181,
    modelId: null,
    mmprojId: null,
    pid: null,
    embeddingRunning: false,
    embeddingPort: 8182,
    embeddingModelId: null,
    slots: slots.map((s) => ({
      role: "chat",
      running: false,
      port: 8181,
      modelId: null,
      mmprojId: null,
      pid: null,
      ...s,
    })) as SlotStatus[],
  };
}

describe("pickSlotPort", () => {
  it("returns vision port when image present and vision running", () => {
    const s = status([
      { role: "chat", running: true, port: 8181 },
      { role: "vision", running: true, port: 8183 },
    ]);
    expect(pickSlotPort(s, true)).toBe(8183);
  });

  it("falls back to chat when image present but vision not running", () => {
    const s = status([{ role: "chat", running: true, port: 8181 }]);
    expect(pickSlotPort(s, true)).toBe(8181);
  });

  it("uses chat for non-image requests even when vision is running", () => {
    const s = status([
      { role: "chat", running: true, port: 8181 },
      { role: "vision", running: true, port: 8183 },
    ]);
    expect(pickSlotPort(s, false)).toBe(8181);
  });

  it("returns null when no slot can serve", () => {
    const s = status([]);
    expect(pickSlotPort(s, false)).toBeNull();
  });
});
