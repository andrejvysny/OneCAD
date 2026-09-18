/*
 * handleProjection — the projected screen behaviour of a world axis.
 *
 * PURE, like the module: the matrices are built here by hand rather than taken
 * from a THREE camera, so a change in three's camera conventions can never make
 * these tests quietly agree with a wrong implementation.
 *
 * The load-bearing case is "perspective, anchor OFF the optical axis". That is
 * the one `DragHandle.orient()`'s camera-rotation shortcut gets wrong, and the
 * reason this module exists — see the RED anchor in that describe block.
 */
import { describe, it, expect } from "vitest";
import {
  projectAxis,
  scalarSensitivity,
  classifyMapping,
  sampleMapping,
  handlePointAt,
  MIN_AXIS_CONDITIONING,
  type AnchorScale,
  type FrozenMapping,
  type LinearHandlePath,
  type Mat4,
  type ProjectionContext,
} from "./handleProjection";
import type { Vec3 } from "./depthProjection";

const W = 800;
const H = 600;

/** Column-major perspective matrix, three's `PerspectiveCamera` convention. */
function perspective(fovYDeg: number, aspect: number, near: number, far: number): Mat4 {
  const f = 1 / Math.tan((fovYDeg * Math.PI) / 360);
  const m = new Array<number>(16).fill(0);
  m[0] = f / aspect;
  m[5] = f;
  m[10] = (far + near) / (near - far);
  m[11] = -1;
  m[14] = (2 * far * near) / (near - far);
  return m;
}

/** Column-major orthographic matrix for a symmetric frustum. */
function orthographic(halfW: number, halfH: number, near: number, far: number): Mat4 {
  const m = new Array<number>(16).fill(0);
  m[0] = 1 / halfW;
  m[5] = 1 / halfH;
  m[10] = -2 / (far - near);
  m[14] = -(far + near) / (far - near);
  m[15] = 1;
  return m;
}

/** `proj · translate(-eye)` — a camera at `eye` looking down world −Z. */
function viewProjAt(proj: Mat4, eye: Vec3): Mat4 {
  const m = proj.slice();
  // Column-major: the translation column is elements 12..15. Post-multiplying by
  // a pure translation adds proj·(-eye) into it.
  m[12] = proj[0] * -eye[0] + proj[4] * -eye[1] + proj[8] * -eye[2] + proj[12];
  m[13] = proj[1] * -eye[0] + proj[5] * -eye[1] + proj[9] * -eye[2] + proj[13];
  m[14] = proj[2] * -eye[0] + proj[6] * -eye[1] + proj[10] * -eye[2] + proj[14];
  m[15] = proj[3] * -eye[0] + proj[7] * -eye[1] + proj[11] * -eye[2] + proj[15];
  return m;
}

/** World units per CSS px at `anchor`, mirroring `screenScale.worldPerPixel`. */
function worldPerPixelPersp(fovYDeg: number, depth: number, height: number): number {
  return (2 * depth * Math.tan((fovYDeg * Math.PI) / 360)) / height;
}

describe("projectAxis — orthographic", () => {
  const proj = orthographic(10, 7.5, 0.1, 100);
  const vp = viewProjAt(proj, [0, 0, 20]);

  it("maps world +X to screen +X, and the scale is the frustum's", () => {
    const p = projectAxis(vp, [0, 0, 0], [1, 0, 0], W, H);
    expect(p).not.toBeNull();
    // Half-width 10 world units spans half the 800px viewport ⇒ 40 px per unit.
    expect(p!.pxPerWorld).toBeCloseTo(40, 9);
    expect(p!.direction[0]).toBeCloseTo(1, 9);
    expect(p!.direction[1]).toBeCloseTo(0, 9);
  });

  it("flips world +Y to screen −Y (NDC up is CSS down)", () => {
    const p = projectAxis(vp, [0, 0, 0], [0, 1, 0], W, H);
    expect(p!.direction[1]).toBeCloseTo(-1, 9);
    expect(p!.pxPerWorld).toBeCloseTo(40, 9);
  });

  it("is anchor-INDEPENDENT, which is exactly why the old shortcut worked here", () => {
    const centre = projectAxis(vp, [0, 0, 0], [1, 0, 0], W, H)!;
    const corner = projectAxis(vp, [9, 7, -3], [1, 0, 0], W, H)!;
    expect(corner.pxPerWorld).toBeCloseTo(centre.pxPerWorld, 9);
    expect(corner.direction[0]).toBeCloseTo(centre.direction[0], 9);
    expect(corner.direction[1]).toBeCloseTo(centre.direction[1], 9);
  });

  it("normalizes the direction, so a non-unit input still reports per-world-unit", () => {
    const unitDir = projectAxis(vp, [0, 0, 0], [1, 0, 0], W, H)!;
    const longDir = projectAxis(vp, [0, 0, 0], [7, 0, 0], W, H)!;
    expect(longDir.pxPerWorld).toBeCloseTo(unitDir.pxPerWorld, 9);
  });
});

