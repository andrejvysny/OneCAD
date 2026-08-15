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

/**
 * Sketch chrome row (prototype 1c), shown only in sketch mode, second row of
 * `EditorShell`'s fused toolbar stack (rendered directly under
 * `FloatingToolbar` inside the SAME bordered/shadowed flex-column parent —
 * see `EditorShell.tsx`). This component owns only its OWN row content:
 * width, border, shadow and position belong to the stack wrapper, not here.
 * Two variants: before a sketch exists (no activeSketchId) a "Select a
 * sketch plane" prompt with Cancel; once entered, the editing pill (name +
 * DOF + Constraints menu + Construction + Cancel + Finish). Cancel discards
 * straight to model mode (matching Esc). Finish routes through the same
 * `finishSketch` shortcut action as Enter: it drains the sketch mutation
 * queue first, then returns to model selection and prompts for a profile.
 * Compact layout per 1c — no flex spacer (that is the docked-bar 1d variant).
 *
 * `ConstraintMenu` retired the third, separately-floating
 * `SketchConstraintToolbar` row: constraint discovery now lives behind a
 * trigger in THIS row instead of its own pill (Sketcher UX cleanup, Track A3).
 *
 * RESPONSIVE (Track A4/A5): every text node is `whitespace-nowrap` — nothing
 * mid-word wraps. On top of that, a priority collapse (narrowest tier first)
 * queries `@container/canvas`, the definitely-sized ancestor `EditorShell`
 * puts around the whole toolbar stack — NOT this row itself, which stays
 * shrink-to-fit (an earlier attempt querying the shrink-to-fit row directly
 * collapsed it to ~0px; see `EditorShell.tsx`'s comment on that div).
 * Collapse order, least-important label first: Construction text → icon
 * only, Constraints text → icon+chevron (in `ConstraintMenu`), status label
 * → "DOF N" only, Cancel text → icon only. The sketch name, active tool and
 * Finish never collapse.
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
      <div className="flex h-[38px] items-center gap-2.5 rounded-b-md border border-t-0 border-sketch-chrome-border bg-sketch-chrome pl-3.5 pr-1.5 shadow-sketch-pill">
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
    <div className="flex h-[38px] items-center gap-2.5 rounded-b-md border border-t-0 border-sketch-chrome-border bg-sketch-chrome pl-3.5 pr-1.5 shadow-sketch-pill">
      <Icon
          name="penEdit"
          size={15}
          strokeWidth={1.8}
          className={cn("shrink-0", "text-accent", ICON_MONO)}
        />
      {/* Never collapses — identifying which sketch is being edited outranks
          every other label in this row. */}
      <span className="shrink-0 whitespace-nowrap text-[12.5px] font-semibold text-sel-text">
        Editing {name}
      </span>
      <span
        className={cn("shrink-0 whitespace-nowrap text-[12px] font-medium", sketchStatusToneClass(tone))}
      >
        {/* Tier 3: "Under-constrained · DOF 3" collapses to "DOF 3" — the
            count is the actionable half, the tone word is redundant with the
            label's own color. */}
        <span className="@max-[660px]/canvas:hidden">{label}</span>
        <span className="hidden @max-[660px]/canvas:inline">DOF {dof}</span>
      </span>
      {/* E2E-only settle probe (sr-only — never shown to a real user). The
          visible label above changes wording/format across tiers and
          statuses; this stays a fixed "DOF: N" so `dofPill()`
          (`e2e/helpers.ts`) has one stable node to poll, instead of the
          status bar's redundant third copy of the same number (P1 fix —
          Sketcher UX hardening). */}
      <span data-testid="sketch-dof" data-dof={dof} className="sr-only">
        DOF: {dof}
      </span>
      <ConstraintMenu />
      {/* Sticky construction DRAW mode only — flipping an existing selection is the
          X key's job (it needs the selection context this button doesn't have), so
          the pressed state here always reads the mode and nothing else. */}
      <Button
        size="sm"
        variant="secondary"
        aria-pressed={construction}
        aria-label="Construction"
        title="Construction (X)"
        className={cn(
          "shrink-0 whitespace-nowrap",
          construction ? "border-transparent bg-sel-bg text-sel-text" : "text-ink-3",
        )}
        onClick={() => sketchStore.getState().toggleConstructionMode()}
      >
        <Icon name="line" size={12} strokeWidth={2} />
        {/* Tier 1 (first to go, least important label in the row). */}
        <span className="@max-[880px]/canvas:hidden">Construction</span>
      </Button>
      <Button
        size="sm"
        variant="secondary"
        aria-label="Cancel"
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
  );
}
