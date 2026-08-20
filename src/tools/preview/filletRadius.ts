/*
 * Fillet radius drag mapping (PURE math).
 *
 * The fillet L1 preview is a documented limitation: we do NOT re-round the mesh
 * on the frontend (that needs OCCT). Instead a vertical drag ON a selected edge
 * adjusts the radius, which drives (a) the live radius chip and (b) a thickened
 * edge highlight, with the exact rounded body arriving from the debounced L2.
 *
 * Mapping (documented): dragging the pointer UP grows the radius. A pixel of
 * upward travel adds `worldPerPx` world units (1:1 with the world scale at the
 * edge's depth), so the feel is consistent across zoom levels. Radius is clamped
 * to a small positive minimum (a zero-radius fillet is a no-op).
 */
import { MM_SUFFIX, formatMillimetres } from "@/units/format";

export interface RadiusDragOpts {
  /** World units per screen pixel at the edge depth (from the camera). */
  worldPerPx: number;
  /** Minimum radius (world units). Default 0.1. */
  min?: number;
  /** Extra gain on top of the 1:1 world mapping. Default 1. */
  sensitivity?: number;
}

/**
 * Radius after dragging from the grab point. `dyPixels` is `downY - currentY`
 * (screen Y grows downward, so up-drag is positive). Result is clamped ≥ min.
 */
export function radiusFromDrag(
  startRadius: number,
  dyPixels: number,
  opts: RadiusDragOpts,
): number {
  const min = opts.min ?? 0.1;
  const gain = opts.sensitivity ?? 1;
  const delta = dyPixels * opts.worldPerPx * gain;
  return Math.max(min, startRadius + delta);
}

/**
 * Format a radius/depth as document text. W2-A routes it through the shared
 * formatter, so trailing zeros are trimmed (`2` not `2.0`, `83.25` not `83.3`).
 * `radiusFromValueText` below still parses the result, and still parses the
 * RUST-composed `valueText` ("2.0 mm") a re-edit seeds from — both are pinned.
 *
 * MILLIMETRE-FIXED ON PURPOSE (WP-C2): this string is a `valueText`, and
 * `radiusFromValueText` reads it back with `parseFloat`, i.e. as mm. Routing it
 * through the DISPLAY formatter would make a re-edit under `displayUnit = "in"`
 * seed 0.079 mm for a 2 mm fillet — the wire/document boundary must never see a
 * display conversion. Anything a user READS as a measurement uses
 * `formatLengthWithUnit` instead.
 */
export function formatMm(value: number): string {
  return `${formatMillimetres(value)} ${MM_SUFFIX}`;
}

/** Default fillet radius (mirrors modelToolMachine.DEFAULT_FILLET_RADIUS). */
export const DEFAULT_FILLET_RADIUS = 2;

/**
 * Parse a fillet feature's display text ("2.0 mm") back to a radius (re-edit
 * seed; mirrors revolve's `angleFromValueText`). A non-numeric / non-positive
 * value falls back to the default radius.
 */
export function radiusFromValueText(text: string, fallback = DEFAULT_FILLET_RADIUS): number {
  const n = Number.parseFloat(text);
  return Number.isFinite(n) && n > 0 ? n : fallback;
}

/*
 * FILLET-CHAMFER-UNIFY direction-driven edge op (W0 addition below — pure,
 * additive; nothing above this line changes).
 *
 * FLIP_HOLD is the type-hysteresis band (how far a signed drag must cross
 * zero before the FSM re-types Fillet↔Chamfer); MIN_VALUE is the magnitude
 * clamp on the committed radius/distance. The band is deliberately LARGER
 * than the clamp — if it were smaller, a drag sitting right at the clamped
 * floor could ping-pong type on sub-pixel noise. The FSM (W1) owns applying
 * both; this module only supplies the constants and the signed value they
 * gate.
 */
export const EDGE_OP_FLIP_HOLD = 0.25;
export const EDGE_OP_MIN_VALUE = 0.1;

/** A 2D direction in canvas-pixel space. Not a THREE type — this module has
 *  no three.js dependency. */
export interface ScreenAxis {
  x: number;
  y: number;
}

/** Screen convention for the no-projection fallback (T3): up = positive,
 *  reproducing today's `radiusFromDrag` up-grows mapping exactly. */
export const SCREEN_UP_AXIS: ScreenAxis = { x: 0, y: -1 };

/**
 * Unit drag axis from the edge midpoint to its projected outward point, with
 * the projected edge-tangent component removed (a drag ALONG the edge must
 * not register as a radius/distance change). Null when the remaining vector
 * is shorter than `minPx` — a head-on edge (outward and tangent nearly
 * collinear on screen) must not normalize a noise vector into a fabricated
 * axis.
 */
export function screenDragAxis(
  pMid: ScreenAxis,
  pOut: ScreenAxis,
  pTan?: ScreenAxis,
  minPx = 3,
): ScreenAxis | null {
  let ax = pOut.x - pMid.x;
  let ay = pOut.y - pMid.y;
  if (pTan) {
    const tx = pTan.x - pMid.x;
    const ty = pTan.y - pMid.y;
    const tLen = Math.hypot(tx, ty);
    if (tLen > 1e-9) {
      const ux = tx / tLen;
      const uy = ty / tLen;
      const proj = ax * ux + ay * uy;
      ax -= proj * ux;
      ay -= proj * uy;
    }
  }
  const axisLen = Math.hypot(ax, ay);
  if (axisLen < minPx) return null;
  return { x: ax / axisLen, y: ay / axisLen };
}