describe("projectAxis — perspective, the case the camera-rotation shortcut gets wrong", () => {
  const FOV = 76; // CameraRig's default
  const proj = perspective(FOV, W / H, 0.1, 1000);
  const vp = viewProjAt(proj, [0, 0, 20]);

  it("on the optical axis, world +X is pure screen +X", () => {
    const p = projectAxis(vp, [0, 0, 0], [1, 0, 0], W, H)!;
    expect(p.direction[0]).toBeCloseTo(1, 9);
    expect(p.direction[1]).toBeCloseTo(0, 9);
  });

  /*
   * RED ANCHOR for the defect. `DragHandle.orient()` computes the screen angle
   * from the axis in camera-rotation space alone, so for a camera with no roll
   * it reports world +Z as "straight up the screen" at EVERY anchor. The real
   * projection disagrees as soon as the anchor leaves the optical axis: +Z runs
   * towards the camera, and off-centre that reads as motion towards the
   * vanishing point, which has a horizontal component.
   *
   * If this ever goes to 0 the module has regressed to the shortcut.
   */
  it("a world axis pointing at the camera acquires a SIDEWAYS component off-centre", () => {
    const onAxis = projectAxis(vp, [0, 0, 0], [0, 0, 1], W, H);
    // Dead centre, +Z is exactly end-on: no screen direction exists at all.
    expect(onAxis).toBeNull();

    const offAxis = projectAxis(vp, [8, 0, 0], [0, 0, 1], W, H)!;
    expect(Math.abs(offAxis.direction[0])).toBeGreaterThan(0.99);
    expect(offAxis.pxPerWorld).toBeGreaterThan(1);
  });

  it("the same world axis projects to DIFFERENT screen directions at different anchors", () => {
    const left = projectAxis(vp, [-8, 0, 0], [0, 0, 1], W, H)!;
    const right = projectAxis(vp, [8, 0, 0], [0, 0, 1], W, H)!;
    // Mirror-image anchors push the vanishing-point motion opposite ways.
    expect(Math.sign(left.direction[0])).toBe(-Math.sign(right.direction[0]));
  });

  it("apparent size falls off with depth: twice as far is half the pixels", () => {
    const near = projectAxis(viewProjAt(proj, [0, 0, 10]), [0, 0, 0], [1, 0, 0], W, H)!;
    const far = projectAxis(viewProjAt(proj, [0, 0, 20]), [0, 0, 0], [1, 0, 0], W, H)!;
    expect(far.pxPerWorld).toBeCloseTo(near.pxPerWorld / 2, 6);
  });

  it("scales with the viewport, so the result is CSS px and not NDC", () => {
    const small = projectAxis(vp, [0, 0, 0], [1, 0, 0], W, H)!;
    const big = projectAxis(vp, [0, 0, 0], [1, 0, 0], W * 2, H * 2)!;
    expect(big.pxPerWorld).toBeCloseTo(small.pxPerWorld * 2, 6);
  });
});

