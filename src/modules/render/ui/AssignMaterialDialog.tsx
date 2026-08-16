/*
 * "Assign material…" — the picker the model tree's context menu opens.
 *
 * WHY A DIALOG AND NOT A SUBMENU. `MenuContribution` is deliberately flat: an
 * item has a title, an `appliesTo` and a `run` (`platform/contributions.ts`).
 * There is no submenu vocabulary, and inventing one — a nested popover, a
 * platform change, twenty registered items one per material — would either
 * widen the platform for one feature or make the menu's length a function of
 * the document. A dialog opened by one flat item keeps the contract as it is,
 * and reuses the mount point the override prompt already needs.
 *
 * The rows go through `assignBodyWithOverridePolicy` like every other assigning
 * surface, so picking a material here can raise the keep/replace prompt — two
 * dialogs in sequence, which is why this one closes FIRST.
 */
import { createPortal } from "react-dom";

import { Button } from "@/ui/Button";

import type { MaterialId } from "../model/material";
import { useRenderDialogStore, renderDialogStore } from "./dialogStore";
import { colorToCss } from "./materialColor";
import { assignBodyWithOverridePolicy } from "./overridePolicy";
import { useRenderStore } from "./state";

export function AssignMaterialDialog() {
  const target = useRenderDialogStore((s) => s.assign);
  const close = renderDialogStore.getState().closeAssign;
  const library = useRenderStore((s) => s.state.library);
  const assigned = useRenderStore((s) =>
    target === null ? undefined : s.state.assignments.bodies[target.bodyId],
  );

  if (target === null) return null;
  const materials = Object.values(library);

  const choose = (materialId: MaterialId | null) => {
    // Close BEFORE assigning: the policy may raise the override prompt, and two
    // stacked modals would leave the user answering the second through the
    // first's scrim.
    close();
    void assignBodyWithOverridePolicy(target.bodyId, materialId, undefined, target.bodyLabel);
  };

  return createPortal(
    <div
      className="fixed inset-0 z-[95] flex items-start justify-center bg-scrim pt-[110px]"
      onClick={close}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Assign material"
        data-testid="assign-material-dialog"
        onClick={(e) => e.stopPropagation()}
        className="w-[320px] rounded-lg border border-border bg-surface p-[20px_22px] font-ui shadow-popover"
      >
        <div className="text-[14px] font-semibold text-ink">Assign material</div>
        <div className="mt-[3px] text-[12px] text-ink-5">{target.bodyLabel ?? target.bodyId}</div>

        <div className="mt-3 max-h-[280px] overflow-auto">
          {materials.length === 0 && (
            <div className="py-2 text-[12px] text-ink-5">
              No materials in this design yet — add one from the Materials tab.
            </div>
          )}
          {materials.map((m) => (
            <button
              key={m.id}
              type="button"
              data-testid={`assign-material-${m.id}`}
              onClick={() => choose(m.id)}
              className="flex w-full cursor-pointer items-center gap-2 rounded-sm px-1.5 py-1.5 text-left hover:bg-hover"
            >
              <span
                aria-hidden="true"
                className="h-3.5 w-3.5 flex-none rounded-[3px] border border-border-strong"
                style={{ background: colorToCss(m.base.base_color) }}
              />
              <span className="flex-1 truncate text-[12.5px] text-ink-2">{m.name}</span>
              {assigned === m.id && <span className="text-[11px] text-ink-5">current</span>}
            </button>
          ))}
        </div>

        <div className="mt-4 flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={close}>
            Cancel
          </Button>
          <Button
            variant="secondary"
            size="sm"
            data-testid="assign-material-none"
            onClick={() => choose(null)}
          >
            None
          </Button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
