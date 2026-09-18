/*
 * handleProjection — where a value handle actually moves ON SCREEN, how
 * trustworthy that answer is, and how to invert it exactly while dragging.
 *
 * PURE matrix math on number triples and a flat 16-number matrix, so it unit-
 * tests with no THREE and no WebGL, exactly like `transformDrag` and
 * `depthProjection`. The engine supplies `projectionMatrix * matrixWorldInverse`;
 * nothing here knows what a camera is.
 *
 * WHY THIS EXISTS. `DragHandle.orient()` used to derive the arrow's screen angle
 * by rotating the world axis into CAMERA space and taking `atan2(y, x)` of the
 * result. That is exact for an ORTHOGRAPHIC camera, where projection is linear
 * and every world direction maps to one screen direction no matter where it
 * sits. Under PERSPECTIVE it is not: the projection divides by `w`, so the
 * screen direction of a world axis depends on the ANCHOR's position in the
 * frustum as well as its orientation.
 *
 * The fix is the real derivative. For anchor `P`, direction `a` and
 * view-projection `M`, with `c = M·[P,1]` and `d = M·[a,0]`, the clip-space
 * point is `c` and moving one unit along `a` adds `d`. NDC is `c.xy/c.w`, so by
 * the quotient rule the NDC derivative along `a` is
 *
 *     dNDC = (d.xy · c.w − c.xy · d.w) / c.w²
 *
 * and CSS pixels follow by the viewport half-extents, with Y flipped because
 * NDC +Y is up and CSS +Y is down.
 *
 * THE HANDLE MOVES (docs/design/astra/modeling-handle-attachment.md §3, §5). A
 * value handle is the point `H(q) = H0 + v·(q − q0)` — it travels as the value
 * changes, and the direction it travels on screen is `J·v`. Two consequences the
 * derivation proves and this module encodes:
 *
 *  - **β = 0.** The projected edge TANGENT is not rejected from that derivative.
 *    A rejected direction is one the point cannot follow: a β = 0.5 blend
 *    overstates the scalar response by exactly 1.2×, and full rejection at a 1°
 *    screen angle rotates the drawn direction ~89° and multiplies the gain by
 *    57.2987. The tangent is edge decoration, never part of the mapping.
 *  - **The inverse is exact, not a constant gain.** `p / g` is right only for an
 *    orthographic camera or constant-depth motion. Under perspective the frozen
 *    coefficients `g` and `k` give `Δq = p / (g − k·p)`, whose derivative
 *    `g / (g − k·p)²` is strictly positive — so one frozen mapping tracks the
 *    pointer exactly for the whole gesture, with no mid-gesture strategy switch.
 *
 * CONDITIONING IS A SCALAR SENSITIVITY, NOT A SINE. `Cq = g · worldPerPx` is
 * dimensionless and includes any semantic factor the path carries (Diameter's
 * `v = r̂/2` halves it). It is NOT |sin∠(axis, view ray)| away from the optical
 * axis: the singular values of the projected derivative are `1` and
 * `sqrt(1 + (x/D)² + (y/D)²)`, so off-centre conditioning can exceed 1 — at FOV
 * 76 and NDC (0.75, 0.65) the +Z conditioning is 0.775404 while the sine is
 * 0.612771. It is reported UNCLAMPED, because clamping hides real sensitivity.
 *
 * DEGENERATE VIEWS ARE REFUSED, NOT APPROXIMATED, and the refusals are
 * SCALE-RELATIVE. Absolute floors (`clip w > 1e-6`, `px/world > 1e-9`) break
 * similarity and unit invariance: shrink a scene and its camera by λ and the
 * same view stops working. Every guard here is a conservative floating-point
 * error bound on the quantity it protects, so the answer depends on the view,
 * never on the units the model happens to be authored in.
 */
import type { Vec3 } from "./depthProjection";

/**
 * A view-projection matrix in THREE's `Matrix4.elements` order — COLUMN-major,
 * so `m[0], m[4], m[8], m[12]` is the first ROW.
 */
export type Mat4 = readonly number[];

/** A direction or displacement in CSS-pixel space (+Y DOWN). */
export type Vec2 = readonly [number, number];