describe("projectAxis — refusals", () => {
  const proj = perspective(76, W / H, 0.1, 1000);
  const vp = viewProjAt(proj, [0, 0, 20]);

  it("refuses an anchor behind the camera", () => {
    expect(projectAxis(vp, [0, 0, 100], [1, 0, 0], W, H)).toBeNull();
  });

  it("refuses an anchor exactly on the camera plane", () => {
    expect(projectAxis(vp, [0, 0, 20], [1, 0, 0], W, H)).toBeNull();
  });

  it("refuses a zero-length direction rather than returning NaN", () => {
    expect(projectAxis(vp, [0, 0, 0], [0, 0, 0], W, H)).toBeNull();
  });

  it("refuses non-finite inputs on every channel", () => {
    expect(projectAxis(vp, [NaN, 0, 0], [1, 0, 0], W, H)).toBeNull();
    expect(projectAxis(vp, [0, 0, 0], [Infinity, 0, 0], W, H)).toBeNull();
    const bad = vp.slice();
    bad[5] = NaN;
    expect(projectAxis(bad, [0, 0, 0], [1, 0, 0], W, H)).toBeNull();
  });

  it("refuses a short matrix and a zero viewport instead of indexing past the end", () => {
    expect(projectAxis([1, 0, 0, 0], [0, 0, 0], [1, 0, 0], W, H)).toBeNull();
    expect(projectAxis(vp, [0, 0, 0], [1, 0, 0], 0, H)).toBeNull();
    expect(projectAxis(vp, [0, 0, 0], [1, 0, 0], W, 0)).toBeNull();
  });

  it("refuses an exactly end-on axis — no direction exists to draw", () => {
    expect(projectAxis(vp, [0, 0, 0], [0, 0, 1], W, H)).toBeNull();
  });
});

describe("scalarSensitivity", () => {
  const FOV = 76;
  const proj = perspective(FOV, W / H, 0.1, 1000);
  const eyeZ = 20;
  const vp = viewProjAt(proj, [0, 0, eyeZ]);
  const wpp = worldPerPixelPersp(FOV, eyeZ, H);
  const at = (deg: number): number => {
    const r = (deg * Math.PI) / 180;
    const p = projectAxis(vp, [0, 0, 0], [Math.sin(r), 0, Math.cos(r)], W, H)!;
    return scalarSensitivity(p.pxPerWorld, wpp);
  };

  it("is 1 for an axis square-on to the screen", () => {
    const p = projectAxis(vp, [0, 0, 0], [1, 0, 0], W, H)!;
    expect(scalarSensitivity(p.pxPerWorld, wpp)).toBeCloseTo(1, 6);
  });

  it("falls towards 0 as the axis tips towards the camera", () => {
    expect(at(3)).toBeLessThan(MIN_AXIS_CONDITIONING);
  });

  it("sits just above the floor at ~5°, and just below at ~4°", () => {
    expect(at(5)).toBeGreaterThan(MIN_AXIS_CONDITIONING);
    expect(at(4)).toBeLessThan(MIN_AXIS_CONDITIONING);
  });

  /*
   * The retired `Math.min(c, 1)` clamp. Conditioning is not a sine: off the
   * optical axis the singular values of the projected derivative are 1 and
   * sqrt(1 + (x/D)² + (y/D)²), so a real, usable sensitivity above 1 exists and
   * clamping it concealed how fast a drag would actually move.
   */
  it("reports a sensitivity above 1 UNCLAMPED", () => {
    const p = projectAxis(vp, [0, 0, 0], [1, 0, 0], W, H)!;
    expect(scalarSensitivity(p.pxPerWorld, wpp * 10)).toBeCloseTo(10, 6);
  });

  it("treats a nonsense scale as unusable rather than huge", () => {
    const p = projectAxis(vp, [0, 0, 0], [1, 0, 0], W, H)!;
    expect(scalarSensitivity(p.pxPerWorld, 0)).toBe(0);
    expect(scalarSensitivity(p.pxPerWorld, -1)).toBe(0);
    expect(scalarSensitivity(p.pxPerWorld, NaN)).toBe(0);
    expect(scalarSensitivity(0, wpp)).toBe(0);
  });
});

