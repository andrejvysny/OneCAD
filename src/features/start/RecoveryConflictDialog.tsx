import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { Button } from "@/ui/Button";
import type { RecoveryInfo } from "@/ipc/types";

type Choice = "restore" | "openSaved" | "cancel";

type RecoveryConflictDialogProps = {
  conflict: { path: string; offer: RecoveryInfo | null };
  onResolve: (choice: Choice) => void;
};

function fileName(path: string): string {
  const file = path.split(/[\\/]/).pop() ?? path;
  return file.replace(/\.[^.]+$/, "");
}

function formatModified(modifiedMs: number): string | null {
  if (!modifiedMs) return null;
  return new Date(modifiedMs).toLocaleString("en-US", {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

/**
 * Shown when the user opens a recent project whose saved file is OLDER than an
 * unresolved autosave — the backend refuses that open (`recoveryPending`) rather
 * than performing it.
 *
 * The refusal is the whole point. The autosave layout is keyed by `documentId`, so
 * reopening the stale file produces a runtime with the same id, and its first
 * autosave overwrites the crash container and re-stamps the marker under this
 * process. The recovery offer would then be gone — silently, permanently, one click
 * away from the banner offering it. So the destructive branch is a decision the user
 * makes explicitly, with Restore as the default.
 *
 * Mirrors `UnsavedChangesDialog`: a webview modal (the app grants no
 * `tauri-plugin-dialog` permission, and Playwright's mock lane asserts on
 * `role="dialog"`), styled from `src/styles/tokens.css` only.
 */
export function RecoveryConflictDialog({ conflict, onResolve }: RecoveryConflictDialogProps) {
  const restoreRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    restoreRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onResolve("cancel");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onResolve]);

  const modified = conflict.offer ? formatModified(conflict.offer.modifiedMs) : null;

  return createPortal(
    <div className="fixed inset-0 z-[200] flex items-center justify-center bg-scrim">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="recovery-conflict-title"
        className="w-[420px] rounded-md border border-border bg-surface p-5 font-ui shadow-popover"
      >
        <div id="recovery-conflict-title" className="text-[14px] font-semibold text-ink-2">
          &ldquo;{fileName(conflict.path)}&rdquo; has newer unsaved changes
        </div>
        <div className="mt-1.5 text-[12.5px] text-ink-4">
          A previous session ended without saving
          {modified && `, and its work was autosaved ${modified}`}. Opening the saved
          version discards those changes permanently.
        </div>
        <div className="mt-4 flex justify-end gap-2">
          <Button variant="ghost" size="md" onClick={() => onResolve("cancel")}>
            Cancel
          </Button>
          <Button variant="ghost" size="md" onClick={() => onResolve("openSaved")}>
            Open Saved Version
          </Button>
          <Button ref={restoreRef} variant="primary" size="md" onClick={() => onResolve("restore")}>
            Restore Changes
          </Button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
