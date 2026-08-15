/*
 * Per-tool live dimension FRAMES (PURE) — the measure/rebuild pair that turns a
 * cursor position into the numbers a gesture is "about", and back.
 *
 * THE CONTRACT, and the property every frame is tested against:
 *
 *     rebuild(measure(p), p) ≈ p        for every p, to 1e-9
 *
 * `rebuild` therefore must not invent anything the values do not carry: the
 * CURSOR argument supplies everything else — the quadrant a rectangle grows into,
 * which side of a centreline a slot bulges to, the direction a radius points.
 * That is what lets a single locked field pin ONE number while the pointer keeps
 * steering the rest of the gesture.
 *
 * Phases are read off `ToolState.anchors.length`, matching each machine's own
 * click ladder. `line` is the exception: its anchors ACCUMULATE the whole chain
 * (so the snap engine sees every placed vertex), so the active vertex is the LAST
 * anchor, not the first.
 *
 * Chip anchors follow one rule per shape kind: an extent measures at its
 * midpoint, an angle sits ~30% out from its vertex, a radius at the midpoint of
 * the centre→cursor ray, a count at the centre.
 */
import type { Point2 } from "@/viewport/engine/sketchBasis";
import {
  foldSigned180,
  norm360,
  type DimFieldId,
  type DimFieldSpec,
  type DimMeasureFrame,
  type DimValues,
} from "./liveDimension";
import { DEFAULT_POLYGON_SIDES } from "./toolMachine";
import { circumcenter, tangentArcTo } from "./arcMath";

/**
 * One tool phase's dimension frame. `measure` reads the gesture's numbers off a
 * cursor position; `rebuild` puts a (possibly locked / rounded) value set back
 * into a cursor position, preserving everything the values do not describe.
 */
export interface DimFrame extends DimMeasureFrame {
  rebuild(values: DimValues, cursor: Point2): Point2;
}

const DEG = 180 / Math.PI;
/** Below this an extent has no direction left to preserve — fall back to plane +U. */
const DEGENERATE = 1e-12;
/** Angle chips sit this far along the segment from their vertex. */
const ANGLE_ANCHOR_T = 0.3;

const dist = (a: Point2, b: Point2): number => Math.hypot(a.x - b.x, a.y - b.y);
const mid = (a: Point2, b: Point2): Point2 => ({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 });
const lerp = (a: Point2, b: Point2, t: number): Point2 => ({
  x: a.x + (b.x - a.x) * t,
  y: a.y + (b.y - a.y) * t,
});
/** `Math.sign` but 0 reads POSITIVE — a zero-extent gesture must still pick a side. */
const side = (n: number): number => (n < 0 ? -1 : 1);
const angleDegOf = (from: Point2, to: Point2): number =>
  norm360(Math.atan2(to.y - from.y, to.x - from.x) * DEG);

/** `ctr` + `r` along the direction of `toward` (plane +U when degenerate). */
function radial(ctr: Point2, r: number, toward: Point2): Point2 {
  const dx = toward.x - ctr.x;
  const dy = toward.y - ctr.y;
  const len = Math.hypot(dx, dy);
  if (len < DEGENERATE) return { x: ctr.x + r, y: ctr.y };
  return { x: ctr.x + (r * dx) / len, y: ctr.y + (r * dy) / len };
}

const field = (
  id: DimFieldId,
  label: string,
  domain: DimFieldSpec["domain"],
  drives: boolean,
  anchor: (p: Point2) => Point2,
  clusterId?: string,
  axisFrom?: (p: Point2) => Point2,
): DimFieldSpec => ({ field: id, label, domain, drives, anchor, clusterId, axisFrom });

/** Single-field value set. Written through a local rather than as a computed-key
 *  literal so the union-typed key stays a `DimFieldId`, not a `string` index. */
function one(id: DimFieldId, value: number): DimValues {
  const out: DimValues = {};
  out[id] = value;
  return out;
}