describe("classifyMapping — the world/proxy switch", () => {
  const FOV = 76;
  const eyeZ = 20;
  const perspCtx: ProjectionContext = {
    viewProj: viewProjAt(perspective(FOV, W / H, 0.1, 1000), [0, 0, eyeZ]),
    viewportWidth: W,
    viewportHeight: H,
  };
  const perspScale: AnchorScale = { worldPerPx: worldPerPixelPersp(FOV, eyeZ, H) };
  const orthoCtx: ProjectionContext = {
    viewProj: viewProjAt(orthographic(10, 7.5, 0.1, 100), [0, 0, 20]),
    viewportWidth: W,
    viewportHeight: H,
  };
  const orthoScale: AnchorScale = { worldPerPx: 1 / 40 };
  const along = (deg: number): LinearHandlePath => {
    const r = (deg * Math.PI) / 180;
    return { q0Mm: 0, point0Mm: [0, 0, 0], dPointDValue: [Math.sin(r), 0, Math.cos(r)] };
  };

  it("a well-conditioned axis maps along its own projected direction", () => {
    const p = projectAxis(perspCtx.viewProj, [0, 0, 0], [1, 0, 0], W, H)!;
    const m = classifyMapping({ q0Mm: 0, point0Mm: [0, 0, 0], dPointDValue: [1, 0, 0] }, perspCtx, perspScale);
    expect(m).toMatchObject({ kind: "world", direction: p.direction, g0PxPerMm: p.pxPerWorld });
  });

  it("an end-on axis is a vertical screen proxy, orthographic and perspective alike", () => {
    for (const [ctx, scale] of [
      [orthoCtx, orthoScale],
      [perspCtx, perspScale],
    ] as [ProjectionContext, AnchorScale][]) {
      const m = classifyMapping({ q0Mm: 0, point0Mm: [0, 0, 0], dPointDValue: [0, 0, 1] }, ctx, scale);
      expect(m).toEqual({
        kind: "proxy",
        q0Mm: 0,
        direction: [0, -1],
        mmPerPx: scale.worldPerPx,
        reason: "noProjection",
      });
    }
  });

  it("switches exactly at MIN_AXIS_CONDITIONING, and says which side refused", () => {
    const below = (Math.asin(MIN_AXIS_CONDITIONING) * 180) / Math.PI - 0.05;
    const above = (Math.asin(MIN_AXIS_CONDITIONING) * 180) / Math.PI + 0.05;
    for (const [ctx, scale] of [
      [orthoCtx, orthoScale],
      [perspCtx, perspScale],
    ] as [ProjectionContext, AnchorScale][]) {
      expect(classifyMapping(along(below), ctx, scale)).toMatchObject({
        kind: "proxy",
        reason: "poorScreenSensitivity",
      });
      expect(classifyMapping(along(above), ctx, scale).kind).toBe("world");
    }
  });

  it("every output stays finite, whatever the scale", () => {
    const finiteMapping = (m: FrozenMapping): boolean =>
      Number.isFinite(m.q0Mm) &&
      (m.kind === "disabled" ||
        (Number.isFinite(m.direction[0]) &&
          Number.isFinite(m.direction[1]) &&
          (m.kind === "proxy" ? Number.isFinite(m.mmPerPx) : Number.isFinite(m.g0PxPerMm))));
    for (const worldPerPx of [0, -1, NaN, Infinity]) {
      const m = classifyMapping(along(45), perspCtx, { worldPerPx });
      expect(m.kind).toBe("disabled");
      expect(finiteMapping(m)).toBe(true);
    }
    expect(finiteMapping(classifyMapping(along(45), perspCtx, perspScale))).toBe(true);
  });
});

describe("handlePointAt", () => {
  it("moves the attachment along the path as the value changes", () => {
    const path: LinearHandlePath = { q0Mm: 0, point0Mm: [1, 2, 3], dPointDValue: [0, 0, 2] };
    expect(handlePointAt(path, 0)).toEqual([1, 2, 3]);
    expect(handlePointAt(path, 5)).toEqual([1, 2, 13]);
  });

  it("is relative to q0, not to zero", () => {
    const path: LinearHandlePath = { q0Mm: 4, point0Mm: [0, 0, 0], dPointDValue: [1, 0, 0] };
    expect(handlePointAt(path, 4)).toEqual([0, 0, 0]);
    expect(handlePointAt(path, 6)).toEqual([2, 0, 0]);
  });
});

