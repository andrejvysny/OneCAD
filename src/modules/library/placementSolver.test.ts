import { describe, it, expect } from "vitest";
import type { ClassifyFrame, ClassifyResult } from "@/ipc/types";
import {
  attachmentAccepts,
  classifySnapKind,
  groundPlanePoint,
  nearestSmallerThread,
  rotationFromLocalZTo,
  solveCandidatePlacement,
  threadNominalDiameterMm,
} from "./placementSolver";

function rotateZ(axis: [number, number, number], angleDeg: number): [number, number, number] {
  // Rodrigues' rotation of world +Z by (axis, angleDeg) — an independent
  // check that `rotationFromLocalZTo` actually produces the requested direction.
  const rad = (angleDeg * Math.PI) / 180;
  const [ux, uy, uz] = axis;
  const len = Math.hypot(ux, uy, uz) || 1;
  const [x, y, z] = [ux / len, uy / len, uz / len];
  const v: [number, number, number] = [0, 0, 1];
  const cosT = Math.cos(rad);
  const sinT = Math.sin(rad);
  const dotUV = x * v[0] + y * v[1] + z * v[2];
  const crossUV: [number, number, number] = [
    y * v[2] - z * v[1],
    z * v[0] - x * v[2],
    x * v[1] - y * v[0],
  ];
  return [
    v[0] * cosT + crossUV[0] * sinT + x * dotUV * (1 - cosT),
    v[1] * cosT + crossUV[1] * sinT + y * dotUV * (1 - cosT),
    v[2] * cosT + crossUV[2] * sinT + z * dotUV * (1 - cosT),
  ];
}

function approxEqual(a: readonly number[], b: readonly number[], eps = 1e-6): boolean {
  return a.every((v, i) => Math.abs(v - b[i]) < eps);
}

describe("rotationFromLocalZTo", () => {
  it("is the identity when the target is already +Z", () => {
    const r = rotationFromLocalZTo([0, 0, 1]);
    expect(r.angleDeg).toBe(0);
  });

  it("is a 180° flip when the target is -Z", () => {
    const r = rotationFromLocalZTo([0, 0, -1]);
    expect(r.angleDeg).toBeCloseTo(180, 6);
    expect(approxEqual(rotateZ(r.axis, r.angleDeg), [0, 0, -1])).toBe(true);
  });

  it("rotates +Z onto an arbitrary direction (round-trip check)", () => {
    const target: [number, number, number] = [1, 1, 1];
    const len = Math.hypot(...target);
    const unit: [number, number, number] = [target[0] / len, target[1] / len, target[2] / len];
    const r = rotationFromLocalZTo(unit);
    expect(approxEqual(rotateZ(r.axis, r.angleDeg), unit)).toBe(true);
  });

  it("rotates +Z onto world X", () => {
    const r = rotationFromLocalZTo([1, 0, 0]);
    expect(approxEqual(rotateZ(r.axis, r.angleDeg), [1, 0, 0])).toBe(true);
  });
});

describe("classifySnapKind", () => {
  const frame: ClassifyFrame = { origin: [0, 0, 0], normal: null, axis: [0, 0, 1], radius: 3 };

  it("maps a cylindrical face to concentric", () => {
    const c: ClassifyResult = { kind: "face", surfaceType: "cylinder", curveType: "", frame };
    expect(classifySnapKind(c)).toBe("concentric");
  });

  it("maps a planar face to coincident", () => {
    const c: ClassifyResult = { kind: "face", surfaceType: "plane", curveType: "", frame };
    expect(classifySnapKind(c)).toBe("coincident");
  });

  it("maps a circular edge to concentricAndCoincident", () => {
    const c: ClassifyResult = { kind: "edge", surfaceType: "", curveType: "circle", frame };
    expect(classifySnapKind(c)).toBe("concentricAndCoincident");
  });

  it("is null with no frame", () => {
    const c: ClassifyResult = { kind: "face", surfaceType: "plane", curveType: "", frame: null };
    expect(classifySnapKind(c)).toBeNull();
  });

  it("is null for an unsupported kind (a line edge)", () => {
    const c: ClassifyResult = { kind: "edge", surfaceType: "", curveType: "line", frame };
    expect(classifySnapKind(c)).toBeNull();
  });
});

