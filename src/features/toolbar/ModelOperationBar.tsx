/*
 * The stable operation strip (spec §4.2, review R05).
 *
 * It owns operation identity, at most two high-value mode readouts, a short
 * pending/invalid state, the result summary, a More menu into the inspector and
 * label placement, and THE single Done/Cancel pair. Nothing here tracks a world
 * point, so it stays available when the anchor is offscreen, the label is in the
 * inspector, or a preview covers the scene.
 *
 * It renders inside `Slots.ToolbarContextual`, which `EditorShell` wraps in a
 * `MeasuredShellRegion` — that is how its rectangle reaches
 * `viewportWorkAreaStore.obstacleClearRect` and keeps the floating label out
 * from under it. It must stay there rather than positioning itself.
 *
 * RESPONSIVE (§4.2): the row wraps into a controlled second line rather than
 * clipping, and the mode readouts + result summary collapse at narrow widths
 * (they remain reachable through More). Title, Cancel and Done never collapse.
 */
import { useRef, useState } from "react";
import {
  activeToolPresentation,
  activeToolPresentationTitle,
  type ActiveToolPreviewState,
} from "@/tools/modelTools/activeToolPresentation";
import type { ToolChipState, ToolValueValidation } from "@/stores/toolChipStore";
import { toolChipStore, useToolChipStore } from "@/stores/toolChipStore";
import { inspectorLayoutStore } from "@/stores/inspectorLayoutStore";
import { toolChipPlacementStore } from "@/stores/toolChipPlacementStore";
import { Popover } from "@/ui/Popover";
import { MenuItem } from "@/ui/MenuItem";
import { cn } from "@/ui/cn";
import { requestConfirm } from "./requestConfirm";

const MODEL_KINDS = new Set([
  "extrudeDepth", "revolveAngle", "revolveAxisPick", "datumOffset", "regionSelect",
  "filletRadius", "shellThickness", "offsetFace", "hole", "gear", "linearPattern",
  "circularPattern", "transform", "mirror", "booleanOp",
]);

interface StripMode {
  key: string;
  testid?: string;
  text: string;
}

/**
 * At most two high-value mode readouts (§4.2). A mode that only repeats the
 * operation title is dropped — `Shell · Shell` is noise, not a readout. The
 * controls that CHANGE these live in the inspector, one More click away, which
 * is what keeps one editable copy of each setting (§4.3).
 */
function stripModes(s: ToolChipState): StripMode[] {
  switch (s.kind) {
    case "extrudeDepth":
      return [
        { key: "boolean", testid: "chip-mode-badge", text: s.booleanMode === "NewBody" ? "New" : s.booleanMode },
        { key: "extent", testid: "chip-end-badge", text: s.endCondition },
      ];
    case "revolveAngle":
      return [{ key: "boolean", testid: "chip-revolve-badge", text: s.booleanMode }];
    case "filletRadius":
      return [{ key: "edgeOp", testid: "chip-edgeop-badge", text: s.edgeOp }];
    case "linearPattern":
    case "circularPattern":
      return [{ key: "axis", testid: "chip-pattern-badge", text: `Axis ${s.axis}` }];
    case "transform":
      return [
        { key: "mode", testid: "chip-transform-badge", text: s.transformMode },
        { key: "axis", text: `Axis ${s.axis}` },
      ];
    case "mirror":
      return [{ key: "plane", testid: "chip-mirror-badge", text: s.plane }];
    case "booleanOp":
      return [{ key: "op", testid: "chip-boolean-badge", text: s.op }];
    default:
      return [];
  }
}

/** The one-line state (§4.2): longer explanations belong to the inspector. */
function shortState(
  validation: ToolValueValidation,
  preview: ActiveToolPreviewState,
): { text: string; tone: "warn" | "muted" } | null {
  if (validation.status === "invalid") return { text: validation.message, tone: "warn" };
  if (validation.status === "pending") return { text: "Checking…", tone: "muted" };
  if (preview === "applying") return { text: "Applying…", tone: "muted" };
  if (preview === "pending") return { text: "Computing preview…", tone: "muted" };
  return null;
}