// ── line / slot centreline: length + angle against a reference ───────────────

/** Shared by every `segmentFrame` — its length + angle anchors sit close
 *  together on a short segment and are meant to visually pair up. */
const SEGMENT_CLUSTER = "segment-len-ang";

/**
 * The chain's reference direction — the ABSOLUTE heading of the segment
 * ending at the second-to-last anchor, when a previous leg exists. Shared by
 * the live angle chip (`segmentFrame`, below) and constraint authoring
 * (`liveDimConstraints.ts`'s `prevSegment`) so the two can never disagree
 * about which direction a chained angle is measured against.
 */
export function chainRefAngleDeg(anchors: Point2[]): number | undefined {
  const n = anchors.length;
  return n < 2 ? undefined : angleDegOf(anchors[n - 2], anchors[n - 1]);
}

/**
 * `a` → cursor as a length and an angle. Shared by the line tool, the slot's
 * centreline phase (identical gesture, identical numbers) and the 3-point
 * arc's CHORD phase — which passes `drives = false`, because a chord is not
 * an entity and no wire kind can express its length or angle.
 *
 * The ANGLE chip is opt-in (`refAngleDeg`): it appears only when the gesture
 * is sketching AGAINST a reference — for the line tool that is a previous
 * chain segment (`chainRefAngleDeg`), the one thing a typed angle can
 * genuinely be measured and constrained against. A FIRST segment has no
 * reference (oblique values author nothing; H/V is auto-inferred at commit),
 * so a floating "∠" chip there is noise, and it is withheld.
 *
 * WITH a reference, the angle is the SIGNED turn from it, folded to the
 * SHORTER arc `(-180°, 180°]` — matching exactly what
 * `liveDimConstraints.ts`'s `angleLadder` authors as the driving `Angle`
 * constraint's value, so the live chip and the committed constraint are never
 * two different numbers. Reflex values (270°, 258°…) never appear: the
 * LONGER way round is never what the chip shows, only the direction and
 * magnitude of the actual turn away from the previous leg. WITHOUT a
 * reference, the angle stays the raw absolute heading to plane +U (unchanged
 * from before — the arc3p chord and a first segment have nothing to fold
 * against).
 */
function segmentFrame(a: Point2, drives = true, refAngleDeg?: number): DimFrame {
  const fields: DimFieldSpec[] = [
    // axisFrom = the segment's own fixed start anchor — the length chip then
    // floats to the side of the line instead of sitting directly on top of it
    // (matches how a constraint badge sits beside its owning entity).
    field("length", "L", "length", drives, (p) => mid(a, p), SEGMENT_CLUSTER, () => a),
  ];
  if (refAngleDeg !== undefined) {
    fields.push(
      // Drives only through the angle LADDER: 0/180 → Horizontal, 90/270 →
      // Vertical, else an Angle against the previous chain segment.
      //
      // NO clusterId: this anchor is provisional anyway — the controller
      // (`SketchController.syncAngleReference`) unconditionally overrides it
      // to the arc preview's own bisector, a FIXED spot relative to the arc's
      // grid-constant radius. Sharing a cluster with `length` would let
      // `HtmlOverlayDriver`'s screen-space push-apart drag it back off that
      // spot whenever the (length-dependent, moving) length chip lands close
      // by — exactly the "sometimes escapes the arc" bug this avoids.
      field("angle", "∠", "angle", drives, (p) => lerp(a, p, ANGLE_ANCHOR_T)),
    );
  }
  const angleOf = (p: Point2): number =>
    refAngleDeg === undefined ? angleDegOf(a, p) : foldSigned180(angleDegOf(a, p) - refAngleDeg);
  return {
    fields,
    measure: (p) => ({ length: dist(a, p), angle: angleOf(p) }),
    rebuild: (v, c) => {
      const len = v.length ?? dist(a, c);
      const rel = v.angle ?? angleOf(c);
      const th = (refAngleDeg === undefined ? rel : refAngleDeg + rel) / DEG;
      return { x: a.x + len * Math.cos(th), y: a.y + len * Math.sin(th) };
    },
  };
}

