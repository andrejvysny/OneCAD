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
