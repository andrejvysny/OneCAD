import { describe, it, expect } from "vitest";
import {
  AUTO_KINDS_LEGACY,
  inferHV,
  inferConstraints,
  lineAngle,
  angleBetweenLines,
  inferPerpendicularPartner,
  inferParallelPartner,
  inferTangentPartner,
  inferConcentricPartner,
  inferEqualRadiusPartner,
  HV_TOLERANCE_RAD,
  PERPENDICULAR_TOLERANCE_RAD,
  PARALLEL_TOLERANCE_RAD,
  TANGENT_TOLERANCE_RAD,
  CONCENTRIC_TOLERANCE,
  EQUAL_RADIUS_TOLERANCE,
  entityPoints,
} from "./autoConstrain";
import type { SketchEntity } from "@/ipc/types";

const deg = (d: number): [number, number] => [Math.cos((d * Math.PI) / 180) * 40, Math.sin((d * Math.PI) / 180) * 40];

describe("inferHV — ±5° horizontal / vertical", () => {
  it("exact horizontal", () => expect(inferHV([0, 0], [40, 0])).toBe("Horizontal"));
  it("exact vertical", () => expect(inferHV([0, 0], [0, 40])).toBe("Vertical"));
  it("3° from horizontal ⇒ Horizontal (within tolerance)", () =>
    expect(inferHV([0, 0], deg(3))).toBe("Horizontal"));
  it("87° ⇒ Vertical (within tolerance of 90°)", () => expect(inferHV([0, 0], deg(87))).toBe("Vertical"));
  it("10° ⇒ neither", () => expect(inferHV([0, 0], deg(10))).toBeNull());
  it("horizontal pointing −X still Horizontal", () => expect(inferHV([0, 0], [-40, 0])).toBe("Horizontal"));
  it("zero-length ⇒ null", () => expect(inferHV([1, 1], [1, 1])).toBeNull());
  it("HV_TOLERANCE_RAD is 5°", () => expect(HV_TOLERANCE_RAD).toBeCloseTo((5 * Math.PI) / 180));
  it("lineAngle basics", () => expect(lineAngle([0, 0], [0, 5])).toBeCloseTo(Math.PI / 2));
});

describe("inferConstraints", () => {
  let n = 0;
  const opts = () => {
    n = 0;
    return { nextConstraintId: () => `c${++n}`, kinds: AUTO_KINDS_LEGACY };
  };

  it("adds Horizontal for a horizontal new line", () => {
    const line: SketchEntity = { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] };
    const cs = inferConstraints([line], [], opts());
    expect(cs).toContainEqual({ id: "c1", type: "Horizontal", entities: ["e1"] });
  });

  it("adds Coincident when a new endpoint lands on an existing endpoint", () => {
    const existing: SketchEntity = { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] };
    const next: SketchEntity = { id: "e2", type: "Line", p0: [40, 0], p1: [40, 20] };
    const cs = inferConstraints([next], [existing], opts());
    const coincident = cs.find((c) => c.type === "Coincident");
    expect(coincident).toBeDefined();
    expect(coincident!.entities).toEqual(["e2", "e1"]);
    expect(coincident!.positions).toEqual(["Start", "End"]);
    // Also Vertical for the vertical new line.
    expect(cs.some((c) => c.type === "Vertical")).toBe(true);
  });

  it("a rectangle's four lines infer 4 coincident + 2 H + 2 V", () => {
    const rect: SketchEntity[] = [
      { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] },
      { id: "e2", type: "Line", p0: [40, 0], p1: [40, 20] },
      { id: "e3", type: "Line", p0: [40, 20], p1: [0, 20] },
      { id: "e4", type: "Line", p0: [0, 20], p1: [0, 0] },
    ];
    const cs = inferConstraints(rect, [], opts());
    expect(cs.filter((c) => c.type === "Coincident")).toHaveLength(4);
    expect(cs.filter((c) => c.type === "Horizontal")).toHaveLength(2);
    expect(cs.filter((c) => c.type === "Vertical")).toHaveLength(2);
  });

  it("does not infer coincidence for a far endpoint (tight tolerance)", () => {
    const existing: SketchEntity = { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] };
    const next: SketchEntity = { id: "e2", type: "Line", p0: [41, 1], p1: [41, 20] };
    const cs = inferConstraints([next], [existing], opts());
    expect(cs.some((c) => c.type === "Coincident")).toBe(false);
  });
});

