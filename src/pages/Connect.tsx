import { useEffect, useState } from "react";
import { Loader2, Wifi, KeyRound } from "lucide-react";
import { pairWithServer, probeServer } from "../lib/api";
import { useApp } from "../lib/store";

/**
 * First-launch / unpaired view shown in the phone PWA. The user enters the
 * desktop's LAN URL and the 6-digit PIN displayed on the desktop's Settings
 * page; we exchange those for a long-lived bearer token and store it.
 */
export function Connect() {
  const { setConnection } = useApp();
  const [url, setUrl] = useState(() => {
    // Default to the origin we were loaded from — the common case is the user
    // opens the LAN URL in mobile Safari, so the host is already correct.
    if (typeof location !== "undefined" && location.origin && /^https?:/.test(location.origin)) {
      return location.origin;
    }
    return "";
  });
  const [pin, setPin] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Re-render when the browser's online/offline status flips so the
  // offline hint below updates without waiting for the user to type.
  const [, force] = useState(0);
  useEffect(() => {
    if (typeof window === "undefined") return;
    const onChange = () => force((n) => n + 1);
    window.addEventListener("online", onChange);
    window.addEventListener("offline", onChange);
    return () => {
      window.removeEventListener("online", onChange);
      window.removeEventListener("offline", onChange);
    };
  }, []);

  async function connect(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    if (!url || !pin) return;
    setBusy(true);
    try {
      const cleanUrl = url.trim().replace(/\/+$/, "");
      if (!(await probeServer(cleanUrl))) {
        throw new Error("Could not reach that URL. Check it's the address shown on the desktop and that you're on the same Wi-Fi.");
      }
      const { token } = await pairWithServer(cleanUrl, pin.trim());
      setConnection({ url: cleanUrl, token });
    } catch (err: any) {
      setError(err?.message ?? String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="safe-top safe-bottom h-screen w-screen flex items-center justify-center bg-[var(--color-bg)] px-5">
      <div className="w-full max-w-sm">
        <div className="flex flex-col items-center mb-6">
          <div className="w-14 h-14 rounded-2xl gradient-accent grid place-items-center mb-3">
            <span className="text-white font-bold text-xl">L</span>
          </div>
          <h1 className="text-xl font-semibold">Connect to LocalMind</h1>
          <p className="text-[var(--color-text-muted)] text-sm text-center mt-1">
            Enter the address and PIN shown on the desktop app's Settings page.
          </p>
        </div>

        <form onSubmit={connect} className="flex flex-col gap-3">
          <label className="flex items-center gap-2 rounded-lg border border-[var(--color-border)] bg-[var(--color-panel)] px-3 py-2.5 focus-within:border-[var(--color-accent)]/60">
            <Wifi size={15} className="text-[var(--color-text-muted)]" />
            <input
              type="url"
              inputMode="url"
              autoComplete="off"
              autoCapitalize="off"
              spellCheck={false}
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="http://192.168.1.x:3939"
              className="flex-1 bg-transparent outline-none text-sm"
            />
          </label>
          <label className="flex items-center gap-2 rounded-lg border border-[var(--color-border)] bg-[var(--color-panel)] px-3 py-2.5 focus-within:border-[var(--color-accent)]/60">
            <KeyRound size={15} className="text-[var(--color-text-muted)]" />
            <input
              type="text"
              inputMode="numeric"
              autoComplete="one-time-code"
              maxLength={6}
              value={pin}
              onChange={(e) => setPin(e.target.value.replace(/\D/g, ""))}
              placeholder="6-digit PIN"
              className="flex-1 bg-transparent outline-none text-sm tracking-[0.4em] font-mono"
            />
          </label>
          {typeof navigator !== "undefined" && !navigator.onLine && (
            <p className="text-xs text-amber-300 bg-amber-500/10 rounded-md px-3 py-2">
              Your phone says it's offline — connect to the same Wi-Fi as the desktop, then try again.
            </p>
          )}
          {error && (
            <p className="text-xs text-[var(--color-danger)] bg-[var(--color-danger)]/10 rounded-md px-3 py-2">{error}</p>
          )}
          <button
            type="submit"
            disabled={busy || !url || pin.length !== 6}
            className="rounded-lg gradient-accent text-white text-sm font-medium py-2.5 disabled:opacity-40 flex items-center justify-center gap-2"
          >
            {busy && <Loader2 size={14} className="animate-spin" />}
            Connect
          </button>
        </form>

        <p className="text-[11px] text-[var(--color-text-subtle)] text-center mt-5">
          The PIN is shown only on the desktop. The token is stored on this device and is sent over your local Wi-Fi.
        </p>
      </div>
    </div>
  );
}
