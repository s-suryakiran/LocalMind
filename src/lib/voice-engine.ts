import type { VoiceEngineKind } from "./types";

export interface PickEngineInput {
  isTauri: boolean;
  override: VoiceEngineKind | null;
}

/** Picks a voice engine. Pure function — Settings UI feeds the override. */
export function pickEngine(i: PickEngineInput): VoiceEngineKind {
  if (i.override) return i.override;
  return i.isTauri ? "sherpa" : "web-speech";
}