/*
 * ARITHMETIC EPSILONS (derivation §3). `u` is the IEEE double unit roundoff and
 * `γ_n = n·u / (1 − n·u)` is Higham's accumulated-rounding factor: a four-term
 * dot product's error is bounded by `γ₇ · Σ|xᵢyᵢ|`. Everything below is a bound
 * on the OPERANDS, so it scales with them — which is exactly what a fixed
 * `1e-6` does not do.
 */
const UNIT_ROUNDOFF = Number.EPSILON / 2; // u = 2⁻⁵³
const gamma = (n: number): number => (n * UNIT_ROUNDOFF) / (1 - n * UNIT_ROUNDOFF);
/** γ₇ = 7.771561172376102e-16 — a 4-term dot product (one row of `M · v`). */
const GAMMA_DOT4 = gamma(7);
/**
 * Relative bound for a product of two such dot products: each factor carries γ₇
 * and the multiply itself carries `u`. Conservative on purpose — the whole point
 * of the guard is that it may refuse a usable view, never accept a fabricated one.
 */
const GAMMA_PRODUCT = 2 * GAMMA_DOT4 + UNIT_ROUNDOFF;

/**
 * Sensitivity floor below which a world mapping is not worth offering, in
 * dimensionless CSS px of handle travel per CSS px of pointer travel.
 *
 * τ = 0.08 is a USABILITY BUDGET, not a numerical tolerance: it is the point
 * where a handle should stop pretending and hand the user a labelled screen
 * proxy, and it bounds the initial scalar gain at `dq/dp ≤ worldPerPx / τ`.
 * Nothing about floating point or OCCT tolerance derives it. It is deliberately
 * above `transformDrag.MIN_VIEW_SIN` (0.05 ≈ 3°), which is where a projection
 * becomes numerically destructive and must refuse outright.
 */
export const MIN_AXIS_CONDITIONING = 0.08;

/** The projected behaviour of one world axis at one anchor, in CSS pixels. */
export interface AxisProjection {
  /**
   * CSS px travelled per world unit along the axis, as `[dx, dy]`. This is the
   * derivative, not a difference of two projected points, so it is exact at the
   * anchor rather than averaged over a step.
   */
  readonly derivative: Vec2;
  /** `|derivative|` — CSS px per world unit. Always finite and > 0. */
  readonly pxPerWorld: number;
  /** Unit screen direction the motion runs along. */
  readonly direction: Vec2;
}

const finite3 = (v: Vec3): boolean =>
  Number.isFinite(v[0]) && Number.isFinite(v[1]) && Number.isFinite(v[2]);

/** Unit vector, or `null` for a non-finite or zero-length input (never NaN out). */
function unit(v: Vec3): Vec3 | null {
  if (!finite3(v)) return null;
  const l = Math.hypot(v[0], v[1], v[2]);
  if (!(l > 0)) return null;
  return [v[0] / l, v[1] / l, v[2] / l];
}

/**
 * `M · [x, y, z, w]`, returning the components the divide needs AND the sum of
 * each row's term magnitudes — the operand scale every error bound below is
 * relative to.
 */
function transform(
  m: Mat4,
  x: number,
  y: number,
  z: number,
  w: number,
): { x: number; y: number; w: number; xMag: number; yMag: number; wMag: number } {
  return {
    x: m[0] * x + m[4] * y + m[8] * z + m[12] * w,
    y: m[1] * x + m[5] * y + m[9] * z + m[13] * w,
    w: m[3] * x + m[7] * y + m[11] * z + m[15] * w,
    xMag: Math.abs(m[0] * x) + Math.abs(m[4] * y) + Math.abs(m[8] * z) + Math.abs(m[12] * w),
    yMag: Math.abs(m[1] * x) + Math.abs(m[5] * y) + Math.abs(m[9] * z) + Math.abs(m[13] * w),
    wMag: Math.abs(m[3] * x) + Math.abs(m[7] * y) + Math.abs(m[11] * z) + Math.abs(m[15] * w),
  };
}

