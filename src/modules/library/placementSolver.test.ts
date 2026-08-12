import { describe, it, expect } from "vitest";
import type { ClassifyFrame, ClassifyResult } from "@/ipc/types";
import {
  attachmentAccepts,
  classifySnapKind,
  rotationFromLocalZTo,
  solveCandidatePlacement,
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