/*
 * H8 — the FROZEN mapping (docs/design/astra/modeling-handle-attachment.md §3,
 * §5, §6). Cameras and expected numbers are the derivation's §6 test vectors,
 * recomputed by the orchestrator; every one of them is quoted in the assertion
 * it pins.
 */
const VP = 1000; // §6 cameras are 1000 × 1000 CSS px

/** §6 camera O: orthographic, eye (0,0,100) down −Z, 100 mm view height. */
const cameraO = (): ProjectionContext => ({
  viewProj: viewProjAt(orthographic(50, 50, 0.1, 1000), [0, 0, 100]),
  viewportWidth: VP,
  viewportHeight: VP,
});
const SCALE_O: AnchorScale = { worldPerPx: 0.1 };

/** §6 camera P60: perspective FOV 60, eye (0,0,100) down −Z. */
const cameraP60 = (): ProjectionContext => ({
  viewProj: viewProjAt(perspective(60, 1, 0.1, 10000), [0, 0, 100]),
  viewportWidth: VP,
  viewportHeight: VP,
});
const scaleP60 = (depth: number): AnchorScale => ({ worldPerPx: worldPerPixelPersp(60, depth, VP) });

const world = (m: FrozenMapping): Extract<FrozenMapping, { kind: "world" }> => {
  expect(m.kind).toBe("world");
  return m as Extract<FrozenMapping, { kind: "world" }>;
};