/** The projected motion of one UN-normalized direction, per unit of its length. */
interface DirectionProjection {
  readonly derivative: Vec2;
  /** `|derivative|` — CSS px per unit of the direction as supplied. */
  readonly pxPerUnit: number;
  readonly direction: Vec2;
  /**
   * `dv.w / c0.w` — how fast the perspective divide changes along the direction,
   * in reciprocal units of the direction as supplied. Exactly 0 for an
   * orthographic camera, which is what makes a constant gain right there.
   */
  readonly kPerUnit: number;
}

/**
 * Exact CSS-pixel derivative of `dir` at `anchor`, WITHOUT normalizing `dir` —
 * so a path whose derivative carries a semantic factor (Diameter's `r̂/2`) keeps
 * it, and the result is "per unit of the value", not "per world unit".
 *
 * Refuses when the matrix or inputs are non-finite, the direction is degenerate,
 * the anchor's clip `w` cannot be CERTIFIED positive, or the projected motion is
 * not certifiably larger than its own rounding error. Every one of those bounds
 * is relative to its operands, so the same view behaves the same at any scale.
 */
function projectDirection(
  viewProj: Mat4,
  anchor: Vec3,
  dir: Vec3,
  viewportWidth: number,
  viewportHeight: number,
): DirectionProjection | null {
  if (viewProj.length < 16) return null;
  for (let i = 0; i < 16; i++) if (!Number.isFinite(viewProj[i])) return null;
  if (!finite3(anchor) || !finite3(dir)) return null;
  if (!(viewportWidth > 0) || !(viewportHeight > 0)) return null;
  if (!(Math.hypot(dir[0], dir[1], dir[2]) > 0)) return null;

  const c = transform(viewProj, anchor[0], anchor[1], anchor[2], 1);
  // A direction is a vector, not a point: w = 0, so the translation column is
  // deliberately not applied.
  const d = transform(viewProj, dir[0], dir[1], dir[2], 0);
  if (!Number.isFinite(c.w) || !Number.isFinite(d.x) || !Number.isFinite(d.y) || !Number.isFinite(d.w)) {
    return null;
  }

  // On or behind the camera plane, or too close to it for its own sign to be
  // certified. RELATIVE to the terms that produced it, not to a fixed 1e-6.
  if (!(c.w > GAMMA_DOT4 * c.wMag)) return null;

  const invW2 = 1 / (c.w * c.w);
  const ndcDx = (d.x * c.w - c.x * d.w) * invW2;
  const ndcDy = (d.y * c.w - c.y * d.w) * invW2;

  // NDC spans [-1, 1] across the viewport, hence the HALF extents; CSS +Y runs
  // down while NDC +Y runs up, hence the negation on Y alone.
  const halfW = viewportWidth / 2;
  const halfH = viewportHeight / 2;
  const dx = halfW * ndcDx;
  const dy = -halfH * ndcDy;
  if (!Number.isFinite(dx) || !Number.isFinite(dy)) return null;

  // Error bound on the same two numerators, propagated through the products and
  // the cancelling subtraction. `|Δ(a·b)| ≤ γ_product · aMag · bMag` because each
  // factor is itself a 4-term dot product, and the subtraction contributes `u` of
  // whatever survives it. `relW` carries the divide's own relative error into the
  // bound, so a barely-certified `c.w` widens the refusal instead of hiding in it.
  const relW = (GAMMA_DOT4 * c.wMag) / c.w;
  const numErr = (aMag: number, bMag: number, cMag: number, dwMag: number, value: number): number =>
    GAMMA_PRODUCT * (aMag * bMag + cMag * dwMag) + (UNIT_ROUNDOFF + 2 * relW) * Math.abs(value);
  const errX = halfW * numErr(d.xMag, c.wMag, c.xMag, d.wMag, d.x * c.w - c.x * d.w) * invW2;
  const errY = halfH * numErr(d.yMag, c.wMag, c.yMag, d.wMag, d.y * c.w - c.y * d.w) * invW2;

  const pxPerUnit = Math.hypot(dx, dy);
  // Exactly end-on projects to (0, 0) with a zero bound and is refused here; a
  // direction whose projection is smaller than its own rounding error carries no
  // certified screen direction either.
  if (!(pxPerUnit > Math.hypot(errX, errY))) return null;

  const kPerUnit = d.w / c.w;
  if (!Number.isFinite(kPerUnit)) return null;

  return {
    derivative: [dx, dy],
    pxPerUnit,
    direction: [dx / pxPerUnit, dy / pxPerUnit],
    kPerUnit,
  };
}

