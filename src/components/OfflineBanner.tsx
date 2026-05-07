import { WifiOff } from "lucide-react";
import { useApp } from "../lib/store";

/**
 * One-line banner shown above the main view when the LocalMind host
 * isn't reachable. The reachability poller in App.tsx flips
 * `useApp().online` based on /api/health probes.
 */
export function OfflineBanner() {
  const online = useApp((s) => s.online);
  if (online) return null;
  return (
    <div
      role="status"
      aria-live="polite"
      className="flex items-center justify-center gap-2 bg-amber-500/15 text-amber-300 text-xs py-1.5 px-3 border-b border-amber-500/20"
    >
      <WifiOff size={13} aria-hidden />
      <span>Host offline — chat is read-only until your computer is reachable.</span>
    </div>
  );
}
