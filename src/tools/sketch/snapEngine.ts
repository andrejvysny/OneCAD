/*
 * snapEngine — the FACADE over the sketch snap decision engine.
 *
 * Generation lives in `snapCandidates.ts`, resolution in `snapArbitration.ts`,
 * the vocabulary in `snapTypes.ts`. This module wires the three together and
 * re-exports the geometry primitives the trim / marquee / hit-test lanes share.
 *
 * ── The model, in one paragraph ──────────────────────────────────────────────
 * A pointer sample produces a set of CANDIDATES, each declaring what it claims
 * (a whole point, an x, an angle, a length…), how far off it is in SCREEN
 * pixels through the plane→screen metric, and the document points that produced
 * it. The resolver enumerates the compatible combinations, scores each on
 * distance plus a bounded semantic preference, applies acquire/release
 * hysteresis, and returns ONE `SnapDecision`: the accepted set, the point they
 * rebuild to, and every rejected candidate with its reason.
 *
 * There is no priority LADDER any more, and there is no "grid gets first
 * refusal" rule. Grid is an ordinary candidate with its own cell-relative
 * reach, and cursor numeric rounding is an ordinary candidate too — which is
 * what lets a polar direction and a rounded length be accepted TOGETHER instead
 * of one silently suppressing the other.
 */
import type { SketchEntity } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import type { DimFrame } from "./liveDimFrames";
import type { DimLocks, DimQuantum } from "./liveDimension";
import {
  generateCandidates,
  type CandidateContext,
  type GestureAnchor,
  type SnapCandidateCache,
  type SnapSourceToggles,
} from "./snapCandidates";
import {
  numericCandidates,
  radialComposition,
  resolveSnap,
  type ArbitrationOutput,
} from "./snapArbitration";
import {
  emptyLatch,
  gridReachPx,
  isotropicMetric,
  releaseRadiusPx,
  type GuideLine,
  type PlaneScreenMetric,
  type SnapDecision,
  type SnapKind,
  type SnapLatch,
} from "./snapTypes";

/*
 * `SnapKind` and `GuideLine` live in `snapTypes.ts` — the composable model needs
 * them and must not depend on this module. Re-exported so existing importers
 * are untouched.
 */
export type { GuideLine, SnapKind } from "./snapTypes";

/* Geometry primitives, re-exported: `trimMath`, `marqueeSelect`, `extendMath`
 * and `sketchHitTest` consume them and have nothing to do with snapping. */
export {
  arcContainsAngle,
  arcQuadrantPoints,
  buildSnapCache,
  circleCircleIntersections,
  circleQuadrantPoints,
  entityIntersections,
  entitySnapPoints,
  nearestOnCircle,
  nearestOnCurve,
  nearestOnSegment,
  segCircleIntersections,
  segEllipseIntersections,
  segSegIntersection,
  type SnapCandidateCache,
} from "./snapCandidates";

/**
 * DEFAULT point-snap reach in screen pixels — the pixel-space analogue of the
 * C++ 2mm sketch-coord radius. The live value is the user's `snapRadius`
 * preference (`snapRadius.ts`, whose "m" IS this number).
 */
export const SNAP_PX = 8;

/**
 * The legacy single-winner view of a decision.
 *
 * DEPRECATED, retained because ~20 call sites and the oracle-pinned
 * `snapEngine.test.ts` read it, and because it is genuinely the right shape for
 * the callers that only want "where does the cursor go, and what is it called".
 * Anything that needs the accepted SET, the provenance or the rejections must
 * use `SnapDecision`. Deletion is tracked in TODO.md.
 */
export interface SnapResult {
  point: Point2;
  kind: SnapKind;
  /** Hint chip text (null when nothing to hint). */
  label: string | null;
  guides: GuideLine[];
  snapped: boolean;
}

export interface SnapOptions {
  /** World units between grid snap lines. */
  gridStep: number;
  /**
   * World units per screen pixel — the ISOTROPIC fallback when no camera metric
   * is available (unit tests, the headless mock lane). Ignored when `metric` is
   * supplied, which the live controller always does.
   */
  pixelWorld: number;
  /** The real plane→screen Jacobian. Anisotropic under an oblique camera, which
   *  is why it is not a scalar (see `snapTypes.PlaneScreenMetric`). */
  metric?: PlaneScreenMetric;
  /** Point-snap reach in SCREEN pixels — the user's `snapRadius` preference. */
  snapPx?: number;
  enableGrid: boolean;
  enableGuideLines: boolean;
  enableGuidePoints: boolean;
  /** Circle/arc/ellipse extremum snaps (default on). */
  enableQuadrant?: boolean;
  /** Entity-entity intersection snaps (default on). */
  enableIntersection?: boolean;
  /** Nearest-point-on-curve snaps (default on). */
  enableOnCurve?: boolean;
  /**
   * Polar tracking (default OFF here — the controller opts in from the pref).
   * INERT without an anchor: a fan centred on nothing would drag a free cursor
   * onto an arbitrary ray.
   */
  enablePolar?: boolean;
  /** Where the polar fan is centred — the gesture's LAST placed anchor. */
  polarAnchor?: Point2 | null;
  /** The direction the chain is travelling in, which adds the
   *  Parallel/Perpendicular pair to the fan. */
  polarRefDir?: Point2 | null;
  /** The entity those two rays are relative to, when known — what makes them
   *  persistable rather than placement-only. */
  polarRefLineId?: string | null;
  /** Alt held ⇒ raw point, no snap, no inference. */
  suppress: boolean;
  /** Extra reference points for H/V alignment (the current chain anchors). */
  recentPoints?: Point2[];
  /** Gesture anchors WITH stable keys — preferred over `recentPoints`, which
   *  cannot produce a stable candidate id. */
  anchors?: readonly GestureAnchor[];
  /** Precomputed entity-derived candidates from `buildSnapCache(entities)`. */
  cache?: SnapCandidateCache;

