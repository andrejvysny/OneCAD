import { describe, it, expect } from "vitest";
import {
  dedupeDimConstraints,
  dimConstraintSpecs,
  resolveDimConstraints,
  type DimConstraintSpec,
} from "./liveDimConstraints";
import { fromWireDimensionValue, toWireDimensionValue } from "@/ipc/angleUnits";
import type { SketchConstraint, SketchEntity } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";

const P = (x: number, y: number): Point2 => ({ x, y });

/** Stand-in committed batch: only ids and types matter to the resolver. */
const batch = (n: number): SketchEntity[] =>
  Array.from({ length: n }, (_, i) => ({ id: `e${i}`, type: "Line" as const }));

function idMinter(): () => string {
  let n = 0;
  return () => `c${n++}`;
}

// ── one case per frame-table row ──────────────────────────────────────────────

describe("dimConstraintSpecs — line", () => {
  it("length pins the committed segment's own endpoints", () => {
    expect(dimConstraintSpecs("line", [P(0, 0)], { length: 50 })).toEqual([
      { type: "Distance", refs: [0, 0], positions: ["Start", "End"], value: 50 },
    ]);
  });

  it("an axis-aligned angle authors Horizontal / Vertical, not Angle", () => {
    expect(dimConstraintSpecs("line", [P(0, 0)], { angle: 0 })).toEqual([
      { type: "Horizontal", refs: [0] },
    ]);
    expect(dimConstraintSpecs("line", [P(0, 0)], { angle: 180 })).toEqual([
      { type: "Horizontal", refs: [0] },
    ]);
    expect(dimConstraintSpecs("line", [P(0, 0)], { angle: 90 })).toEqual([
      { type: "Vertical", refs: [0] },
    ]);
    expect(dimConstraintSpecs("line", [P(0, 0)], { angle: 270 })).toEqual([
      { type: "Vertical", refs: [0] },
    ]);
  });

  it("an oblique angle becomes an Angle RELATIVE to the previous chain segment", () => {
    // Previous segment ran +U (0°); the typed absolute angle is 37°.
    const specs = dimConstraintSpecs("line", [P(0, 0), P(10, 0)], { angle: 37 }, { prevLineId: "prev" });
    expect(specs).toEqual([
      { type: "Angle", refs: [{ id: "prev" }, 0], value: 37 },
    ]);
  });

  it("subtracts the previous segment's own direction", () => {
    // Previous segment ran straight up (90°); typed 37° absolute ⇒ 53° between.
    const specs = dimConstraintSpecs("line", [P(0, 0), P(0, 10)], { angle: 37 }, { prevLineId: "prev" });
    expect(specs).toHaveLength(1);
    expect(specs[0].value).toBeCloseTo(53, 9);
  });

  it("folds an obtuse difference into [0, 180] (matches angleBetweenDeg)", () => {
    const specs = dimConstraintSpecs("line", [P(0, 0), P(10, 0)], { angle: 310 }, { prevLineId: "prev" });
    expect(specs[0].value).toBeCloseTo(50, 9);
  });

  it("authors NOTHING for an oblique FIRST segment — no reference exists", () => {
    expect(dimConstraintSpecs("line", [P(0, 0)], { angle: 37 })).toEqual([]);
    // A prevLineId without a previous anchor pair is equally unusable.
    expect(dimConstraintSpecs("line", [P(0, 0)], { angle: 37 }, { prevLineId: "prev" })).toEqual([]);
  });

  it("authors both fields when both are locked", () => {
    const specs = dimConstraintSpecs("line", [P(0, 0)], { length: 50, angle: 90 });
    expect(specs.map((s) => s.type)).toEqual(["Distance", "Vertical"]);
  });
});

describe("dimConstraintSpecs — line in ARC MODE (SP-4 W3)", () => {
  const arc = { arcMode: true };

  it("a locked radius authors Radius on the committed arc", () => {
    expect(dimConstraintSpecs("line", [P(0, 0), P(10, 0)], { radius: 12 }, arc)).toEqual([
      { type: "Radius", refs: [0], value: 12 },
    ]);
  });

  it("IGNORES a length / angle lock — neither describes the arc that commits", () => {
    expect(dimConstraintSpecs("line", [P(0, 0), P(10, 0)], { length: 50, angle: 90 }, arc)).toEqual([]);
    // Even alongside a radius: the leftovers belong to the straight leg before it.
    expect(
      dimConstraintSpecs("line", [P(0, 0), P(10, 0)], { length: 50, angle: 90, radius: 7 }, arc),
    ).toEqual([{ type: "Radius", refs: [0], value: 7 }]);
  });

  it("nothing locked ⇒ nothing authored", () => {
    expect(dimConstraintSpecs("line", [P(0, 0)], {}, arc)).toEqual([]);
  });

  it("arcMode OFF (the default) leaves the straight-leg ladder exactly as it was", () => {
    const locks = { length: 50, angle: 90 };
    expect(dimConstraintSpecs("line", [P(0, 0)], locks, { arcMode: false })).toEqual(
      dimConstraintSpecs("line", [P(0, 0)], locks),
    );
    // A radius lock is meaningless on a straight leg and authors nothing.
    expect(dimConstraintSpecs("line", [P(0, 0)], { radius: 12 })).toEqual([]);
  });
});

