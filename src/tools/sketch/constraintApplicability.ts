/*
 * constraintApplicability — PURE applicability matrix, ported VERBATIM from the
 * legacy C++ `onecad::ui::evaluateConstraintApplicability`
 * (worker/src/sketch/ConstraintApplicability.cpp @ b4ddcccc, 2026-07-16 — see
 * that file's header comment for the OneCAD-CPP origin). Cited line numbers
 * below refer to that file unless noted otherwise.
 *
 * The C++ original operates on `SelectionItem[]` resolved against a `Sketch*`,
 * where EVERY entity (including a Line's endpoints) is a separate `EntityID`.
 * Our frontend `SketchEntity` model (src/ipc/types.ts) inlines Line/Circle/Arc
 * coordinates instead of referencing separate point entities (see the
 * `sketchWireMap.ts` header for why) — so targets are pre-resolved via
 * `constraintTarget.ts` (`toConstraintTarget`) into a `kind` tag (point / line /
 * circle / arc) BEFORE this module ever sees them. `evaluateApplicability` below
 * mirrors the C++ control flow 1:1 on top of that resolved vocabulary.
 *
 * No wiring to stores/engine/ipc client — only TYPE imports from `@/ipc/types`
 * plus the sibling pure `constraintTarget.ts` module.
 */
import type { SketchConstraintType, SketchEntity } from "@/ipc/types";
import {
  resolveTargetPoint,
  type SketchConstraintTarget,
  type SketchConstraintTargetPoint,
} from "./constraintTarget";

/** One applicable constraint: its type, the ordered targets to author it with,
 *  and whether it carries a numeric value (Distance/HorizontalDistance/
 *  VerticalDistance/Angle/Radius/Diameter — mirrors `isDimensional` in
 *  sketchWireMap.ts, duplicated here to keep this module import-free of ipc/). */
export interface ApplicableConstraint {
  type: SketchConstraintType;
  targets: SketchConstraintTarget[];
  dimensional: boolean;
}

/**
 * Wire-encodability per constraint type (mirrors `toWireConstraint`,
 * src/ipc/sketchWireMap.ts:265-333). All 18 `SketchConstraintType` kinds are
 * listed: the 13 kinds `toWireConstraint` maps today are `true`; the 5 it
 * currently returns `null` for (Fixed/OnCurve/Tangent/Concentric/Symmetric —
 * `toWireConstraint`'s trailing comment, sketchWireMap.ts:328-330) are ALSO
 * `true` here, flagged `// pending S4b wire extension` — a parallel WP is
 * wiring them in, and this table is the flip-ready target state so that WP
 * flips nothing here. `evaluateApplicability` filters its output through this
 * table (never offer what the wire can't send); a test pins the invariant that
 * every type the matrix can ever emit is `true`.
 */
export const WIRE_ENCODABLE: Record<SketchConstraintType, boolean> = {
  Coincident: true,
  Horizontal: true,
  Vertical: true,
  Fixed: true, // pending S4b wire extension
  Midpoint: true, // pending S4b wire extension — never emitted by evaluateApplicability (excluded, mirrors C++)
  OnCurve: true, // pending S4b wire extension
  Parallel: true,
  Perpendicular: true,
  Tangent: true, // pending S4b wire extension — never emitted (no Tangent rule in the C++ matrix)
  Concentric: true, // pending S4b wire extension
  Equal: true, // never emitted by evaluateApplicability — deliberately excluded, mirrors C++
  Distance: true,
  HorizontalDistance: true,
  VerticalDistance: true,
  Angle: true,
  Radius: true,
  Diameter: true,
  Symmetric: true, // pending S4b wire extension
};

/** Mirrors `isDimensional` (sketchWireMap.ts:254-261/336-338); duplicated (not
 *  imported) so this module stays clear of the `src/ipc/` layer per the module
 *  boundary. Keep in sync if the wire's dimensional set ever changes. */
const DIMENSIONAL: ReadonlySet<SketchConstraintType> = new Set([
  "Distance",
  "HorizontalDistance",
  "VerticalDistance",
  "Angle",
  "Radius",
  "Diameter",
]);

/** SketchTypes.h `constants::COINCIDENCE_TOLERANCE` (1e-6 mm) — the geometric
 *  tolerance for coincidence checks, reused below for coordinate-based joining. */
const COINCIDENCE_TOLERANCE = 1e-6;

function isCircular(t: SketchConstraintTarget): boolean {
  return t.kind === "circle" || t.kind === "arc";
}
function isCurve(t: SketchConstraintTarget): boolean {
  return t.kind === "line" || t.kind === "circle" || t.kind === "arc";
}
function sameCoord(a: [number, number] | null, b: [number, number] | null): boolean {
  if (!a || !b) return false;
  return (
    Math.abs(a[0] - b[0]) <= COINCIDENCE_TOLERANCE && Math.abs(a[1] - b[1]) <= COINCIDENCE_TOLERANCE
  );
}

/**
 * Port of `hasLineBetweenPoints` (ConstraintApplicability.cpp:32-50). The C++
 * version does EXACT entity-id equality (`line->startPointId() == pointA`)
 * because every C++ line stores its two endpoints as separate Point EntityIDs.
 * Our `SketchEntity` Line has no such id refs — only inlined `p0`/`p1`
 * coordinates (see this file's header) — so the faithful equivalent resolves
 * each point target's coordinate and looks for a Line entity whose OWN two
 * endpoints coincide with the two target coordinates (either order), within
 * `COINCIDENCE_TOLERANCE`. This also covers a Line's own two endpoints (the
 * common case — selecting one line's Start + End — is coordinate-identical to
 * itself), so a single coordinate scan subsumes both cases.
 *
 * Returns the JOINING line's target (not just a boolean) — the caller needs it
 * to author Horizontal/Vertical against the actual line entity (`toWireConstraint`
 * expects a `line` ref, not two points; ConstraintApplicability.cpp:149-152 only
 * proves it's applicable, since C++ constraint AUTHORING is a separate path).
 */
