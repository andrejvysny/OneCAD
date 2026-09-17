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
  axisConditioning,
  axisIsWellConditioned,
  classifyHandleMapping,
  projectRejectedAxis,
  MIN_AXIS_CONDITIONING,
  type HandleMapping,
  type Mat4,
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

describe("axisConditioning", () => {
  const FOV = 76;
  const proj = perspective(FOV, W / H, 0.1, 1000);
  const eyeZ = 20;
  const vp = viewProjAt(proj, [0, 0, eyeZ]);
  const wpp = worldPerPixelPersp(FOV, eyeZ, H);

  it("is 1 for an axis square-on to the screen", () => {
    const p = projectAxis(vp, [0, 0, 0], [1, 0, 0], W, H)!;
    expect(axisConditioning(p, wpp)).toBeCloseTo(1, 6);
    expect(axisIsWellConditioned(p, wpp)).toBe(true);
  });

  it("falls towards 0 as the axis tips towards the camera", () => {
    const shallow = Math.sin((3 * Math.PI) / 180);
    const deep = Math.cos((3 * Math.PI) / 180);
    const p = projectAxis(vp, [0, 0, 0], [shallow, 0, deep], W, H)!;
    const c = axisConditioning(p, wpp);
    expect(c).toBeLessThan(MIN_AXIS_CONDITIONING);
    expect(axisIsWellConditioned(p, wpp)).toBe(false);
  });

  it("sits just above the floor at ~5°, and just below at ~4°", () => {
    const at = (deg: number): number => {
      const r = (deg * Math.PI) / 180;
      const p = projectAxis(vp, [0, 0, 0], [Math.sin(r), 0, Math.cos(r)], W, H)!;
      return axisConditioning(p, wpp);
    };
    expect(at(5)).toBeGreaterThan(MIN_AXIS_CONDITIONING);
    expect(at(4)).toBeLessThan(MIN_AXIS_CONDITIONING);
  });

  it("never exceeds 1, and treats a nonsense scale as unusable rather than huge", () => {
    const p = projectAxis(vp, [0, 0, 0], [1, 0, 0], W, H)!;
    expect(axisConditioning(p, wpp * 10)).toBeLessThanOrEqual(1);
    expect(axisConditioning(p, 0)).toBe(0);
    expect(axisConditioning(p, -1)).toBe(0);
    expect(axisConditioning(p, NaN)).toBe(0);
  });

  it("honours an explicit floor, so the policy can be tuned in ONE place", () => {
    const r = (4 * Math.PI) / 180;
    const p = projectAxis(vp, [0, 0, 0], [Math.sin(r), 0, Math.cos(r)], W, H)!;
    expect(axisIsWellConditioned(p, wpp, 0.5)).toBe(false);
    expect(axisIsWellConditioned(p, wpp, 0.01)).toBe(true);
  });
});

/*
 * R06. An edge op's glyph used the exact projected OUTWARD axis while its drag
 * used a finite-difference screen axis with the projected edge TANGENT rejected,
 * so the two disagreed whenever the tangent was not screen-perpendicular to the
 * outward axis. One exact tangent-rejected projection now feeds both.
 */
