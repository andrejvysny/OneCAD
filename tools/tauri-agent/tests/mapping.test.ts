import { describe, expect, test } from "bun:test";
import { isAgentError } from "../src/errors.ts";
import {
  checkGlobalPoint,
  checkPoint,
  cssToGlobal,
  globalToCss,
  pointInNativeOcclusion,
  pointInRect,
  pointInWindow,
  rectCenter,
  rectIsFinite,
  rectStable,
  windowToGlobal,
} from "../src/geometry/mapping.ts";
import type { OwnedWindow } from "../src/geometry/mapping.ts";
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

describe("checkGlobalPoint", () => {
  // The calibrated main window: global (200,100) 800x600, and the traffic lights in its own
  // content-box css space.
  const g = geom({ innerPositionPx: { x: 200, y: 100 }, innerSizePx: { width: 800, height: 600 }, scaleFactor: 1 });
  const lights: Rect[] = [{ x: 0, y: 0, width: 80, height: 28 }];
  const MAIN: OwnedWindow = { windowId: 42, bounds: { x: 200, y: 100, width: 800, height: 600 } };
  /** A second window of the SAME app, nowhere near the main one. */
  const SECONDARY: OwnedWindow = { windowId: 43, bounds: { x: 1200, y: 700, width: 400, height: 300 } };
  /** A modal sheet that overhangs the main window's bottom edge — legitimate, and unreachable
   *  through `checkPoint`, which only knows the main window. */
  const SHEET: OwnedWindow = { windowId: 44, bounds: { x: 300, y: 500, width: 600, height: 400 } };
  const anchored = { geom: g, rects: lights };

  test("admits a point in the main window", () => {
    expect(checkGlobalPoint({ x: 500, y: 300 }, [MAIN], anchored)).toEqual({ x: 500, y: 300 });
  });

  test("admits a point in a secondary window of this application", () => {
    expect(checkGlobalPoint({ x: 1300, y: 800 }, [MAIN, SECONDARY], anchored)).toEqual({ x: 1300, y: 800 });
  });

  test("admits a point in a modal that extends past the main window", () => {
    // (600, 850) is below the main window's bottom edge (100 + 600 = 700), so `checkPoint` would
    // refuse it. That is exactly the NSSavePanel case this gate exists for.
    expect(pointInWindow({ x: 600, y: 850 }, g)).toBe(false);
    expect(checkGlobalPoint({ x: 600, y: 850 }, [MAIN, SHEET], anchored)).toEqual({ x: 600, y: 850 });
  });

  test("refuses a point in another application's window", () => {
    // Inside SECONDARY's rect, but SECONDARY is not in the allowed list: it belongs to someone else.
    expect(codeOf(() => checkGlobalPoint({ x: 1300, y: 800 }, [MAIN], anchored))).toBe("POINT_OUTSIDE_WINDOW");
    expect(codeOf(() => checkGlobalPoint({ x: 5000, y: 5000 }, [MAIN, SECONDARY], null))).toBe(
      "POINT_OUTSIDE_WINDOW",
    );
  });

  test("the refusal names the windows it did check", () => {
    try {
      checkGlobalPoint({ x: 1300, y: 800 }, [MAIN], anchored);
      throw new Error("expected a refusal");
    } catch (e) {
      if (!isAgentError(e)) throw e;
      expect(e.details?.windows).toEqual([{ windowId: 42, bounds: MAIN.bounds }]);
    }
  });

  test("an empty window list is WINDOW_NOT_FOUND, not a silent pass", () => {
    expect(codeOf(() => checkGlobalPoint({ x: 500, y: 300 }, [], anchored))).toBe("WINDOW_NOT_FOUND");
  });

  test("a non-finite point is refused as an invalid target, never as 'outside the window'", () => {
    expect(codeOf(() => checkGlobalPoint({ x: Number.NaN, y: 300 }, [MAIN], anchored))).toBe("INVALID_TARGET");
    expect(codeOf(() => checkGlobalPoint({ x: 500, y: Number.POSITIVE_INFINITY }, [MAIN], anchored))).toBe(
      "INVALID_TARGET",
    );
  });

  test("occlusion applies inside the anchor window", () => {
    // css (20, 14) in the main window is global (220, 114) — under the traffic lights.
    expect(codeOf(() => checkGlobalPoint({ x: 220, y: 114 }, [MAIN], anchored))).toBe("ELEMENT_OCCLUDED");
    expect(checkGlobalPoint({ x: 220, y: 114 }, [MAIN], null)).toEqual({ x: 220, y: 114 });
  });

  test("occlusion does NOT leak into another window of the same app", () => {
    // A point 14 pt below a secondary window's top-left corner is under NOTHING the config knows
    // about: those rects were measured against the main window's content box. Interpreting them
    // in any other window would refuse a legitimate native target.
    const panel: OwnedWindow = { windowId: 43, bounds: { x: 210, y: 110, width: 400, height: 300 } };
    expect(checkGlobalPoint({ x: 220, y: 114 }, [panel, MAIN], anchored)).toEqual({ x: 220, y: 114 });
  });

  test("the front-most containing window decides, because it is the one that receives the click", () => {
    // Same point, two overlapping windows of this app. CGWindowList order is front-to-back, so
    // the first match is the window a click would actually land in — here the panel, which has no
    // occlusion rects of its own.
    const panel: OwnedWindow = { windowId: 43, bounds: { x: 200, y: 100, width: 200, height: 100 } };
    expect(checkGlobalPoint({ x: 220, y: 114 }, [panel, MAIN], anchored)).toEqual({ x: 220, y: 114 });
    expect(codeOf(() => checkGlobalPoint({ x: 220, y: 114 }, [MAIN, panel], anchored))).toBe("ELEMENT_OCCLUDED");
  });

  test("far edges are half-open, exactly as pointInWindow is", () => {
    expect(checkGlobalPoint({ x: 999.9, y: 699.9 }, [MAIN], null)).toEqual({ x: 999.9, y: 699.9 });
    expect(codeOf(() => checkGlobalPoint({ x: 1000, y: 400 }, [MAIN], null))).toBe("POINT_OUTSIDE_WINDOW");
    expect(codeOf(() => checkGlobalPoint({ x: 500, y: 700 }, [MAIN], null))).toBe("POINT_OUTSIDE_WINDOW");
  });

  test("a window whose bounds did not survive the wire contains nothing", () => {
    const broken = { windowId: 9, bounds: { x: Number.NaN, y: 0, width: 100, height: 100 } } as OwnedWindow;
    expect(codeOf(() => checkGlobalPoint({ x: 50, y: 50 }, [broken], null))).toBe("POINT_OUTSIDE_WINDOW");
  });
});

describe("rect helpers", () => {
  test("pointInRect is half-open on the far edges and screens non-finite input", () => {
    const r: Rect = { x: 10, y: 20, width: 100, height: 30 };
    expect(pointInRect({ x: 10, y: 20 }, r)).toBe(true);
    expect(pointInRect({ x: 109.9, y: 49.9 }, r)).toBe(true);
    expect(pointInRect({ x: 110, y: 30 }, r)).toBe(false);
    expect(pointInRect({ x: 50, y: 50 }, r)).toBe(false);
    expect(pointInRect({ x: Number.NaN, y: 30 }, r)).toBe(false);
    expect(pointInRect({ x: 50, y: 30 }, { ...r, width: Number.NaN })).toBe(false);
  });

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