/**
 * The exact CSS-pixel derivative of `dir` at `anchor`, per WORLD unit.
 *
 * `dir` need not arrive normalized — it is normalized here, so the result is
 * always "per world unit" regardless of what the caller passed. A path that
 * carries a semantic factor must go through {@link classifyMapping} instead,
 * which keeps it.
 */
export function projectAxis(
  viewProj: Mat4,
  anchor: Vec3,
  dir: Vec3,
  viewportWidth: number,
  viewportHeight: number,
): AxisProjection | null {
  const a = unit(dir);
  if (!a) return null;
  const p = projectDirection(viewProj, anchor, a, viewportWidth, viewportHeight);
  if (!p) return null;
  return { derivative: p.derivative, pxPerWorld: p.pxPerUnit, direction: p.direction };
}

/**
 * The dimensionless scalar sensitivity `Cq = g · s`: CSS px the handle travels
 * per CSS px of pointer travel, for one unit of the bound value.
 *
 * `worldPerPx` is the caller's own scale at the SAME anchor (the engine's
 * `screenScale.worldPerPixel`), so the product cancels units and leaves the
 * geometric term. Feeding a scale measured at a DIFFERENT point — the orbit
 * target, say — silently biases the answer, which is the same trap
 * `ViewportEngine.planePixelWorld` sets for anything off-pivot.
 *
 * UNCLAMPED: off the optical axis this legitimately exceeds 1 (see the module
 * doc). 0 means "no usable scale", which is not a measurement.
 */
export function scalarSensitivity(pxPerValue: number, worldPerPx: number): number {
  if (!Number.isFinite(worldPerPx) || worldPerPx <= 0) return 0;
  if (!Number.isFinite(pxPerValue) || pxPerValue <= 0) return 0;
  const c = pxPerValue * worldPerPx;
  return Number.isFinite(c) ? c : 0;
}

/** The camera side of a classification — all of it, frozen together at grab. */
export interface ProjectionContext {
  readonly viewProj: Mat4;
  readonly viewportWidth: number;
  readonly viewportHeight: number;
}

/**
 * World units per CSS px AT THE PATH'S OWN ANCHOR. `null` (and any non-positive
 * or non-finite value) means the caller could not establish one — deliberately
 * not a numeric sentinel, because "no depth" and "a very small depth" must not
 * share a representation.
 */
export interface AnchorScale {
  readonly worldPerPx: number;
}

/**
 * The world path a value handle's attachment point travels along:
 * `H(q) = point0Mm + dPointDValue · (q − q0Mm)`.
 *
 * `dPointDValue` is NOT normalized — its length is the semantic gain (Diameter's
 * `r̂/2` moves the wall half a millimetre per millimetre of Ø), and dropping it
 * would double that drag.
 */
export interface LinearHandlePath {
  readonly q0Mm: number;
  readonly point0Mm: Vec3;
  readonly dPointDValue: Vec3;
}

/**
 * Why a mapping is not a world mapping. Mirrors the derivation's §8 diagnostic
 * ids (`poor-screen-sensitivity`, `numeric-uncertain`, `missing-reference`) in
 * repo casing.
 */
export type MappingRefusal =
  | "poorScreenSensitivity"
  | "noProjection"
  | "noScale";

/**
 * The ONE description a value handle's glyph, pick and gesture share, taken at
 * the grab and not recomputed until the gesture ends.
 *
 * Freezing the whole object — coefficients, direction, scale and valid interval,
 * not merely a strategy name — is what makes the gesture track exactly: a
 * re-derived projection changes gain under the user's hand mid-drag.
 */
