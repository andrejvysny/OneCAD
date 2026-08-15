/*
 * SelectionDimensionLabels — the read-only length label for a SELECTED but
 * still-unconstrained edge (Shapr3D convention: selecting any line shows its
 * live length even before it's dimensioned). `SketchObject` already draws the
 * witness-line geometry for these in the scene (`rebuildDimensionLines`,
 * driven by the SAME `layoutDimensionLines` this reads); this layer only
 * supplies the number, anchored at the baseline's own midpoint so the text
 * reads as sitting ON the measurement line, not floating elsewhere.
 *
 * A CONSTRAINED edge's value is NOT duplicated here — `layoutDimensionLines`
 * marks it `editable: true` and it keeps showing through the existing
 * `ConstraintBadgeLayer`/`DimensionInput` chip, which is the one that can
 * actually commit an edit.
 */
import { useEffect, useMemo, useRef } from "react";
import { useToolStore } from "@/stores/toolStore";
import { useSketchStore } from "@/stores/sketchStore";
import { useSketchSelectionStore } from "@/stores/sketchSelectionStore";
import { useViewportEngine } from "@/viewport/engineBridge";
import { planePointToWorld } from "@/viewport/engine/sketchBasis";
import { layoutDimensionLines } from "@/tools/sketch/dimensionLineLayout";
import { formatLengthWithUnit } from "@/units/format";

export function SelectionDimensionLabels() {
  const mode = useToolStore((s) => s.mode);
  const session = useSketchStore((s) => s.session);
  const selected = useSketchSelectionStore((s) => s.selected);
  const engine = useViewportEngine();
  const refs = useRef(new Map<string, HTMLDivElement>());

  // Whole-entity picks only (`!sel.point`) — selecting one endpoint of a line
  // is a point pick, not "measure this edge".
  const selectedEntityIds = useMemo(
    () => selected.filter((s) => !s.point).map((s) => s.entityId),
    [selected],
  );

  const dims = useMemo(() => {
    if (!session) return [];
    return layoutDimensionLines(session.entities, session.constraints, selectedEntityIds).filter(
      (d) => !d.editable,
    );
  }, [session, selectedEntityIds]);

  const plane = session?.plane ?? null;

  useEffect(() => {
    if (!engine || !plane || mode !== "sketch") return;
    const overlay = engine.overlay;
    const ids: string[] = [];
    for (const d of dims) {
      const el = refs.current.get(d.id);
      if (!el) continue;
      const mid = {
        x: (d.baseline[0].x + d.baseline[1].x) / 2,
        y: (d.baseline[0].y + d.baseline[1].y) / 2,
      };
      overlay.register(d.id, el, planePointToWorld(plane, mid));
      ids.push(d.id);
    }
    engine.invalidate();
    return () => {
      for (const id of ids) overlay.unregister(id);
    };
  }, [engine, plane, dims, mode]);

  if (mode !== "sketch" || !session || dims.length === 0) return null;

  return (
    <div
      data-testid="selection-dimension-labels"
      className="pointer-events-none absolute inset-x-0 bottom-[34px] top-0 z-[3] overflow-hidden"
    >
      {dims.map((d) => (
        <div
          key={d.id}
          ref={(el) => {
            if (el) refs.current.set(d.id, el);
            else refs.current.delete(d.id);
          }}
        >
          <span className="inline-block -translate-x-1/2 -translate-y-1/2 whitespace-nowrap rounded-sm bg-surface px-1 font-mono text-[11px] text-ink-2 shadow-ctrl">
            {formatLengthWithUnit(d.value)}
          </span>
        </div>
      ))}
    </div>
  );
}