describe("classifyMapping / sampleMapping — the exact perspective inverse", () => {
  // §6 "Exact perspective drag": H0 = 0, v = (0.6, 0, 0.8).
  const path: LinearHandlePath = { q0Mm: 0, point0Mm: [0, 0, 0], dPointDValue: [0.6, 0, 0.8] };

  it("freezes g = 5.196152 px/mm and k = −0.008 /mm at the grab", () => {
    const m = world(classifyMapping(path, cameraP60(), scaleP60(100)));
    expect(m.g0PxPerMm).toBeCloseTo(5.196152, 6);
    expect(m.kPerMm).toBeCloseTo(-0.008, 12);
    expect(m.direction[0]).toBeCloseTo(1, 12);
    expect(m.direction[1]).toBeCloseTo(0, 12);
    expect(m.q0Mm).toBe(0);
  });

  it("50 px is 8.934710 mm — a constant gain wrongly gives 9.622504 mm", () => {
    const m = world(classifyMapping(path, cameraP60(), scaleP60(100)));
    const sample = sampleMapping(m, [50, 0]);
    expect(sample.valueMm).toBeCloseTo(8.934710, 6);
    expect(sample.saturated).toBeUndefined();
    // The mapping this replaces: p / g.
    expect(50 / m.g0PxPerMm).toBeCloseTo(9.622504, 6);
    expect(Math.abs(sample.valueMm - 50 / m.g0PxPerMm)).toBeGreaterThan(0.6);
  });

  it("dq/dp = g/(g − kp)² > 0 everywhere inside the valid interval", () => {
    const m = world(classifyMapping(path, cameraP60(), scaleP60(100)));
    for (const p of [-500, -100, -1, 0, 1, 100, 1000, 10000]) {
      // Central difference: second order, so it pins the analytic slope rather
      // than the step's own truncation error.
      const h = 1e-3;
      const a = sampleMapping(m, [p - h, 0]).valueMm;
      const b = sampleMapping(m, [p + h, 0]).valueMm;
      const slope = (b - a) / (2 * h);
      expect(slope).toBeGreaterThan(0);
      const den = m.g0PxPerMm - m.kPerMm * p;
      expect(slope).toBeCloseTo(m.g0PxPerMm / (den * den), 6);
    }
  });

  it("the valid interval stops where the frozen sensitivity envelope reaches τ", () => {
    const m = world(classifyMapping(path, cameraP60(), scaleP60(100)));
    // Cq0 = 0.6, so the depth factor is capped at 0.6/0.08 = 7.5 ⇒ Δq ≥ −812.5 mm,
    // and 1 + kΔq > 0 ⇒ Δq < −1/k = 125 mm.
    expect(m.validDeltaMm[0]).toBeCloseTo(-812.5, 6);
    expect(m.validDeltaMm[1]).toBeCloseTo(125, 6);

    // §6 "Perspective sensitivity limit": the stop is at p = −562.916512 px.
    const atLimit = sampleMapping(m, [-562.916512, 0]);
    expect(atLimit.valueMm).toBeCloseTo(-812.5, 4);
    expect(atLimit.saturated).toBeUndefined();

    const past = sampleMapping(m, [-600, 0]);
    expect(past.valueMm).toBeCloseTo(-812.5, 6);
    expect(past.saturated).toBe("viewLimit");

    // Beyond the pole the denominator flips sign; the stop must HOLD, never wrap.
    const wayPast = sampleMapping(m, [-5000, 0]);
    expect(wayPast.valueMm).toBeCloseTo(-812.5, 6);
    expect(wayPast.saturated).toBe("viewLimit");
  });

  it("carries q0 so the sample is the value, not a delta", () => {
    const m = world(classifyMapping({ ...path, q0Mm: 2 }, cameraP60(), scaleP60(100)));
    expect(sampleMapping(m, [50, 0]).valueMm).toBeCloseTo(2 + 8.934710, 6);
  });

  it("an orthographic camera has k = 0 and an unbounded valid interval", () => {
    const m = world(classifyMapping({ q0Mm: 0, point0Mm: [0, 0, 0], dPointDValue: [1, 0, 0] }, cameraO(), SCALE_O));
    expect(m.kPerMm).toBe(0);
    expect(m.g0PxPerMm).toBeCloseTo(10, 9);
    expect(m.validDeltaMm[0]).toBe(-Infinity);
    expect(m.validDeltaMm[1]).toBe(Infinity);
    expect(sampleMapping(m, [10, 0]).valueMm).toBeCloseTo(1, 12);
  });
});

