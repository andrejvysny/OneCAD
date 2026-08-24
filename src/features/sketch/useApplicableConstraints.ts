/*
 * useApplicableConstraints — the ONE shared derivation both constraint UIs read
 * (the persistent toolbar + the floating context chips), so they always offer the
 * identical set. Resolves the current sketch selection (`sketchSelectionStore`)
 * against the live session entities (`sketchStore`) into `SketchConstraintTarget`s,
 * runs the pure applicability matrix, and computes the selection centroid the
 * context chips anchor to. Pure derived state — empty selection ⇒ empty set, so
 * both UIs dismiss naturally on Esc / selection change (design item 5).
 */
import { useMemo } from "react";
import type { SketchConstraint, SketchEntity } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import { useSketchSelectionStore, type SketchSel } from "@/stores/sketchSelectionStore";
import { useSketchStore } from "@/stores/sketchStore";
import { toConstraintTarget } from "@/tools/sketch/constraintTarget";
import {
  evaluateApplicability,
  type ApplicableConstraint,
} from "@/tools/sketch/constraintApplicability";
import { entityAnchor, entityPointCoord } from "./badgeLayout";

export interface ApplicableConstraints {
  applicables: ApplicableConstraint[];
  /** Plane-coord centroid of the involved geometry (context-chip anchor), or null. */
  centroid: Point2 | null;
  /**
   * The Fixed constraint already pinning the selected point, when the
   * selection is exactly one named point — lets the context chips offer
   * "Disconnect" instead of (or alongside) "Fixed" for an already-pinned
   * point. Null for any other selection shape.
   */
  existingFixed: SketchConstraint | null;
}

/**
 * The applicable set for a selection — the SAME derivation the hook exposes,
 * without the React binding, so the keyboard lane (`useShortcuts`'s
 * `applyConstraint` action, plan item 6) offers exactly what the menu and the
 * context chips do instead of re-deriving applicability of its own.
 */
export function applicableConstraintsFor(
  selected: SketchSel[],
  entities: SketchEntity[],
): ApplicableConstraint[] {
  const targets = selected
    .map((sel) => toConstraintTarget(sel, entities))
    .filter((t): t is NonNullable<typeof t> => t !== null);
  return evaluateApplicability(targets, entities);
}

export function useApplicableConstraints(): ApplicableConstraints {
  const selected = useSketchSelectionStore((s) => s.selected);
  const session = useSketchStore((s) => s.session);
  return useMemo(() => {
    const entities: SketchEntity[] = session?.entities ?? [];
    return {
      applicables: applicableConstraintsFor(selected, entities),
      centroid: selectionCentroid(selected, entities),
      existingFixed: existingFixedOn(selected, session?.constraints ?? []),
    };
  }, [selected, session]);
}

/** The Fixed constraint bound to the single selected point, if any. */
function existingFixedOn(selected: SketchSel[], constraints: SketchConstraint[]): SketchConstraint | null {
  if (selected.length !== 1 || !selected[0].point) return null;
  const sel = selected[0];
  return (
    constraints.find(
      (c) => c.type === "Fixed" && c.entities[0] === sel.entityId && c.positions?.[0] === sel.point,
    ) ?? null
  );
}

/** Average the representative anchor of each pick (named point / entity anchor). */
function selectionCentroid(selected: SketchSel[], entities: SketchEntity[]): Point2 | null {
  const pts: Point2[] = [];
  for (const sel of selected) {
    const e = entities.find((x) => x.id === sel.entityId);
    if (!e) continue;
    const p = sel.point ? entityPointCoord(e, sel.point) : entityAnchor(e);
    if (p) pts.push(p);
  }
  if (pts.length === 0) return null;
  const sum = pts.reduce((acc, p) => ({ x: acc.x + p.x, y: acc.y + p.y }), { x: 0, y: 0 });
  return { x: sum.x / pts.length, y: sum.y / pts.length };
}
