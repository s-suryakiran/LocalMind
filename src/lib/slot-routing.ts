import type { LlamaStatus } from "./types";

/**
 * Mirrors the Rust-side route_chat_port logic. Image-bearing requests
 * prefer the vision slot when loaded; otherwise (or for text-only
 * requests) they go to the chat slot. Returns null if no slot can serve.
 */
export function pickSlotPort(status: LlamaStatus, hasImage: boolean): number | null {
  const find = (role: "chat" | "vision") =>
    status.slots.find((s) => s.role === role && s.running) ?? null;
  if (hasImage) {
    const v = find("vision");
    if (v) return v.port;
  }
  const c = find("chat");
  return c ? c.port : null;
}

/**
 * Inspect a chat-completions body to decide if it has any image content.
 * Used by the desktop direct-call path; the LAN proxy path uses a
 * server-side equivalent in server.rs.
 */
export function bodyHasImage(messages: { content: unknown }[]): boolean {
  for (const m of messages) {
    if (typeof m.content === "string") continue;
    if (Array.isArray(m.content)) {
      for (const part of m.content) {
        if (part && typeof part === "object" && (part as { type?: string }).type === "image_url") {
          return true;
        }
      }
    }
  }
  return false;
}
