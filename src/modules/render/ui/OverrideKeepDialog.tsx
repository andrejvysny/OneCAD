/*
 * "This body has face overrides" — the keep/replace prompt.
 *
 * Raised by `overridePolicy.ts` and by nothing else: every surface that assigns
 * a body material goes through that one helper, so this dialog has exactly one
 * opener no matter which gesture started it.
 *
 * THREE ANSWERS, not two. Dismissing (Esc, the scrim, Cancel) is a real answer
 * — the user opened a question they did not mean to — and it resolves as
 * `"cancel"`, which writes NOTHING. A dialog whose only exits both mutate the
 * document would make an accidental drop unavoidable.
 *
 * Follows `SaveAsComponentDialog`'s markup exactly (portal + scrim + a
 * `role="dialog"` card), because a second modal shape in the same app is a
 * visual bug that no test would catch.
 */
import { useEffect } from "react";
import { createPortal } from "react-dom";

import { Button } from "@/ui/Button";

import { useRenderDialogStore, renderDialogStore } from "./dialogStore";

export function OverrideKeepDialog() {
  const pending = useRenderDialogStore((s) => s.override);
  const answer = renderDialogStore.getState().answerOverride;

  // Esc cancels, like every other modal in the app. Registered only while the
  // dialog is up, so it never competes with the editor's own shortcuts.
  useEffect(() => {
    if (pending === null) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        answer("cancel");
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [pending, answer]);

  if (pending === null) return null;
  const { request } = pending;
  const count = request.overrideElementIds.length;
  const bodyName = request.bodyLabel ?? "this body";

  return createPortal(
    <div
      className="fixed inset-0 z-[95] flex items-start justify-center bg-scrim pt-[110px]"
      onClick={() => answer("cancel")}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Face overrides"
        data-testid="material-override-dialog"
        onClick={(e) => e.stopPropagation()}
        className="w-[420px] rounded-lg border border-border bg-surface p-[20px_22px] font-ui shadow-popover"
      >
        <div className="text-[14px] font-semibold text-ink">
          {bodyName} has {count} face {count === 1 ? "override" : "overrides"}
        </div>
        <div className="mt-[3px] text-[12px] text-ink-5">
          Assigning <span className="text-ink-3">{request.materialName}</span> to the body does not
          change those faces — they keep the material they were given directly. Replace them to
          make the whole body one material.
        </div>
        <div className="mt-4 flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={() => answer("cancel")}>
            Cancel
          </Button>
          <Button
            variant="secondary"
            size="sm"
            data-testid="material-override-keep"
            onClick={() => answer("keep")}
          >
            Keep face overrides
          </Button>
          <Button
            variant="primary"
            size="sm"
            data-testid="material-override-replace"
            onClick={() => answer("replace")}
          >
            Replace all
          </Button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
