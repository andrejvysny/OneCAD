import { describe, it, expect } from "vitest";
import { evaluateApplicability, WIRE_ENCODABLE } from "./constraintApplicability";
import { toConstraintTarget, type SketchConstraintTarget } from "./constraintTarget";
import type { ConstraintPosition, SketchConstraintType, SketchEntity } from "@/ipc/types";

// Fixture mirrors the C++ prototype test's sketch 1:1
// (worker/tests/prototypes/proto_sketch_constraint_applicability.cpp:26-39),
// minus the Ellipse (out of scope — no "Ellipse" `SketchEntityType` in the
// frontend wire slice; the ellipse assertions there have no TS equivalent).
const entities: SketchEntity[] = [
  { id: "p1", type: "Point", p0: [0, 0] },
  { id: "p2", type: "Point", p0: [10, 0] },
  { id: "line", type: "Line", p0: [0, 0], p1: [10, 0] },
  { id: "p3", type: "Point", p0: [0, 4] },
  { id: "p4", type: "Point", p0: [10, 4] },
  { id: "line2", type: "Line", p0: [0, 4], p1: [10, 4] },
  { id: "circleCenter", type: "Point", p0: [5, 5] },
  { id: "circle", type: "Circle", center: [5, 5], radius: 2.5 },
  { id: "arcCenter", type: "Point", p0: [12, 5] },
  { id: "arc", type: "Arc", center: [12, 5], radius: 2.5, start: [14.5, 5], end: [13.3508, 7.1035] },
  // Added for the Tangent/Equal/Midpoint frontend-extension coverage below —
  // a second circle + arc (for circle×circle / arc×arc pairs).
  { id: "circle2", type: "Circle", center: [30, 5], radius: 4 },
  { id: "arc2", type: "Arc", center: [40, 5], radius: 2.5, start: [42.5, 5], end: [41.3508, 7.1035] },
];

/** Resolve a fixture ref via the real adapter — throws if the fixture is wrong. */
function T(entityId: string, point?: ConstraintPosition): SketchConstraintTarget {
  const t = toConstraintTarget({ entityId, point }, entities);
  if (!t) throw new Error(`fixture bug: ${entityId}${point ? `.${point}` : ""} did not resolve`);
  return t;
}

const typesOf = (rs: { type: SketchConstraintType }[]): SketchConstraintType[] => rs.map((r) => r.type);

describe("evaluateApplicability — 1 target (ConstraintApplicability.cpp:92-107)", () => {
  it("a Line alone ⇒ Horizontal, Vertical (cpp:95-98)", () => {
    const rs = evaluateApplicability([T("line")], entities);
    expect(typesOf(rs)).toEqual(["Horizontal", "Vertical"]);
  });

  it("a Point alone ⇒ Fixed (cpp:99-101)", () => {
    const rs = evaluateApplicability([T("p1")], entities);
    expect(typesOf(rs)).toEqual(["Fixed"]);
  });

  it("a Circle alone ⇒ Radius, Diameter (cpp:102-105)", () => {
    const rs = evaluateApplicability([T("circle")], entities);
    expect(typesOf(rs)).toEqual(["Radius", "Diameter"]);
  });

  it("an Arc alone ⇒ Radius, Diameter (cpp:102-105)", () => {
    const rs = evaluateApplicability([T("arc")], entities);
    expect(typesOf(rs)).toEqual(["Radius", "Diameter"]);
  });
});