describe("projectRejectedAxis", () => {
  // 40 px per world unit on both screen axes, looking down world −Z.
  const vp = viewProjAt(orthographic(10, 7.5, 0.1, 100), [0, 0, 20]);
  /** That camera's world units per CSS px: (top − bottom) / H. */
  const WPP = 15 / H;
  // Perpendicular in 3D, as a fillet's outward axis and its edge tangent are:
  // (1,−1,−1)·(1,0,1) = 0. On screen outward ∝ (10,10) and tangent ∝ (10,0).
  const outward: Vec3 = [1, -1, -1];
  const tangent: Vec3 = [1, 0, 1];

  it("with no tangent is exactly projectAxis", () => {
    const perspVp = viewProjAt(perspective(76, W / H, 0.1, 1000), [0, 0, 20]);
    for (const [m, anchor, axis] of [
      [vp, [0, 0, 0], outward],
      [perspVp, [8, -3, 1], [0.2, 0.4, 1]],
    ] as [Mat4, Vec3, Vec3][]) {
      expect(projectRejectedAxis(m, anchor, axis, null, W, H, 0.025)).toEqual(projectAxis(m, anchor, axis, W, H));
    }
  });

  it("the 45° counterexample: outward (10,10)·k with tangent (10,0)·k rejects to vertical", () => {
    const raw = projectAxis(vp, [0, 0, 0], outward, W, H)!;
    expect(raw.direction[0]).toBeCloseTo(Math.SQRT1_2, 9);
    expect(raw.direction[1]).toBeCloseTo(Math.SQRT1_2, 9);
    const t = projectAxis(vp, [0, 0, 0], tangent, W, H)!;
    expect(t.direction[0]).toBeCloseTo(1, 9);
    expect(t.direction[1]).toBeCloseTo(0, 9);

    const r = projectRejectedAxis(vp, [0, 0, 0], outward, tangent, W, H, WPP)!;
    expect(r.direction[0]).toBeCloseTo(0, 9);
    expect(r.direction[1]).toBeCloseTo(1, 9);
    // Only the tangent-perpendicular part of the motion remains: 40/√3 px.
    expect(r.pxPerWorld).toBeCloseTo(40 / Math.sqrt(3), 9);
    expect(r.derivative[0]).toBeCloseTo(0, 9);
    expect(r.derivative[1]).toBeCloseTo(r.pxPerWorld, 9);
  });

  it("refuses when the tangent runs along the axis on screen — nothing is left to drag", () => {
    expect(projectRejectedAxis(vp, [0, 0, 0], [1, 0, 1], [1, 0, -1], W, H, WPP)).toBeNull();
    expect(projectRejectedAxis(vp, [0, 0, 0], outward, [-2, 2, 5], W, H, WPP)).toBeNull();
  });

  it("does not reject a tangent that has no screen direction of its own", () => {
    // The tangent points straight at the camera: no usable derivative.
    expect(projectRejectedAxis(vp, [0, 0, 0], outward, [0, 0, 1], W, H, WPP)).toEqual(
      projectAxis(vp, [0, 0, 0], outward, W, H),
    );
  });

  it("refuses whenever projectAxis refuses the axis", () => {
    expect(projectRejectedAxis(vp, [0, 0, 0], [0, 0, 1], tangent, W, H, WPP)).toBeNull();
    expect(projectRejectedAxis(vp, [NaN, 0, 0], outward, tangent, W, H, WPP)).toBeNull();
  });
});

/** Column-major `a · b`. */
function mul(a: Mat4, b: Mat4): Mat4 {
  const out = new Array<number>(16).fill(0);
  for (let c = 0; c < 4; c++) {
    for (let r = 0; r < 4; r++) {
      let sum = 0;
      for (let k = 0; k < 4; k++) sum += a[r + 4 * k] * b[k + 4 * c];
      out[r + 4 * c] = sum;
    }
  }
  return out;
}

/*
 * H4b W2 (orchestrator decision): the tangent is rejected only when the TANGENT
 * itself is well conditioned on screen. A nearly end-on edge projects to a
 * vanishing, round-off-dominated direction; rejecting it in full rotated a
 * square-on outward axis 45° with ×1.414 gain, flipping with the tilt azimuth.
 * Continuity across the threshold is deliberately NOT asserted here.
 */
describe("projectRejectedAxis — the tangent's own conditioning gates the rejection", () => {
  // Orthographic, 40 px per world unit, looking along world +Y with +Z up.
  const view: Mat4 = [1, 0, 0, 0, 0, 0, -1, 0, 0, 1, 0, 0, 0, 0, -50, 1];
  const vp = mul(orthographic(10, 7.5, 0.1, 100), view);
  const wpp = 15 / H;
  const edge = (tiltDeg: number, azimuthDeg: number): { tangent: Vec3; outward: Vec3 } => {
    const t = (tiltDeg * Math.PI) / 180;
    const a = (azimuthDeg * Math.PI) / 180;
    const ox = Math.cos(t);
    const oy = -Math.sin(t) * Math.cos(a);
    const n = Math.hypot(ox, oy);
    return {
      tangent: [Math.sin(t) * Math.cos(a), Math.cos(t), Math.sin(t) * Math.sin(a)],
      outward: [ox / n, oy / n, 0], // square-on, perpendicular to the tangent
    };
  };

  for (const tiltDeg of [0.01, 0.5]) {
    for (const azimuthDeg of [0, 45]) {
      it(`a nearly end-on tangent (tilt ${tiltDeg}°, azimuth ${azimuthDeg}°) rejects nothing`, () => {
        const { tangent, outward } = edge(tiltDeg, azimuthDeg);
        const along = projectAxis(vp, [0, 0, 0], tangent, W, H);
        expect(along === null || axisConditioning(along, wpp) < MIN_AXIS_CONDITIONING).toBe(true);
        expect(projectRejectedAxis(vp, [0, 0, 0], outward, tangent, W, H, wpp)).toEqual(
          projectAxis(vp, [0, 0, 0], outward, W, H),
        );
      });
    }
  }

  it("a well-conditioned tangent is still rejected", () => {
    const { tangent, outward } = edge(30, 45);
    const along = projectAxis(vp, [0, 0, 0], tangent, W, H)!;
    expect(axisConditioning(along, wpp)).toBeGreaterThanOrEqual(MIN_AXIS_CONDITIONING);
    const raw = projectAxis(vp, [0, 0, 0], outward, W, H)!;
    const rejected = projectRejectedAxis(vp, [0, 0, 0], outward, tangent, W, H, wpp)!;
    // Nothing of the result runs along the tangent's screen direction any more…
    const dot = rejected.direction[0] * along.direction[0] + rejected.direction[1] * along.direction[1];
    expect(dot).toBeCloseTo(0, 9);
    // …which the raw outward axis did.
    expect(Math.abs(raw.direction[0] * along.direction[0] + raw.direction[1] * along.direction[1])).toBeGreaterThan(0.1);
  });

  it("an unusable scale rejects nothing", () => {
    const { tangent, outward } = edge(30, 45);
    expect(projectRejectedAxis(vp, [0, 0, 0], outward, tangent, W, H, 0)).toEqual(
      projectAxis(vp, [0, 0, 0], outward, W, H),
    );
  });
});