export type FrozenMapping =
  | {
      readonly kind: "world";
      /** The value the handle sat at when this was frozen. */
      readonly q0Mm: number;
      /** Unit CSS-px direction (+Y down) the handle travels for increasing q. */
      readonly direction: Vec2;
      /** `g` — CSS px of handle travel per unit of value, at `q0Mm`. */
      readonly g0PxPerMm: number;
      /** `k = dv.w / c0.w` — the perspective foreshortening rate. 0 = orthographic. */
      readonly kPerMm: number;
      /**
       * The connected interval of `Δq` around 0 on which this mapping stays
       * valid: the perspective divide stays positive AND the frozen sensitivity
       * envelope `Cq(q) = Cq₀ / (1 + kΔq)` stays at or above τ. Exhausting it is
       * a view limit that HOLDS, never a switch to another strategy.
       */
      readonly validDeltaMm: readonly [number, number];
      /** `Cq₀`, unclamped — the number to log, above 1 included. */
      readonly conditioning: number;
    }
  | {
      readonly kind: "proxy";
      readonly q0Mm: number;
      /** Up the screen increases the value. */
      readonly direction: Vec2;
      readonly mmPerPx: number;
      readonly reason: MappingRefusal;
    }
  | {
      readonly kind: "disabled";
      readonly q0Mm: number;
      readonly reason: MappingRefusal;
    };

/** What one pointer displacement means under a frozen mapping. */
export interface DragSample {
  readonly valueMm: number;
  /** Set when the frozen valid interval clipped the sample (derivation §2.9). */
  readonly saturated?: "viewLimit";
}

/** Up the screen, CSS px (+Y down): the proxy's increasing direction. */
const PROXY_DIRECTION: Vec2 = [0, -1];

const proxyOrDisabled = (q0Mm: number, scale: AnchorScale | null, reason: MappingRefusal): FrozenMapping => {
  const usable = scale !== null && Number.isFinite(scale.worldPerPx) && scale.worldPerPx > 0;
  if (!usable) return { kind: "disabled", q0Mm, reason: "noScale" };
  return { kind: "proxy", q0Mm, direction: PROXY_DIRECTION, mmPerPx: scale.worldPerPx, reason };
};

/**
 * The connected `Δq` interval containing 0 on which the frozen mapping holds.
 *
 * Two constraints, both from the derivation §3: the perspective divide must stay
 * positive (`1 + kΔq > 0`) and the sensitivity envelope must stay above τ
 * (`1 + kΔq ≤ Cq₀/τ`). The pole is the open end; the sensitivity bound is always
 * reached first from inside, because `Cq₀/τ ≥ 1`.
 */
function validDeltaInterval(kPerMm: number, conditioning: number, floor: number): readonly [number, number] {
  if (kPerMm === 0) return [-Infinity, Infinity];
  const envelope = (conditioning / floor - 1) / kPerMm;
  const pole = -1 / kPerMm;
  return kPerMm > 0 ? [pole, envelope] : [envelope, pole];
}

/**
 * Classify — once, at the grab — how this path's handle maps to the pointer.
 *
 * `world` when the certified scalar sensitivity clears τ; `proxy` (a labelled
 * vertical screen drag at the anchor's own scale) when the projection refuses or
 * the sensitivity does not; `disabled` when no positive scale could be
 * established at all, where the honest answer is a numeric control and no drag.
 */
export function classifyMapping(
  path: LinearHandlePath,
  camera: ProjectionContext,
  scale: AnchorScale | null,
  floor: number = MIN_AXIS_CONDITIONING,
): FrozenMapping {
  const q0Mm = Number.isFinite(path.q0Mm) ? path.q0Mm : 0;
  const usable = scale !== null && Number.isFinite(scale.worldPerPx) && scale.worldPerPx > 0;
  if (!usable) return { kind: "disabled", q0Mm, reason: "noScale" };

  const projected = projectDirection(
    camera.viewProj,
    path.point0Mm,
    path.dPointDValue,
    camera.viewportWidth,
    camera.viewportHeight,
  );
  if (!projected) return proxyOrDisabled(q0Mm, scale, "noProjection");

  const conditioning = scalarSensitivity(projected.pxPerUnit, scale.worldPerPx);
  if (!(conditioning >= floor)) return proxyOrDisabled(q0Mm, scale, "poorScreenSensitivity");

  return {
    kind: "world",
    q0Mm,
    direction: projected.direction,
    g0PxPerMm: projected.pxPerUnit,
    kPerMm: projected.kPerUnit,
    validDeltaMm: validDeltaInterval(projected.kPerUnit, conditioning, floor),
    conditioning,
  };
}