describe("evaluateApplicability — 2 targets (mirrors the prototype test 1:1)", () => {
  // proto_sketch_constraint_applicability.cpp:44-48 (pointPoint)
  it("point & point (no joining line) ⇒ Distance, Coincident, H/V-Distance — NOT H/V", () => {
    const rs = evaluateApplicability([T("p1"), T("p3")], entities);
    expect(typesOf(rs)).toEqual(["Distance", "Coincident", "HorizontalDistance", "VerticalDistance"]);
  });

  // proto_sketch_constraint_applicability.cpp:50-54 (pointLine), extended by the
  // Midpoint frontend addition: a free point + a Line now also offers Midpoint
  // (after OnCurve — see the module header / evaluateApplicability doc).
  it("point & line ⇒ Distance, OnCurve, Midpoint", () => {
    const rs = evaluateApplicability([T("p1"), T("line")], entities);
    expect(typesOf(rs)).toEqual(["Distance", "OnCurve", "Midpoint"]);
  });

  // proto_sketch_constraint_applicability.cpp:56-60 (pointCircle) — Midpoint is
  // Line-only, so a point + Circle stays OnCurve-only (frontend addition doesn't
  // reach here).
  it("point & circle ⇒ OnCurve only (NOT Distance, NOT Midpoint)", () => {
    const rs = evaluateApplicability([T("p1"), T("circle")], entities);
    expect(typesOf(rs)).toEqual(["OnCurve"]);
  });

  // point & arc — same reasoning as point & circle (Midpoint never offered for
  // an Arc curve target).
  it("point & arc ⇒ OnCurve only (NOT Midpoint)", () => {
    const rs = evaluateApplicability([T("p1"), T("arc")], entities);
    expect(typesOf(rs)).toEqual(["OnCurve"]);
  });

  // proto_sketch_constraint_applicability.cpp:69-73 (lineCircle) — neither is a
  // point, and a circle is not a `line`, so the PORTED matrix offers nothing;
  // the Tangent frontend addition now offers exactly Tangent for this pair.
  it("line & circle ⇒ Tangent only (frontend addition; no Distance/Parallel/etc.)", () => {
    const rs = evaluateApplicability([T("line"), T("circle")], entities);
    expect(typesOf(rs)).toEqual(["Tangent"]);
  });

  it("line & arc ⇒ Tangent only", () => {
    const rs = evaluateApplicability([T("line"), T("arc")], entities);
    expect(typesOf(rs)).toEqual(["Tangent"]);
  });

  // proto_sketch_constraint_applicability.cpp:75-78 (circleArc), extended by the
  // Tangent/Equal frontend additions (both circular×circular pairs).
  it("circle & arc ⇒ Concentric, Tangent, Equal", () => {
    const rs = evaluateApplicability([T("circle"), T("arc")], entities);
    expect(typesOf(rs)).toEqual(["Concentric", "Tangent", "Equal"]);
  });

  it("circle & circle ⇒ Concentric, Tangent, Equal", () => {
    const rs = evaluateApplicability([T("circle"), T("circle2")], entities);
    expect(typesOf(rs)).toEqual(["Concentric", "Tangent", "Equal"]);
  });

  it("arc & arc ⇒ Concentric, Tangent, Equal", () => {
    const rs = evaluateApplicability([T("arc"), T("arc2")], entities);
    expect(typesOf(rs)).toEqual(["Concentric", "Tangent", "Equal"]);
  });

  // proto_sketch_constraint_applicability.cpp:80-86 (lineLine), extended by the
  // Equal frontend addition (after Angle).
  it("line & line ⇒ Distance, Parallel, Perpendicular, Angle, Equal", () => {
    const rs = evaluateApplicability([T("line"), T("line2")], entities);
    expect(typesOf(rs)).toEqual(["Distance", "Parallel", "Perpendicular", "Angle", "Equal"]);
  });
});

describe("Tangent — frontend extension (evaluateApplicability header)", () => {
  it("offered pairs: line+circle, line+arc, circle+circle, circle+arc, arc+arc", () => {
    const pairs: [string, string][] = [
      ["line", "circle"],
      ["line", "arc"],
      ["circle", "circle2"],
      ["circle", "arc"],
      ["arc", "arc2"],
    ];
    for (const [x, y] of pairs) {
      const rs = evaluateApplicability([T(x), T(y)], entities);
      expect(rs.some((r) => r.type === "Tangent")).toBe(true);
    }
  });

  it("never offered for line+line", () => {
    const rs = evaluateApplicability([T("line"), T("line2")], entities);
    expect(rs.some((r) => r.type === "Tangent")).toBe(false);
  });

  it("never offered when either target is a point", () => {
    expect(evaluateApplicability([T("p1"), T("circle")], entities).some((r) => r.type === "Tangent")).toBe(false);
    expect(evaluateApplicability([T("p1"), T("arc")], entities).some((r) => r.type === "Tangent")).toBe(false);
    expect(evaluateApplicability([T("p1"), T("p3")], entities).some((r) => r.type === "Tangent")).toBe(false);
  });

  it("targets carry the selection order (any order accepted)", () => {
    const rs = evaluateApplicability([T("circle"), T("line")], entities);
    const tangent = rs.find((r) => r.type === "Tangent")!;
    expect(tangent.targets).toEqual([T("circle"), T("line")]);
  });

  it("appears after Concentric in the circular×circular branch", () => {
    const rs = evaluateApplicability([T("circle"), T("arc")], entities);
    expect(typesOf(rs).indexOf("Tangent")).toBeGreaterThan(typesOf(rs).indexOf("Concentric"));
  });
});