  // ── live-dimension (numeric) inputs ──
  /** The current phase's dimension frame; absent ⇒ no numeric candidates. */
  frame?: DimFrame | null;
  toolId?: string;
  /** Gesture anchors as the tool machine holds them (phase + radial centre). */
  toolAnchors?: readonly Point2[];
  arcMode?: boolean;
  locks?: DimLocks;
  /** Null/absent ⇒ cursor rounding off; typed locks still apply. */
  quantum?: DimQuantum | null;

  /** The retained target from the previous sample. Absent ⇒ no hysteresis. */
  latch?: SnapLatch;
  traceId?: number;
}

/**
 * Resolve one pointer sample into a full {@link SnapDecision} plus the latch to
 * carry into the next sample.
 *
 * Alt (`suppress`) short-circuits everything: no candidates, no rounding, the
 * raw point. That is the escape hatch the whole model depends on being total.
 */
export function computeSnapDecision(
  raw: Point2,
  entities: SketchEntity[],
  opts: SnapOptions,
): ArbitrationOutput {
  const traceId = opts.traceId ?? 0;
  if (opts.suppress) {
    return {
      decision: {
        raw,
        point: raw,
        accepted: [],
        rejected: [],
        primaryId: null,
        primaryKind: "none",
        label: null,
        guides: [],
        snapped: false,
        traceId,
      },
      latch: emptyLatch(),
    };
  }

  const metric = opts.metric ?? isotropicMetric(opts.pixelWorld);
  const acquirePx = opts.snapPx ?? SNAP_PX;
  const releasePx = releaseRadiusPx(acquirePx);
  const sources: SnapSourceToggles = {
    guidePoints: opts.enableGuidePoints,
    guideLines: opts.enableGuideLines,
    quadrant: opts.enableQuadrant ?? true,
    intersection: opts.enableIntersection ?? true,
    onCurve: opts.enableOnCurve ?? true,
    polar: opts.enablePolar ?? false,
    grid: opts.enableGrid,
  };
  const anchors: GestureAnchor[] =
    opts.anchors !== undefined
      ? [...opts.anchors]
      : (opts.recentPoints ?? []).map((point, i) => ({ point, key: `a${i}` }));

  const ctx: CandidateContext = {
    raw,
    metric,
    entities,
    cache: opts.cache,
    gridStep: opts.gridStep,
    acquirePx,
    reachPx: releasePx,
    gridReachPx: gridReachPx(metric, opts.gridStep, acquirePx),
    sources,
    polarAnchor: opts.polarAnchor ?? null,
    polarRefDir: opts.polarRefDir ?? null,
    polarRefLineId: opts.polarRefLineId ?? null,
    anchors,
  };
  const candidates = generateCandidates(ctx);

  const toolAnchors = opts.toolAnchors ?? anchors.map((a) => a.point);
  const arcMode = opts.arcMode ?? false;
  const numeric = numericCandidates(
    {
      frame: opts.frame ?? null,
      toolId: opts.toolId ?? "",
      anchors: toolAnchors,
      arcMode,
      locks: opts.locks ?? {},
      quantum: opts.quantum ?? null,
    },
    raw,
    metric,
  );

  return resolveSnap({
    raw,
    metric,
    candidates,
    numeric,
    acquirePx,
    releasePx,
    gridReachPx: ctx.gridReachPx,
    latch: opts.latch ?? emptyLatch(),
    frame: opts.frame ?? null,
    radial: opts.frame ? radialComposition(opts.toolId ?? "", toolAnchors, arcMode) : null,
    traceId,
  });
}

/** Narrow a decision to the legacy single-winner shape. */
export function snapResultOf(decision: SnapDecision): SnapResult {
  return {
    point: decision.point,
    kind: decision.primaryKind,
    label: decision.label,
    guides: [...decision.guides],
    snapped: decision.snapped,
  };
}

/** The legacy entry point — one decision, narrowed. See {@link SnapResult}. */
export function computeSnap(
  raw: Point2,
  entities: SketchEntity[],
  opts: SnapOptions,
): SnapResult {
  return snapResultOf(computeSnapDecision(raw, entities, opts).decision);
}