describe("dimConstraintSpecs — rect / centerRect", () => {
  it("W drives the horizontal edge (index 0), H the vertical one (index 1)", () => {
    expect(dimConstraintSpecs("rect", [P(0, 0)], { width: 80, height: 40 })).toEqual([
      { type: "Distance", refs: [0, 0], positions: ["Start", "End"], value: 80 },
      { type: "Distance", refs: [1, 1], positions: ["Start", "End"], value: 40 },
    ]);
  });

  it("centerRect authors the same refs (FULL extents already)", () => {
    expect(dimConstraintSpecs("centerRect", [P(5, 5)], { width: 80 })).toEqual([
      { type: "Distance", refs: [0, 0], positions: ["Start", "End"], value: 80 },
    ]);
  });
});

describe("dimConstraintSpecs — circle", () => {
  it("Ø authors Diameter, R authors Radius", () => {
    expect(dimConstraintSpecs("circle", [P(0, 0)], { diameter: 25 })).toEqual([
      { type: "Diameter", refs: [0], value: 25 },
    ]);
    expect(dimConstraintSpecs("circle", [P(0, 0)], { radius: 12.5 })).toEqual([
      { type: "Radius", refs: [0], value: 12.5 },
    ]);
  });

  it("never authors both halves of the alias", () => {
    const specs = dimConstraintSpecs("circle", [P(0, 0)], { diameter: 25, radius: 12.5 });
    expect(specs).toHaveLength(1);
    expect(specs[0].type).toBe("Diameter");
  });
});

describe("dimConstraintSpecs — arc", () => {
  it("a radius locked in phase 1 authors Radius when phase 2 commits", () => {
    expect(dimConstraintSpecs("arc", [P(0, 0), P(20, 0)], { radius: 20 })).toEqual([
      { type: "Radius", refs: [0], value: 20 },
    ]);
  });

  it("the sweep angle authors nothing — no wire kind expresses it", () => {
    expect(dimConstraintSpecs("arc", [P(0, 0), P(20, 0)], { angle: 90 })).toEqual([]);
  });

  it("authors nothing from the non-committing phase", () => {
    expect(dimConstraintSpecs("arc", [P(0, 0)], { radius: 20 })).toEqual([]);
  });
});

describe("dimConstraintSpecs — arc3p", () => {
  it("the typed radius authors Radius on the committed arc", () => {
    expect(dimConstraintSpecs("arc3p", [P(0, 0), P(20, 0)], { radius: 15 })).toEqual([
      { type: "Radius", refs: [0], value: 15 },
    ]);
  });

  it("the CHORD's length and angle author nothing — a chord is not an entity", () => {
    expect(dimConstraintSpecs("arc3p", [P(0, 0)], { length: 20, angle: 0 })).toEqual([]);
    expect(dimConstraintSpecs("arc3p", [P(0, 0), P(20, 0)], { length: 20, angle: 90 })).toEqual([]);
  });

  it("authors nothing from the non-committing phase", () => {
    expect(dimConstraintSpecs("arc3p", [P(0, 0)], { radius: 15 })).toEqual([]);
  });
});

describe("dimConstraintSpecs — ellipse authors nothing at all", () => {
  it("neither axis has a constraint kind", () => {
    expect(dimConstraintSpecs("ellipse", [P(0, 0), P(40, 0)], { major: 40, minor: 10 })).toEqual([]);
  });
});

describe("dimConstraintSpecs — polygon", () => {
  it("drives the CONSTRUCTION circumcircle at index n", () => {
    expect(dimConstraintSpecs("polygon", [P(0, 0)], { radius: 30 }, { sides: 5 })).toEqual([
      { type: "Radius", refs: [5], value: 30 },
    ]);
  });

  it("drops rather than guesses the index when sides is unknown", () => {
    expect(dimConstraintSpecs("polygon", [P(0, 0)], { radius: 30 })).toEqual([]);
  });

  it("the side count itself authors nothing", () => {
    expect(dimConstraintSpecs("polygon", [P(0, 0)], { sides: 5 }, { sides: 5 })).toEqual([]);
  });
});

describe("dimConstraintSpecs — slot", () => {
  it("L drives wall 0, W drives cap 2's RADIUS (half the width)", () => {
    expect(dimConstraintSpecs("slot", [P(0, 0), P(40, 0)], { length: 40, width: 20 })).toEqual([
      { type: "Distance", refs: [0, 0], positions: ["Start", "End"], value: 40 },
      { type: "Radius", refs: [2], value: 10 },
    ]);
  });

  it("the angle only ever reaches the H/V rungs — a slot has no previous segment", () => {
    expect(dimConstraintSpecs("slot", [P(0, 0), P(0, 40)], { angle: 90 })).toEqual([
      { type: "Vertical", refs: [0] },
    ]);
    expect(dimConstraintSpecs("slot", [P(0, 0), P(40, 0)], { angle: 37 })).toEqual([]);
  });
});