describe("Equal — frontend extension (evaluateApplicability header)", () => {
  it("offered pairs: line+line, circle+circle, circle+arc, arc+arc", () => {
    const pairs: [string, string][] = [
      ["line", "line2"],
      ["circle", "circle2"],
      ["circle", "arc"],
      ["arc", "arc2"],
    ];
    for (const [x, y] of pairs) {
      const rs = evaluateApplicability([T(x), T(y)], entities);
      expect(rs.some((r) => r.type === "Equal")).toBe(true);
    }
  });

  it("never offered for a mixed line×circular pair", () => {
    expect(evaluateApplicability([T("line"), T("circle")], entities).some((r) => r.type === "Equal")).toBe(false);
    expect(evaluateApplicability([T("line"), T("arc")], entities).some((r) => r.type === "Equal")).toBe(false);
  });

  it("never offered for a 3-target selection (Equal is 2-target only)", () => {
    const rs = evaluateApplicability([T("line"), T("line2"), T("circle")], entities);
    expect(rs.some((r) => r.type === "Equal")).toBe(false);
  });

  it("appears after Angle in the line×line branch", () => {
    const rs = evaluateApplicability([T("line"), T("line2")], entities);
    expect(typesOf(rs).indexOf("Equal")).toBeGreaterThan(typesOf(rs).indexOf("Angle"));
  });

  it("appears after Tangent in the circular×circular branch", () => {
    const rs = evaluateApplicability([T("circle"), T("arc")], entities);
    expect(typesOf(rs).indexOf("Equal")).toBeGreaterThan(typesOf(rs).indexOf("Tangent"));
  });
});

describe("Midpoint — frontend extension (evaluateApplicability header)", () => {
  it("a free Point + a Line ⇒ offered, ordered [point, line]", () => {
    const rs = evaluateApplicability([T("p1"), T("line")], entities);
    const mid = rs.find((r) => r.type === "Midpoint")!;
    expect(mid).toBeDefined();
    expect(mid.targets).toEqual([T("p1"), T("line")]);
  });

  it("order-normalized even when the Line is selected FIRST", () => {
    const rs = evaluateApplicability([T("line2"), T("p3")], entities);
    const mid = rs.find((r) => r.type === "Midpoint")!;
    expect(mid.targets).toEqual([T("p3"), T("line2")]);
  });

  it("a Line's own Start/End (as a named point) + a DIFFERENT line ⇒ offered", () => {
    const rs = evaluateApplicability([T("line", "Start"), T("line2")], entities);
    expect(rs.some((r) => r.type === "Midpoint")).toBe(true);
  });

  it("a Circle's Center (as a named point) + a Line ⇒ offered", () => {
    const rs = evaluateApplicability([T("circle", "Center"), T("line")], entities);
    expect(rs.some((r) => r.type === "Midpoint")).toBe(true);
  });

  it("an Arc's Center (as a named point) + a Line ⇒ offered", () => {
    const rs = evaluateApplicability([T("arc", "Center"), T("line")], entities);
    expect(rs.some((r) => r.type === "Midpoint")).toBe(true);
  });

  it("EXCLUDED: an Arc's Start/End (unmappable — no synthesized wire point) + a Line", () => {
    const rs = evaluateApplicability([T("arc", "Start"), T("line")], entities);
    expect(rs.some((r) => r.type === "Midpoint")).toBe(false);
    const rsEnd = evaluateApplicability([T("arc", "End"), T("line")], entities);
    expect(rsEnd.some((r) => r.type === "Midpoint")).toBe(false);
  });

  it("EXCLUDED: a Line's own virtual Midpoint position + a DIFFERENT line", () => {
    const rs = evaluateApplicability([T("line", "Midpoint"), T("line2")], entities);
    expect(rs.some((r) => r.type === "Midpoint")).toBe(false);
  });

  it("EXCLUDED: a line's own endpoint + that SAME line (degenerate same-entity pair)", () => {
    const rs = evaluateApplicability([T("line", "Start"), T("line")], entities);
    expect(rs.some((r) => r.type === "Midpoint")).toBe(false);
  });

  it("EXCLUDED: point + Circle/Arc (Midpoint is Line-only)", () => {
    expect(evaluateApplicability([T("p1"), T("circle")], entities).some((r) => r.type === "Midpoint")).toBe(false);
    expect(evaluateApplicability([T("p1"), T("arc")], entities).some((r) => r.type === "Midpoint")).toBe(false);
  });

  it("appears after OnCurve in the point×line branch", () => {
    const rs = evaluateApplicability([T("p1"), T("line")], entities);
    expect(typesOf(rs).indexOf("Midpoint")).toBeGreaterThan(typesOf(rs).indexOf("OnCurve"));
  });
});

