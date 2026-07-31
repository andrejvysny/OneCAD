import { Button } from "@/ui/Button";
import { Icon } from "@/icons/Icon";
import { cn } from "@/ui/cn";
import { useToolStore } from "@/stores/toolStore";
import { useViewportStore } from "@/stores/viewportStore";
import { useDocumentStore } from "@/stores/documentStore";
import { sketchStore, useSketchStore } from "@/stores/sketchStore";
import { runAction } from "@/shortcuts/useShortcuts";
import { sketchStatusText } from "./constraintStatus";

/**
 * Floating sketch chrome pill (prototype 1c), shown only in sketch mode. Two
 * variants: before a sketch exists (no activeSketchId) a "Select a sketch plane"
 * prompt with Cancel; once entered, the editing pill (name + DOF + Finish).
 * Cancel discards straight to model mode (matching Esc). Finish routes through
 * the same `finishSketch` shortcut action as Enter: it drains the sketch
 * mutation queue first, then returns to model selection and prompts for a profile. Compact layout
 * per 1c — no flex spacer (that is the docked-bar 1d variant).
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
      <div className="absolute left-1/2 top-[62px] z-[29] flex h-[38px] -translate-x-1/2 items-center gap-2.5 rounded-md border border-sketch-chrome-border bg-sketch-chrome pl-3.5 pr-1.5 shadow-sketch-pill">
        <Icon name="penEdit" size={15} strokeWidth={1.8} className="text-accent" />
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
    <div className="absolute left-1/2 top-[62px] z-[29] flex h-[38px] -translate-x-1/2 items-center gap-2.5 rounded-md border border-sketch-chrome-border bg-sketch-chrome pl-3.5 pr-1.5 shadow-sketch-pill">
      <Icon name="penEdit" size={15} strokeWidth={1.8} className="text-accent" />
      <span className="text-[12.5px] font-semibold text-sel-text">
        Editing {name}
      </span>
      <span className={cn("text-[12px] font-medium", tone === "ok" ? "text-ink-4" : "text-warn")}>
        {label}
      </span>
      {/* Sticky construction DRAW mode only — flipping an existing selection is the
          X key's job (it needs the selection context this button doesn't have), so
          the pressed state here always reads the mode and nothing else. */}
      <Button
        size="sm"
        variant="secondary"
        aria-pressed={construction}
        title="Construction (X)"
        className={cn(construction ? "border-transparent bg-sel-bg text-sel-text" : "text-ink-3")}
        onClick={() => sketchStore.getState().toggleConstructionMode()}
      >
        <Icon name="line" size={12} strokeWidth={2} />
        Construction
      </Button>
      <Button size="sm" variant="secondary" className="text-ink-3" onClick={cancel}>
        <Icon name="x" size={11} strokeWidth={2.2} />
        Cancel
      </Button>
      <Button size="sm" variant="primary" onClick={finish}>
        <Icon name="check" size={12} strokeWidth={2.4} />
        Finish sketch
      </Button>
    </div>
  );
}
