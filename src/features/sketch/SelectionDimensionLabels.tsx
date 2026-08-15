/*
 * SelectionDimensionLabels — the read-only length label for a SELECTED but
 * still-unconstrained edge (Shapr3D convention: selecting any line shows its
 * live length even before it's dimensioned). DELIBERATELY LIGHTWEIGHT (Sketcher
 * UX cleanup, Track B3): `SketchObject`'s dimension-line witnesses (offset
 * baseline + extension ticks + arrowheads) are now AUTHORED-ONLY, so a merely
 * selected edge draws no 3D geometry at all here — just this inline number at
 * the edge's own midpoint, so passive selection reads as a measurement, not
 * as a dimension constraint.
 *
 * A CONSTRAINED edge's value is NOT duplicated here — `hasAuthoredLength`
 * excludes it, and it keeps showing through the existing
 * `ConstraintBadgeLayer`/`DimensionInput` chip (plus its own full witness),
 * which is the one that can actually commit an edit.
 */
import { useEffect, useMemo, useRef } from "react";
import { useToolStore } from "@/stores/toolStore";
import { useSketchStore } from "@/stores/sketchStore";
import { useSketchSelectionStore } from "@/stores/sketchSelectionStore";
import { useViewportEngine } from "@/viewport/engineBridge";
import { planePointToWorld } from "@/viewport/engine/sketchBasis";
import {
  hasAuthoredLength,
  unconstrainedEdgeLabel,
  type UnconstrainedEdgeLabel,
} from "@/tools/sketch/dimensionLineLayout";
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

  const labels = useMemo(() => {
    if (!session) return [];
    const byId = new Map(session.entities.map((e) => [e.id, e]));
    const out: UnconstrainedEdgeLabel[] = [];
    for (const entityId of selectedEntityIds) {
      if (hasAuthoredLength(entityId, session.constraints)) continue;
      const e = byId.get(entityId);
      if (!e) continue;
      const label = unconstrainedEdgeLabel(e);
      if (label) out.push(label);
    }
    return out;
  }, [session, selectedEntityIds]);

  const plane = session?.plane ?? null;

  useEffect(() => {
    if (!engine || !plane || mode !== "sketch") return;
    const overlay = engine.overlay;
    const ids: string[] = [];
    for (const l of labels) {
      const el = refs.current.get(l.id);
      if (!el) continue;
      overlay.register(l.id, el, planePointToWorld(plane, l.at));
      ids.push(l.id);
    }
    engine.invalidate();
    return () => {
      for (const id of ids) overlay.unregister(id);
    };
  }, [engine, plane, labels, mode]);

  if (mode !== "sketch" || !session || labels.length === 0) return null;

  return (
    <div
      data-testid="selection-dimension-labels"
      className="pointer-events-none absolute inset-x-0 bottom-[34px] top-0 z-[3] overflow-hidden"
    >
      {labels.map((l) => (
        <div
          key={l.id}
          ref={(el) => {
            if (el) refs.current.set(l.id, el);
            else refs.current.delete(l.id);
          }}
        >
          {/* Deliberately near-transparent — a passive measurement, not a
              dimension-constraint chip (which keeps its `shadow-ctrl` box in
              ConstraintBadgeLayer). */}
          <span className="inline-block -translate-x-1/2 -translate-y-1/2 whitespace-nowrap rounded-sm bg-canvas/70 px-1 font-mono text-[11px] text-ink-3">
            {formatLengthWithUnit(l.value)}
          </span>
        </div>
      ))}
    </div>
  );
}