describe("hasLineBetweenPoints port (ConstraintApplicability.cpp:32-50, used at cpp:149-152)", () => {
  it("two free points that ARE a line's own endpoints ⇒ Horizontal/Vertical also applicable, targeting that line", () => {
    const rs = evaluateApplicability([T("p1"), T("p2")], entities);
    expect(typesOf(rs)).toEqual([
      "Distance",
      "Coincident",
      "HorizontalDistance",
      "VerticalDistance",
      "Horizontal",
      "Vertical",
    ]);
    const horiz = rs.find((r) => r.type === "Horizontal")!;
    expect(horiz.targets).toEqual([{ kind: "line", entityId: "line" }]);
  });

  it("a line's own Start + End (selected directly, not via free points) ⇒ same join found", () => {
    const rs = evaluateApplicability([T("line", "Start"), T("line", "End")], entities);
    expect(rs.some((r) => r.type === "Horizontal")).toBe(true);
    expect(rs.some((r) => r.type === "Vertical")).toBe(true);
  });

  it("two free points NOT joined by any line ⇒ no Horizontal/Vertical (negative case)", () => {
    const rs = evaluateApplicability([T("p1"), T("p3")], entities);
    expect(rs.some((r) => r.type === "Horizontal" || r.type === "Vertical")).toBe(false);
  });
});

describe("evaluateApplicability — 3 targets (ConstraintApplicability.cpp:109-123)", () => {
  // proto_sketch_constraint_applicability.cpp:88-93
  it("2 points + 1 line ⇒ Symmetric, targets ordered [point1, point2, axis-line]", () => {
    const rs = evaluateApplicability([T("p1"), T("p3"), T("line2")], entities);
    expect(rs).toEqual([
      { type: "Symmetric", targets: [T("p1"), T("p3"), T("line2")], dimensional: false },
    ]);
  });

  it("ordering is normalized even when the line is selected FIRST", () => {
    const rs = evaluateApplicability([T("line2"), T("p1"), T("p3")], entities);
    expect(rs[0].targets).toEqual([T("p1"), T("p3"), T("line2")]);
  });

  it("3 points (no line) ⇒ empty", () => {
    expect(evaluateApplicability([T("p1"), T("p2"), T("p3")], entities)).toEqual([]);
  });

  it("2 lines + 1 point ⇒ empty", () => {
    expect(evaluateApplicability([T("line"), T("line2"), T("p1")], entities)).toEqual([]);
  });

  it("point + line + circle (wrong shape) ⇒ empty", () => {
    expect(evaluateApplicability([T("p1"), T("line"), T("circle")], entities)).toEqual([]);
  });
});

describe("evaluateApplicability — counts other than 1/2/3 (ConstraintApplicability.cpp:78-80,125-127)", () => {
  it("0 targets ⇒ empty", () => {
    expect(evaluateApplicability([], entities)).toEqual([]);
  });

  it("4 targets ⇒ empty", () => {
    expect(evaluateApplicability([T("p1"), T("p2"), T("p3"), T("p4")], entities)).toEqual([]);
  });
});