// ── rect / centerRect: axis-aligned box ──────────────────────────────────────

/**
 * `rect` (k = 1, corner→corner) and `centerRect` (k = 2, centre→corner) share one
 * frame: both measure a box off one anchor, and the only differences are the
 * extent FACTOR — centerRect reports FULL extents, because that is the number a
 * user means by "80 wide" — and where the two chips sit on the box.
 */
function boxFrame(a: Point2, k: number): DimFrame {
  const widthAnchor = (p: Point2): Point2 =>
    k === 1 ? { x: (a.x + p.x) / 2, y: a.y } : { x: a.x, y: p.y };
  const heightAnchor = (p: Point2): Point2 =>
    k === 1 ? { x: p.x, y: (a.y + p.y) / 2 } : { x: p.x, y: a.y };
  return {
    fields: [
      field("width", "W", "length", true, widthAnchor),
      field("height", "H", "length", true, heightAnchor),
    ],
    measure: (p) => ({ width: k * Math.abs(p.x - a.x), height: k * Math.abs(p.y - a.y) }),
    rebuild: (v, c) => ({
      x: a.x + (side(c.x - a.x) * (v.width ?? k * Math.abs(c.x - a.x))) / k,
      y: a.y + (side(c.y - a.y) * (v.height ?? k * Math.abs(c.y - a.y))) / k,
    }),
  };
}

// ── circle / arc / ellipse / polygon: radial ─────────────────────────────────

/**
 * `radius` wins over its `diameter` alias whenever BOTH survive projection.
 * Rounding the radius keeps both chips on whole numbers (Ø is then a multiple of
 * 2q); rounding the diameter would strand the R chip on half-steps. A LOCK drops
 * the sibling value upstream (`projectEvent`), so the locked field is the only
 * one that reaches here.
 */
const resolveRadius = (v: DimValues, fallback: number): number =>
  v.radius ?? (v.diameter !== undefined ? v.diameter / 2 : fallback);

function circleFrame(ctr: Point2): DimFrame {
  const anchor = (p: Point2): Point2 => mid(ctr, p);
  // Diameter and radius share ONE anchor point by construction — cluster them
  // so they separate instead of stacking exactly on top of each other.
  const cluster = "circle-r-d";
  return {
    fields: [
      field("diameter", "Ø", "length", true, anchor, cluster),
      field("radius", "R", "length", true, anchor, cluster),
    ],
    measure: (p) => ({ diameter: 2 * dist(ctr, p), radius: dist(ctr, p) }),
    rebuild: (v, c) => radial(ctr, resolveRadius(v, dist(ctr, c)), c),
  };
}

/** A lone radial extent — the arc's radius phase and the ellipse's major axis. */
function radialFrame(ctr: Point2, id: DimFieldId, label: string, drives: boolean): DimFrame {
  return {
    fields: [field(id, label, "length", drives, (p) => mid(ctr, p))],
    measure: (p) => one(id, dist(ctr, p)),
    rebuild: (v, c) => radial(ctr, v[id] ?? dist(ctr, c), c),
  };
}

/**
 * Arc phase 2 — the CCW sweep from the placed start point, in [0, 360).
 *
 * `rebuild` keeps the CURSOR's own radius rather than the start point's: `arcTool`
 * projects the click onto the circle regardless, so the arc is identical either
 * way, and preserving it makes the round-trip property exact for ANY cursor
 * instead of only for ones already sitting on the circle. A zero sweep reads 0;
 * rejecting 0 and 360 is the chip's input validation, not a measurement rule.
 */