describe("dimConstraintSpecs — no dimensions", () => {
  it("point and unknown tools author nothing", () => {
    expect(dimConstraintSpecs("point", [P(0, 0)], { length: 5 })).toEqual([]);
    expect(dimConstraintSpecs("nope", [P(0, 0)], { length: 5 })).toEqual([]);
  });
});

// ── angle unit seam ───────────────────────────────────────────────────────────

describe("ANGLE UNIT SEAM — this module emits DEGREES, never radians", () => {
  it("round-trips through angleUnits unchanged", () => {
    const specs = dimConstraintSpecs("line", [P(0, 0), P(10, 0)], { angle: 37 }, { prevLineId: "prev" });
    const deg = specs[0].value as number;
    expect(deg).toBeCloseTo(37, 9);
    const wire = toWireDimensionValue("Angle", deg);
    expect(wire).toBeCloseTo((37 * Math.PI) / 180, 12);
    expect(fromWireDimensionValue("Angle", wire)).toBeCloseTo(37, 9);
  });

  it("leaves a length dimension in mm on both sides of the seam", () => {
    const [spec] = dimConstraintSpecs("line", [P(0, 0)], { length: 50 });
    expect(toWireDimensionValue("Distance", spec.value as number)).toBe(50);
  });
});

// ── resolution ────────────────────────────────────────────────────────────────

describe("resolveDimConstraints", () => {
  it("binds numeric refs to the committed batch, in order", () => {
    const specs: DimConstraintSpec[] = [
      { type: "Distance", refs: [0, 0], positions: ["Start", "End"], value: 50 },
      { type: "Radius", refs: [2], value: 10 },
    ];
    expect(resolveDimConstraints(specs, batch(4), idMinter())).toEqual([
      { id: "c0", type: "Distance", entities: ["e0", "e0"], positions: ["Start", "End"], value: 50 },
      { id: "c1", type: "Radius", entities: ["e2"], value: 10 },
    ]);
  });

  it("resolves an { id } ref to an entity OUTSIDE the batch", () => {
    const specs: DimConstraintSpec[] = [{ type: "Angle", refs: [{ id: "prev" }, 0], value: 37 }];
    expect(resolveDimConstraints(specs, batch(1), idMinter())).toEqual([
      { id: "c0", type: "Angle", entities: ["prev", "e0"], value: 37 },
    ]);
  });

  it("DROPS a spec whose index is out of range rather than mis-binding it", () => {
    const specs: DimConstraintSpec[] = [
      { type: "Radius", refs: [9], value: 10 },
      { type: "Horizontal", refs: [0] },
    ];
    const out = resolveDimConstraints(specs, batch(2), idMinter());
    expect(out).toEqual([{ id: "c0", type: "Horizontal", entities: ["e0"] }]);
  });

  it("omits positions when every slot is entity-level, and omits an absent value", () => {
    const out = resolveDimConstraints([{ type: "Horizontal", refs: [0] }], batch(1), idMinter());
    expect(out[0].positions).toBeUndefined();
    expect(out[0].value).toBeUndefined();
  });

  it("mints ids from the caller's factory", () => {
    const out = resolveDimConstraints(
      [{ type: "Horizontal", refs: [0] }, { type: "Vertical", refs: [1] }],
      batch(2),
      idMinter(),
    );
    expect(out.map((c) => c.id)).toEqual(["c0", "c1"]);
  });
});

// ── dedupe ────────────────────────────────────────────────────────────────────

const H = (id: string, entity: string): SketchConstraint => ({
  id,
  type: "Horizontal",
  entities: [entity],
});

describe("dedupeDimConstraints", () => {
  it("drops a typed Horizontal the inference already produced for that line", () => {
    expect(dedupeDimConstraints([H("dim0", "e0")], [H("inf0", "e0")])).toEqual([]);
  });

  it("keeps it when the inference is about a DIFFERENT entity", () => {
    const authored = [H("dim0", "e0")];
    expect(dedupeDimConstraints(authored, [H("inf0", "e1")])).toEqual(authored);
  });

  it("keeps a different KIND on the same entity", () => {
    const authored: SketchConstraint[] = [{ id: "dim0", type: "Vertical", entities: ["e0"] }];
    expect(dedupeDimConstraints(authored, [H("inf0", "e0")])).toEqual(authored);
  });

  it("is order-insensitive across the entity list", () => {
    const authored: SketchConstraint[] = [
      { id: "dim0", type: "Angle", entities: ["a", "b"], value: 37 },
    ];
    const existing: SketchConstraint[] = [{ id: "inf0", type: "Angle", entities: ["b", "a"] }];
    expect(dedupeDimConstraints(authored, existing)).toEqual([]);
  });

  it("passes everything through when nothing collides", () => {
    const authored: SketchConstraint[] = [
      { id: "dim0", type: "Distance", entities: ["e0", "e0"], value: 50 },
      H("dim1", "e0"),
    ];
    expect(dedupeDimConstraints(authored, [])).toEqual(authored);
  });
});
