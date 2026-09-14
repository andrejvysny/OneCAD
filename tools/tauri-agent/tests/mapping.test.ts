import { describe, expect, test } from "bun:test";
import { isAgentError } from "../src/errors.ts";
import {
  checkPoint,
  cssToGlobal,
  globalToCss,
  pointInNativeOcclusion,
  pointInWindow,
  rectCenter,
  rectIsFinite,
  rectStable,
  windowToGlobal,
} from "../src/geometry/mapping.ts";
import type { Rect, WindowGeom } from "../src/geometry/types.ts";

function geom(over: Partial<WindowGeom> = {}): WindowGeom {
  return {
    innerPositionPx: { x: 200, y: 100 },
    innerSizePx: { width: 1600, height: 1000 },
    scaleFactor: 1,
    nativeBoundsPt: { x: 200, y: 100, width: 1600, height: 1000 },
    windowId: 42,
    ...over,
  };
}

function codeOf(fn: () => unknown): string {
  try {
    fn();
  } catch (e) {
    return isAgentError(e) ? e.code : `not-an-AgentError:${String(e)}`;
  }
  return "no-throw";
}

describe("cssToGlobal / globalToCss", () => {
  test("scaleFactor 1: origin is innerPosition", () => {
    expect(cssToGlobal({ x: 0, y: 0 }, geom())).toEqual({ x: 200, y: 100 });
    expect(cssToGlobal({ x: 10, y: 20 }, geom())).toEqual({ x: 210, y: 120 });
  });

  test("scaleFactor 2: innerPosition is physical and must be divided", () => {
    const g = geom({ innerPositionPx: { x: 400, y: 200 }, scaleFactor: 2 });
    expect(cssToGlobal({ x: 0, y: 0 }, g)).toEqual({ x: 200, y: 100 });
    expect(cssToGlobal({ x: 50, y: 25 }, g)).toEqual({ x: 250, y: 125 });
  });

  test("round trip is exact at sf 1 and sf 2", () => {
    for (const sf of [1, 2]) {
      const g = geom({ innerPositionPx: { x: 300 * sf, y: 150 * sf }, scaleFactor: sf });
      for (const p of [
        { x: 0, y: 0 },
        { x: 17.5, y: 923.25 },
        { x: 1599.5, y: 0.5 },
      ]) {
        expect(globalToCss(cssToGlobal(p, g), g)).toEqual(p);
      }
    }
  });

  test("negative display origin (monitor left of / above the primary)", () => {
    const g = geom({ innerPositionPx: { x: -3440, y: -220 }, scaleFactor: 1 });
    expect(cssToGlobal({ x: 10, y: 10 }, g)).toEqual({ x: -3430, y: -210 });
    expect(globalToCss({ x: -3430, y: -210 }, g)).toEqual({ x: 10, y: 10 });

    const hidpi = geom({ innerPositionPx: { x: -2880, y: -400 }, scaleFactor: 2 });
    expect(cssToGlobal({ x: 40, y: 60 }, hidpi)).toEqual({ x: -1400, y: -140 });
    expect(globalToCss({ x: -1400, y: -140 }, hidpi)).toEqual({ x: 40, y: 60 });
  });

  test("windowToGlobal matches cssToGlobal (Overlay title bar: content == window)", () => {
    const g = geom({ scaleFactor: 2, innerPositionPx: { x: 40, y: 80 } });
    expect(windowToGlobal({ x: 12, y: 34 }, g)).toEqual(cssToGlobal({ x: 12, y: 34 }, g));
  });

  test("a non-positive scale factor refuses rather than producing Infinity", () => {
    expect(codeOf(() => cssToGlobal({ x: 1, y: 1 }, geom({ scaleFactor: 0 })))).toBe("CALIBRATION_FAILED");
    expect(codeOf(() => cssToGlobal({ x: 1, y: 1 }, geom({ scaleFactor: Number.NaN })))).toBe("CALIBRATION_FAILED");
  });
});

describe("pointInWindow", () => {
  const g = geom({ innerPositionPx: { x: 200, y: 100 }, innerSizePx: { width: 800, height: 600 }, scaleFactor: 1 });

  test("the near edges are inside, the far edges are not", () => {
    expect(pointInWindow({ x: 200, y: 100 }, g)).toBe(true);
    expect(pointInWindow({ x: 999.999, y: 699.999 }, g)).toBe(true);
    // origin + size addresses the first point OUTSIDE — another application's pixel.
    expect(pointInWindow({ x: 1000, y: 400 }, g)).toBe(false);
    expect(pointInWindow({ x: 500, y: 700 }, g)).toBe(false);
    expect(pointInWindow({ x: 199.999, y: 400 }, g)).toBe(false);
    expect(pointInWindow({ x: 500, y: 99.999 }, g)).toBe(false);
  });

  test("logical size, not physical, bounds the window on a 2x display", () => {
    const hidpi = geom({ innerPositionPx: { x: 0, y: 0 }, innerSizePx: { width: 1600, height: 1200 }, scaleFactor: 2 });
    expect(pointInWindow({ x: 799, y: 599 }, hidpi)).toBe(true);
    expect(pointInWindow({ x: 800, y: 300 }, hidpi)).toBe(false);
  });

  test("negative origins bound correctly", () => {
    const left = geom({ innerPositionPx: { x: -1200, y: -300 }, innerSizePx: { width: 400, height: 200 }, scaleFactor: 1 });
    expect(pointInWindow({ x: -1200, y: -300 }, left)).toBe(true);
    expect(pointInWindow({ x: -801, y: -101 }, left)).toBe(true);
    expect(pointInWindow({ x: -800, y: -200 }, left)).toBe(false);
    expect(pointInWindow({ x: -1201, y: -200 }, left)).toBe(false);
  });

  test("a non-finite point is never inside", () => {
    expect(pointInWindow({ x: Number.NaN, y: 400 }, g)).toBe(false);
    expect(pointInWindow({ x: Number.POSITIVE_INFINITY, y: 400 }, g)).toBe(false);
  });
});