export function ModelOperationBar() {
  const state = useToolChipStore((current) => current);
  const [menuOpen, setMenuOpen] = useState(false);
  const moreRef = useRef<HTMLButtonElement | null>(null);
  if (!MODEL_KINDS.has(state.kind)) return null;
  const presentation = activeToolPresentation(state);
  if (!presentation) return null;
  const title = activeToolPresentationTitle(presentation);
  const modes = stripModes(state).filter((mode) => mode.text !== title).slice(0, 2);
  const status = shortState(presentation.validation, presentation.preview);

  const placeInInspector = () => {
    // Opening the drawer FIRST: the dock host is `hidden` + `inert` while it is
    // closed, so docking into it would put the only primary field out of reach
    // and the label would bounce straight back to its anchor (§4.4).
    inspectorLayoutStore.getState().setOpen(true);
    toolChipPlacementStore.getState().dock();
    setMenuOpen(false);
  };

  return (
    <div
      data-testid="model-operation-bar"
      role="toolbar"
      aria-label={`${title} operation`}
      className="flex min-h-[36px] max-w-full flex-wrap items-center justify-center gap-1.5 rounded-b-md border border-t-0 border-border bg-surface px-2 py-1 shadow-card"
    >
      <span data-testid="model-operation-title" className="whitespace-nowrap px-1 text-[12.5px] font-semibold text-ink">
        {title}
      </span>
      {/* Collapse tier 2: the mode readouts. They are a READOUT — the controls
          that change them stay reachable in the inspector, one More click away
          — so losing them costs nothing essential. */}
      {modes.map((mode) => (
        <span
          key={mode.key}
          data-testid={mode.testid}
          className="whitespace-nowrap rounded-full bg-chip px-2 py-0.5 text-[11px] text-ink-3 @max-[640px]/canvas:hidden"
        >
          {mode.text}
        </span>
      ))}
      {/* Collapse tier 1 (first to go): the body-lifecycle summary (D18). Also
          the tool's live region — it lives here rather than in the canvas
          subtree, which is `aria-hidden` decoration. */}
      {state.resultSummary && (
        <span
          data-testid="chip-result-summary"
          role="status"
          aria-live="polite"
          className="min-w-0 truncate px-1 text-[11px] text-ink-5 @max-[860px]/canvas:hidden"
        >
          {state.resultSummary}
        </span>
      )}
      {status && (
        <span
          data-testid="tool-validation"
          role={presentation.validation.status === "invalid" ? "alert" : "status"}
          className={cn(
            "min-w-0 truncate px-1 text-[11px]",
            status.tone === "warn" ? "text-warn" : "text-ink-5",
          )}
        >
          {status.text}
        </span>
      )}
      <button
        type="button"
        ref={moreRef}
        data-testid="model-operation-more"
        aria-label="Operation settings and label placement"
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        onClick={() => setMenuOpen((open) => !open)}
        className="whitespace-nowrap rounded-full bg-chip px-2 py-0.5 text-[11px] text-ink-3 hover:bg-hover-2"
      >
        More…
      </button>
      <button
        type="button"
        data-testid="model-operation-cancel"
        aria-label="Cancel"
        disabled={!presentation.canCancel}
        onClick={() => toolChipStore.getState().onCancel?.()}
        className="whitespace-nowrap rounded-full bg-chip px-2 py-0.5 text-[11px] font-medium text-ink-3 hover:bg-hover-2 disabled:opacity-40"
      >
        Cancel
      </button>
      <button
        type="button"
        data-testid="model-operation-done"
        aria-label="Done"
        disabled={!presentation.canConfirm}
        onClick={requestConfirm}
        className="whitespace-nowrap rounded-full bg-accent px-2 py-0.5 text-[11px] font-medium text-on-accent hover:opacity-90 disabled:opacity-40"
      >
        Done
      </button>
      <Popover
        open={menuOpen}
        onClose={() => setMenuOpen(false)}
        anchorRef={moreRef}
        placement="bottom-end"
        width={220}
        className="p-1"
        ariaLabel={`${title} settings`}
      >
        <div role="menu" aria-label={`${title} settings`}>
          <MenuItem
            label="Operation settings…"
            data-testid="model-operation-more-settings"
            onClick={() => {
              inspectorLayoutStore.getState().setOpen(true);
              setMenuOpen(false);
            }}
          />
          {/* Placement, without a permanent header row on every label (§4.4). */}
          <MenuItem
            label="Follow handle"
            data-testid="model-operation-place-follow"
            onClick={() => {
              toolChipPlacementStore.getState().anchor();
              setMenuOpen(false);
            }}
          />
          <MenuItem
            label="Place label"
            data-testid="model-operation-place-manual"
            onClick={() => {
              toolChipPlacementStore.getState().setPlacing(true);
              setMenuOpen(false);
            }}
          />
          <MenuItem
            label="Show value in inspector"
            data-testid="model-operation-place-inspector"
            onClick={placeInInspector}
          />
        </div>
      </Popover>
    </div>
  );
}