describe("attachmentAccepts", () => {
  it("shankAxis-style accepts admits concentric and concentricAndCoincident", () => {
    const accepts = ["cylinder", "hole", "circularEdge"];
    expect(attachmentAccepts(accepts, "concentric")).toBe(true);
    expect(attachmentAccepts(accepts, "concentricAndCoincident")).toBe(true);
    expect(attachmentAccepts(accepts, "coincident")).toBe(false);
  });

  it("headSeat-style accepts admits only coincident", () => {
    expect(attachmentAccepts(["plane"], "coincident")).toBe(true);
    expect(attachmentAccepts(["plane"], "concentric")).toBe(false);
  });
});

describe("solveCandidatePlacement", () => {
  it("coincident: seats at the pick point, local +Z aligned to the target normal", () => {
    const frame: ClassifyFrame = { origin: [5, 5, 5], normal: [0, 0, 1], axis: null, radius: null };
    const placement = solveCandidatePlacement("coincident", frame, [1, 2, 3], false);
    expect(placement.translate).toEqual([1, 2, 3]);
    expect(approxEqual(rotateZ(placement.rotate.axis, placement.rotate.angleDeg), [0, 0, 1])).toBe(
      true,
    );
  });

  it("coincident + flip aligns local +Z to the OPPOSITE of the target normal", () => {
    const frame: ClassifyFrame = { origin: [0, 0, 0], normal: [0, 0, 1], axis: null, radius: null };
    const placement = solveCandidatePlacement("coincident", frame, [0, 0, 0], true);
    expect(approxEqual(rotateZ(placement.rotate.axis, placement.rotate.angleDeg), [0, 0, -1])).toBe(
      true,
    );
  });

  it("concentric: seats on the axis, projecting the pick onto the line", () => {
    // Axis is the world Z line through (0,0,0); a pick off-axis at (3,4,7)
    // must project to (0,0,7) — the nearest point on the line.
    const frame: ClassifyFrame = { origin: [0, 0, 0], normal: null, axis: [0, 0, 1], radius: 3 };
    const placement = solveCandidatePlacement("concentric", frame, [3, 4, 7], false);
    expect(approxEqual(placement.translate, [0, 0, 7])).toBe(true);
    expect(approxEqual(rotateZ(placement.rotate.axis, placement.rotate.angleDeg), [0, 0, 1])).toBe(
      true,
    );
  });

  it("concentricAndCoincident: seats exactly at the frame origin (the rim center), ignoring the pick", () => {
    const frame: ClassifyFrame = { origin: [2, 2, 2], normal: null, axis: [0, 0, 1], radius: 3 };
    const placement = solveCandidatePlacement("concentricAndCoincident", frame, [99, 99, 99], false);
    expect(placement.translate).toEqual([2, 2, 2]);
  });

  it("throws for coincident with no normal (a malformed frame)", () => {
    const frame: ClassifyFrame = { origin: [0, 0, 0], normal: null, axis: null, radius: null };
    expect(() => solveCandidatePlacement("coincident", frame, [0, 0, 0], false)).toThrow();
  });
});

// ── attachment frames (spec §2.1/§5; WP-F1.1) ────────────────────────────────
//
// PARITY DISCIPLINE: these exact cases are mirrored 1:1 in
// `worker/tests/test_component_mate_solver.cpp` — `ComponentMateSolver.cpp` is
// a verbatim port of this file, and the FE ghost and the worker's regen
// re-seat MUST agree or a component jumps the moment it is committed.

/** `p` applied to a component-LOCAL point: `R(axis, angleDeg)·p + translate`. */
function applyPlacement(
  p: ReturnType<typeof solveCandidatePlacement>,
  local: [number, number, number],
): [number, number, number] {
  const d = applyRotation(p, local);
  return [d[0] + p.translate[0], d[1] + p.translate[1], d[2] + p.translate[2]];
}

/** `p`'s rotation applied to a component-local DIRECTION (translation ignored). */
function applyRotation(
  p: ReturnType<typeof solveCandidatePlacement>,
  local: [number, number, number],
): [number, number, number] {
  // Rodrigues again — an independent oracle, never the solver's own matrices.
  const rad = (p.rotate.angleDeg * Math.PI) / 180;
  const [ax, ay, az] = p.rotate.axis;
  const len = Math.hypot(ax, ay, az) || 1;
  const u: [number, number, number] = [ax / len, ay / len, az / len];
  const c = Math.cos(rad);
  const s = Math.sin(rad);
  const dotUV = u[0] * local[0] + u[1] * local[1] + u[2] * local[2];
  const crossUV: [number, number, number] = [
    u[1] * local[2] - u[2] * local[1],
    u[2] * local[0] - u[0] * local[2],
    u[0] * local[1] - u[1] * local[0],
  ];
  return [
    local[0] * c + crossUV[0] * s + u[0] * dotUV * (1 - c),
    local[1] * c + crossUV[1] * s + u[1] * dotUV * (1 - c),
    local[2] * c + crossUV[2] * s + u[2] * dotUV * (1 - c),
  ];
}