// ── M6c parity: perpendicular / parallel / tangent ────────────────────────────

describe("angleBetweenLines (folded to [0, π/2])", () => {
  it("parallel lines ⇒ 0", () => {
    expect(angleBetweenLines([0, 0], [10, 0], [0, 5], [10, 5])).toBeCloseTo(0);
  });
  it("perpendicular lines ⇒ π/2", () => {
    expect(angleBetweenLines([0, 0], [10, 0], [0, 0], [0, 10])).toBeCloseTo(Math.PI / 2);
  });
  it("45° lines ⇒ π/4", () => {
    expect(angleBetweenLines([0, 0], [10, 0], [0, 0], [10, 10])).toBeCloseTo(Math.PI / 4);
  });
  it("folds obtuse crossings back below π/2", () => {
    // 150° between directions folds to 30°.
    expect(angleBetweenLines([0, 0], [10, 0], [0, 0], [-10, 10 * Math.tan((30 * Math.PI) / 180)])).toBeCloseTo(
      (30 * Math.PI) / 180,
    );
  });
});

describe("tolerances mirror AutoConstrainer.h (all 5°)", () => {
  it("perpendicular / parallel / tangent are all 5°", () => {
    const fiveDeg = (5 * Math.PI) / 180;
    expect(PERPENDICULAR_TOLERANCE_RAD).toBeCloseTo(fiveDeg);
    expect(PARALLEL_TOLERANCE_RAD).toBeCloseTo(fiveDeg);
    expect(TANGENT_TOLERANCE_RAD).toBeCloseTo(fiveDeg);
    expect(HV_TOLERANCE_RAD).toBeCloseTo(fiveDeg);
  });
  it("CONCENTRIC_TOLERANCE is 2.0mm (coincidenceTolerance, AutoConstrainer.h:61)", () => {
    expect(CONCENTRIC_TOLERANCE).toBe(2.0);
  });
  it("EQUAL_RADIUS_TOLERANCE is 0.5mm (RADIUS_TOLERANCE, AutoConstrainer.cpp:699)", () => {
    expect(EQUAL_RADIUS_TOLERANCE).toBe(0.5);
  });
});

describe("inferPerpendicularPartner", () => {
  const refs = [{ id: "r1", p0: [0, 0] as [number, number], p1: [40, 40] as [number, number] }]; // 45°
  it("matches a line meeting at 90±5°", () => {
    // 135° line is perpendicular to the 45° reference.
    expect(inferPerpendicularPartner([40, 40], [0, 80], refs)).toBe("r1");
  });
  it("rejects a line meeting well off 90°", () => {
    // Parallel (45°) is 0° apart, not perpendicular.
    expect(inferPerpendicularPartner([0, 10], [40, 50], refs)).toBeNull();
  });
  it("accepts within 5° of 90° but not beyond", () => {
    const at = (deg: number): [number, number] => [40 * Math.cos((deg * Math.PI) / 180), 40 * Math.sin((deg * Math.PI) / 180)];
    expect(inferPerpendicularPartner([0, 0], at(45 + 87), refs)).toBe("r1"); // 87° ⇒ within
    expect(inferPerpendicularPartner([0, 0], at(45 + 80), refs)).toBeNull(); // 80° ⇒ beyond
  });
});

describe("inferParallelPartner", () => {
  const refs = [{ id: "r1", p0: [0, 0] as [number, number], p1: [40, 40] as [number, number] }]; // 45°
  it("matches a line within ±5° of the reference direction", () => {
    expect(inferParallelPartner([0, 20], [40, 60], refs)).toBe("r1"); // exactly parallel
  });
  it("rejects a perpendicular line", () => {
    expect(inferParallelPartner([40, 40], [0, 80], refs)).toBeNull();
  });
});