function findLineBetweenPoints(
  entities: SketchEntity[],
  a: SketchConstraintTargetPoint,
  b: SketchConstraintTargetPoint,
): SketchConstraintTarget | null {
  const pa = resolveTargetPoint(a, entities);
  const pb = resolveTargetPoint(b, entities);
  if (!pa || !pb) return null;
  for (const e of entities) {
    if (e.type !== "Line" || !e.p0 || !e.p1) continue;
    const joins = (sameCoord(e.p0, pa) && sameCoord(e.p1, pb)) || (sameCoord(e.p0, pb) && sameCoord(e.p1, pa));
    if (joins) return { kind: "line", entityId: e.id };
  }
  return null;
}

/** Canonical dedup key for a target — mirrors the C++ `seen.insert(elementId)`
 *  (ConstraintApplicability.cpp:64-75): a re-selection of the SAME logical thing
 *  is rejected. In our model that "thing" is `(kind, entityId, position)` for a
 *  point (Start/Center-aliased points are normalized to "Start" by
 *  `toConstraintTarget`, so this collapses correctly) or `(kind, entityId)` for
 *  a whole-entity target. */
function targetKey(t: SketchConstraintTarget): string {
  return t.kind === "point" ? `point:${t.entityId}:${t.position}` : `${t.kind}:${t.entityId}`;
}

/**
 * Compute the applicable constraints for a selection of resolved targets
 * (`toConstraintTarget` output), against the full session `entities` (needed
 * for the `findLineBetweenPoints` global scan — ConstraintApplicability.cpp's
 * probe scans `sketch->getAllEntities()`, not just the selection).
 *
 * Ported control flow (ConstraintApplicability.cpp:54-170):
 *   - dedup selection by target identity (cpp:62-75), empty ⇒ `[]` (cpp:78-80)
 *   - abort ⇒ `[]` if any target's entity is missing from `entities`
 *     (cpp:84-90: `if (!entity) return result;`)
 *   - 1 target  (cpp:92-107): line → Horizontal, Vertical; point → Fixed;
 *     circle|arc → Radius, Diameter
 *   - 3 targets (cpp:109-123): 2 points + 1 line → Symmetric (else `[]`)
 *   - counts other than 1/2/3 → `[]` (cpp:125-127)
 *   - 2 targets (cpp:129-169): Distance; point&point → Coincident,
 *     HorizontalDistance, VerticalDistance (+ Horizontal/Vertical if a line
 *     joins them); circular&circular → Concentric; line&line → Parallel,
 *     Perpendicular, Angle; point&curve → OnCurve
 * Equal/Midpoint are deliberately excluded — the C++ matrix never emits them.
 * The result is pre-filtered by `WIRE_ENCODABLE` and its order is deterministic
 * (matches the C++ source's sequential `insert()` calls, since a real
 * `unordered_set` has no defined order of its own).
 */
export function evaluateApplicability(
  targets: SketchConstraintTarget[],
  entities: SketchEntity[],
): ApplicableConstraint[] {
  const deduped: SketchConstraintTarget[] = [];
  const seen = new Set<string>();
  for (const t of targets) {
    const key = targetKey(t);
    if (seen.has(key)) continue;
    seen.add(key);
    deduped.push(t);
  }

  const count = deduped.length;
  if (count === 0) return [];

  const liveIds = new Set(entities.map((e) => e.id));
  for (const t of deduped) if (!liveIds.has(t.entityId)) return [];

  const results: ApplicableConstraint[] = [];
  const push = (type: SketchConstraintType, ts: SketchConstraintTarget[]): void => {
    if (!WIRE_ENCODABLE[type]) return;
    results.push({ type, targets: ts, dimensional: DIMENSIONAL.has(type) });
  };

  if (count === 1) {
    const t = deduped[0];
    if (t.kind === "line") {
      push("Horizontal", [t]);
      push("Vertical", [t]);
    }
    if (t.kind === "point") {
      push("Fixed", [t]);
    }
    if (t.kind === "circle" || t.kind === "arc") {
      push("Radius", [t]);
      push("Diameter", [t]);
    }
    return results;
  }

  if (count === 3) {
    const points = deduped.filter((t) => t.kind === "point");
    const lines = deduped.filter((t) => t.kind === "line");
    if (points.length === 2 && lines.length === 1) {
      push("Symmetric", [points[0], points[1], lines[0]]);
    }
    return results;
  }

  if (count !== 2) return [];

  const [a, b] = deduped;
  const aPoint = a.kind === "point";
  const bPoint = b.kind === "point";
  const aLine = a.kind === "line";
  const bLine = b.kind === "line";

  if ((aPoint || aLine) && (bPoint || bLine)) {
    push("Distance", [a, b]);
  }

  if (aPoint && bPoint) {
    push("Coincident", [a, b]);
    push("HorizontalDistance", [a, b]);
    push("VerticalDistance", [a, b]);
    const line = findLineBetweenPoints(entities, a as SketchConstraintTargetPoint, b as SketchConstraintTargetPoint);
    if (line) {
      push("Horizontal", [line]);
      push("Vertical", [line]);
    }
  }

  if (isCircular(a) && isCircular(b)) {
    push("Concentric", [a, b]);
  }

  if (aLine && bLine) {
    push("Parallel", [a, b]);
    push("Perpendicular", [a, b]);
    push("Angle", [a, b]);
  }

  if ((aPoint && isCurve(b)) || (bPoint && isCurve(a))) {
    push("OnCurve", [a, b]);
  }

  return results;
}