describe("solveCandidatePlacement with an attachment frame", () => {
  it("is byte-identical to the un-composed solve when no frame is declared", () => {
    // The compatibility guarantee: a component with no `[attachments].frame`
    // (every seeded package today) keeps its exact former numbers — the
    // absent case does not route through the matrix path at all.
    const frame: ClassifyFrame = { origin: [0, 0, 0], normal: null, axis: [0, 0, 1], radius: 3 };
    const bare = solveCandidatePlacement("concentric", frame, [3, 4, 7], false);
    expect(solveCandidatePlacement("concentric", frame, [3, 4, 7], false, null)).toEqual(bare);
    expect(solveCandidatePlacement("concentric", frame, [3, 4, 7], false, undefined)).toEqual(bare);
  });

  it("offset origin: the ATTACHMENT point lands on the seat, not the model origin", () => {
    // A component whose mate point sits 10mm up its own local +Z, seated on a
    // +Z plane at (1,2,3): the body must sit 10mm BELOW the pick, at (1,2,-7).
    // Before WP-F1.1 it seated at (1,2,3) — the whole defect this closes.
    const frame: ClassifyFrame = { origin: [5, 5, 5], normal: [0, 0, 1], axis: null, radius: null };
    const selfFrame = { origin: [0, 0, 10] as const, z: [0, 0, 1] as const, x: [1, 0, 0] as const };
    const p = solveCandidatePlacement("coincident", frame, [1, 2, 3], false, selfFrame);
    expect(approxEqual(p.translate, [1, 2, -7])).toBe(true);
    expect(p.rotate.angleDeg).toBe(0);
    expect(approxEqual(applyPlacement(p, [0, 0, 10]), [1, 2, 3])).toBe(true);
  });

  it("rotated frame z: the attachment axis aligns to the target axis", () => {
    // The attachment's local +X (its `z`) is the seating axis. Target axis is
    // world +Z through the origin, pick projects to (0,0,4); the attachment
    // origin is 2mm out along local +X, so the body lands at (0,0,2).
    const frame: ClassifyFrame = { origin: [0, 0, 0], normal: null, axis: [0, 0, 1], radius: 3 };
    const selfFrame = { origin: [2, 0, 0] as const, z: [1, 0, 0] as const, x: [0, 0, 1] as const };
    const p = solveCandidatePlacement("concentric", frame, [0, 0, 4], false, selfFrame);
    expect(approxEqual(p.translate, [0, 0, 2])).toBe(true);
    expect(approxEqual(applyPlacement(p, [2, 0, 0]), [0, 0, 4])).toBe(true);
    expect(approxEqual(applyRotation(p, [1, 0, 0]), [0, 0, 1])).toBe(true);
  });

  it("composes a non-degenerate rotation (target normal off every axis)", () => {
    // Neither the seat rotation nor the frame rotation is the identity or a
    // 180° flip here — this is the general arm of `axisAngleFromMat`.
    const frame: ClassifyFrame = { origin: [0, 0, 0], normal: [0, 1, 0], axis: null, radius: null };
    const selfFrame = { origin: [1, 2, 3] as const, z: [0, 0, 1] as const, x: [0, 1, 0] as const };
    const p = solveCandidatePlacement("coincident", frame, [4, 5, 6], false, selfFrame);
    // The two invariants that define the mate: the attachment point is ON the
    // seat, and the attachment's own +Z is ALONG the target normal.
    expect(approxEqual(applyPlacement(p, [1, 2, 3]), [4, 5, 6])).toBe(true);
    expect(approxEqual(applyRotation(p, [0, 0, 1]), [0, 1, 0])).toBe(true);
    // …and the roll is the one the frame asked for, not an arbitrary one.
    expect(approxEqual(applyRotation(p, [0, 1, 0]), [1, 0, 0])).toBe(true);
  });

  it("flip inverts the seating direction with a frame just as it does without one", () => {
    const frame: ClassifyFrame = { origin: [0, 0, 0], normal: [0, 0, 1], axis: null, radius: null };
    const selfFrame = { origin: [0, 0, 10] as const, z: [0, 0, 1] as const, x: [1, 0, 0] as const };
    const p = solveCandidatePlacement("coincident", frame, [0, 0, 0], true, selfFrame);
    expect(approxEqual(applyRotation(p, [0, 0, 1]), [0, 0, -1])).toBe(true);
    // Flipped, the 10mm offset points the other way: the body sits ABOVE.
    expect(approxEqual(p.translate, [0, 0, 10])).toBe(true);
    expect(approxEqual(applyPlacement(p, [0, 0, 10]), [0, 0, 0])).toBe(true);
  });
});

