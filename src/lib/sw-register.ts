/**
 * Wraps `vite-plugin-pwa`'s registerSW so the rest of the app sees a stable
 * surface (no virtual: imports leaking into components). `prompt` mode
 * means the new SW sits in `waiting` until we explicitly call `update()`,
 * giving us a chance to ask the user before swapping the bundle.
 */
import { registerSW } from "virtual:pwa-register";
import { isTauri } from "./util";

let updateFn: ((reload?: boolean) => Promise<void>) | null = null;
const updateListeners = new Set<() => void>();

export function initServiceWorker() {
  // No-op in Tauri (the desktop app doesn't run as a PWA in the same
  // sense — webview owns the lifecycle). Avoid double-installing.
  if (isTauri()) return;

  updateFn = registerSW({
    onNeedRefresh() {
      updateListeners.forEach((cb) => cb());
    },
    onOfflineReady() {
      // First install succeeded — fine to silently log.
      console.info("[sw] offline shell ready");
    },
  });
}

export function onUpdateAvailable(cb: () => void): () => void {
  updateListeners.add(cb);
  return () => updateListeners.delete(cb);
}

export async function applyUpdate() {
  if (!updateFn) return;
  await updateFn(true);
}