function arcSweepFrame(ctr: Point2, start: Point2): DimFrame {
  const base = angleDegOf(ctr, start);
  const sweep = (p: Point2): number => norm360(angleDegOf(ctr, p) - base);
  return {
    fields: [field("angle", "∠", "angle", false, (p) => lerp(ctr, p, ANGLE_ANCHOR_T))],
    measure: (p) => ({ angle: sweep(p) }),
    rebuild: (v, c) => {
      const a = (base + (v.angle ?? sweep(c))) / DEG;
      const r = dist(ctr, c);
      return { x: ctr.x + r * Math.cos(a), y: ctr.y + r * Math.sin(a) };
    },
  };
}

function polygonFrame(ctr: Point2, sides: number): DimFrame {
  return {
    fields: [
      // Radius FIRST: an armed digit types the radius (the side count is the
      // idle-state digit verb — SketchController routes 3-9 to `stepSides`).
      field("radius", "R", "length", true, (p) => mid(ctr, p)),
      field("sides", "n", "count", false, () => ctr),
    ],
    measure: (p) => ({ radius: dist(ctr, p), sides }),
    // The side count never MOVES the vertex — it is a machine verb, not geometry.
    rebuild: (v, c) => radial(ctr, v.radius ?? dist(ctr, c), c),
  };
}

// ── ellipse minor / slot width: perpendicular extent off an axis ─────────────

interface Axis {
  o: Point2;
  /** Unit axis direction (plane +U when the axis is degenerate). */
  d: Point2;
  /** Unit CCW normal of `d`. */
  n: Point2;
}

function axisOf(o: Point2, end: Point2): Axis {
  const dx = end.x - o.x;
  const dy = end.y - o.y;
  const len = Math.hypot(dx, dy);
  if (len < DEGENERATE) return { o, d: { x: 1, y: 0 }, n: { x: 0, y: 1 } };
  const d = { x: dx / len, y: dy / len };
  return { o, d, n: { x: -d.y, y: d.x } };
}

/** SIGNED perpendicular offset of `p` from the axis (sign = which side). */
const offsetOf = (ax: Axis, p: Point2): number =>
  (p.x - ax.o.x) * ax.n.x + (p.y - ax.o.y) * ax.n.y;

/** `p` projected onto the infinite axis line. */
function project(ax: Axis, p: Point2): Point2 {
  const t = (p.x - ax.o.x) * ax.d.x + (p.y - ax.o.y) * ax.d.y;
  return { x: ax.o.x + ax.d.x * t, y: ax.o.y + ax.d.y * t };
}

/**
 * A perpendicular extent off an axis: the ellipse's minor radius (k = 1) and the
 * slot's FULL width (k = 2 — a slot is quoted by its width, not its cap radius).
 * The cursor's SIDE of the axis is preserved, so a locked value never flips the
 * shape across its own centreline.
 */
function perpFrame(
  o: Point2,
  end: Point2,
  id: DimFieldId,
  label: string,
  drives: boolean,
  k: number,
): DimFrame {
  const ax = axisOf(o, end);
  return {
    fields: [field(id, label, "length", drives, (p) => mid(project(ax, p), p))],
    measure: (p) => one(id, k * Math.abs(offsetOf(ax, p))),
    rebuild: (v, c) => {
      const off = offsetOf(ax, c);
      const w = (v[id] ?? k * Math.abs(off)) / k;
      const base = project(ax, c);
      return { x: base.x + ax.n.x * side(off) * w, y: base.y + ax.n.y * side(off) * w };
    },
  };
}

// ── 3-point arc: the radius of the arc through two placed endpoints ──────────

/**
 * `arc3p` phase 2 — ONE field, the RADIUS of the arc through both placed
 * endpoints and the cursor.
 *
 * Stated in the chord's own frame (origin S, +d along the chord, +n its CCW
 * normal): the centre is forced onto the perpendicular bisector at u = c/2, so
 * only its HEIGHT k is free, and
 *
 *     k = (u² − c·u + v²) / (2v)          R² = k² + (c/2)²
 *
 * `rebuild` has that one degree of freedom to spend and three cursor facts to
 * preserve: where along the chord the cursor sits (u), which side of the chord
 * the centre is on (sign k), and which side of the centre's own height the
 * cursor is on — the last is what keeps a minor arc minor instead of flipping to
 * the major one. A radius below c/2 describes no circle through both endpoints,
 * so it is CLAMPED to c/2 (the smallest arc that still spans the chord) rather
 * than rejected mid-gesture.
 */
