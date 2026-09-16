/*
 * Stable operation controls. Parameter editing remains on the anchored label;
 * this bar owns operation identity, high-value mode readout, inspector access,
 * and explicit completion without following a world point.
 */
import { activeToolPresentation, activeToolPresentationTitle } from "@/tools/modelTools/activeToolPresentation";
import { toolChipStore, useToolChipStore } from "@/stores/toolChipStore";
import { inspectorLayoutStore } from "@/stores/inspectorLayoutStore";

const MODEL_KINDS = new Set([
  "extrudeDepth", "revolveAngle", "revolveAxisPick", "datumOffset", "regionSelect",
  "filletRadius", "shellThickness", "offsetFace", "hole", "gear", "linearPattern",
  "circularPattern", "transform", "mirror", "booleanOp",
]);

export function ModelOperationBar() {
  const state = useToolChipStore((current) => current);
  if (!MODEL_KINDS.has(state.kind)) return null;
  const presentation = activeToolPresentation(state);
  if (!presentation) return null;
  const modes = presentation.modes.slice(0, 2);

  return (
    <div
      data-testid="model-operation-bar"
      role="toolbar"
      aria-label={`${activeToolPresentationTitle(presentation)} operation`}
      className="flex max-w-full flex-wrap items-center justify-center gap-1 rounded-lg border border-border bg-surface px-2 py-1 shadow-card"
    >
      <span className="px-1 text-[12px] font-semibold text-ink">{activeToolPresentationTitle(presentation)}</span>
      {modes.map((mode) => (
        <span key={mode.id} className="rounded-full bg-chip px-2 py-0.5 text-[11px] text-ink-3">
          {mode.label}: {String(mode.value)}
        </span>
      ))}
      <button
        type="button"
        data-testid="model-operation-more"
        aria-label="Show operation settings in inspector"
        onClick={() => inspectorLayoutStore.getState().setOpen(true)}
        className="rounded-full bg-chip px-2 py-0.5 text-[11px] text-ink-3 hover:bg-hover-2"
      >
        More…
      </button>
      <button
        type="button"
        data-testid="model-operation-cancel"
        aria-label="Cancel"
        disabled={!presentation.canCancel}
        onClick={() => toolChipStore.getState().onCancel?.()}
        className="rounded-full bg-chip px-2 py-0.5 text-[11px] font-medium text-ink-3 hover:bg-hover-2 disabled:opacity-40"
      >
        Cancel
      </button>
      <button
        type="button"
        data-testid="model-operation-done"
        aria-label="Done"
        disabled={!presentation.canConfirm}
        onClick={() => toolChipStore.getState().onConfirm?.()}
        className="rounded-full bg-accent px-2 py-0.5 text-[11px] font-medium text-on-accent hover:opacity-90 disabled:opacity-40"
      >
        Done
      </button>
    </div>
  );
}