describe("inferTangentPartner", () => {
  // Line along +X; its END is at (10,0).
  const refs = [{ id: "L", p0: [0, 0] as [number, number], p1: [10, 0] as [number, number] }];
  it("matches an arc starting tangent at the line endpoint", () => {
    // center above the endpoint ⇒ radial along −Y ⇒ start tangent along +X (== line).
    expect(inferTangentPartner([10, 10], [10, 0], refs)).toBe("L");
  });
  it("rejects when the arc starts tangent-normal to the line", () => {
    // center to the side ⇒ start tangent along ±Y ⇒ not aligned with the +X line.
    expect(inferTangentPartner([20, 0], [10, 0], refs)).toBeNull();
  });
  it("rejects when the arc start is not on any line endpoint", () => {
    expect(inferTangentPartner([50, 60], [50, 50], refs)).toBeNull();
  });
});

describe("inferConcentricPartner", () => {
  it("exact hit (distance 0)", () => {
    const refs = [{ id: "r1", center: [0, 0] as [number, number], radius: 5 }];
    expect(inferConcentricPartner([0, 0], refs)).toBe("r1");
  });
  it("1.9mm hit (within CONCENTRIC_TOLERANCE)", () => {
    const refs = [{ id: "r1", center: [1.9, 0] as [number, number], radius: 5 }];
    expect(inferConcentricPartner([0, 0], refs)).toBe("r1");
  });
  it("2.0mm boundary MISS (strict <, verbatim AutoConstrainer::inferConcentric seeding)", () => {
    const refs = [{ id: "r1", center: [2, 0] as [number, number], radius: 5 }];
    expect(inferConcentricPartner([0, 0], refs)).toBeNull();
  });
  it("nearest-of-two wins", () => {
    const refs = [
      { id: "far", center: [1.8, 0] as [number, number], radius: 5 },
      { id: "near", center: [0.5, 0] as [number, number], radius: 5 },
    ];
    expect(inferConcentricPartner([0, 0], refs)).toBe("near");
  });
  it("arc↔circle cross-type — the ref set carries no entity kind, only center/radius", () => {
    const refs = [{ id: "existingArc", center: [5, 5] as [number, number], radius: 12 }];
    expect(inferConcentricPartner([5, 5], refs)).toBe("existingArc");
  });
  it("ignores Line/Point references — only Circle/Arc feed the concentric ref set", () => {
    const existingLine: SketchEntity = { id: "l1", type: "Line", p0: [10, 10], p1: [40, 10] };
    const existingPoint: SketchEntity = { id: "p1", type: "Point", p0: [10, 10] };
    const circle: SketchEntity = { id: "c1", type: "Circle", center: [10, 10], radius: 5 };
    const cs = inferConstraints([circle], [existingLine, existingPoint], { nextConstraintId: ids(), kinds: AUTO_KINDS_LEGACY });
    // The circle's center lands exactly on the line's Start and on the Point ⇒ Coincident,
    // but neither is a Circle/Arc so there is no concentric partner to match against.
    expect(cs.some((c) => c.type === "Concentric")).toBe(false);
    expect(cs.some((c) => c.type === "Coincident")).toBe(true);
  });
});

describe("inferEqualRadiusPartner", () => {
  it("exact hit (Δr = 0)", () => {
    const refs = [{ id: "r1", center: [0, 0] as [number, number], radius: 5 }];
    expect(inferEqualRadiusPartner(5, refs)).toBe("r1");
  });
  it("0.4mm hit (within EQUAL_RADIUS_TOLERANCE)", () => {
    const refs = [{ id: "r1", center: [0, 0] as [number, number], radius: 5.4 }];
    expect(inferEqualRadiusPartner(5, refs)).toBe("r1");
  });
  it("0.5mm boundary MISS (strict <, verbatim AutoConstrainer::inferEqualRadius seeding)", () => {
    const refs = [{ id: "r1", center: [0, 0] as [number, number], radius: 5.5 }];
    expect(inferEqualRadiusPartner(5, refs)).toBeNull();
  });
  it("nearest-diff wins", () => {
    const refs = [
      { id: "far", center: [0, 0] as [number, number], radius: 5.3 },
      { id: "near", center: [0, 0] as [number, number], radius: 5.1 },
    ];
    expect(inferEqualRadiusPartner(5, refs)).toBe("near");
  });
  it("arc↔circle cross-type — the ref set carries no entity kind, only center/radius", () => {
    const refs = [{ id: "arcRef", center: [0, 0] as [number, number], radius: 5.2 }];
    expect(inferEqualRadiusPartner(5, refs)).toBe("arcRef");
  });
});