function arcThroughFrame(s: Point2, e: Point2): DimFrame {
  const ax = axisOf(s, e);
  const half = dist(s, e) / 2;
  const along = (p: Point2): number => (p.x - s.x) * ax.d.x + (p.y - s.y) * ax.d.y;
  /** Centre height in the chord frame; ±∞ for a cursor ON the chord line. */
  const heightOf = (u: number, v: number): number => (u * u - 2 * half * u + v * v) / (2 * v);
  const radiusAt = (p: Point2): number => {
    const ctr = circumcenter(s, e, p);
    return ctr ? dist(ctr, s) : 0;
  };
  return {
    // Anchored off the CHORD midpoint, not the centre: the centre moves with the
    // cursor (and runs to infinity as the arc flattens), the chord does not.
    fields: [field("radius", "R", "length", true, (p) => mid(mid(s, e), p))],
    measure: (p) => ({ radius: radiusAt(p) }),
    rebuild: (v, c) => {
      const off = offsetOf(ax, c);
      const k0 = heightOf(along(c), off);
      const r = Math.max(v.radius ?? radiusAt(c), half);
      const k = side(k0) * Math.sqrt(Math.max(0, r * r - half * half));
      // A cursor past the circle's own extent along the chord names a u the
      // circle never reaches: clamp it to the extreme point, so a typed radius is
      // still exactly the radius that commits. Never clamps in the identity case
      // (the cursor is on its own circle), so the round-trip stays exact.
      const u = Math.min(Math.max(along(c), half - r), half + r);
      const w = Math.sqrt(Math.max(0, r * r - (u - half) * (u - half)));
      const off2 = k + side(off - k0) * w;
      return { x: s.x + ax.d.x * u + ax.n.x * off2, y: s.y + ax.d.y * u + ax.n.y * off2 };
    },
  };
}

// ── line tool, arc mode: the radius of the tangent arc off the last anchor ────

/**
 * The line tool's TANGENT-ARC leg (SP-4 W3) — ONE field, the arc's RADIUS.
 *
 * There is exactly one arc leaving `anchor` along `t` through any given cursor
 * (`tangentArcTo`), so the radius is the whole of what a number can say here. The
 * frame therefore has one degree of freedom to spend and two cursor facts to
 * preserve: which SIDE of the tangent the arc bends to (σ, the sign of the signed
 * centre offset — `startsAtAnchor` reports it, since s > 0 ⇔ CCW ⇔ the arc starts
 * at the anchor) and how far around it TURNS (θ, the cursor's own sweep).
 *
 * Re-radiusing is then a rigid statement: put the centre back on the same side at
 * the new distance, C' = anchor + σ·R·n, and travel the same turn from the anchor,
 * cursor' = C' + rot(σ·θ)·(anchor − C'). At R = R_cursor that IS the cursor
 * (rotating the anchor's radial by the CCW sweep lands on the far end by
 * definition), so the round-trip is exact rather than merely close.
 *
 * A cursor on the tangent ray describes no finite circle: the value set says
 * nothing that could place a point, so `rebuild` hands the cursor back unchanged.
 */
