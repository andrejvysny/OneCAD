/*
 * snapTypes — the PUBLIC vocabulary of the sketch snap decision engine.
 *
 * The legacy engine returned ONE winner off a rigid type ladder, which cannot
 * represent two facts about the same click: a polar direction AND a rounded
 * length, or an x guide AND a y guide. This module replaces that single result
 * with a COMPOSABLE model:
 *
 *   candidate  — one placement intent (a point, an axis coordinate, a ray, a
 *                numeric field value), carrying what it CLAIMS, how far off it
 *                is in SCREEN pixels, and the document refs that produced it.
 *   decision   — the compatible SET of candidates that won, the point they
 *                rebuild to, and every rejected candidate with its reason.
 *   latch      — the retained target, so tiny pointer motion cannot flicker
 *                between two equally-close candidates.
 *
 * Everything here is DATA. The generation half lives in `snapCandidates.ts`,
 * the scoring/compatibility half in `snapArbitration.ts`, and `snapEngine.ts`
 * is the facade both are reached through.
 *
 * PIXELS, NOT WORLD UNITS. Every distance in this module is CSS pixels measured
 * through `PlaneScreenMetric` — the local 2×2 plane→screen Jacobian. A single
 * `pixelWorld` scalar is wrong under an oblique camera, where one plane unit is
 * worth a different number of pixels along u than along v.
 */
import type { ConstraintPosition } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import type { DimFieldId } from "./liveDimension";

/** A point in CSS pixels, canvas-relative. */
export interface ScreenPoint {
  x: number;
  y: number;
}

/**
 * The local plane→screen Jacobian at a point, in CSS pixels per plane unit.
 *
 *   [ m00 m01 ]   maps a plane delta (du, dv) to a screen delta (dx, dy):
 *   [ m10 m11 ]   dx = m00·du + m01·dv,  dy = m10·du + m11·dv
 *
 * Columns are the projected unit-u and unit-v screen deltas. Under an
 * orthographic camera looking straight down the plane normal this is a scaled
 * rotation; under any oblique or perspective view it is a general 2×2 matrix,
 * which is exactly why one scalar cannot stand in for it.
 */
export interface PlaneScreenMetric {
  m00: number;
  m01: number;
  m10: number;
  m11: number;
}

/** Which GENERATOR produced a candidate — the axis every user toggle acts on. */
export type SnapSource =
  | "geometryPoint"
  | "curve"
  | "guide"
  | "polar"
  | "grid"
  | "numeric";

/**
 * What a candidate IS, for the marker glyph, the hint chip and persistence.
 *
 * Superset of the legacy `SnapResult["kind"]`: `numeric` is new (cursor value
 * rounding became an explicit candidate in P2 instead of a downstream rule that
 * fired off `kind === "none"`).
 */
export type SnapKind =
  | "none"
  | "endpoint"
  | "midpoint"
  | "center"
  | "origin"
  | "quadrant"
  | "intersection"
  | "onCurve"
  | "alignH"
  | "alignV"
  | "alignHV"
  | "polar"
  | "grid"
  | "numeric";

/**
 * The degrees of freedom a candidate CLAIMS. Two candidates compose only when
 * their claims are disjoint — `point` conflicts with everything, `x` and `y`
 * compose, `angle` composes with `length`.
 */
export type SnapClaim =
  | "x"
  | "y"
  | "point"
  | "angle"
  | "length"
  | "width"
  | "height"
  | "radius";

/**
 * A frontend point address — `(entityId, position)`, the pair
 * `sketchTopology` owns. Carried on a candidate so an accepted placement can
 * author a constraint against the point that actually produced it, instead of
 * being re-derived from the final coordinate later.
 */
export interface FrontendPointRef {
  entityId: string;
  position: ConstraintPosition;
}

/** The relation an accepted candidate WOULD persist, if the auto-constrain
 *  mode allows it. Placement-only candidates carry none. */
