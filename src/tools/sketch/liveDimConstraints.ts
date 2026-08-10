/*
 * Live dimension → DRIVING constraints (PURE). Turns the fields a user TYPED
 * during a draw gesture into `SketchConstraint`s over the entities that same
 * gesture is about to commit.
 *
 * WHAT IS AND IS NOT AUTHORED — the honest rule, stated once and enforced by the
 * per-tool branches below: a typed value always drives the GEOMETRY exactly, but
 * it authors a constraint only where a wire kind exists to express it. Ellipse
 * major/minor (an Ellipse curve takes no constraints at all), arc sweep angle,
 * polygon side count and a FIRST segment's oblique absolute angle author nothing.
 * Everything else becomes Distance / Radius / Diameter / Angle / Horizontal /
 * Vertical.
 *
 * `refs` index into the step's `committed` array exactly like `ToolConstraintSpec`
 * — the tool never saw an id — with one addition: `{ id }` names an entity OUTSIDE
 * the batch (the previous chain segment an `Angle` is measured against). A
 * malformed ref is DROPPED, never guessed, mirroring `resolveToolConstraints`.
 *
 * ANGLE UNIT SEAM: `SketchConstraint.value` for `Angle` is DEGREES — the app
 * domain. This module emits degrees and NEVER converts; `sketchWireMap` /
 * `SetDimension` route every deg↔rad conversion through `ipc/angleUnits.ts`.
 */
import type { ConstraintPosition, SketchConstraint, SketchConstraintType, SketchEntity } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import { norm360, type DimLocks } from "./liveDimension";
import { chainRefAngleDeg } from "./liveDimFrames";

/** A batch index, or an entity id from OUTSIDE this step's committed array. */
export type DimRef = number | { id: string };

export interface DimConstraintSpec {
  type: SketchConstraintType;
  refs: DimRef[];
  /** Aligned 1:1 with `refs`; `undefined` ⇒ an ENTITY-level ref. */
  positions?: (ConstraintPosition | undefined)[];
  /** mm, or DEGREES for `Angle`. Omitted for Horizontal / Vertical. */
  value?: number;
}

/** What the caller knows that the anchors alone cannot say. */
export interface DimAuthorContext {
  /** Entity id of the previous chain segment — the only reference an ABSOLUTE
   *  typed angle can be turned into a (relative) `Angle` constraint against. */
  prevLineId?: string;
  /** Polygon side count — also the batch index of its construction circumcircle. */
  sides?: number;
  /** The line tool's leg is a TANGENT ARC (SP-4 W3), so its one field is a radius
   *  and neither `length` nor `angle` describes what commits. */
  arcMode?: boolean;
}

/** The previous chain segment: its entity id AND its absolute direction. */
interface AnglePrev {
  id: string;
  angleDeg: number;
}

/** Locked angles come off the 1° quantized ladder or a typed whole number, so an
 *  exact-match epsilon is all the ladder rungs need. */
const ANGLE_EPS = 1e-6;

const near = (a: number, b: number): boolean => Math.abs(a - b) < ANGLE_EPS;

/** Fold a signed angle difference into [0, 180] — the range `angleBetweenDeg`
 *  (and therefore every existing Angle badge) already reports in. */
function foldTo0_180(deg: number): number {
  const d = norm360(deg);
  return d > 180 ? 360 - d : d;
}

/**
 * Typed angle → the strongest constraint that expresses it: axis-aligned ⇒
 * Horizontal / Vertical (one entity, no value); otherwise an `Angle` against
 * the previous chain segment; otherwise NOTHING — an oblique first segment
 * has no reference, and inventing one would be a lie.
 *
 * `deg` is ABSOLUTE when there is no `prev` to be relative to (unchanged),
 * and the chip's own SIGNED turn away from `prev.angleDeg` when there is
 * (`liveDimFrames.ts`'s `segmentFrame` — the live chip shows exactly this
 * number, never a different one from what gets authored here). Folding the
 * resolved absolute heading `t` back through `near()` against 0/90/180/270
 * reproduces the OLD absolute-angle ladder exactly; the `Angle` value itself
 * is `foldTo0_180(deg)` — `deg` already IS the signed delta the old code
 * computed as `t − prev.angleDeg`, so the authored VALUE is bit-for-bit
 * unchanged, only its live-chip representation is.
 */