describe("inferConstraints — perpendicular / parallel / tangent + precedence", () => {
  let n = 0;
  const opts = () => {
    n = 0;
    return { nextConstraintId: () => `c${++n}`, kinds: AUTO_KINDS_LEGACY };
  };

  it("infers Perpendicular for two non-axis lines meeting at 90°", () => {
    const l1: SketchEntity = { id: "e1", type: "Line", p0: [0, 0], p1: [40, 40] }; // 45°
    const l2: SketchEntity = { id: "e2", type: "Line", p0: [40, 40], p1: [0, 80] }; // 135°
    const cs = inferConstraints([l2], [l1], opts());
    const perp = cs.find((c) => c.type === "Perpendicular");
    expect(perp).toBeDefined();
    expect(perp!.entities).toEqual(["e2", "e1"]);
  });

  it("infers Parallel for two non-axis parallel lines", () => {
    const l1: SketchEntity = { id: "e1", type: "Line", p0: [0, 0], p1: [40, 40] }; // 45°
    const l2: SketchEntity = { id: "e2", type: "Line", p0: [0, 20], p1: [40, 60] }; // 45°, offset
    const cs = inferConstraints([l2], [l1], opts());
    const par = cs.find((c) => c.type === "Parallel");
    expect(par).toBeDefined();
    expect(par!.entities).toEqual(["e2", "e1"]);
  });

  it("H/V wins over Parallel (a horizontal line parallel to a horizontal ref gets only Horizontal)", () => {
    const l1: SketchEntity = { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }; // horizontal
    const l2: SketchEntity = { id: "e2", type: "Line", p0: [0, 10], p1: [40, 10] }; // horizontal, parallel
    const cs = inferConstraints([l2], [l1], opts());
    expect(cs.some((c) => c.type === "Horizontal" && c.entities[0] === "e2")).toBe(true);
    expect(cs.some((c) => c.type === "Parallel")).toBe(false);
    expect(cs.some((c) => c.type === "Perpendicular")).toBe(false);
  });

  // ── the WELD GATE (SP-4 W3) ────────────────────────────────────────────────
  //
  // An entity-level Tangent(line, arc) is `distance(arcCenter, line) == radius`,
  // which is REDUNDANT once the two are welded endpoint-to-endpoint — PlaneGCS
  // reports the pair as OverConstrained (worker ctest
  // test_endpoint_weld_makes_entity_tangent_redundant; same reason slotTool
  // dropped four Tangents and filletSketchCorner authors none). Our tools SNAP
  // endpoints exactly, so this fires on the ordinary tangent-arc gesture.

  it("a batch that WELDS the arc to the line emits Coincident and NO Tangent", () => {
    const line: SketchEntity = { id: "L", type: "Line", p0: [0, 0], p1: [10, 0] };
    // Arc START is exactly the line END ⇒ Coincident ⇒ the Tangent is suppressed.
    const arc: SketchEntity = { id: "A", type: "Arc", center: [10, 10], radius: 10, start: [10, 0], end: [0, 10] };
    const cs = inferConstraints([arc], [line], opts());
    expect(cs.some((c) => c.type === "Coincident")).toBe(true);
    expect(cs.some((c) => c.type === "Tangent")).toBe(false);
  });

  it("a NEAR-but-not-welded arc still gets its Tangent (the gate is not a mute)", () => {
    // 0.001mm apart: past the 1e-6 Coincident snap tolerance, well inside the 2mm
    // TANGENT_COINCIDENCE_TOL — so no weld exists to make the Tangent redundant.
    const line: SketchEntity = { id: "L", type: "Line", p0: [0, 0], p1: [10, 0.001] };
    const arc: SketchEntity = { id: "A", type: "Arc", center: [10, 10], radius: 10, start: [10, 0], end: [0, 10] };
    const cs = inferConstraints([arc], [line], opts());
    expect(cs.some((c) => c.type === "Coincident")).toBe(false);
    const tan = cs.find((c) => c.type === "Tangent");
    expect(tan).toBeDefined();
    expect(tan!.entities).toEqual(["A", "L"]);
  });

  it("ORDER PIN — a non-welded arc emits Coincident(s) first, then Tangent", () => {
    // Arc START is 0.001mm off L's end (Tangent, no weld); its END is exactly on
    // M's start (a weld — but to a DIFFERENT line, so it gates nothing).
    const l: SketchEntity = { id: "L", type: "Line", p0: [0, 0], p1: [10, 0.001] };
    const m: SketchEntity = { id: "M", type: "Line", p0: [0, 10], p1: [-10, 10] };
    const arc: SketchEntity = { id: "A", type: "Arc", center: [10, 10], radius: 10, start: [10, 0], end: [0, 10] };
    const cs = inferConstraints([arc], [l, m], opts());
    expect(cs.map((c) => c.type)).toEqual(["Coincident", "Tangent"]);
    expect(cs[0].entities).toEqual(["A", "M"]);
    expect(cs[1].entities).toEqual(["A", "L"]);
  });
});