describe("native occlusion", () => {
  const lights: Rect[] = [{ x: 0, y: 0, width: 80, height: 28 }];

  test("half-open on the far edges", () => {
    expect(pointInNativeOcclusion({ x: 0, y: 0 }, lights)).toBe(true);
    expect(pointInNativeOcclusion({ x: 79.9, y: 27.9 }, lights)).toBe(true);
    expect(pointInNativeOcclusion({ x: 80, y: 14 }, lights)).toBe(false);
    expect(pointInNativeOcclusion({ x: 40, y: 28 }, lights)).toBe(false);
    expect(pointInNativeOcclusion({ x: -1, y: 14 }, lights)).toBe(false);
  });

  test("an empty rect list occludes nothing", () => {
    expect(pointInNativeOcclusion({ x: 0, y: 0 }, [])).toBe(false);
  });

  test("a rect carrying the wrong key names occludes nothing at all", () => {
    // The config once shipped {x,y,w,h}: every comparison became NaN and the gate
    // silently passed the cursor through the traffic lights.
    const wrongShape = [{ x: 0, y: 0, w: 80, h: 28 }] as unknown as Rect[];
    expect(pointInNativeOcclusion({ x: 20, y: 14 }, wrongShape)).toBe(false);
  });
});

describe("checkPoint", () => {
  const g = geom({ innerPositionPx: { x: 200, y: 100 }, innerSizePx: { width: 800, height: 600 }, scaleFactor: 1 });
  const lights: Rect[] = [{ x: 0, y: 0, width: 80, height: 28 }];

  test("returns the global point for a clear css point", () => {
    expect(checkPoint({ x: 300, y: 200 }, g, lights)).toEqual({ x: 500, y: 300 });
  });

  test("outside the window refuses with POINT_OUTSIDE_WINDOW", () => {
    expect(codeOf(() => checkPoint({ x: 800, y: 10 }, g, lights))).toBe("POINT_OUTSIDE_WINDOW");
    expect(codeOf(() => checkPoint({ x: -1, y: 10 }, g, lights))).toBe("POINT_OUTSIDE_WINDOW");
  });

  test("under the traffic lights refuses with ELEMENT_OCCLUDED and names the reason", () => {
    expect(codeOf(() => checkPoint({ x: 20, y: 14 }, g, lights))).toBe("ELEMENT_OCCLUDED");
    try {
      checkPoint({ x: 20, y: 14 }, g, lights);
      throw new Error("expected a refusal");
    } catch (e) {
      if (!isAgentError(e)) throw e;
      expect(e.details?.reason).toBe("native-titlebar-controls");
    }
  });

  test("window bounds are checked before occlusion", () => {
    const shifted = geom({ innerPositionPx: { x: 0, y: 0 }, innerSizePx: { width: 10, height: 10 }, scaleFactor: 1 });
    expect(codeOf(() => checkPoint({ x: 40, y: 14 }, shifted, lights))).toBe("POINT_OUTSIDE_WINDOW");
  });
});

describe("rect helpers", () => {
  test("rectStable tolerates 0.5 px of sub-pixel jitter and nothing more", () => {
    const a: Rect = { x: 10, y: 20, width: 100, height: 30 };
    expect(rectStable(a, { x: 10.5, y: 20.5, width: 100.5, height: 30.5 })).toBe(true);
    expect(rectStable(a, { x: 10.51, y: 20, width: 100, height: 30 })).toBe(false);
    expect(rectStable(a, { x: 10, y: 20, width: 100, height: 34 })).toBe(false);
    expect(rectStable(a, { x: 14, y: 20, width: 100, height: 30 }, 5)).toBe(true);
  });

  test("a rect whose numbers did not survive the wire is never stable", () => {
    const a: Rect = { x: 10, y: 20, width: 100, height: 30 };
    expect(rectStable(a, { ...a, x: Number.NaN })).toBe(false);
    expect(rectStable(a, { ...a, height: Number.POSITIVE_INFINITY })).toBe(false);
    // The embedded driver serializes NaN/Infinity as `null`, and `null - null` is
    // 0 — without a finiteness gate a blanked rect reads as a perfectly still one.
    const blanked = { x: null, y: null, width: null, height: null } as unknown as Rect;
    expect(rectStable(blanked, blanked)).toBe(false);
    expect(rectStable(a, blanked)).toBe(false);
  });

  test("rectIsFinite rejects the shapes rectStable must never accept", () => {
    expect(rectIsFinite({ x: 0, y: 0, width: 1, height: 1 })).toBe(true);
    expect(rectIsFinite({ x: 0, y: 0, width: 1, height: Number.NaN })).toBe(false);
    expect(rectIsFinite({ x: null, y: 0, width: 1, height: 1 } as unknown as Rect)).toBe(false);
    expect(rectIsFinite(undefined)).toBe(false);
  });

  test("rectCenter applies the offset from the centre", () => {
    const r: Rect = { x: 10, y: 20, width: 100, height: 40 };
    expect(rectCenter(r)).toEqual({ x: 60, y: 40 });
    expect(rectCenter(r, { x: -5, y: 7 })).toEqual({ x: 55, y: 47 });
  });
});
