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
  norm360,
  type DimFieldId,
  type DimFieldSpec,
  type DimMeasureFrame,
  type DimValues,
} from "./liveDimension";
import { DEFAULT_POLYGON_SIDES } from "./toolMachine";

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
): DimFieldSpec => ({ field: id, label, domain, drives, anchor });

/** Single-field value set. Written through a local rather than as a computed-key
 *  literal so the union-typed key stays a `DimFieldId`, not a `string` index. */
function one(id: DimFieldId, value: number): DimValues {
  const out: DimValues = {};
  out[id] = value;
  return out;
}

// ── line / slot centreline: length + absolute angle ──────────────────────────

/** `a` → cursor as a length and an ABSOLUTE angle to plane +U. Shared by the line
 *  tool and the slot's centreline phase (identical gesture, identical numbers). */
function segmentFrame(a: Point2): DimFrame {
  return {
    fields: [
      field("length", "L", "length", true, (p) => mid(a, p)),
      // Drives only through the angle LADDER: 0/180 → Horizontal, 90/270 →
      // Vertical, else an Angle against the previous chain segment. A first
      // segment's oblique angle has no reference, so it stays geometry-only.
      field("angle", "∠", "angle", true, (p) => lerp(a, p, ANGLE_ANCHOR_T)),
    ],
    measure: (p) => ({ length: dist(a, p), angle: angleDegOf(a, p) }),
    rebuild: (v, c) => {
      const len = v.length ?? dist(a, c);
      const th = (v.angle ?? angleDegOf(a, c)) / DEG;
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
  return {
    fields: [
      field("diameter", "Ø", "length", true, anchor),
      field("radius", "R", "length", true, anchor),
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

// ── phase table ───────────────────────────────────────────────────────────────

/**
 * The frame for `toolId` at the phase `anchors` describes, or null when the tool
 * has no live dimensions there (point tool, phase 0, an unknown id).
 *
 * `sides` is `ToolState.sides` — polygon only.
 */
export function dimFrame(toolId: string, anchors: Point2[], sides?: number): DimFrame | null {
  const n = anchors.length;
  if (n === 0) return null;
  switch (toolId) {
    case "line":
      // Anchors accumulate the whole chain — the ACTIVE vertex is the last one.
      return segmentFrame(anchors[n - 1]);
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
    case "ellipse":
      // Neither axis authors anything: an Ellipse curve takes no constraints.
      if (n === 1) return radialFrame(anchors[0], "major", "a", false);
      return n === 2 ? perpFrame(anchors[0], anchors[1], "minor", "b", false, 1) : null;
    case "polygon":
      return n === 1 ? polygonFrame(anchors[0], sides ?? DEFAULT_POLYGON_SIDES) : null;
    case "slot":
      if (n === 1) return segmentFrame(anchors[0]);
      return n === 2 ? perpFrame(anchors[0], anchors[1], "width", "W", true, 2) : null;
    default:
      return null;
  }
}