// ── W3 P3: an Ellipse contributes its CENTRE (and nothing else) ──────────────

/** Deterministic constraint-id minter (mirrors the local helpers above). */
function ids(): () => string {
  let n = 0;
  return () => `c${++n}`;
}

describe("entityPoints / inferConstraints — Ellipse", () => {
  const ell: SketchEntity = {
    id: "el1",
    type: "Ellipse",
    center: [10, 0],
    majorR: 8,
    minorR: 3,
    rotation: 0.5,
  };

  it("exposes exactly one constrainable point: its Center", () => {
    expect(entityPoints(ell)).toEqual([{ entityId: "el1", position: "Center", coord: [10, 0] }]);
  });

  it("auto-coincides its centre with an existing endpoint it was snapped to", () => {
    const existing: SketchEntity[] = [{ id: "l1", type: "Line", p0: [0, 0], p1: [10, 0] }];
    const out = inferConstraints([ell], existing, { nextConstraintId: ids(), kinds: AUTO_KINDS_LEGACY });
    expect(out).toEqual([
      {
        id: "c1",
        type: "Coincident",
        entities: ["el1", "l1"],
        positions: ["Center", "End"],
      },
    ]);
  });

  it("infers nothing from the ellipse CURVE (no H/V, no Tangent, no Parallel)", () => {
    const existing: SketchEntity[] = [{ id: "l1", type: "Line", p0: [0, 50], p1: [40, 50] }];
    expect(inferConstraints([ell], existing, { nextConstraintId: ids(), kinds: AUTO_KINDS_LEGACY })).toEqual([]);
  });
});

// ── Locked reference geometry as an auto-constrain TARGET (SKETCH-ON-FACE W2) ─
//
// PINS CURRENT BEHAVIOUR, deliberately: `inferConstraints` does NOT filter
// `referenceLocked` entities out of its reference set, and that is the intended
// design — a projected host-face boundary is exactly what a user snaps a new
// profile to, and a constraint whose other end is Fixed-pinned simply drives the
// USER's geometry (which is the point of pinning). Adding a lock filter here
// would silently drop the auto-constraints that make sketch-on-face usable, so
// this file is the tripwire if anyone tries.

describe("inferConstraints — locked reference geometry participates", () => {
  let n = 0;
  const opts = () => {
    n = 0;
    return { nextConstraintId: () => `c${++n}`, kinds: AUTO_KINDS_LEGACY };
  };

  /** A projected boundary segment: locked, but geometrically ordinary. */
  const refLine: SketchEntity = {
    id: "ref1",
    type: "Line",
    p0: [0, 0],
    p1: [40, 0],
    referenceLocked: true,
  };

  it("emits Coincident when a new endpoint lands on a LOCKED endpoint", () => {
    const next: SketchEntity = { id: "e2", type: "Line", p0: [40, 0], p1: [40, 20] };
    const cs = inferConstraints([next], [refLine], opts());
    const coincident = cs.find((c) => c.type === "Coincident");
    expect(coincident).toBeDefined();
    expect(coincident!.entities).toEqual(["e2", "ref1"]);
    expect(coincident!.positions).toEqual(["Start", "End"]);
  });

  it("emits Perpendicular / Parallel against a LOCKED reference line", () => {
    // H/V wins over both, so use off-axis geometry (mirrors the tests above).
    const refDiag: SketchEntity = { id: "ref2", type: "Line", p0: [0, 0], p1: [40, 40], referenceLocked: true };

    const perp: SketchEntity = { id: "e2", type: "Line", p0: [40, 40], p1: [0, 80] };
    expect(inferConstraints([perp], [refDiag], opts()).find((c) => c.type === "Perpendicular")).toMatchObject({
      entities: ["e2", "ref2"],
    });

    const par: SketchEntity = { id: "e3", type: "Line", p0: [0, 20], p1: [40, 60] };
    expect(inferConstraints([par], [refDiag], opts()).find((c) => c.type === "Parallel")).toMatchObject({
      entities: ["e3", "ref2"],
    });
  });

  it("the locked entity is never itself given an H/V constraint (it is not NEW)", () => {
    // Only `newEntities` get orientation constraints; the reference set is read-only.
    const next: SketchEntity = { id: "e2", type: "Line", p0: [0, 20], p1: [40, 20] };
    const cs = inferConstraints([next], [refLine], opts());
    expect(cs.every((c) => !c.entities.includes("ref1") || c.type === "Coincident")).toBe(true);
    expect(cs.some((c) => c.type === "Horizontal" && c.entities[0] === "e2")).toBe(true);
  });
});