export interface SnapRelationIntent {
  kind:
    | "Coincident"
    | "Midpoint"
    | "FixedOrigin"
    | "OnCurve"
    | "HorizontalPoints"
    | "VerticalPoints"
    | "Parallel"
    | "Perpendicular"
    | "Tangent";
  refs: readonly FrontendPointRef[];
  curveIds?: readonly string[];
  lineIds?: readonly string[];
}

/**
 * How a candidate CONSTRAINS the final point. The resolver applies projections
 * in canonical claim order (point → x → y → ray → numeric), so a set of
 * compatible candidates rebuilds exactly one point.
 */
export type SnapProjection =
  | { kind: "point"; point: Point2 }
  | { kind: "axisX"; x: number }
  | { kind: "axisY"; y: number }
  | { kind: "ray"; origin: Point2; dir: Point2 }
  | {
      kind: "numericField";
      field: DimFieldId;
      rawValue: number;
      targetValue: number;
      quantum: number;
      /** `typed` is MANDATORY and never rejected; `cursor` is optional and
       *  loses to any higher-authority geometry claim. */
      authority: "typed" | "cursor";
    };

/**
 * A dashed guide drawn through the snapped point.
 *
 * H/V arms are stated as a CONSTANT COORDINATE (vertical ⇒ constant x) because
 * they are always axis-aligned; `ref` is the reference point that produced the
 * alignment, so the indicator draws only the segment between the two rather
 * than a full-plane cross. A polar guide is one ray of the fan centred on the
 * gesture anchor.
 */
export type GuideLine =
  | { orientation: "vertical"; value: number; ref: Point2 }
  | { orientation: "horizontal"; value: number; ref: Point2 }
  | { orientation: "polar"; origin: Point2; dir: Point2 };

/** One placement intent, fully scored. */
export interface SnapCandidate {
  /** Stable across entity order and cache rebuilds — see `snapCandidates.ts`. */
  id: string;
  source: SnapSource;
  kind: SnapKind;
  projection: SnapProjection;
  /** Where this candidate ALONE would put the point. The resolver MUST use
   *  `projection` for composition; this is preview/diagnostic only. */
  previewPoint: Point2;
  claims: readonly SnapClaim[];
  /** Screen distance from the raw cursor, through `PlaneScreenMetric`. */
  errorPx: number;
  /** Bounded semantic preference (≤2px magnitude). Negative = preferred. */
  semanticBiasPx: number;
  /** `errorPx + semanticBiasPx` — the only number candidates are ranked on. */
  scorePx: number;
  label: string;
  guides: readonly GuideLine[];
  refs: readonly FrontendPointRef[];
  relationIntents: readonly SnapRelationIntent[];
}

/** Why a generated candidate did not reach the accepted set. */
export interface RejectedSnapCandidate {
  candidateId: string;
  reason:
    | "outside-acquire-radius"
    | "outside-release-radius"
    | "claim-conflict"
    | "higher-authority"
    | "retained-target"
    | "numeric-incompatible"
    | "disabled-source"
    | "invalid-projection"
    /** P5: an extremum with no independent point address — placement only. */
    | "derived-point-not-persistable";
}

/** The resolved placement for one pointer sample. */
export interface SnapDecision {
  /** The unprojected cursor in plane coords. */
  raw: Point2;
  /** The point the tool machine and the commit both receive. */
  point: Point2;
  accepted: readonly SnapCandidate[];
  rejected: readonly RejectedSnapCandidate[];
  primaryId: string | null;
  primaryKind: SnapKind;
  label: string | null;
  guides: readonly GuideLine[];
  snapped: boolean;
  traceId: number;
}

/** Cross-frame retention state. Owned by the controller, transitioned purely. */
export interface SnapLatch {
  primaryId: string | null;
  acceptedIds: readonly string[];
  /** A candidate currently beating the retained one; must hold the advantage
   *  for {@link CHALLENGER_FRAMES} consecutive frames before it switches. */
  challengerId: string | null;
  challengerFrames: number;
}