describe("classifyHandleMapping", () => {
  const FOV = 76;
  const eyeZ = 20;
  const perspVp = viewProjAt(perspective(FOV, W / H, 0.1, 1000), [0, 0, eyeZ]);
  const perspWpp = worldPerPixelPersp(FOV, eyeZ, H);
  const orthoVp = viewProjAt(orthographic(10, 7.5, 0.1, 100), [0, 0, 20]);
  const orthoWpp = 1 / 40;
  const tilted = (deg: number): Vec3 => {
    const r = (deg * Math.PI) / 180;
    return [Math.sin(r), 0, Math.cos(r)];
  };
  const finiteMapping = (m: HandleMapping): boolean =>
    Number.isFinite(m.direction[0]) &&
    Number.isFinite(m.direction[1]) &&
    Number.isFinite(m.worldPerPx) &&
    (m.pxPerWorld === null || Number.isFinite(m.pxPerWorld));

  it("a well-conditioned axis maps along its projected direction", () => {
    const p = projectAxis(perspVp, [0, 0, 0], [1, 0, 0], W, H)!;
    const m = classifyHandleMapping(p, perspWpp);
    expect(m.strategy).toBe("axis");
    expect(m.direction).toEqual(p.direction);
    expect(m.pxPerWorld).toBe(p.pxPerWorld);
    expect(m.worldPerPx).toBe(perspWpp);
  });

  it("an end-on axis is a vertical screen proxy, orthographic and perspective alike", () => {
    for (const [vp, wpp] of [
      [orthoVp, orthoWpp],
      [perspVp, perspWpp],
    ] as [Mat4, number][]) {
      const m = classifyHandleMapping(projectAxis(vp, [0, 0, 0], [0, 0, 1], W, H), wpp);
      expect(m).toEqual({ strategy: "screenProxy", direction: [0, -1], pxPerWorld: null, worldPerPx: wpp });
    }
  });

  it("switches exactly at MIN_AXIS_CONDITIONING", () => {
    const below = (Math.asin(MIN_AXIS_CONDITIONING) * 180) / Math.PI - 0.05;
    const above = (Math.asin(MIN_AXIS_CONDITIONING) * 180) / Math.PI + 0.05;
    for (const [vp, wpp] of [
      [orthoVp, orthoWpp],
      [perspVp, perspWpp],
    ] as [Mat4, number][]) {
      const lo = projectAxis(vp, [0, 0, 0], tilted(below), W, H)!;
      const hi = projectAxis(vp, [0, 0, 0], tilted(above), W, H)!;
      expect(axisConditioning(lo, wpp)).toBeLessThan(MIN_AXIS_CONDITIONING);
      expect(axisConditioning(hi, wpp)).toBeGreaterThan(MIN_AXIS_CONDITIONING);
      expect(classifyHandleMapping(lo, wpp).strategy).toBe("screenProxy");
      expect(classifyHandleMapping(hi, wpp).strategy).toBe("axis");
    }
  });

  it("an unusable scale is a proxy, and every output stays finite", () => {
    const p = projectAxis(perspVp, [0, 0, 0], [1, 0, 0], W, H)!;
    for (const wpp of [0, -1, NaN, Infinity]) {
      const m = classifyHandleMapping(p, wpp);
      expect(m.strategy).toBe("screenProxy");
      expect(m.worldPerPx).toBe(0);
      expect(finiteMapping(m)).toBe(true);
    }
    expect(finiteMapping(classifyHandleMapping(null, perspWpp))).toBe(true);
    expect(finiteMapping(classifyHandleMapping(p, perspWpp))).toBe(true);
  });
});