// ── Concentric / Equal auto-constraint port (AutoConstrainer.cpp:645-742) ────

describe("inferConstraints — Concentric / Equal", () => {
  it("circle-over-circle emits Concentric then Equal", () => {
    const existing: SketchEntity = { id: "e1", type: "Circle", center: [0, 0], radius: 10 };
    const next: SketchEntity = { id: "e2", type: "Circle", center: [1, 1], radius: 10.2 }; // dist √2mm, Δr 0.2mm
    const cs = inferConstraints([next], [existing], { nextConstraintId: ids(), kinds: AUTO_KINDS_LEGACY });
    const concentric = cs.find((c) => c.type === "Concentric");
    const equal = cs.find((c) => c.type === "Equal");
    expect(concentric).toMatchObject({ entities: ["e2", "e1"] });
    expect(equal).toMatchObject({ entities: ["e2", "e1"] });
    expect(cs.indexOf(concentric!)).toBeLessThan(cs.indexOf(equal!));
    expect(cs.some((c) => c.type === "Coincident")).toBe(false); // not exactly-snapped, no gate involved
  });

  it("arc path: Tangent, then Concentric, then Equal", () => {
    // The line END is 0.001mm off the arc START: near enough for the tangent rule
    // (2mm), far enough that no Coincident welds them and trips the weld gate.
    const line: SketchEntity = { id: "L", type: "Line", p0: [0, 0], p1: [10, 0.001] };
    // Center offset 1mm from the arc's (within CONCENTRIC_TOLERANCE, outside the
    // tight Coincident snap tolerance) so Concentric fires without the gate tripping.
    const circle: SketchEntity = { id: "C", type: "Circle", center: [11, 10], radius: 10 };
    const arc: SketchEntity = { id: "A", type: "Arc", center: [10, 10], radius: 10, start: [10, 0], end: [0, 10] };
    const cs = inferConstraints([arc], [line, circle], { nextConstraintId: ids(), kinds: AUTO_KINDS_LEGACY });
    const tan = cs.findIndex((c) => c.type === "Tangent");
    const con = cs.findIndex((c) => c.type === "Concentric");
    const eq = cs.findIndex((c) => c.type === "Equal");
    expect(tan).toBeGreaterThanOrEqual(0);
    expect(con).toBeGreaterThan(tan);
    expect(eq).toBeGreaterThan(con);
    expect(cs[con]).toMatchObject({ entities: ["A", "C"] });
    expect(cs[eq]).toMatchObject({ entities: ["A", "C"] });
  });

  it("a same-batch Coincident between two Centers suppresses Concentric (not Equal)", () => {
    const existing: SketchEntity = { id: "e1", type: "Circle", center: [5, 5], radius: 10 };
    const next: SketchEntity = { id: "e2", type: "Circle", center: [5, 5], radius: 10.3 }; // exact-snapped center
    const cs = inferConstraints([next], [existing], { nextConstraintId: ids(), kinds: AUTO_KINDS_LEGACY });
    expect(cs.some((c) => c.type === "Coincident" && c.positions?.[0] === "Center")).toBe(true);
    expect(cs.some((c) => c.type === "Concentric")).toBe(false);
    expect(cs.find((c) => c.type === "Equal")).toMatchObject({ entities: ["e2", "e1"] });
  });

  it("intra-batch: two new circles in one commit still infer Concentric/Equal", () => {
    const circleA: SketchEntity = { id: "cA", type: "Circle", center: [0, 0], radius: 5 };
    const circleB: SketchEntity = { id: "cB", type: "Circle", center: [1, 1], radius: 5.1 };
    const cs = inferConstraints([circleA, circleB], [], { nextConstraintId: ids(), kinds: AUTO_KINDS_LEGACY });
    expect(cs.find((c) => c.type === "Concentric")).toMatchObject({ entities: ["cB", "cA"] });
    expect(cs.find((c) => c.type === "Equal")).toMatchObject({ entities: ["cB", "cA"] });
  });

  it("an Ellipse emits neither Concentric nor Equal", () => {
    const existing: SketchEntity = { id: "c1", type: "Circle", center: [10, 0], radius: 8 };
    const ellipse: SketchEntity = { id: "el1", type: "Ellipse", center: [10, 0], majorR: 8, minorR: 3, rotation: 0 };
    const cs = inferConstraints([ellipse], [existing], { nextConstraintId: ids(), kinds: AUTO_KINDS_LEGACY });
    expect(cs.some((c) => c.type === "Concentric")).toBe(false);
    expect(cs.some((c) => c.type === "Equal")).toBe(false);
    expect(cs.some((c) => c.type === "Coincident")).toBe(true); // centre still coincides, per entityPoints
  });

  it("3mm center offset + 2mm radius delta ⇒ neither fires (both outside tolerance)", () => {
    const existing: SketchEntity = { id: "c1", type: "Circle", center: [0, 0], radius: 10 };
    const next: SketchEntity = { id: "c2", type: "Circle", center: [3, 0], radius: 12 };
    const cs = inferConstraints([next], [existing], { nextConstraintId: ids(), kinds: AUTO_KINDS_LEGACY });
    expect(cs).toEqual([]);
  });
});

