/*
 * Measure V1a — the PURE state + math behind the read-only measure tool
 * (W2-B). No stores, no engine, no ipc client: `ModelToolController` owns the
 * imperative wiring, this owns the rules.
 *
 * It answers from evidence the kernel already computes (`ElementInfo.magnitude`,
 * `.center`, `.normal`, `.surfaceType`) — no verb of its own:
 *   1. how long is this edge       (arc length, mm)
 *   2. how big is this face        (area, mm²)
 *   3. how far apart are these two (centre-to-centre distance + ΔX/ΔY/ΔZ, mm)
 *   4. what is the ANGLE between these two planar faces  (WP-C1)
 *   5. how far apart are these two PARALLEL planes       (WP-C1)
 *
 * (4) and (5) are derived HERE from two `QueryElement` normals rather than by a
 * new worker verb. Two unit normals determine the dihedral angle exactly, so a
 * round-trip would add latency and a second source of truth for the same number.
 * A body's volume/area/centroid is the opposite case — it is not recoverable from
 * element descriptors at all, and goes through `QueryMassProperties` (SCHEMA §7.5).
 *
 * CENTRE, NOT CENTROID — the load-bearing caveat. `ElementInfo.center` is the
 * kernel's BOUNDING-BOX centre (`Bnd_Box` in `ElementMap::computeDescriptor`),
 * not a centre of mass. For a symmetric face the two coincide; for an L-shaped
 * or arc-bounded one they do not, sometimes by a lot. The distance below is
 * therefore honestly labelled "center ↔ center" everywhere it surfaces, and
 * must never be presented as a centroid distance or as a minimum separation
 * (the closest points of two faces are a different, harder question).
 */
import { radToDeg } from "@/ipc/angleUnits";
import type { ElementInfo } from "@/ipc/types";

/** How many picks the tool retains. A third pick replaces the OLDEST. */
export const MAX_PICKS = 2;

/** One measured element — the subset of `ElementInfo` the tool actually uses. */
export interface MeasurePick {
  bodyId: string;
  elementId: string;
  /** `face` | `edge` | `vertex` | `body`. */
  kind: string;
  /** Area (mm²) for a face, arc length (mm) for an edge. */
  magnitude: number;
  /** Bounding-box centre — see the module header. */
  center: [number, number, number];
  /** OCCT `GeomAbs_CurveType` ordinal for an edge; `-1` when absent. */
  curveType: number;
  /**
   * OCCT `GeomAbs_SurfaceType` ordinal for a face; **0 == plane**, `-1` absent.
   * The angle reading below is gated on this — see {@link planeOf}.
   */
  surfaceType: number;
  /** The descriptor's normal. Only a PLANE normal when `surfaceType === 0`. */
  normal: [number, number, number];
  /** Whether the descriptor actually carried a normal. */
  hasNormal: boolean;
}

/** Ordered picks, oldest first. Empty is the armed-but-untouched state. */
export interface MeasureState {
  picks: MeasurePick[];
}

/**
 * The angle between two picked PLANES, in the two forms CAD tools show it.
 *
 * A pair of planes subtends two supplementary dihedral angles, and which one a
 * user means depends on which side of each plane the material is on — evidence a
 * bounding-box descriptor does not carry. Shapr3D and Fusion both resolve this by
 * showing the pair, so the reading is `"60° / 120°"` and the user reads off the
 * one that matches what they see. Picking ONE silently would be wrong half the
 * time, with nothing on screen to reveal it.
 *
 * `acuteDeg` = `acos(|n1·n2|)` ∈ [0°, 90°]; `obtuseDeg` = `180 − acuteDeg`.
 * `isRight` marks the 90° case, where the two forms coincide and showing both
 * ("90° / 90°") would look like a formatting bug.
 */
export interface MeasureAngle {
  acuteDeg: number;
  obtuseDeg: number;
  isRight: boolean;
}

/** The derived two-pick relationship, or null with fewer than two picks. */
export interface MeasureSummary {
  /** Straight-line distance between the two bounding-box centres (mm). */
  distance: number;
  /** Per-axis separation `b − a` (mm), in the same order as the picks. */
  delta: [number, number, number];
  /**
   * The dihedral angle, when BOTH picks are planar faces that are not parallel.
   * Null otherwise — an angle between an edge and a face, or between two curved
   * faces, is a different measurement this tool does not compute.
   */
  angle: MeasureAngle | null;
  /**
   * Perpendicular separation of two PARALLEL planes (mm), when both picks are
   * planar faces whose normals are (anti)parallel within {@link PARALLEL_EPS}.
   *
   * Parallel planes have no meaningful angle (0° / 180° says nothing), but they
   * DO have an exact, basis-independent distance — `|(c₂ − c₁) · n|` — and it is
   * the number the user was almost certainly after. Unlike the centre-to-centre
   * distance beside it, this one is a true minimum separation: the projection
   * onto the shared normal is unaffected by where in each plane the two
   * bounding-box centres happen to sit.
   */
  planeOffset: number | null;
}