describe("classifyMapping — the threshold is on SCALAR sensitivity", () => {
  /** A unit direction whose conditioning under camera O is exactly `c`. */
  const radial = (c: number): Vec3 => [c, 0, Math.sqrt(1 - c * c)];

  it("Diameter's r̂/2 halves Cq: radial 0.1598/0.1602 ⇒ scalar 0.0799/0.0801", () => {
    const half = (c: number): LinearHandlePath => {
      const r = radial(c);
      return { q0Mm: 0, point0Mm: [0, 0, 0], dPointDValue: [r[0] / 2, r[1] / 2, r[2] / 2] };
    };
    const lo = classifyMapping(half(0.1598), cameraO(), SCALE_O);
    const hi = classifyMapping(half(0.1602), cameraO(), SCALE_O);
    expect(lo.kind).toBe("proxy");
    expect(hi.kind).toBe("world");
    expect(world(hi).g0PxPerMm * SCALE_O.worldPerPx).toBeCloseTo(0.0801, 9);
  });

  it("the same radial direction at gain 1 stays a world mapping on both sides", () => {
    for (const c of [0.1598, 0.1602]) {
      const m = classifyMapping({ q0Mm: 0, point0Mm: [0, 0, 0], dPointDValue: radial(c) }, cameraO(), SCALE_O);
      expect(m.kind).toBe("world");
    }
  });

  it("a proxy maps 10 px up to +1 mm at the anchor's own scale", () => {
    const m = classifyMapping({ q0Mm: 3, point0Mm: [0, 0, 0], dPointDValue: [0, 0, 1] }, cameraO(), SCALE_O);
    expect(m.kind).toBe("proxy");
    expect(m).toMatchObject({ direction: [0, -1], mmPerPx: 0.1, q0Mm: 3 });
    expect(sampleMapping(m, [0, -10]).valueMm).toBeCloseTo(4, 12);
  });

  it("no usable scale disables the drag rather than inventing a metric", () => {
    for (const scale of [null, { worldPerPx: 0 }, { worldPerPx: -1 }, { worldPerPx: NaN }]) {
      const m = classifyMapping({ q0Mm: 5, point0Mm: [0, 0, 0], dPointDValue: [1, 0, 0] }, cameraO(), scale);
      expect(m.kind).toBe("disabled");
      expect(sampleMapping(m, [100, 100]).valueMm).toBe(5);
    }
  });

  it("logs the UNCLAMPED conditioning — off-centre perspective exceeds 1", () => {
    // §6 off-centre check: P76, depth 100 mm, NDC (0.75, 0.65). The +Z
    // conditioning there is 0.775404 and the maximum over directions is 1.265406.
    const f = 500 / Math.tan((38 * Math.PI) / 180);
    const ctx: ProjectionContext = {
      viewProj: viewProjAt(perspective(76, 1, 0.1, 10000), [0, 0, 100]),
      viewportWidth: VP,
      viewportHeight: VP,
    };
    const scale = { worldPerPx: worldPerPixelPersp(76, 100, VP) };
    const anchor: Vec3 = [(0.75 * 500 * 100) / f, (0.65 * 500 * 100) / f, 0];
    const m = world(classifyMapping({ q0Mm: 0, point0Mm: anchor, dPointDValue: [0, 0, 1] }, ctx, scale));
    expect(m.g0PxPerMm * scale.worldPerPx).toBeCloseTo(0.775404, 5);
    expect(m.conditioning).toBeCloseTo(0.775404, 5);

    // A direction that conditions ABOVE 1 must report above 1, not be clamped.
    const tilted = world(
      classifyMapping({ q0Mm: 0, point0Mm: anchor, dPointDValue: [0.585964, 0.507836, 0.632456] }, ctx, scale),
    );
    expect(tilted.conditioning).toBeGreaterThan(1);
  });
});

describe("classifyMapping — invariances the absolute floors used to break", () => {
  const path = (): LinearHandlePath => ({ q0Mm: 0, point0Mm: [0, 0, 0], dPointDValue: [0.6, 0, 0.8] });

  /** The whole scene AND camera scaled by λ (derivation §2.6). */
  const scaled = (lambda: number): { ctx: ProjectionContext; scale: AnchorScale } => ({
    ctx: {
      viewProj: viewProjAt(perspective(60, 1, 0.1 * lambda, 10000 * lambda), [0, 0, 100 * lambda]),
      viewportWidth: VP,
      viewportHeight: VP,
    },
    scale: { worldPerPx: worldPerPixelPersp(60, 100 * lambda, VP) },
  });

  it.each([1e-9, 1e-4, 1, 1e4, 1e9])("similarity λ=%p: screen identical, world gains × λ", (lambda) => {
    const base = world(classifyMapping(path(), cameraP60(), scaleP60(100)));
    const { ctx, scale } = scaled(lambda);
    const m = world(classifyMapping(path(), ctx, scale));
    expect(m.direction[0]).toBeCloseTo(base.direction[0], 9);
    expect(m.direction[1]).toBeCloseTo(base.direction[1], 9);
    expect(m.conditioning).toBeCloseTo(base.conditioning, 9);
    expect(m.g0PxPerMm).toBeCloseTo(base.g0PxPerMm / lambda, 6 - Math.log10(1 / lambda));
    expect(m.kPerMm).toBeCloseTo(base.kPerMm / lambda, 6 - Math.log10(1 / lambda));
    // The SCREEN answer is the invariant: 50 px is the same drag in scaled units.
    expect(sampleMapping(m, [50, 0]).valueMm / lambda).toBeCloseTo(8.934710, 5);
  });

  it("unit invariance: the same scene read in inches gives the same screen answer", () => {
    const { ctx, scale } = scaled(1 / 25.4);
    const m = world(classifyMapping(path(), ctx, scale));
    // 7 places: the derivation quotes Δq to 7 significant figures, so dividing
    // it by 25.4 carries its rounding, not the mapping's.
    expect(sampleMapping(m, [50, 0]).valueMm).toBeCloseTo(8.934710 / 25.4, 7);
    expect(m.direction[0]).toBeCloseTo(1, 9);
  });
});