/*
 * Constraint V2 policy (SKETCH-V2 P1, spec §35).
 *
 * The blocks above exercise `AUTO_KINDS_LEGACY` — they are the C++
 * `AutoConstrainer` parity record and must keep passing so the ported rules stay
 * pinned. THIS block exercises the DEFAULT, which is the product policy: four
 * kinds are no longer authored behind the user's back.
 */
describe("auto-constraint policy — the V2 default set", () => {
  let n = 0;
  const v2 = () => {
    n = 0;
    return { nextConstraintId: () => `c${++n}` }; // no `kinds` ⇒ AUTO_KINDS_V2
  };
  const types = (cs: { type: string }[]) => cs.map((c) => c.type);

  it("still infers Horizontal and Vertical", () => {
    const h: SketchEntity = { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] };
    const v: SketchEntity = { id: "e2", type: "Line", p0: [0, 0], p1: [0, 40] };
    expect(types(inferConstraints([h], [], v2()))).toEqual(["Horizontal"]);
    expect(types(inferConstraints([v], [], v2()))).toEqual(["Vertical"]);
  });

  it("still infers Perpendicular", () => {
    const ref: SketchEntity = { id: "r1", type: "Line", p0: [0, 0], p1: deg(30) };
    const l: SketchEntity = { id: "e1", type: "Line", p0: [100, 100], p1: [100 + deg(120)[0], 100 + deg(120)[1]] };
    expect(types(inferConstraints([l], [ref], v2()))).toContain("Perpendicular");
  });

  it("still infers Coincident — it is the weld until shared topology lands (§36)", () => {
    const a: SketchEntity = { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] };
    const b: SketchEntity = { id: "e2", type: "Line", p0: [40, 0], p1: [40, 40] };
    expect(types(inferConstraints([b], [a], v2()))).toContain("Coincident");
  });

  it("no longer infers Parallel", () => {
    const ref: SketchEntity = { id: "r1", type: "Line", p0: [0, 0], p1: deg(30) };
    const l: SketchEntity = { id: "e1", type: "Line", p0: [0, 50], p1: [deg(30)[0], 50 + deg(30)[1]] };
    expect(types(inferConstraints([l], [ref], v2()))).not.toContain("Parallel");
    // …and the rule itself still works when explicitly requested.
    expect(
      types(inferConstraints([l], [ref], { nextConstraintId: () => "x", kinds: AUTO_KINDS_LEGACY })),
    ).toContain("Parallel");
  });

  it("no longer infers Tangent", () => {
    const ref: SketchEntity = { id: "r1", type: "Line", p0: [0, 0], p1: [40, 0] };
    const arc: SketchEntity = { id: "e1", type: "Arc", center: [40, 20], radius: 20, start: [40, 0], end: [60, 20] };
    expect(types(inferConstraints([arc], [ref], v2()))).not.toContain("Tangent");
  });

  it("no longer infers Concentric or Equal", () => {
    const a: SketchEntity = { id: "cA", type: "Circle", center: [10, 10], radius: 5 };
    const b: SketchEntity = { id: "cB", type: "Circle", center: [10, 10], radius: 5 };
    const out = types(inferConstraints([b], [a], v2()));
    expect(out).not.toContain("Concentric");
    expect(out).not.toContain("Equal");
  });
});