/** Pointer displacement in px along a world mapping's own direction, `Δq → p`. */
function forwardPx(mapping: Extract<FrozenMapping, { kind: "world" }>, deltaMm: number): number {
  if (!Number.isFinite(deltaMm)) return deltaMm;
  const divide = 1 + mapping.kPerMm * deltaMm;
  // At the pole the required pointer travel diverges, so that end is unreachable
  // from any finite pointer position.
  if (!(divide > 0)) return deltaMm > 0 ? Infinity : -Infinity;
  return (mapping.g0PxPerMm * deltaMm) / divide;
}

/**
 * Invert a frozen mapping: what value does this pointer displacement mean?
 *
 * The world branch is the EXACT perspective inverse `Δq = p / (g − k·p)`, whose
 * derivative `g / (g − k·p)²` is strictly positive — pointer and handle never
 * disagree about direction, and the naive `p / g` is only its tangent at p = 0
 * (50 px on the derivation's P60 case is 8.934710 mm, not 9.622504 mm).
 *
 * Outside the frozen valid interval the value HOLDS at the limit and reports
 * `saturated`, so the caller can say "View limits this drag" and rebase rather
 * than silently swapping strategy mid-gesture.
 */
export function sampleMapping(mapping: FrozenMapping, displacementPx: Vec2): DragSample {
  if (mapping.kind === "disabled") return { valueMm: mapping.q0Mm };
  const [dx, dy] = displacementPx;
  if (!Number.isFinite(dx) || !Number.isFinite(dy)) return { valueMm: mapping.q0Mm };
  const p = dx * mapping.direction[0] + dy * mapping.direction[1];

  if (mapping.kind === "proxy") return { valueMm: mapping.q0Mm + p * mapping.mmPerPx };

  const [lo, hi] = mapping.validDeltaMm;
  const pLo = forwardPx(mapping, lo);
  const pHi = forwardPx(mapping, hi);
  const clamped = Math.min(Math.max(p, pLo), pHi);
  const divide = mapping.g0PxPerMm - mapping.kPerMm * clamped;
  // `clamped` is inside the interval, so the divide is positive there; the
  // fallback only catches a pathological frozen coefficient.
  const deltaMm = divide > 0 ? clamped / divide : clamped >= 0 ? hi : lo;
  const valueMm = mapping.q0Mm + deltaMm;
  return clamped === p ? { valueMm } : { valueMm, saturated: "viewLimit" };
}

/**
 * What a drawn witness segment CLAIMS (derivation §2.4). Every witness declares
 * one of these, because a successful prepare does not establish that a value the
 * user has merely asked for has already been built.
 *
 *  - `measuredReference` — a quantity the kernel measured on this snapshot.
 *  - `targetConstruction` — a target against a frozen reference surface, until
 *    the kernel validates it.
 *  - `parameterConstruction` — the length of a construction the parameter
 *    drives. It measures the construction, NOT the geometry the op will build.
 */
export type WitnessMeaning = "measuredReference" | "targetConstruction" | "parameterConstruction";

/** One drawn witness segment and the claim its label is allowed to make. */
export interface ValueWitness {
  readonly meaning: WitnessMeaning;
  readonly label: string;
  readonly fromMm: Vec3;
  readonly toMm: Vec3;
}

/** Where the handle sits for `valueMm`: `H(q) = point0 + dPointDValue·(q − q0)`. */
export function handlePointAt(path: LinearHandlePath, valueMm: number): Vec3 {
  const t = valueMm - path.q0Mm;
  return [
    path.point0Mm[0] + path.dPointDValue[0] * t,
    path.point0Mm[1] + path.dPointDValue[1] * t,
    path.point0Mm[2] + path.dPointDValue[2] * t,
  ];
}