function angleLadder(deg: number, ref: DimRef, prev: AnglePrev | null): DimConstraintSpec[] {
  const t = norm360(prev ? prev.angleDeg + deg : deg);
  if (near(t, 0) || near(t, 180)) return [{ type: "Horizontal", refs: [ref] }];
  if (near(t, 90) || near(t, 270)) return [{ type: "Vertical", refs: [ref] }];
  if (!prev) return [];
  return [{ type: "Angle", refs: [{ id: prev.id }, ref], value: foldTo0_180(deg) }];
}

/** The chain segment BEFORE the one being committed, when there is one — its
 *  direction is `chainRefAngleDeg` (`liveDimFrames.ts`), the SAME reference
 *  the live angle chip measured against, so the chip and the authored
 *  constraint can never disagree about which direction is "0". */
function prevSegment(anchors: Point2[], ctx: DimAuthorContext): AnglePrev | null {
  const angleDeg = chainRefAngleDeg(anchors);
  if (!ctx.prevLineId || angleDeg === undefined) return null;
  return { id: ctx.prevLineId, angleDeg };
}

/** A Distance pinning one line's own two endpoints (`committed[i]`). */
const selfDistance = (i: number, value: number): DimConstraintSpec => ({
  type: "Distance",
  refs: [i, i],
  positions: ["Start", "End"],
  value,
});

// ── per-tool authoring ────────────────────────────────────────────────────────

function lineSpecs(anchors: Point2[], locks: DimLocks, ctx: DimAuthorContext): DimConstraintSpec[] {
  if (anchors.length === 0) return []; // committed = [Line]
  // Arc mode commits an ARC, whose only expressible number is its radius. A
  // length / angle lock left over from a straight leg describes the segment that
  // is NOT being committed, so it authors nothing rather than binding the arc.
  if (ctx.arcMode) {
    return locks.radius === undefined ? [] : [{ type: "Radius", refs: [0], value: locks.radius }];
  }
  const out: DimConstraintSpec[] = [];
  if (locks.length !== undefined) out.push(selfDistance(0, locks.length));
  if (locks.angle !== undefined) out.push(...angleLadder(locks.angle, 0, prevSegment(anchors, ctx)));
  return out;
}

/** rect + centerRect. `rectLines` order puts the HORIZONTAL edge at 0 and the
 *  VERTICAL one at 1, which is what makes W and H land on the right edges. */
function boxSpecs(anchors: Point2[], locks: DimLocks): DimConstraintSpec[] {
  if (anchors.length !== 1) return [];
  const out: DimConstraintSpec[] = [];
  if (locks.width !== undefined) out.push(selfDistance(0, locks.width));
  if (locks.height !== undefined) out.push(selfDistance(1, locks.height));
  return out;
}

/** Ø and R are aliases — `liveDimStep` guarantees at most one is locked, and Ø
 *  (the Tab-order first field) wins if both somehow arrive. */
function circleSpecs(anchors: Point2[], locks: DimLocks): DimConstraintSpec[] {
  if (anchors.length !== 1) return [];
  if (locks.diameter !== undefined) return [{ type: "Diameter", refs: [0], value: locks.diameter }];
  if (locks.radius !== undefined) return [{ type: "Radius", refs: [0], value: locks.radius }];
  return [];
}

/** The radius is locked in phase 1 and PERSISTS into phase 2, where the arc
 *  commits. The sweep authors nothing — no wire kind expresses it. */
function arcSpecs(anchors: Point2[], locks: DimLocks): DimConstraintSpec[] {
  if (anchors.length !== 2 || locks.radius === undefined) return [];
  return [{ type: "Radius", refs: [0], value: locks.radius }];
}

/** 3-point arc: the radius is the ONE field, typed in phase 2 where the arc also
 *  commits. The chord's own length and angle author nothing — a chord is not an
 *  entity, so no wire kind can name it. */
function arc3pSpecs(anchors: Point2[], locks: DimLocks): DimConstraintSpec[] {
  if (anchors.length !== 2 || locks.radius === undefined) return [];
  return [{ type: "Radius", refs: [0], value: locks.radius }];
}