describe("auto-constraint determinism — the coincidence partner is not array order", () => {
  const ids = () => {
    let n = 0;
    return () => `c${++n}`;
  };

  /** Two reference lines already welded at (40,0): a new endpoint landing there
   *  has TWO equally-valid partners, and the old `refs.find` picked whichever
   *  came first in `existing`. */
  const a: SketchEntity = { id: "a", type: "Line", p0: [0, 0], p1: [40, 0] };
  const b: SketchEntity = { id: "b", type: "Line", p0: [40, 0], p1: [40, 40] };
  const fresh: SketchEntity = { id: "z", type: "Line", p0: [40, 0], p1: [80, 40] };

  it("binds the same partner regardless of the order of `existing`", () => {
    const one = inferConstraints([fresh], [a, b], { nextConstraintId: ids() });
    const two = inferConstraints([fresh], [b, a], { nextConstraintId: ids() });
    const coincOf = (cs: { type: string; entities: string[]; positions?: string[] }[]) =>
      cs.filter((c) => c.type === "Coincident").map((c) => [c.entities, c.positions]);
    expect(coincOf(one)).toEqual(coincOf(two));
  });

  it("prefers the NEAREST reference point over an equally-tolerated farther one", () => {
    const near: SketchEntity = { id: "near", type: "Line", p0: [40, 0], p1: [40, 40] };
    const far: SketchEntity = { id: "far", type: "Line", p0: [40.5, 0], p1: [80, 0] };
    const out = inferConstraints([fresh], [far, near], {
      nextConstraintId: ids(),
      coincidenceTol: 1.0,
    });
    const c = out.find((x) => x.type === "Coincident")!;
    expect(c.entities).toContain("near");
  });
});

describe("auto-constraint suppression — the Alt escape hatch (spec §38)", () => {
  /** What `SketchController` passes when Alt was held at commit. */
  const NONE = new Set<never>();

  it("an EMPTY kind set authors nothing at all", () => {
    // Geometry deliberately rich enough that every rule would otherwise fire:
    // an axis-aligned line (H), landing on an existing endpoint (Coincident),
    // starting at the origin (Fixed anchor).
    // The EXISTING line must stay clear of the origin — the origin anchor is
    // one-per-sketch, and a reference point already sitting there consumes it.
    const existing: SketchEntity = { id: "a", type: "Line", p0: [100, 0], p1: [140, 0] };
    const fresh: SketchEntity = { id: "z", type: "Line", p0: [0, 0], p1: [100, 0] };
    const out = inferConstraints([fresh], [existing], {
      nextConstraintId: () => "c1",
      originAccepted: true,
      kinds: NONE,
    });
    expect(out).toEqual([]);
  });

  it("…and the SAME geometry under the default policy does author them", () => {
    const existing: SketchEntity = { id: "a", type: "Line", p0: [100, 0], p1: [140, 0] };
    const fresh: SketchEntity = { id: "z", type: "Line", p0: [0, 0], p1: [100, 0] };
    let n = 0;
    const out = inferConstraints([fresh], [existing], {
      nextConstraintId: () => `c${++n}`,
      originAccepted: true,
    });
    const kinds = new Set(out.map((c) => c.type));
    expect(kinds.has("Horizontal")).toBe(true);
    expect(kinds.has("Fixed")).toBe(true);
    expect(kinds.has("Coincident")).toBe(true);
  });
});
