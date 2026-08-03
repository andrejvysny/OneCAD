import { describe, it, expect } from "vitest";
import type { ElementInfo } from "@/ipc/types";
import {
  MAX_PICKS,
  measureAdd,
  measureClear,
  measureInit,
  measureSummary,
  pickFromElementInfo,
  planeOf,
  type MeasurePick,
} from "./measureTool";

function pick(id: string, center: [number, number, number], over: Partial<MeasurePick> = {}): MeasurePick {
  return {
    bodyId: "body1",
    elementId: id,
    kind: "face",
    magnitude: 800,
    center,
    curveType: -1,
    // A PLANE by default (surfaceType 0 == GeomAbs_Plane) facing +Z, so a case
    // that wants the angle reading only has to override the normal.
    surfaceType: 0,
    normal: [0, 0, 1],
    hasNormal: true,
    ...over,
  };
}

describe("measureInit / measureClear", () => {
  it("starts with no picks", () => {
    expect(measureInit().picks).toEqual([]);
  });

  it("clear drops everything", () => {
    let s = measureAdd(measureInit(), pick("a", [0, 0, 0]));
    s = measureAdd(s, pick("b", [1, 0, 0]));
    expect(measureClear().picks).toEqual([]);
    expect(s.picks).toHaveLength(2); // clear() is pure — the old state is untouched
  });
});

describe("pickFromElementInfo", () => {
  it("carries the measurable fields through verbatim", () => {
    const info: ElementInfo = {
      elementId: "el_7",
      topoKey: "f:22",
      bodyId: "body_x",
      kind: "face",
      surfaceType: 0,
      curveType: -1,
      center: [-10, 20, 25],
      normal: [0, 0, 1],
      hasNormal: true,
      size: 44.72,
      magnitude: 800,
    };
    expect(pickFromElementInfo("body1", info)).toEqual({
      bodyId: "body1",
      elementId: "el_7",
      kind: "face",
      magnitude: 800,
      center: [-10, 20, 25],
      curveType: -1,
      // WP-C1: the plane evidence the angle reading is derived from.
      surfaceType: 0,
      normal: [0, 0, 1],
      hasNormal: true,
    });
  });

  it("copies the center array (a later mutation of the DTO cannot leak in)", () => {
    const info = {
      elementId: "el_1",
      topoKey: "f:1",
      bodyId: "b",
      kind: "face",
      surfaceType: 0,
      curveType: -1,
      center: [1, 2, 3] as [number, number, number],
      normal: [0, 0, 1] as [number, number, number],
      hasNormal: true,
      size: 1,
      magnitude: 1,
    };
    const p = pickFromElementInfo("body1", info);
    info.center[0] = 999;
    expect(p.center[0]).toBe(1);
  });
});

describe("measureAdd — keep-latest-2", () => {
  it("holds a single pick", () => {
    const s = measureAdd(measureInit(), pick("a", [0, 0, 0]));
    expect(s.picks.map((p) => p.elementId)).toEqual(["a"]);
    expect(measureSummary(s)).toBeNull();
  });

  it("holds two picks in click order", () => {
    let s = measureAdd(measureInit(), pick("a", [0, 0, 0]));
    s = measureAdd(s, pick("b", [1, 0, 0]));
    expect(s.picks.map((p) => p.elementId)).toEqual(["a", "b"]);
  });

  it("a THIRD pick replaces the OLDEST (b↔c, not a↔b)", () => {
    let s = measureAdd(measureInit(), pick("a", [0, 0, 0]));
    s = measureAdd(s, pick("b", [1, 0, 0]));
    s = measureAdd(s, pick("c", [2, 0, 0]));
    expect(s.picks).toHaveLength(MAX_PICKS);
    expect(s.picks.map((p) => p.elementId)).toEqual(["b", "c"]);
  });

  it("keeps rolling on further picks", () => {
    let s = measureInit();
    for (const id of ["a", "b", "c", "d", "e"]) s = measureAdd(s, pick(id, [0, 0, 0]));
    expect(s.picks.map((p) => p.elementId)).toEqual(["d", "e"]);
  });

  it("re-picking the same element refreshes it in place — never pairs it with itself", () => {
    let s = measureAdd(measureInit(), pick("a", [0, 0, 0]));
    s = measureAdd(s, pick("a", [0, 0, 0], { magnitude: 950 }));
    expect(s.picks).toHaveLength(1);
    expect(s.picks[0].magnitude).toBe(950);
    expect(measureSummary(s)).toBeNull(); // no bogus distance-0 reading
  });

  it("the same elementId on a DIFFERENT body is a distinct pick", () => {
    let s = measureAdd(measureInit(), pick("el_1", [0, 0, 0]));
    s = measureAdd(s, { ...pick("el_1", [3, 0, 0]), bodyId: "body2" });
    expect(s.picks).toHaveLength(2);
    expect(measureSummary(s)?.distance).toBeCloseTo(3, 6);
  });

  it("does not mutate the input state", () => {
    const s0 = measureAdd(measureInit(), pick("a", [0, 0, 0]));
    const s1 = measureAdd(s0, pick("b", [1, 0, 0]));
    expect(s0.picks).toHaveLength(1);
    expect(s1).not.toBe(s0);
  });
});