/** The construction circumcircle commits at index `sides`; the vertices ride it
 *  via OnCurve, so its Radius drives the whole ring. Without `sides` that index
 *  is unknowable — drop rather than bind the wrong entity. */
function polygonSpecs(anchors: Point2[], locks: DimLocks, ctx: DimAuthorContext): DimConstraintSpec[] {
  if (anchors.length !== 1 || locks.radius === undefined || ctx.sides === undefined) return [];
  return [{ type: "Radius", refs: [ctx.sides], value: locks.radius }];
}

/** `slotDrafts` order: 0 = wall(+n) (same length as the centreline), 2 = cap@p1
 *  (the authored `Equal(2,3)` ties the other cap to it). The slot's own angle
 *  never has a previous segment — H/V or nothing. */
function slotSpecs(anchors: Point2[], locks: DimLocks): DimConstraintSpec[] {
  if (anchors.length !== 2) return [];
  const out: DimConstraintSpec[] = [];
  if (locks.length !== undefined) out.push(selfDistance(0, locks.length));
  if (locks.width !== undefined) out.push({ type: "Radius", refs: [2], value: locks.width / 2 });
  if (locks.angle !== undefined) out.push(...angleLadder(locks.angle, 0, null));
  return out;
}

/**
 * Locked fields → constraint specs over the step's `committed` array. `anchors`
 * are the gesture's anchors BEFORE the committing click (they identify the phase
 * and carry the previous chain segment's direction).
 */
export function dimConstraintSpecs(
  toolId: string,
  anchors: Point2[],
  locks: DimLocks,
  ctx: DimAuthorContext = {},
): DimConstraintSpec[] {
  switch (toolId) {
    case "line":
      return lineSpecs(anchors, locks, ctx);
    case "rect":
    case "centerRect":
      return boxSpecs(anchors, locks);
    case "circle":
      return circleSpecs(anchors, locks);
    case "arc":
      return arcSpecs(anchors, locks);
    case "arc3p":
      return arc3pSpecs(anchors, locks);
    case "polygon":
      return polygonSpecs(anchors, locks, ctx);
    case "slot":
      return slotSpecs(anchors, locks);
    default:
      return []; // ellipse authors nothing; point has no dimensions
  }
}

/**
 * Bind specs to the ids the step's `committed` drafts were just assigned, in the
 * SAME order (that ordering is the whole meaning of a numeric ref). An
 * out-of-range index drops the spec rather than mis-binding it — the same rule,
 * for the same reason, as `resolveToolConstraints`.
 */
export function resolveDimConstraints(
  specs: DimConstraintSpec[],
  newEntities: SketchEntity[],
  nextConstraintId: () => string,
): SketchConstraint[] {
  const out: SketchConstraint[] = [];
  for (const spec of specs) {
    const entities = spec.refs.map((r) => (typeof r === "number" ? newEntities[r]?.id : r.id));
    if (entities.some((id) => id === undefined)) continue; // malformed ref — never guess
    const c: SketchConstraint = {
      id: nextConstraintId(),
      type: spec.type,
      entities: entities as string[],
    };
    if (spec.positions?.some((p) => p !== undefined)) {
      c.positions = spec.positions.map((p) => p ?? "") as ConstraintPosition[];
    }
    if (spec.value !== undefined) c.value = spec.value;
    out.push(c);
  }
  return out;
}

/** (kind, entity set) — order-insensitive, because "Horizontal on line X" is one
 *  relation however the two producers happened to list its entities. */
const relationKey = (c: SketchConstraint): string =>
  `${c.type}|${[...c.entities].sort().join(",")}`;

/**
 * Drop every authored dimension constraint the batch is ALREADY getting.
 *
 * LOAD-BEARING, not tidiness: a typed 0° authors Horizontal, and `autoConstrain`
 * infers Horizontal for anything within ±5° of an axis — so the two agree by
 * construction on exactly the gestures a user is most likely to type. The draw
 * commit path has no reject-on-conflict lane, so the duplicate would strand the
 * sketch in OverConstrained with no way back.
 */
export function dedupeDimConstraints(
  authored: SketchConstraint[],
  existing: SketchConstraint[],
): SketchConstraint[] {
  const seen = new Set(existing.map(relationKey));
  return authored.filter((c) => !seen.has(relationKey(c)));
}
