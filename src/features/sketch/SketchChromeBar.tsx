import { Button } from "@/ui/Button";
import { ICON_MONO, Icon } from "@/icons/Icon";
import { cn } from "@/ui/cn";
import { useToolStore } from "@/stores/toolStore";
import { useViewportStore } from "@/stores/viewportStore";
import { useDocumentStore } from "@/stores/documentStore";
import { sketchStore } from "@/stores/sketchStore";
import { runAction } from "@/shortcuts/useShortcuts";
import { SketchErrorPulse } from "./SketchErrorPulse";
import { ProjectionBanner } from "./ProjectionBanner";
import { hasCurrentSketchEvaluation } from "./constraintStatus";

/**
 * Sketch chrome row (prototype 1c), shown only in sketch mode, second row of
 * `EditorShell`'s fused toolbar stack (rendered directly under
 * `FloatingToolbar` inside the SAME bordered/shadowed flex-column parent —
 * see `EditorShell.tsx`). This component owns only its OWN row content:
 * width, border, shadow and position belong to the stack wrapper, not here.
 * Two variants: before a sketch exists (no activeSketchId) a "Select a
 * sketch plane" prompt with Cancel; once entered, the editing pill (name +
 * Cancel + Finish). Cancel DISCARDS (WP-U7 D-1): it arms the sketch store's exit
 * intent and flips the mode, and the SketchController reverts the session to its
 * state at entry — deleting the sketch if this visit minted it. Esc and Finish
 * keep their edits.
 * Finish routes through the same `finishSketch` shortcut action as Enter: it
 * drains the sketch mutation queue first, then returns to model selection
 * and prompts for a profile. Compact layout per 1c — no flex spacer (that is
 * the docked-bar 1d variant).
 *
 * DOF/constraint-status text lives ONLY in the inspector panel's
 * `SketchState` card now (`InspectorPanel.tsx`) — this row used to duplicate
 * it, which meant two places could show a stale/mismatched status mid-solve.
 * `ConstraintMenu` and the Construction toggle moved out too: Constraints
 * into the same inspector card, Construction into `FloatingToolbar`'s tool
 * row (it is a sticky draw MODIFIER, not chrome about the sketch itself).
 *
 * RESPONSIVE (Track A4/A5): every text node is `whitespace-nowrap` — nothing
 * mid-word wraps. On top of that, a priority collapse (narrowest tier first)
 * queries `@container/canvas`, the definitely-sized ancestor `EditorShell`
 * puts around the whole toolbar stack — NOT this row itself, which stays
 * shrink-to-fit (an earlier attempt querying the shrink-to-fit row directly
 * collapsed it to ~0px; see `EditorShell.tsx`'s comment on that div).
 * Collapse order, least-important label first: Cancel text → icon only. The
 * sketch name, active tool and Finish never collapse.
 *
 * It also HOSTS `SketchErrorPulse` — a plain child component, not a
 * contribution (the same relationship `ConstraintMenu` has): the pulse portals
 * itself to `document.body`, so this row supplies only its sketch-mode
 * lifetime, not its position.
 */
export function SketchChromeBar() {
  const mode = useToolStore((s) => s.mode);
  const setMode = useToolStore((s) => s.setMode);
  const activeSketchId = useViewportStore((s) => s.activeSketchId);
  const sketch = useDocumentStore((s) =>
    activeSketchId ? s.sketches[activeSketchId] : undefined,
  );

  if (mode !== "sketch") return null;

  // WP-U7 D-1: Cancel DISCARDS — the intent is armed on the store first, because
  // the mode flip is the only channel to the SketchController, which consumes it
  // in its exit path. Esc and Finish leave the intent at "keep".
  const cancel = () => {
    sketchStore.getState().setExitIntent("discard");
    setMode("model");
  };
  const finish = () => runAction({ type: "finishSketch" });

  // Plane-pick phase: no sketch yet — prompt for a plane; Cancel returns to model.
  if (!activeSketchId) {
    return (
      <div className="flex min-h-[38px] max-w-full flex-wrap items-center justify-center gap-2.5 rounded-b-md border border-t-0 border-sketch-chrome-border bg-sketch-chrome pl-3.5 pr-1.5 shadow-sketch-pill">
        <SketchErrorPulse />
        <Icon
          name="penEdit"
          size={15}
          strokeWidth={1.8}
          className={cn("text-accent", ICON_MONO)}
        />
        <span className="text-[12.5px] font-semibold text-sel-text">
          Select a sketch plane
        </span>
        <Button size="sm" variant="secondary" className="text-ink-3" onClick={cancel}>
          <Icon name="x" size={11} strokeWidth={2.2} />
          Cancel
        </Button>
      </div>
    );
  }

  const name = sketch?.name ?? "Sketch";
  const dof = hasCurrentSketchEvaluation(sketch) ? sketch.dof : undefined;

  return (
    <>
    <div className="flex min-h-[38px] max-w-full flex-wrap items-center justify-center gap-2.5 rounded-b-md border border-t-0 border-sketch-chrome-border bg-sketch-chrome pl-3.5 pr-1.5 shadow-sketch-pill">
      <SketchErrorPulse />
      <Icon
          name="penEdit"
          size={15}
          strokeWidth={1.8}
          className={cn("shrink-0", "text-accent", ICON_MONO)}
        />
      {/* Never collapses — identifying which sketch is being edited outranks
          every other label in this row. */}
      <span
        title={`Editing ${name}`}
        className="min-w-0 max-w-48 truncate whitespace-nowrap text-[12.5px] font-semibold text-sel-text"
      >
        Editing {name}
      </span>
      {/* E2E-only settle probe (sr-only — never shown to a real user; the
          visible status now lives solely in the inspector panel's
          `SketchState` card). Numeric output requires current solver evidence. */}
      <span data-testid="sketch-dof" data-dof={dof} className="sr-only">
        {dof === undefined ? "Not evaluated" : `DOF: ${dof}`}
      </span>
      <Button
        size="sm"
        variant="secondary"
        aria-label="Cancel"
        title="Discard changes since entering the sketch"
        className="shrink-0 whitespace-nowrap text-ink-3"
        onClick={cancel}
      >
        <Icon name="x" size={11} strokeWidth={2.2} />
        {/* Tier 4 (last to go — Cancel stays reachable by icon alone). */}
        <span className="@max-[560px]/canvas:hidden">Cancel</span>
      </Button>
      {/* Never collapses. */}
      <Button size="sm" variant="primary" className="shrink-0 whitespace-nowrap" onClick={finish}>
        <Icon name="check" size={12} strokeWidth={2.4} />
        Finish sketch
      </Button>
    </div>
    {/* Second chrome row, present only while the open sketch holds projected
        body geometry (WP-P). A plain child, not a contribution — see its own
        doc comment for why. */}
    <ProjectionBanner />
    </>
  );
}