/** A latch with nothing retained. */
export function emptyLatch(): SnapLatch {
  return { primaryId: null, acceptedIds: [], challengerId: null, challengerFrames: 0 };
}

// ── Trace ────────────────────────────────────────────────────────────────────

/** One candidate's row in a decision trace (a flat, log-safe projection). */
export interface SnapTraceCandidate {
  id: string;
  kind: SnapKind;
  source: SnapSource;
  claims: readonly SnapClaim[];
  errorPx: number;
  biasPx: number;
  scorePx: number;
  refs: readonly string[];
}

/**
 * Everything needed to explain ONE decision after the fact. Emitted on an
 * accepted-set CHANGE, a committed click, a latch reset, or a projection
 * failure — never per identical pointer frame (log.ts policy).
 */
export interface SnapDecisionTrace {
  traceId: number;
  interactionEpoch: number;
  sessionGeneration: number;
  /** `hover` or the click that committed. */
  origin: "hover" | "click";
  raw: Point2 | null;
  client: ScreenPoint | null;
  projection: "perspective" | "orthographic" | "none";
  metric: PlaneScreenMetric | null;
  gridStep: number;
  uCellPx: number;
  vCellPx: number;
  pointRadiusPx: number;
  gridRadiusPx: number;
  sources: Readonly<Record<string, boolean>>;
  candidates: readonly SnapTraceCandidate[];
  priorLatch: SnapLatch;
  acceptedIds: readonly string[];
  rejected: readonly RejectedSnapCandidate[];
  point: Point2 | null;
  altHeld: boolean;
}

// ── Normative constants ──────────────────────────────────────────────────────

/**
 * Release radius for a LATCHED target: wider than acquire, so a target already
 * held survives sub-pixel jitter at the acquire boundary.
 * S/M/L ⇒ 8 / 12 / 17 px.
 */
export function releaseRadiusPx(acquirePx: number): number {
  return Math.max(acquirePx + 3, Math.ceil(acquirePx * 1.4));
}

/**
 * Grid capture as a fraction of the SMALLER projected cell edge.
 *
 * Below the square cell's covering radius (√2/2 ≈ 0.707) on purpose: a grid
 * reach that covers the whole cell leaves no mid-cell region where cursor
 * numeric rounding can answer instead, which is what made grid and
 * dimension-rounding mutually exclusive in the legacy ladder.
 */
export const GRID_REACH_FACTOR = 0.35;

/**
 * Grid capture reach, in CSS pixels — CELL-RELATIVE, capped by the user's
 * point reach.
 *
 * Both halves matter. Without the cell term, a zoomed-out grid whose cell is
 * 3px across is captured everywhere by an 8px point radius, and cursor numeric
 * rounding can never answer. Without the cap, a zoomed-in grid whose cell is
 * 400px across would capture 140px away, far outside anything the user would
 * read as "near a grid line".
 *
 * Returns 0 (no grid capture) for a degenerate metric or a non-positive step.
 */
export function gridReachPx(
  metric: PlaneScreenMetric,
  gridStep: number,
  acquirePx: number,
): number {
  if (!(gridStep > 0)) return 0;
  const uCellPx = metricNorm(metric, gridStep, 0);
  const vCellPx = metricNorm(metric, 0, gridStep);
  const cell = Math.min(uCellPx, vCellPx);
  if (!Number.isFinite(cell) || cell < GRID_MIN_CELL_PX) return 0;
  return Math.min(acquirePx, GRID_REACH_FACTOR * cell);
}

/**
 * Below this projected cell size the grid is INERT.
 *
 * A sub-pixel grid is not a target the user can see or aim at, and capturing to
 * it is nearly free — so without a floor it would win every arbitration by cost
 * while contributing nothing, and (because grid claims the whole point) would
 * suppress the cursor numeric rounding that is the useful answer at that zoom.
 */
export const GRID_MIN_CELL_PX = 4;

/** Radius (px) inside which the origin gets its absolute-capture bias. */
export const ORIGIN_CORE_PX = 2;

