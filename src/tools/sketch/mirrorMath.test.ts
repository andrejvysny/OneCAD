/*
 * mirrorMath (PURE): reflection geometry + the mirror-entity matrix. Reflection is
 * checked across a diagonal + an axis-aligned line; the arc case pins the winding
 * swap; the builder cases pin the per-kind constraint column (Line → 2 Symmetric,
 * Circle → Symmetric + Equal, Arc → none, Point → 1 Symmetric).
 */
import { describe, it, expect } from "vitest";
import type { SketchEntity } from "@/ipc/types";
import { reflectPoint, mirrorEntity, mirrorEntities, type MirrorMinters, type XY } from "./mirrorMath";

/** Deterministic minters: e1,e2,… / c1,c2,… so ids in assertions are stable. */
function minters(): MirrorMinters {
  let e = 0;
  let c = 0;
  return { entityId: () => `e${++e}`, constraintId: () => `c${++c}` };
}

const near = (a: XY, b: XY): void => {
  expect(a[0]).toBeCloseTo(b[0], 9);
  expect(a[1]).toBeCloseTo(b[1], 9);
};

describe("reflectPoint", () => {
  it("reflects across the diagonal y = x (swaps coords)", () => {
    near(reflectPoint([2, 0], [0, 0], [1, 1]), [0, 2]);
    near(reflectPoint([3, 1], [0, 0], [1, 1]), [1, 3]);
  });

  it("reflects across the X axis (negates y)", () => {
    near(reflectPoint([3, 5], [0, 0], [1, 0]), [3, -5]);
  });

  it("reflects across a vertical line x = 4", () => {
    near(reflectPoint([1, 7], [4, 0], [4, 9]), [7, 7]);
  });

  it("leaves a point on the axis fixed", () => {
    near(reflectPoint([5, 5], [0, 0], [1, 1]), [5, 5]);
  });

  it("is identity for a degenerate (zero-length) axis", () => {
    near(reflectPoint([2, 3], [1, 1], [1, 1]), [2, 3]);
  });
});

describe("mirrorEntity — Point", () => {
  it("builds a mirrored Point + one Symmetric{Start↔Start, axis}", () => {
    const src: SketchEntity = { id: "p0", type: "Point", p0: [2, 3] };
    const r = mirrorEntity(src, "ax", [0, 0], [1, 0], minters())!; // mirror across X
    expect(r.entity).toMatchObject({ id: "e1", type: "Point" });
    near(r.entity.p0 as XY, [2, -3]);
    expect(r.constraints).toEqual([
      { id: "c1", type: "Symmetric", entities: ["p0", "e1", "ax"], positions: ["Start", "Start"] },
    ]);
  });
});

describe("mirrorEntity — Line", () => {
  it("builds a mirrored Line + two Symmetric (Start↔Start, End↔End)", () => {
    const src: SketchEntity = { id: "l0", type: "Line", p0: [0, 10], p1: [20, 10] };
    const r = mirrorEntity(src, "ax", [0, 0], [100, 0], minters())!; // mirror across X
    expect(r.entity.type).toBe("Line");
    near(r.entity.p0 as XY, [0, -10]);
    near(r.entity.p1 as XY, [20, -10]);
    expect(r.constraints).toEqual([
      { id: "c1", type: "Symmetric", entities: ["l0", "e1", "ax"], positions: ["Start", "Start"] },
      { id: "c2", type: "Symmetric", entities: ["l0", "e1", "ax"], positions: ["End", "End"] },
    ]);
  });
});

describe("mirrorEntity — Circle", () => {
  it("builds a mirrored Circle + Symmetric{Center} + Equal (radius lock)", () => {
    const src: SketchEntity = { id: "ci0", type: "Circle", center: [5, 8], radius: 4 };
    const r = mirrorEntity(src, "ax", [0, 0], [1, 0], minters())!;
    expect(r.entity).toMatchObject({ type: "Circle", radius: 4 });
    near(r.entity.center as XY, [5, -8]);
    expect(r.constraints).toEqual([
      { id: "c1", type: "Symmetric", entities: ["ci0", "e1", "ax"], positions: ["Center", "Center"] },
      { id: "c2", type: "Equal", entities: ["ci0", "e1"] },
    ]);
  });
});

describe("mirrorEntity — Arc (copy only, winding flip)", () => {
  it("swaps start/end so the reflected arc still sweeps CCW; no constraints", () => {
    // Quarter arc CCW from (1,0)[0°] to (0,1)[90°], mirrored across the X axis.
    const src: SketchEntity = {
      id: "a0",
      type: "Arc",
      center: [0, 0],
      radius: 1,
      start: [1, 0],
      end: [0, 1],
    };
    const r = mirrorEntity(src, "ax", [0, 0], [1, 0], minters())!;
    expect(r.entity.type).toBe("Arc");
    near(r.entity.center as XY, [0, 0]);
    expect(r.entity.radius).toBe(1);
    // CCW start of the copy is the reflected OLD end; its end is the reflected OLD start.
    near(r.entity.start as XY, [0, -1]); // reflect(old.end = (0,1))
    near(r.entity.end as XY, [1, 0]); //   reflect(old.start = (1,0))
    expect(r.constraints).toEqual([]);
  });

  it("winding pin: reflected start→end angles are 2φ−endAngle → 2φ−startAngle", () => {
    // Arc CCW from 45° to 135° about origin, mirrored across the vertical line x=0
    // (axis angle φ = 90°). Reflected: start at 2·90−135 = 45°, end at 2·90−45 = 135°.
    const c = Math.SQRT1_2;
    const src: SketchEntity = {
      id: "a0",
      type: "Arc",
      center: [0, 0],
      radius: 1,
      start: [c, c], // 45°
      end: [-c, c], // 135°
    };
    const r = mirrorEntity(src, "ax", [0, 0], [0, 1], minters())!;
    // reflect(end=(-c,c)) across x=0 → (c,c) [45°] is the new start.
    near(r.entity.start as XY, [c, c]);
    // reflect(start=(c,c)) across x=0 → (-c,c) [135°] is the new end.
    near(r.entity.end as XY, [-c, c]);
  });
});

