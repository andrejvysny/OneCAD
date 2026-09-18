/*
 * Fillet radius drag mapping (PURE math).
 *
 * The fillet L1 preview is a documented limitation: we do NOT re-round the mesh
 * on the frontend (that needs OCCT). Instead a drag on the value handle adjusts
 * the radius, which drives (a) the live radius chip and (b) a thickened edge
 * highlight, with the exact rounded body arriving from the debounced L2.
 *
 * Mapping: the pointer's travel inverted through the gesture's FROZEN mapping
 * (`handleProjection.sampleMapping`). The floor, and its rebase, are the
 * caller's (`depthProjection.flooredDrag` at `EDGE_OP_MIN_VALUE`).
 */
import { MM_SUFFIX, formatMillimetres } from "@/units/format";
import { handlePointAt, type LinearHandlePath, type ValueWitness } from "./handleProjection";
import type { Vec3 } from "./depthProjection";

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
 * FILLET-CHAMFER-UNIFY edge op (W0). MIN_VALUE is the smallest radius/distance
 * the op authors: a drag floors (and rebases) there, and a typed value below it
 * is refused, never converted (spec §8.2). The type is the explicit segment's
 * alone — no drag direction re-types it.
 */
export const EDGE_OP_MIN_VALUE = 0.1;

/*
 * H8 — the Fillet/Chamfer handle ATTACHMENT (derivation §5 "Fillet / Chamfer",
 * §7 "False fillet measurement").
 *
 * A generic GEOMETRIC fillet attachment is not available from what the frontend
 * has. For a certified convex right-angle corner the true blend midpoint moves
 * `−(√2−1)·q·b` — 0.828427 mm INWARD at q = 2 mm — and on the corresponding
 * concave corner that sign reverses. Neither is `+q·b`, and adjacent-face
 * identities alone certify neither the blend section nor the convexity. Drawing
 * the parameter handle as a fillet contact point would therefore visibly lie.
 *
 * So the handle is an explicit PARAMETER construction: one representative edge,
 * one point on it, and a segment whose length IS the value being dragged.
 */

/**
 * The world path the edge op's handle travels: `H(q) = E + q·b`, `dH/dq = b`.
 *
 * `anchor` is the representative edge's display attachment point
 * (`edgeDirection.edgePolylinePoint`) and `outward` its own locally resolved
 * outward direction — never a chain mean, which cancels on a closed contour.
 * Null for a degenerate or non-finite input, the module-wide refusal convention.
 */
export function edgeParameterPath(anchor: Vec3, outward: Vec3): LinearHandlePath | null {
  if (!anchor.every((v) => Number.isFinite(v))) return null;
  if (!outward.every((v) => Number.isFinite(v))) return null;
  const l = Math.hypot(outward[0], outward[1], outward[2]);
  if (!(l > 0)) return null;
  return {
    q0Mm: 0,
    point0Mm: [anchor[0], anchor[1], anchor[2]],
    dPointDValue: [outward[0] / l, outward[1] / l, outward[2] / l],
  };
}

/**
 * The witness segment E → H(q) and the ONLY claim it may make.
 *
 * It measures the construction's length q. It does NOT measure the resulting
 * fillet's radius from a centre, the chamfer's bevel width, or any displacement
 * of the edge — hence `parameterConstruction` and the "parameter" wording. A
 * chain says how many edges share the one value, because the handle sits on one
 * representative and the op applies to all of them.
 */
export function edgeParameterWitness(
  kind: "Fillet" | "Chamfer",
  path: LinearHandlePath,
  valueMm: number,
  edgeCount: number,
): ValueWitness {
  const noun = kind === "Chamfer" ? "Chamfer distance parameter" : "Radius parameter";
  const shared = edgeCount > 1 ? ` · shared by ${edgeCount} edges` : "";
  return {
    meaning: "parameterConstruction",
    label: `${noun}${shared}`,
    fromMm: path.point0Mm,
    toMm: handlePointAt(path, valueMm),
  };
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
