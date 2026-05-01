import { useEffect, useRef, useState } from "react";
import { KeyRound, X } from "lucide-react";

// Phase 3 chunk D: modal that captures the worker token alongside the
// endpoint when the user adds a peer (manually or from discovery). The same
// modal handles "edit existing worker token" — we just preload `initialToken`
// and rename the title.
//
// Kept in components/ rather than inline in Synapse.tsx because the Synapse
// page is already the longest file in the app and three call sites need
// this dialog (add via discovery, add manually, edit existing chip).

export interface AddPeerDialogProps {
  /** Visible when truthy. Always pass the endpoint — the dialog otherwise
   *  has no context for what the user is configuring. */
  open: boolean;
  endpoint: string;
  /** Optional friendly name (mDNS hostname). Falls back to endpoint. */
  hostname?: string;
  /** When supplied, dialog runs in "edit" mode: title says "Update token"
   *  and the field starts populated. */
  initialToken?: string;
  onCancel: () => void;
  onConfirm: (token: string) => void;
}

export function AddPeerDialog({
  open,
  endpoint,
  hostname,
  initialToken,
  onCancel,
  onConfirm,
}: AddPeerDialogProps) {
  const [token, setToken] = useState(initialToken ?? "");
  const inputRef = useRef<HTMLInputElement>(null);

  // Reset state when the dialog re-opens for a different peer. Without this,
  // closing on peer A then opening for peer B would show A's typed token.
  useEffect(() => {
    if (open) {
      setToken(initialToken ?? "");
      // Defer focus until the dialog is actually rendered + visible —
      // focusing during the same tick as the open transition causes the
      // Tauri webview to scroll oddly on first paint.
      const id = requestAnimationFrame(() => inputRef.current?.focus());
      return () => cancelAnimationFrame(id);
    }
  }, [open, initialToken]);

  if (!open) return null;
  const isEdit = initialToken !== undefined;
  const trimmed = token.trim();

  return (
    <div
      className="fixed inset-0 z-[100] grid place-items-center bg-black/60 backdrop-blur-sm"
      onClick={onCancel}
    >
      <div
        className="w-[420px] max-w-[92vw] rounded-lg border border-[var(--color-border)] bg-[var(--color-panel)] shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-[var(--color-border-soft)]">
          <h3 className="text-sm font-semibold flex items-center gap-2">
            <KeyRound size={14} className="text-[var(--color-accent)]" />
            {isEdit ? "Update worker token" : "Add Synapse worker"}
          </h3>
          <button
            onClick={onCancel}
            className="text-[var(--color-text-muted)] hover:text-[var(--color-text)]"
            aria-label="Close"
          >
            <X size={15} />
          </button>
        </div>

        <div className="px-4 py-4 flex flex-col gap-3">
          <div>
            <div className="text-sm font-medium">{hostname ?? endpoint}</div>
            {hostname && (
              <div className="text-xs text-[var(--color-text-muted)] font-mono">
                {endpoint}
              </div>
            )}
          </div>

          <div>
            <label className="text-xs text-[var(--color-text-muted)] mb-1 block">
              Worker token
            </label>
            <input
              ref={inputRef}
              type="text"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder="Paste from the worker's Synapse tab"
              className="w-full text-sm font-mono bg-[var(--color-panel-2)] border border-[var(--color-border)] rounded-md px-3 py-2 placeholder:text-[var(--color-text-subtle)] focus:outline-none focus:border-[var(--color-accent)]/60"
              onKeyDown={(e) => {
                if (e.key === "Enter" && trimmed) onConfirm(trimmed);
                if (e.key === "Escape") onCancel();
              }}
            />
            <div className="text-[11px] text-[var(--color-text-subtle)] mt-1.5 leading-snug">
              Open LocalMind on the worker, go to <span className="text-[var(--color-text-muted)]">Synapse</span>,
              and copy the token from the <span className="text-[var(--color-text-muted)]">This machine</span> card.
            </div>
          </div>
        </div>

        <div className="flex items-center justify-end gap-2 px-4 py-3 border-t border-[var(--color-border-soft)]">
          <button
            onClick={onCancel}
            className="text-sm px-3 py-1.5 rounded-md bg-[var(--color-panel-2)] border border-[var(--color-border)] hover:border-[var(--color-text-muted)]"
          >
            Cancel
          </button>
          <button
            onClick={() => onConfirm(trimmed)}
            disabled={!trimmed}
            className="text-sm px-3 py-1.5 rounded-md gradient-accent text-white disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {isEdit ? "Save" : "Add"}
          </button>
        </div>
      </div>
    </div>
  );
}
