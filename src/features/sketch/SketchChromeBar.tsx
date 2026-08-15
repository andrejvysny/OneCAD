import { Button } from "@/ui/Button";
import { ICON_MONO, Icon } from "@/icons/Icon";
import { cn } from "@/ui/cn";
import { useToolStore } from "@/stores/toolStore";
import { useViewportStore } from "@/stores/viewportStore";
import { useDocumentStore } from "@/stores/documentStore";
import { sketchStore, useSketchStore } from "@/stores/sketchStore";
import { runAction } from "@/shortcuts/useShortcuts";
import { sketchStatusText, sketchStatusToneClass } from "./constraintStatus";
import { ConstraintMenu } from "./ConstraintMenu";
import { SKETCH_CHROME_TOP } from "@/features/toolbar/chromeLayout";

/**
 * Floating sketch chrome pill (prototype 1c), shown only in sketch mode. Two
 * variants: before a sketch exists (no activeSketchId) a "Select a sketch plane"
 * prompt with Cancel; once entered, the editing pill (name + DOF + Constraints
 * menu + Construction + Cancel + Finish). Cancel discards straight to model
 * mode (matching Esc). Finish routes through the same `finishSketch` shortcut
 * action as Enter: it drains the sketch mutation queue first, then returns to
 * model selection and prompts for a profile. Compact layout per 1c — no flex
 * spacer (that is the docked-bar 1d variant).
 *
 * Sits flush under `FloatingToolbar` (`chromeLayout.ts` derives this row's
 * top offset from that one's height) — the two read as one fused two-row
 * pill rather than stacked panels. `ConstraintMenu` retired the third,
 * separately-floating `SketchConstraintToolbar` row: constraint discovery
 * now lives behind a trigger in THIS row instead of its own pill (Sketcher
 * UX cleanup, Track A3).
 *
 * RESPONSIVE (Track A4): every text node is `whitespace-nowrap` — nothing
 * mid-word wraps, ever. A CSS container-query priority collapse was tried on
 * top of this (hide labels least-important-first below a width threshold)
 * and REVERTED: `container-type: inline-size` on an absolutely-positioned,
 * shrink-to-fit flex row applies size containment to its OWN inline axis,
 * which disables the very "size to content" behavior the pill relies on to
 * stay tight around its content — it collapsed to ~0px, so every
 * `@min-[…]` query read as never-true and `@max-[…]:hidden` as always-true,
 * permanently hiding the DOF status (caught by e2e, not vitest — jsdom
 * doesn't compute real container-query layout). A real fix needs the query
 * context on a SEPARATE, definitely-sized (not shrink-to-fit) wrapper ancestor
 * around this pill, not on the pill itself — left as a follow-up; `nowrap`
 * alone already fixes the reported bug (mid-word wrapping), just without a
 * narrow-viewport collapse tier on top of it.
 */
export function SketchChromeBar() {
  const mode = useToolStore((s) => s.mode);
  const setMode = useToolStore((s) => s.setMode);
  const activeSketchId = useViewportStore((s) => s.activeSketchId);
  const sketch = useDocumentStore((s) =>
    activeSketchId ? s.sketches[activeSketchId] : undefined,
  );
  const construction = useSketchStore((s) => s.constructionMode);

  if (mode !== "sketch") return null;

  const cancel = () => setMode("model");
  const finish = () => runAction({ type: "finishSketch" });

  // Plane-pick phase: no sketch yet — prompt for a plane; Cancel returns to model.
  if (!activeSketchId) {
    return (
      <div
        style={{ top: SKETCH_CHROME_TOP }}
        className="absolute left-1/2 z-[29] flex h-[38px] -translate-x-1/2 items-center gap-2.5 rounded-b-md border border-t-0 border-sketch-chrome-border bg-sketch-chrome pl-3.5 pr-1.5 shadow-sketch-pill"
      >
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
  const dof = sketch?.dof ?? 0;
  const { label, tone } = sketchStatusText(sketch?.status ?? "under", dof);

  return (
    <div
      style={{ top: SKETCH_CHROME_TOP }}
      className="absolute left-1/2 z-[29] flex h-[38px] -translate-x-1/2 items-center gap-2.5 rounded-b-md border border-t-0 border-sketch-chrome-border bg-sketch-chrome pl-3.5 pr-1.5 shadow-sketch-pill"
    >
      <Icon
          name="penEdit"
          size={15}
          strokeWidth={1.8}
          className={cn("shrink-0", "text-accent", ICON_MONO)}
        />
      <span className="shrink-0 whitespace-nowrap text-[12.5px] font-semibold text-sel-text">
        Editing {name}
      </span>
      <span
        className={cn("shrink-0 whitespace-nowrap text-[12px] font-medium", sketchStatusToneClass(tone))}
      >
        {label}
      </span>
      <ConstraintMenu />
      {/* Sticky construction DRAW mode only — flipping an existing selection is the
          X key's job (it needs the selection context this button doesn't have), so
          the pressed state here always reads the mode and nothing else. */}
      <Button
        size="sm"
        variant="secondary"
        aria-pressed={construction}
        title="Construction (X)"
        className={cn(
          "shrink-0 whitespace-nowrap",
          construction ? "border-transparent bg-sel-bg text-sel-text" : "text-ink-3",
        )}
        onClick={() => sketchStore.getState().toggleConstructionMode()}
      >
        <Icon name="line" size={12} strokeWidth={2} />
        Construction
      </Button>
      <Button size="sm" variant="secondary" className="shrink-0 whitespace-nowrap text-ink-3" onClick={cancel}>
        <Icon name="x" size={11} strokeWidth={2.2} />
        Cancel
      </Button>
      <Button size="sm" variant="primary" className="shrink-0 whitespace-nowrap" onClick={finish}>
        <Icon name="check" size={12} strokeWidth={2.4} />
        Finish sketch
      </Button>
    </div>
  );
}