describe("mirrorEntities — aggregation", () => {
  it("mirrors a mixed set, minting fresh ids and concatenating constraints in order", () => {
    const sources: SketchEntity[] = [
      { id: "l0", type: "Line", p0: [0, 1], p1: [2, 1] },
      { id: "ci0", type: "Circle", center: [3, 1], radius: 1 },
    ];
    const { entities, constraints } = mirrorEntities(sources, "ax", [0, 0], [1, 0], minters());
    expect(entities.map((e) => e.id)).toEqual(["e1", "e2"]);
    expect(entities.map((e) => e.type)).toEqual(["Line", "Circle"]);
    // Line → 2 Symmetric, Circle → Symmetric + Equal ⇒ 4 constraints total.
    expect(constraints.map((c) => c.type)).toEqual(["Symmetric", "Symmetric", "Symmetric", "Equal"]);
    expect(constraints.map((c) => c.id)).toEqual(["c1", "c2", "c3", "c4"]);
  });

  it("preserves the construction flag on the copy", () => {
    const sources: SketchEntity[] = [
      { id: "l0", type: "Line", p0: [0, 1], p1: [2, 1], construction: true },
    ];
    const { entities } = mirrorEntities(sources, "ax", [0, 0], [1, 0], minters());
    expect(entities[0].construction).toBe(true);
  });
});

// ── W3 P3: Ellipse — copy only, rotation' = 2φ − rotation ────────────────────

describe("mirrorEntity — Ellipse", () => {
  const src: SketchEntity = {
    id: "el1",
    type: "Ellipse",
    center: [10, 5],
    majorR: 8,
    minorR: 3,
    rotation: Math.PI / 6,
  };

  it("reflects the centre, keeps the radii, and maps rotation θ → 2φ − θ", () => {
    // Axis = the X axis (φ = 0) ⇒ rotation −π/6, folded into [0, 2π).
    const r = mirrorEntity(src, "ax", [0, 0], [1, 0], minters())!;
    expect(r.entity.type).toBe("Ellipse");
    expect(r.entity.center).toEqual([10, -5]);
    expect(r.entity.majorR).toBe(8);
    expect(r.entity.minorR).toBe(3);
    expect(r.entity.rotation).toBeCloseTo(2 * Math.PI - Math.PI / 6, 12);
  });

  it("maps rotation across a 45° axis (φ = π/4 ⇒ θ → π/2 − θ)", () => {
    const r = mirrorEntity(src, "ax", [0, 0], [1, 1], minters())!;
    expect(r.entity.rotation).toBeCloseTo(Math.PI / 2 - Math.PI / 6, 12);
  });

  it("authors NO constraints (no constraint ties two ellipses' rotations)", () => {
    expect(mirrorEntity(src, "ax", [0, 0], [1, 0], minters())!.constraints).toEqual([]);
  });

  it("preserves the construction flag", () => {
    const r = mirrorEntity({ ...src, construction: true }, "ax", [0, 0], [1, 0], minters())!;
    expect(r.entity.construction).toBe(true);
  });

  it("keeps the rotation untouched for a DEGENERATE axis (identity reflection)", () => {
    const r = mirrorEntity(src, "ax", [3, 3], [3, 3], minters())!;
    expect(r.entity.center).toEqual([10, 5]);
    expect(r.entity.rotation).toBeCloseTo(Math.PI / 6, 12);
  });

  it("returns null for an ellipse missing a radius", () => {
    expect(
      mirrorEntity({ id: "bad", type: "Ellipse", center: [0, 0], majorR: 4 }, "ax", [0, 0], [1, 0], minters()),
    ).toBeNull();
  });

  it("mirrors the SAME point set — reflecting the major-axis end lands on the copy", () => {
    const r = mirrorEntity(src, "ax", [0, 0], [1, 0], minters())!;
    const e = r.entity;
    // Source major-axis end, reflected across y = 0.
    const srcEnd: [number, number] = [
      src.center![0] + src.majorR! * Math.cos(src.rotation!),
      src.center![1] + src.majorR! * Math.sin(src.rotation!),
    ];
    const expected = reflectPoint(srcEnd, [0, 0], [1, 0]);
    const copyEnd = [
      e.center![0] + e.majorR! * Math.cos(e.rotation!),
      e.center![1] + e.majorR! * Math.sin(e.rotation!),
    ];
    expect(copyEnd[0]).toBeCloseTo(expected[0], 10);
    expect(copyEnd[1]).toBeCloseTo(expected[1], 10);
  });
});