/** Bounded semantic preferences, in pixels. Nothing here may exceed ±2. */
export const SEMANTIC_BIAS_PX: Readonly<Record<string, number>> = {
  originCore: -2.0,
  origin: -0.5,
  endpoint: -1.0,
  midpoint: -0.75,
  center: -0.75,
  intersection: -0.5,
  quadrant: -0.25,
  guide: 0,
  polar: 0,
  grid: 0,
  onCurve: 0.75,
  numeric: 0,
};

/** A challenger must beat the retained target by this much to switch. */
export const HYSTERESIS_ADVANTAGE_PX = 2;

/** …and must hold that advantage for this many consecutive pointer frames. */
export const CHALLENGER_FRAMES = 2;

/** Cost added per OPTIONAL candidate after the first, so a weak extra relation
 *  cannot join the set without paying for the complexity it adds. */
export const SET_COMPLEXITY_PENALTY_PX = 0.25;

/** Slack on the "the rebuilt point did not run away" guard, in pixels. */
export const SET_DISPLACEMENT_SLACK_PX = 0.25;

/** Below this |det| the plane→screen metric is degenerate (edge-on view). */
export const METRIC_DET_EPS = 1e-9;

/** Plane-unit dedupe tolerance for exact primitives. */
export const CANDIDATE_DEDUPE_TOL = 1e-9;

// ── Metric helpers (pure, allocation-free) ───────────────────────────────────

/** Screen delta of a plane delta, in CSS pixels. */
export function metricApply(m: PlaneScreenMetric, du: number, dv: number): ScreenPoint {
  return { x: m.m00 * du + m.m01 * dv, y: m.m10 * du + m.m11 * dv };
}

/** Screen LENGTH of a plane delta, in CSS pixels. */
export function metricNorm(m: PlaneScreenMetric, du: number, dv: number): number {
  return Math.hypot(m.m00 * du + m.m01 * dv, m.m10 * du + m.m11 * dv);
}

/** Screen distance between two plane points. */
export function metricDistance(m: PlaneScreenMetric, a: Point2, b: Point2): number {
  return metricNorm(m, a.x - b.x, a.y - b.y);
}

/**
 * `A = Mᵀ M`, the metric tensor every nearest-point projection minimizes
 * against. Returned as its three distinct entries (`A` is symmetric).
 */
export interface MetricTensor {
  a11: number;
  a12: number;
  a22: number;
}

export function metricTensor(m: PlaneScreenMetric): MetricTensor {
  return {
    a11: m.m00 * m.m00 + m.m10 * m.m10,
    a12: m.m00 * m.m01 + m.m10 * m.m11,
    a22: m.m01 * m.m01 + m.m11 * m.m11,
  };
}

/** `uᵀ A v` for two plane vectors. */
export function tensorDot(
  A: MetricTensor,
  ux: number,
  uy: number,
  vx: number,
  vy: number,
): number {
  return A.a11 * ux * vx + A.a12 * (ux * vy + uy * vx) + A.a22 * uy * vy;
}

/** Determinant of the metric — near zero means an edge-on (degenerate) view. */
export function metricDet(m: PlaneScreenMetric): number {
  return m.m00 * m.m11 - m.m01 * m.m10;
}

/** True when every entry is finite and the metric is invertible. */
export function metricUsable(m: PlaneScreenMetric | null): m is PlaneScreenMetric {
  if (!m) return false;
  if (![m.m00, m.m01, m.m10, m.m11].every(Number.isFinite)) return false;
  return Math.abs(metricDet(m)) >= METRIC_DET_EPS;
}

/**
 * An ISOTROPIC metric from a single world-units-per-pixel scalar — the
 * fallback for callers that have no camera (unit tests, the headless mock
 * lane). Equivalent to the legacy `pixelWorld` threshold model.
 */
export function isotropicMetric(pixelWorld: number): PlaneScreenMetric {
  const s = pixelWorld > 0 ? 1 / pixelWorld : 1;
  return { m00: s, m01: 0, m10: 0, m11: s };
}