/**
 * How close to (anti)parallel two unit normals must be before the pair is
 * treated as parallel: `|n1·n2| ≥ 1 − 1e-6`, i.e. about 0.081°.
 *
 * Tight on purpose. The alternative failure is worse than a missed reading: a
 * loose threshold would report a plane "offset" for faces that are not parallel,
 * where `|(c₂−c₁)·n|` is not a separation between anything.
 */
export const PARALLEL_EPS = 1e-6;

export function measureInit(): MeasureState {
  return { picks: [] };
}

/** A `MeasurePick` from one backend `ElementInfo` read. */
export function pickFromElementInfo(bodyId: string, info: ElementInfo): MeasurePick {
  return {
    bodyId,
    elementId: info.elementId,
    kind: info.kind,
    magnitude: info.magnitude,
    center: [info.center[0], info.center[1], info.center[2]],
    curveType: info.curveType,
    surfaceType: info.surfaceType,
    normal: [info.normal[0], info.normal[1], info.normal[2]],
    hasNormal: info.hasNormal,
  };
}

/**
 * Add a pick, keeping the LATEST `MAX_PICKS`.
 *
 * A third pick drops the OLDEST, so the pair on screen is always "the last thing
 * I clicked, measured against the thing before it" — click A, B, C and you get
 * B↔C. Dropping the NEWEST instead (or refusing the click) would leave the user
 * clicking with no visible effect, which reads as a broken tool.
 *
 * Re-picking the SAME element is idempotent: it refreshes that pick in place
 * rather than pairing an element with itself, which would report distance 0 and
 * look like a measurement rather than a mis-click.
 */
export function measureAdd(state: MeasureState, pick: MeasurePick): MeasureState {
  const key = (p: MeasurePick) => `${p.bodyId}#${p.elementId}`;
  const existing = state.picks.findIndex((p) => key(p) === key(pick));
  if (existing >= 0) {
    const picks = [...state.picks];
    picks[existing] = pick;
    return { picks };
  }
  return { picks: [...state.picks, pick].slice(-MAX_PICKS) };
}

/** Drop every pick (Esc / disarm / tool switch). */
export function measureClear(): MeasureState {
  return measureInit();
}

/**
 * The UNIT normal of a pick, but only when the pick really is a plane.
 *
 * Three gates, all load-bearing: the pick must be a face, its `surfaceType` must
 * be `0` (`GeomAbs_Plane` — a cylinder's descriptor also carries a `normal`, and
 * it is not a plane normal), and the descriptor must actually have supplied one
 * (`hasNormal`; the fallback `(0,0,1)` would otherwise read as "faces +Z").
 * A zero or non-finite vector returns null rather than NaN.
 */
export function planeOf(pick: MeasurePick): [number, number, number] | null {
  if (pick.kind !== "face" || pick.surfaceType !== 0 || !pick.hasNormal) return null;
  const [x, y, z] = pick.normal;
  const len = Math.hypot(x, y, z);
  if (!Number.isFinite(len) || len < 1e-12) return null;
  return [x / len, y / len, z / len];
}

/**
 * Centre-to-centre distance + per-axis deltas, plus the plane relationship
 * (angle, or offset when parallel). Null below two picks.
 *
 * `delta` is `second − first` in pick order, so its sign is meaningful.
 */
export function measureSummary(state: MeasureState): MeasureSummary | null {
  if (state.picks.length < MAX_PICKS) return null;
  const [a, b] = state.picks;
  const delta: [number, number, number] = [
    b.center[0] - a.center[0],
    b.center[1] - a.center[1],
    b.center[2] - a.center[2],
  ];

  let angle: MeasureAngle | null = null;
  let planeOffset: number | null = null;
  const na = planeOf(a);
  const nb = planeOf(b);
  if (na && nb) {
    // Clamped because float error can push a dot product a hair past ±1, and
    // `Math.acos(1.0000000000000002)` is NaN — which would render as "NaN°".
    const dotAbs = Math.min(1, Math.abs(na[0] * nb[0] + na[1] * nb[1] + na[2] * nb[2]));
    if (dotAbs >= 1 - PARALLEL_EPS) {
      // Parallel: project the centre separation onto the shared normal. Signless —
      // "which side" is an orientation question the bbox centres cannot answer.
      planeOffset = Math.abs(delta[0] * na[0] + delta[1] * na[1] + delta[2] * na[2]);
    } else {
      const acuteDeg = radToDeg(Math.acos(dotAbs));
      angle = { acuteDeg, obtuseDeg: 180 - acuteDeg, isRight: Math.abs(acuteDeg - 90) < 1e-9 };
    }
  }

  return {
    distance: Math.hypot(delta[0], delta[1], delta[2]),
    delta,
    angle,
    planeOffset,
  };
}
