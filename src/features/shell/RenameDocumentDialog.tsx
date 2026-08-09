import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Button } from "@/ui/Button";
import { TextInput } from "@/ui/TextInput";
import { documentStore, useDocumentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";

/**
 * Rename the open document (File ▸ Rename…).
 *
 * It lives in the File menu rather than as a double-click on the title: the
 * title sits inside the window's drag region, where a click already means
 * "start dragging the window", and a rename you can trigger by mis-clicking the
 * chrome is a rename you will trigger by accident.
 *
 * FLAGGED — DISPLAY-ONLY. There is no `RenameDocument` command in the protocol:
 * the Rust runtime derives the title from the file it loaded, and
 * `renameRecentProject` needs a path the editor does not hold. So this writes
 * `DocumentState.displayTitle`, and the dialog SAYS the file is untouched
 * rather than implying a save happened. Persisting a rename is Save As until
 * that command lands.
 *
 * Portals to `document.body`: the title bar is a plain flex row with no
 * positioned ancestor, so an in-tree `absolute inset-0` would size itself
 * against the wrong box.
 */
export function RenameDocumentDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const title = useDocumentStore((s) => s.title);
  const displayTitle = useDocumentStore((s) => s.displayTitle);
  const current = displayTitle ?? title;
  const [value, setValue] = useState(current);
  const input = useRef<HTMLInputElement>(null);

  // Re-seed on every open: the field must show the name as it is NOW, not the
  // one it was left with the last time the dialog closed.
  useEffect(() => {
    if (!open) return;
    setValue(current);
    input.current?.focus();
    input.current?.select();
    // `current` is deliberately not a dependency — re-seeding while the user is
    // typing (a projection can land mid-edit) would eat their keystrokes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  if (!open) return null;

  const next = value.trim();
  // An empty name would leave the document with no identity at all; an unchanged
  // one has nothing to commit.
  const canCommit = next.length > 0 && next !== current;

  const commit = () => {
    if (!canCommit) return;
    onClose();
    documentStore.getState().setDisplayTitle(next);
    viewportStore
      .getState()
      .setStatusHint(`Renamed to “${next}” — shown here only; the file on disk is unchanged.`);
  };

  return createPortal(
    <div
      className="fixed inset-0 z-[95] flex items-start justify-center bg-scrim pt-[120px]"
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Rename document"
        data-testid="rename-document-dialog"
        onClick={(e) => e.stopPropagation()}
        className="w-[380px] rounded-lg border border-border bg-surface p-[20px_22px] font-ui shadow-popover"
      >
        <div className="text-[14px] font-semibold text-ink">Rename document</div>
        <div className="mt-[3px] text-[12px] text-ink-5">
          Changes the name shown in the app. The file on disk keeps its current name — use
          File ▸ Save As… to write it under a new one.
        </div>

        <TextInput
          ref={input}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          aria-label="Document name"
          data-testid="rename-document-input"
          wrapperClassName="mt-4 w-full"
          onKeyDown={(e) => {
            // The bar sits under global shortcuts (letters arm tools, Escape
            // cancels the active tool) — keep every keystroke inside the field.
            e.stopPropagation();
            if (e.key === "Enter") commit();
            else if (e.key === "Escape") onClose();
          }}
        />

        <div className="mt-5 flex justify-end gap-2">
          <Button variant="secondary" data-testid="rename-document-cancel" onClick={onClose}>
            Cancel
          </Button>
          <Button disabled={!canCommit} data-testid="rename-document-commit" onClick={commit}>
            Rename
          </Button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
