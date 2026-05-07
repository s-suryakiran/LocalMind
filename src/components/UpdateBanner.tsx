import { useEffect, useState } from "react";
import { RotateCw } from "lucide-react";
import { applyUpdate, onUpdateAvailable } from "../lib/sw-register";

export function UpdateBanner() {
  const [available, setAvailable] = useState(false);

  useEffect(() => onUpdateAvailable(() => setAvailable(true)), []);

  if (!available) return null;
  return (
    <div className="flex items-center justify-center gap-2 bg-emerald-500/15 text-emerald-300 text-xs py-1.5 px-3 border-b border-emerald-500/20">
      <RotateCw size={13} aria-hidden />
      <span>A new version of LocalMind is ready.</span>
      <button
        onClick={() => applyUpdate()}
        className="rounded-md bg-emerald-500/20 hover:bg-emerald-500/30 px-2 py-0.5 font-medium"
      >
        Reload
      </button>
    </div>
  );
}
