import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { appStore, useAppStore } from "@/stores/appStore";
import { useDocumentStore } from "@/stores/documentStore";
import { Button } from "@/ui/Button";

/**
 * Unsaved-changes confirmation — shown before EVERY close/quit path once the
 * open document is dirty: TitleBar ×, ⌘W, the native window-close button, and
 * ⌘Q all route through `appStore.requestClose`, which arms `pendingCloseIntent`
 * instead of closing unconditionally (see App.tsx's `onCloseRequested`
 * subscription for the native paths).
 *
 * Mounted at the APP root, not inside EditorScreen: the native close/⌘Q must be
 * answerable from the start screen too, or Rust's ExitGuard latches unresolved.
 *
 * A WEBVIEW dialog, not `tauri-plugin-dialog`: the app's capabilities grant no
 * dialog permission (`core:default` only — see CLAUDE.md), and a webview modal
 * is what the Playwright mock lane can actually assert against (`role="dialog"`).
 * Styled from `src/styles/tokens.css` only, mirroring
 * `src/features/start/RecoveryCard.tsx`'s token usage.
 */
export function UnsavedChangesDialog() {
  const intent = useAppStore((s) => s.pendingCloseIntent);
  const title = useDocumentStore((s) => s.title);
  const [busy, setBusy] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);

  const resolve = async (action: "save" | "discard" | "cancel"): Promise<void> => {
    setBusy(true);
    try {
      await appStore.getState().confirmClose(action);
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    if (!intent) return;
    cancelRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        void resolve("cancel");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- resolve is stable enough here
  }, [intent]);

  if (!intent) return null;

  return createPortal(
    <div className="fixed inset-0 z-[200] flex items-center justify-center bg-black/40">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="unsaved-changes-title"
        className="w-[380px] rounded-md border border-border bg-white p-5 font-ui shadow-popover"
      >
        <div id="unsaved-changes-title" className="text-[14px] font-semibold text-ink-2">
          Save changes to &ldquo;{title || "Untitled"}&rdquo;?
        </div>
        <div className="mt-1.5 text-[12.5px] text-ink-4">
          Your changes will be lost if you don&rsquo;t save them.
        </div>
        <div className="mt-4 flex justify-end gap-2">
          <Button ref={cancelRef} variant="ghost" size="md" disabled={busy} onClick={() => void resolve("cancel")}>
            Cancel
          </Button>
          <Button variant="ghost" size="md" disabled={busy} onClick={() => void resolve("discard")}>
            Don&rsquo;t Save
          </Button>
          <Button variant="primary" size="md" disabled={busy} onClick={() => void resolve("save")}>
            Save
          </Button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