describe("measureSummary — center ↔ center distance", () => {
  it("is null below two picks", () => {
    expect(measureSummary(measureInit())).toBeNull();
    expect(measureSummary(measureAdd(measureInit(), pick("a", [0, 0, 0])))).toBeNull();
  });

  it("computes a 3-4-5 style distance to 6dp", () => {
    let s = measureAdd(measureInit(), pick("a", [0, 0, 0]));
    s = measureAdd(s, pick("b", [3, 4, 12]));
    const sum = measureSummary(s);
    expect(sum?.distance).toBeCloseTo(13, 6);
    expect(sum?.delta).toEqual([3, 4, 12]);
  });

  it("matches the real-worker fixture geometry (top face ↔ origin)", () => {
    // wire_contract.rs pins the extruded rect's top face centre at (-10,20,25).
    let s = measureAdd(measureInit(), pick("origin", [0, 0, 0]));
    s = measureAdd(s, pick("top", [-10, 20, 25]));
    expect(measureSummary(s)?.distance).toBeCloseTo(Math.hypot(10, 20, 25), 6);
    expect(measureSummary(s)?.distance).toBeCloseTo(33.5410196, 6);
  });

  it("delta is second − first, so its SIGN is meaningful", () => {
    let s = measureAdd(measureInit(), pick("a", [10, 10, 10]));
    s = measureAdd(s, pick("b", [4, 12, 0]));
    expect(measureSummary(s)?.delta).toEqual([-6, 2, -10]);
  });

  it("is symmetric in magnitude when the picks are swapped", () => {
    const a = pick("a", [1, 2, 3]);
    const b = pick("b", [-4, 8, 0.5]);
    const ab = measureSummary(measureAdd(measureAdd(measureInit(), a), b));
    const ba = measureSummary(measureAdd(measureAdd(measureInit(), b), a));
    expect(ab?.distance).toBeCloseTo(ba?.distance ?? -1, 9);
    expect(ab?.delta.map((d) => -d)).toEqual(ba?.delta);
  });

  it("reports 0 for two distinct elements that share a bbox centre", () => {
    // Two coincident-centre faces is a legitimate reading, not a bug — which is
    // exactly why the label says "center ↔ center" and not "distance".
    let s = measureAdd(measureInit(), pick("a", [5, 5, 5]));
    s = measureAdd(s, pick("b", [5, 5, 5]));
    expect(measureSummary(s)?.distance).toBe(0);
  });
});

// ── WP-C1: the plane relationship (angle / parallel offset) ──────────────────

/** A pick that IS a plane with the given unit normal. */
function planePick(
  id: string,
  center: [number, number, number],
  normal: [number, number, number],
): MeasurePick {
  return pick(id, center, { surfaceType: 0, normal, hasNormal: true });
}

function summaryOf(a: MeasurePick, b: MeasurePick) {
  return measureSummary(measureAdd(measureAdd(measureInit(), a), b));
}

describe("planeOf — what counts as a plane", () => {
  it("accepts a planar face and returns a UNIT normal", () => {
    expect(planeOf(planePick("a", [0, 0, 0], [0, 0, 3]))).toEqual([0, 0, 1]);
  });

  it("rejects a CURVED face even though its descriptor carries a normal", () => {
    // surfaceType 1 stands in for GeomAbs_Cylinder. Its `normal` is a surface
    // normal at some point, not a plane normal — measuring an "angle between
    // planes" from it would be a fabricated number.
    expect(planeOf(pick("cyl", [0, 0, 0], { surfaceType: 1 }))).toBeNull();
  });

  it("rejects a face whose descriptor carried NO normal", () => {
    // The DTO's fallback is (0,0,1); trusting it would make every such face
    // "face +Z" and silently produce angles against a normal nobody measured.
    expect(planeOf(pick("f", [0, 0, 0], { hasNormal: false }))).toBeNull();
  });

  it("rejects an edge and a degenerate normal", () => {
    expect(planeOf(pick("e", [0, 0, 0], { kind: "edge", surfaceType: -1 }))).toBeNull();
    expect(planeOf(planePick("z", [0, 0, 0], [0, 0, 0]))).toBeNull();
  });
});