/**
 * Signed value after dragging `(dxPx, dyPx)` (RAW screen deltas — clientX −
 * downX, clientY − downY; no up-positive massaging) along `axis`. UNCLAMPED
 * and un-abs'd on purpose: the sign lives entirely in `axis` (e.g.
 * `SCREEN_UP_AXIS.y = -1` makes an upward drag positive, matching
 * `radiusFromDrag`'s convention exactly), and the caller owns clamping plus
 * the Fillet/Chamfer type split around zero.
 */
export function signedValueFromDrag(
  startValue: number,
  dxPx: number,
  dyPx: number,
  axis: ScreenAxis,
  opts: RadiusDragOpts,
): number {
  const gain = opts.sensitivity ?? 1;
  const proj = dxPx * axis.x + dyPx * axis.y;
  return startValue + proj * opts.worldPerPx * gain;
}

/*
 * WP4 — clamping a value to what the KERNEL said it will accept.
 *
 * Until this existed the only bound on a fillet radius was `EDGE_OP_MIN_VALUE`,
 * a fixed 0.1 mm floor with NO ceiling at all, so a user learned that 6 mm does
 * not fit on a 10 mm box by arming the op and watching it refuse. The
 * `AnalyzeEdgeOpRange` answer (SCHEMA §7.6) replaces the guess with a
 * measurement, and this function is the only place that measurement becomes a
 * clamp.
 *
 * The whole design rests on ONE rule: `confidence` decides what may be enforced,
 * and it is read BEFORE any bound. Reaching past it to `bestKnownMax` because it
 * happens to be non-null is how a coarse or non-monotonic answer turns into a
 * ceiling the kernel never claimed.
 */

/** The measured half of an `AnalyzeEdgeOpRange` answer this module needs. */
export interface EdgeOpRangeGuard {
  confidence: "none" | "nonMonotonic" | "lowerOnly" | "bracketed" | "coarse";
  lowerBound: number | null;
  bestKnownMax: number | null;
  provenUpperBound: number | null;
  feasibleIntervals: { lower: number; upper: number }[];
}

/** What {@link clampToEdgeOpRange} did, so the caller can say so on screen. */
export interface EdgeOpClampResult {
  /** The value to use. Equal to the input when nothing was enforced. */
  value: number;
  /** True when the guard moved the value. */
  clamped: boolean;
  /** `"floor"` / `"ceiling"` / `"interval"` — which obligation moved it. */
  reason: "none" | "floor" | "ceiling" | "interval";
}

function keep(value: number): EdgeOpClampResult {
  return { value, clamped: false, reason: "none" };
}

/**
 * Clamp `value` to what the analysis PROVED, honouring the confidence ladder.
 *
 * - `none` — nothing was proven, so nothing is enforced. This is also what the
 *   mock lane and a refusal produce, which is why "no answer" and "an answer
 *   that proved nothing" behave identically: neither is evidence.
 * - `nonMonotonic` — an island was observed. A single ceiling would licence a
 *   value inside the gap that was MEASURED to fail, so the intervals are the
 *   answer: a value already inside one is kept, and one outside is pulled to the
 *   nearest interval endpoint.
 * - `lowerOnly` — a floor was proven and no ceiling was. Raise, never cap.
 * - `bracketed` — a complete monotonic bracket. `bestKnownMax` is a hard
 *   ceiling and the largest value actually built.
 * - `coarse` — the search stopped early, so the real frontier may sit far below
 *   `provenUpperBound`. Cap at `bestKnownMax` and NEVER at `provenUpperBound`:
 *   the first is a value the kernel built, the second is only a value it
 *   refused, and everything between them is unmeasured.
 *
 * `provenUpperBound` is therefore never used as a ceiling on any rung. It is
 * carried in the guard because it is what makes `bracketed` meaningful — the
 * frontier is bracketed BETWEEN the two — not because a UI may offer it.
 */
export function clampToEdgeOpRange(value: number, guard?: EdgeOpRangeGuard | null): EdgeOpClampResult {
  if (!guard || guard.confidence === "none") return keep(value);

  if (guard.confidence === "nonMonotonic") {
    const intervals = guard.feasibleIntervals;
    if (intervals.length === 0) return keep(value);
    if (intervals.some((i) => value >= i.lower && value <= i.upper)) return keep(value);
    // Nearest PROBED endpoint. Never an interior point of the gap and never a
    // midpoint: only the endpoints were built.
    let best = intervals[0].lower;
    let bestDistance = Infinity;
    for (const interval of intervals) {
      for (const endpoint of [interval.lower, interval.upper]) {
        const distance = Math.abs(endpoint - value);
        if (distance < bestDistance) {
          bestDistance = distance;
          best = endpoint;
        }
      }
    }
    return { value: best, clamped: true, reason: "interval" };
  }

  // The floor applies on every remaining rung: a proven `lowerBound` is the
  // smallest value the kernel built, so anything under it is known to fail.
  if (guard.lowerBound !== null && value < guard.lowerBound) {
    return { value: guard.lowerBound, clamped: true, reason: "floor" };
  }
  if (guard.confidence === "lowerOnly") return keep(value);

  const ceiling = guard.bestKnownMax;
  if (ceiling !== null && value > ceiling) {
    return { value: ceiling, clamped: true, reason: "ceiling" };
  }
  return keep(value);
}
