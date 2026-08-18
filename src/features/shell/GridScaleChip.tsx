/*
 * Grid scale readout — "one square is this big" (Shapr3D's grid legend).
 *
 * The grid step is ADAPTIVE (`chooseGridStep`), so without a readout the cells
 * are a ruler with no numbers on it: the same square is 1 mm at one zoom and
 * 200 mm at another, and nothing on screen says which. It is also the snap
 * quantum when snap-to-grid is on, so the number is functional, not decorative.
 *
 * A plain child of `ViewportRoot`, NOT a registered panel: it is glued to the
 * viewport it describes, exactly like the geometry chips it sits beside, and a
 * viewport-chrome contribution would put a shell panel in charge of a number
 * only the engine knows (same precedent as ConstraintMenu inside SketchChromeBar).
 */
import { MonoValue } from "@/ui/MonoValue";
import { useViewportStore } from "@/stores/viewportStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { formatLengthWithUnit } from "@/units/format";

/**
 * Renders nothing while the grid is hidden — the legend describes lines that
 * are on screen, and one for an invisible grid is noise.
 */
export function GridScaleChip() {
  const gridVisible = useViewportStore((s) => s.gridVisible);
  const gridStep = useViewportStore((s) => s.gridStep);
  // The value is stored in mm and READ in the display unit, so this component
  // has to re-render on a unit switch — `formatLengthWithUnit` reads the
  // preference at call time and would otherwise render a stale unit.
  const unit = useSettingsStore((s) => s.displayUnit);
  if (!gridVisible) return null;

  return (
    <div
      data-testid="grid-scale"
      // Non-interactive, like every other viewport chip: the canvas underneath
      // is picked constantly and this must never swallow a click.
      //
      // `right-[264px]`, not `right-3`: the canvas runs full-bleed under the
      // inspector panel, so the VISIBLE right edge is the panel's left edge.
      // Same offset `CornerCluster` uses for the same reason.
      className="pointer-events-none absolute bottom-3 right-[264px] z-[2]"
    >
      <span className="flex items-center gap-1.5 rounded-md border border-border bg-surface px-2 py-1 shadow-ctrl">
        {/* One cell, drawn. Cheaper to read than the word "grid". */}
        <span aria-hidden="true" className="h-[9px] w-[9px] rounded-[1px] border border-ink-6" />
        <MonoValue className="text-[11.5px] text-ink-5">
          {formatLengthWithUnit(gridStep, unit)}
        </MonoValue>
      </span>
    </div>
  );
}