describe("measureSummary — angle between two planes", () => {
  it("perpendicular planes read 90°, and the two forms COLLAPSE to one", () => {
    const s = summaryOf(planePick("a", [0, 0, 0], [0, 0, 1]), planePick("b", [5, 0, 0], [1, 0, 0]));
    expect(s?.angle?.acuteDeg).toBeCloseTo(90, 9);
    expect(s?.angle?.obtuseDeg).toBeCloseTo(90, 9);
    expect(s?.angle?.isRight).toBe(true);
    expect(s?.planeOffset).toBeNull();
  });

  it("60°/120° — both supplementary forms are reported", () => {
    // Normals 60° apart ⇒ acos(|n1·n2|) = 60.
    const n: [number, number, number] = [Math.cos(Math.PI / 3), 0, Math.sin(Math.PI / 3)];
    const s = summaryOf(planePick("a", [0, 0, 0], [1, 0, 0]), planePick("b", [1, 0, 0], n));
    expect(s?.angle?.acuteDeg).toBeCloseTo(60, 9);
    expect(s?.angle?.obtuseDeg).toBeCloseTo(120, 9);
    expect(s?.angle?.isRight).toBe(false);
  });

  it("uses |n·n|, so FLIPPING one normal reports the same pair", () => {
    // A face's outward normal direction is an orientation fact the reading must
    // not depend on: the dihedral between two PLANES is the same either way.
    const n: [number, number, number] = [Math.cos(Math.PI / 3), 0, Math.sin(Math.PI / 3)];
    const flipped: [number, number, number] = [-n[0], -n[1], -n[2]];
    const a = summaryOf(planePick("a", [0, 0, 0], [1, 0, 0]), planePick("b", [1, 0, 0], n));
    const b = summaryOf(planePick("a", [0, 0, 0], [1, 0, 0]), planePick("b", [1, 0, 0], flipped));
    expect(b?.angle?.acuteDeg).toBeCloseTo(a?.angle?.acuteDeg ?? -1, 12);
  });

  it("never yields NaN when the dot product floats a hair past 1", () => {
    // acos(1 + 1e-16) is NaN and would render as "NaN°"; the clamp prevents it.
    const s = summaryOf(
      planePick("a", [0, 0, 0], [1, 0, 0]),
      planePick("b", [1, 0, 0], [1 + 1e-15, 0, 0]),
    );
    expect(Number.isNaN(s?.planeOffset ?? NaN)).toBe(false);
    expect(s?.angle).toBeNull(); // that pair is parallel, not a 0° angle
  });

  it("is NOT reported when either pick is not a plane", () => {
    const s = summaryOf(
      planePick("a", [0, 0, 0], [0, 0, 1]),
      pick("cyl", [5, 0, 0], { surfaceType: 1 }),
    );
    expect(s?.angle).toBeNull();
    expect(s?.planeOffset).toBeNull();
    expect(s?.distance).toBeCloseTo(5, 9); // the distance reading still stands
  });
});

describe("measureSummary — parallel planes report an OFFSET, not an angle", () => {
  it("two +Z planes 30 apart in Z are 30 apart", () => {
    const s = summaryOf(planePick("a", [0, 0, 0], [0, 0, 1]), planePick("b", [0, 0, 30], [0, 0, 1]));
    expect(s?.angle).toBeNull();
    expect(s?.planeOffset).toBeCloseTo(30, 9);
  });

  it("the offset ignores IN-PLANE separation — it is a true perpendicular distance", () => {
    // The bbox centres are far apart sideways; the plane separation is still 30.
    const s = summaryOf(
      planePick("a", [0, 0, 0], [0, 0, 1]),
      planePick("b", [400, -250, 30], [0, 0, 1]),
    );
    expect(s?.planeOffset).toBeCloseTo(30, 9);
    // …and it is a DIFFERENT number from the centre-to-centre distance beside it,
    // which is exactly why the label must not just say "Distance".
    expect(s?.distance).toBeGreaterThan(400);
  });

  it("ANTI-parallel planes (a box's two opposite faces) also report an offset", () => {
    const s = summaryOf(
      planePick("bottom", [0, 0, 0], [0, 0, -1]),
      planePick("top", [0, 0, 30], [0, 0, 1]),
    );
    expect(s?.angle).toBeNull();
    expect(s?.planeOffset).toBeCloseTo(30, 9);
  });

  it("COINCIDENT planes report 0 — an honest reading, not a missing one", () => {
    const s = summaryOf(planePick("a", [0, 0, 7], [0, 0, 1]), planePick("b", [5, 5, 7], [0, 0, 1]));
    expect(s?.planeOffset).toBe(0);
  });

  it("a pair just OUTSIDE the parallel tolerance is an angle, not an offset", () => {
    // 0.5° apart — well past PARALLEL_EPS (≈0.081°).
    const t = (0.5 * Math.PI) / 180;
    const s = summaryOf(
      planePick("a", [0, 0, 0], [0, 0, 1]),
      planePick("b", [0, 0, 30], [0, Math.sin(t), Math.cos(t)]),
    );
    expect(s?.planeOffset).toBeNull();
    expect(s?.angle?.acuteDeg).toBeCloseTo(0.5, 6);
  });
});