// ── auto-size (spec §5.3's hole row, §5.4 step 3; WP-A3) ────────────────────

describe("threadNominalDiameterMm", () => {
  it("reads the nominal diameter out of a metric designation", () => {
    expect(threadNominalDiameterMm("M6")).toBe(6);
    expect(threadNominalDiameterMm("M2.5")).toBe(2.5);
    expect(threadNominalDiameterMm(" M12 ")).toBe(12);
  });

  it("returns null for anything that is not an M<number> designation", () => {
    // An inch series or a free-text size opts OUT of auto-sizing rather than
    // being guessed at — a wrong guess here places the wrong hardware.
    expect(threadNominalDiameterMm("1/4-20")).toBeNull();
    expect(threadNominalDiameterMm("608")).toBeNull();
    expect(threadNominalDiameterMm("M")).toBeNull();
    expect(threadNominalDiameterMm("")).toBeNull();
  });
});

describe("nearestSmallerThread", () => {
  const DOMAIN = ["M2", "M2.5", "M3", "M4", "M5", "M6", "M8", "M10", "M12"];

  it("picks the largest size that still fits the hole", () => {
    // An M6 clearance hole (6.6) takes an M6, never the M8 that cannot pass.
    expect(nearestSmallerThread(6.6, DOMAIN)).toBe("M6");
    // An M8 clearance hole (9.0) takes an M8.
    expect(nearestSmallerThread(9.0, DOMAIN)).toBe("M8");
    // Exactly nominal still fits (a press fit is the user's business).
    expect(nearestSmallerThread(6.0, DOMAIN)).toBe("M6");
    // Just under nominal drops a size rather than forcing it.
    expect(nearestSmallerThread(5.99, DOMAIN)).toBe("M5");
  });

  it("is not fooled by domain ordering", () => {
    expect(nearestSmallerThread(6.6, ["M12", "M3", "M8", "M6"])).toBe("M6");
  });

  it("returns null when nothing fits, rather than substituting the smallest", () => {
    // The founding invariant in miniature: a 1 mm hole gets no fastener, not
    // the nearest one that would silently be wrong.
    expect(nearestSmallerThread(1.0, DOMAIN)).toBeNull();
  });

  it("returns null for a domain with no metric designations, and for junk input", () => {
    expect(nearestSmallerThread(10, ["608", "6001"])).toBeNull();
    expect(nearestSmallerThread(0, DOMAIN)).toBeNull();
    expect(nearestSmallerThread(Number.NaN, DOMAIN)).toBeNull();
  });
});

describe("groundPlanePoint — the free-space drop point (spec §5.4 steps 1/6)", () => {
  it("meets z = 0 where the ray actually crosses it", () => {
    // Straight down from 100 mm up, offset in x/y: the drop lands under the
    // camera point, at z exactly 0.
    expect(groundPlanePoint([12, -5, 100], [0, 0, -1])).toEqual([12, -5, 0]);
    // A 45° ray from (0,0,10) travels 10 along +x before it lands.
    const p = groundPlanePoint([0, 0, 10], [Math.SQRT1_2, 0, -Math.SQRT1_2]);
    expect(p![0]).toBeCloseTo(10, 9);
    expect(p![1]).toBeCloseTo(0, 9);
    expect(p![2]).toBeCloseTo(0, 9);
  });

  it("is null when the ray cannot reach the plane", () => {
    // Parallel to it — no intersection exists, and inventing one would teleport
    // the ghost somewhere the user never pointed.
    expect(groundPlanePoint([0, 0, 40], [1, 0, 0])).toBeNull();
    // Pointing AWAY from it: the crossing is behind the camera, not in front.
    expect(groundPlanePoint([0, 0, 40], [0, 0, 1])).toBeNull();
  });
});