describe("projectAxis — the guards are scale-relative, not absolute", () => {
  /** Perspective scene + camera scaled by λ (the clip w scales with it). */
  const vpAt = (lambda: number): Mat4 =>
    viewProjAt(perspective(60, 1, 0.1 * lambda, 10000 * lambda), [0, 0, 100 * lambda]);

  it("accepts a clip w far below the retired 1e-6 floor", () => {
    // λ = 1e-9 puts the anchor's clip w at 1e-7: well formed, just small.
    const p = projectAxis(vpAt(1e-9), [0, 0, 0], [1, 0, 0], VP, VP);
    expect(p).not.toBeNull();
    expect(p!.direction[0]).toBeCloseTo(1, 9);
  });

  it("accepts a px-per-world far below the retired 1e-9 floor", () => {
    // λ = 1e12 leaves ~8.66e-10 px per world unit — tiny, but exact.
    const p = projectAxis(vpAt(1e12), [0, 0, 0], [1, 0, 0], VP, VP);
    expect(p).not.toBeNull();
    expect(p!.pxPerWorld).toBeLessThan(1e-9);
    expect(p!.direction[0]).toBeCloseTo(1, 9);
  });

  it("still refuses an anchor on or behind the camera plane at any scale", () => {
    for (const lambda of [1e-9, 1, 1e9]) {
      expect(projectAxis(vpAt(lambda), [0, 0, 100 * lambda], [1, 0, 0], VP, VP)).toBeNull();
      expect(projectAxis(vpAt(lambda), [0, 0, 200 * lambda], [1, 0, 0], VP, VP)).toBeNull();
    }
  });

  it("still refuses an exactly end-on axis at any scale", () => {
    for (const lambda of [1e-9, 1, 1e9]) {
      expect(projectAxis(vpAt(lambda), [0, 0, 0], [0, 0, 1], VP, VP)).toBeNull();
    }
  });
});

/*
 * β = 0 (derivation §3). The projected edge tangent is NOT rejected from a
 * handle that MOVES: the point H(q) = E + q·b travels along J·b, and a rejected
 * direction is one it cannot follow. A β = 0.5 blend overstates the scalar
 * response by 1.2×, and full rejection at a 1° screen angle multiplies the gain
 * by 57.2987.
 */
describe("the motion mapping ignores the edge tangent entirely", () => {
  /** §6 "Near-end-on tangent": a = +X, tangent tilted off +Y, azimuth 45°. */
  const tangentAt = (tiltDeg: number): Vec3 => {
    const t = (tiltDeg * Math.PI) / 180;
    const a = Math.PI / 4;
    return [Math.sin(t) * Math.cos(a), Math.cos(t), Math.sin(t) * Math.sin(a)];
  };

  it.each([0.01, 0.5, 2])("a tangent tilted %p° leaves j = (10, 0) and +10 px ⇒ +1 mm", (tiltDeg) => {
    const ctx = cameraO();
    const tangent = tangentAt(tiltDeg);
    // The tangent projects to a real, differently-directed screen axis…
    const along = projectAxis(ctx.viewProj, [0, 0, 0], tangent, VP, VP);
    expect(along).not.toBeNull();
    // …and changes nothing about the outward axis's mapping.
    const m = world(classifyMapping({ q0Mm: 0, point0Mm: [0, 0, 0], dPointDValue: [1, 0, 0] }, ctx, SCALE_O));
    expect(m.g0PxPerMm).toBeCloseTo(10, 9);
    expect(m.direction[0]).toBeCloseTo(1, 12);
    expect(m.direction[1]).toBeCloseTo(0, 12);
    expect(sampleMapping(m, [10, 0]).valueMm).toBeCloseTo(1, 12);
  });
});