describe("dedup — duplicate selection of the same target is rejected (cpp:64-75)", () => {
  it("selecting the same point twice collapses to a 1-target evaluation (Fixed only)", () => {
    const rs = evaluateApplicability([T("p1"), T("p1")], entities);
    expect(typesOf(rs)).toEqual(["Fixed"]);
  });

  it("a line's BODY and that same line's Start point are DIFFERENT targets (not deduped)", () => {
    const rs = evaluateApplicability([T("line"), T("line", "Start")], entities);
    // Evaluated as a real (line, point) pair, not collapsed to a 1-target Fixed/H/V.
    expect(typesOf(rs)).toEqual(["Distance", "OnCurve"]);
  });
});

describe("entity-existence guard (ConstraintApplicability.cpp:84-90: `if (!entity) return result;`)", () => {
  it("a target whose entity is absent from `entities` aborts the WHOLE evaluation ⇒ empty", () => {
    const ghost: SketchConstraintTarget = { kind: "line", entityId: "ghost-not-in-sketch" };
    const rs = evaluateApplicability([ghost, T("circle")], entities);
    expect(rs).toEqual([]);
  });
});

describe("encodability invariant (S4b design item 3)", () => {
  // The exhaustive set of types the matrix (ConstraintApplicability.cpp:92-169,
  // PLUS the Tangent/Equal/Midpoint frontend-extension rules — see this module's
  // header) can EVER emit — every row exercised by the scenarios above. Kept as a
  // static list (not accumulated across tests) so this test is self-contained and
  // does not depend on suite execution order. Now covers all 18 kinds.
  const MATRIX_EMITTABLE_TYPES: SketchConstraintType[] = [
    "Horizontal",
    "Vertical",
    "Fixed",
    "Radius",
    "Diameter",
    "Distance",
    "Coincident",
    "HorizontalDistance",
    "VerticalDistance",
    "Concentric",
    "Parallel",
    "Perpendicular",
    "Angle",
    "OnCurve",
    "Symmetric",
    "Tangent",
    "Equal",
    "Midpoint",
  ];

  it("WIRE_ENCODABLE defines all 18 SketchConstraintType kinds", () => {
    expect(Object.keys(WIRE_ENCODABLE)).toHaveLength(18);
  });

  it("every type the matrix can ever emit is wire-encodable (never offer what the wire can't send)", () => {
    for (const type of MATRIX_EMITTABLE_TYPES) {
      expect(WIRE_ENCODABLE[type]).toBe(true);
    }
  });

  it("re-derives MATRIX_EMITTABLE_TYPES by exhaustively running the matrix over every fixture combo", () => {
    // Cross-check the static list above against a fresh run (guards the list
    // itself from silently drifting out of sync with the implementation).
    const ids = entities.map((e) => e.id);
    const singles = ids.map((id) => [T(id)]);
    const pairs: SketchConstraintTarget[][] = [];
    for (const a of ids) for (const b of ids) if (a !== b) pairs.push([T(a), T(b)]);
    const triples: SketchConstraintTarget[][] = [];
    for (const a of ids)
      for (const b of ids)
        for (const c of ids) if (a !== b && b !== c && a !== c) triples.push([T(a), T(b), T(c)]);

    const seen = new Set<SketchConstraintType>();
    for (const targets of [...singles, ...pairs, ...triples]) {
      for (const r of evaluateApplicability(targets, entities)) seen.add(r.type);
    }

    expect([...seen].sort()).toEqual([...MATRIX_EMITTABLE_TYPES].sort());
  });

  it("all 18 SketchConstraintType kinds are represented in MATRIX_EMITTABLE_TYPES", () => {
    // Tangent/Equal/Midpoint are frontend-extension rules (not ported from the
    // C++ matrix — see the module header) but are now offered like every other
    // kind, so the matrix's emittable set covers the FULL SketchConstraintType
    // vocabulary.
    expect(MATRIX_EMITTABLE_TYPES).toContain("Equal");
    expect(MATRIX_EMITTABLE_TYPES).toContain("Midpoint");
    expect(MATRIX_EMITTABLE_TYPES).toContain("Tangent");
    expect(MATRIX_EMITTABLE_TYPES).toHaveLength(18);
  });
});
