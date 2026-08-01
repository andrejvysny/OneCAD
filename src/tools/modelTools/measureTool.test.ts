import { describe, it, expect } from "vitest";
import type { ElementInfo } from "@/ipc/types";
import {
  MAX_PICKS,
  measureAdd,
  measureClear,
  measureInit,
  measureSummary,
  pickFromElementInfo,
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
