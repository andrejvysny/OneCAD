/*
 * Measure V1a — the PURE state + math behind the read-only measure tool
 * (W2-B). No stores, no engine, no ipc client: `ModelToolController` owns the
 * imperative wiring, this owns the rules.
 *
 * V1a answers exactly three questions, and only from evidence the kernel already
 * computes (`ElementInfo.magnitude`, `ElementInfo.center`):
 *   1. how long is this edge      (arc length, mm)
 *   2. how big is this face       (area, mm²)
 *   3. how far apart are these two (centre-to-centre distance + ΔX/ΔY/ΔZ, mm)
 *
 * CENTRE, NOT CENTROID — the load-bearing caveat. `ElementInfo.center` is the
 * kernel's BOUNDING-BOX centre (`Bnd_Box` in `ElementMap::computeDescriptor`),
 * not a centre of mass. For a symmetric face the two coincide; for an L-shaped
 * or arc-bounded one they do not, sometimes by a lot. The distance below is
 * therefore honestly labelled "center ↔ center" everywhere it surfaces, and
 * must never be presented as a centroid distance or as a minimum separation
 * (the closest points of two faces are a different, harder question).
 */
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
}

/** Ordered picks, oldest first. Empty is the armed-but-untouched state. */
export interface MeasureState {
  picks: MeasurePick[];
}

/** The derived two-pick relationship, or null with fewer than two picks. */
export interface MeasureSummary {
  /** Straight-line distance between the two bounding-box centres (mm). */
  distance: number;
  /** Per-axis separation `b − a` (mm), in the same order as the picks. */
  delta: [number, number, number];
}

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
 * Centre-to-centre distance + per-axis deltas, or null below two picks.
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
  return { distance: Math.hypot(delta[0], delta[1], delta[2]), delta };
}