function tangentArcFrame(anchor: Point2, t: Point2): DimFrame {
  const len = Math.hypot(t.x, t.y);
  // Unit normal of the tangent — the line every candidate centre sits on.
  const n: Point2 =
    len < DEGENERATE ? { x: 0, y: 1 } : { x: -t.y / len, y: t.x / len };
  return {
    fields: [field("radius", "R", "length", true, (p) => mid(anchor, p))],
    measure: (p) => ({ radius: tangentArcTo(anchor, t, p)?.radius ?? 0 }),
    rebuild: (v, c) => {
      const arc = tangentArcTo(anchor, t, c);
      if (!arc) return c;
      // |R|: a negative typed radius would mirror the arc across its own tangent,
      // which is a side flip the cursor — not the number — is meant to own.
      const r = Math.abs(v.radius ?? arc.radius);
      const sigma = arc.startsAtAnchor ? 1 : -1;
      const ctr = { x: anchor.x + sigma * r * n.x, y: anchor.y + sigma * r * n.y };
      const a = sigma * arc.sweep;
      const cos = Math.cos(a);
      const sin = Math.sin(a);
      const vx = anchor.x - ctr.x;
      const vy = anchor.y - ctr.y;
      return { x: ctr.x + cos * vx - sin * vy, y: ctr.y + sin * vx + cos * vy };
    },
  };
}

// ── phase table ───────────────────────────────────────────────────────────────

/** What the LINE tool's chain is doing that its anchors cannot say (SP-4 W3). */
export interface DimChain {
  /** The next leg is a tangent arc. */
  arcMode?: boolean;
  /** The direction the chain leaves its last anchor travelling in. */
  tangent?: Point2;
}

/**
 * The frame for `toolId` at the phase `anchors` describes, or null when the tool
 * has no live dimensions there (point tool, phase 0, an unknown id).
 *
 * `sides` is `ToolState.sides` — polygon only. `chain` is the line tool's arc
 * mode + chain tangent; omitted (the default everywhere else) it changes nothing.
 */
export function dimFrame(
  toolId: string,
  anchors: Point2[],
  sides?: number,
  chain?: DimChain,
): DimFrame | null {
  const n = anchors.length;
  if (n === 0) return null;
  switch (toolId) {
    case "line":
      // Anchors accumulate the whole chain — the ACTIVE vertex is the last one.
      // In arc mode the leg is an arc, so length + absolute angle stop describing
      // it and the radius takes over as the single field.
      if (chain?.arcMode && chain.tangent) return tangentArcFrame(anchors[n - 1], chain.tangent);
      // The chain accumulated every placed vertex — the ACTIVE leg is the one
      // leaving the last one. Only a leg against a PREVIOUS segment has an angle
      // to be measured and authored; a first leg draws freely.
      return segmentFrame(anchors[n - 1], true, chainRefAngleDeg(anchors));
    case "rect":
      return n === 1 ? boxFrame(anchors[0], 1) : null;
    case "centerRect":
      return n === 1 ? boxFrame(anchors[0], 2) : null;
    case "circle":
      return n === 1 ? circleFrame(anchors[0]) : null;
    case "arc":
      if (n === 1) return radialFrame(anchors[0], "radius", "R", true);
      // The sweep authors nothing — no wire kind expresses an arc's own sweep.
      return n === 2 ? arcSweepFrame(anchors[0], anchors[1]) : null;
    case "arc3p":
      // Phase 1 is the CHORD: it steers the gesture but is not an entity, so
      // neither of its numbers can author anything (drives = false) and it has
      // no line to be angled against (no angle chip, either).
      if (n === 1) return segmentFrame(anchors[0], false);
      return n === 2 ? arcThroughFrame(anchors[0], anchors[1]) : null;
    case "ellipse":
      // Neither axis authors anything: an Ellipse curve takes no constraints.
      if (n === 1) return radialFrame(anchors[0], "major", "a", false);
      return n === 2 ? perpFrame(anchors[0], anchors[1], "minor", "b", false, 1) : null;
    case "polygon":
      return n === 1 ? polygonFrame(anchors[0], sides ?? DEFAULT_POLYGON_SIDES) : null;
    case "slot":
      // The centreline is a FIRST segment too — it has no previous chain leg to
      // be angled against, so only its length is a live chip.
      if (n === 1) return segmentFrame(anchors[0], true);
      return n === 2 ? perpFrame(anchors[0], anchors[1], "width", "W", true, 2) : null;
    default:
      return null;
  }
}
