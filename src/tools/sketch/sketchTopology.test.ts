/*
 * sketchTopology P0.5 primitives — addressing, and the strict/lenient migration.
 *
 * The centrepiece is `the (type x position) table`: an exhaustive 5x4 comparison
 * of the new strict `pointCoordOf` against the THREE resolvers it replaces. The
 * migration narrows 10 cells (all previously answered by `sketchWireMap`'s
 * lenient fallback) and widens exactly 1 (the deliberate `Point.Center` alias).
 * That table is the evidence that the swap is safe, and the place to look when
 * a hydrated constraint suddenly resolves to null.
 *
 * The two twins being replaced are reproduced here literally rather than
 * imported where they are private, so this test keeps working as they are
 * deleted — and so it cannot become tautological by importing the thing it is
 * meant to be checking.
 */
import { describe, it, expect } from "vitest";
import type { ConstraintPosition, SketchEntity, SketchEntityType } from "@/ipc/types";
import {
  canonicalRole,
  constraintMarshalBlocker,
  coordOfPointKey,
  defaultPointRole,
  entityPointSlots,
  parsePointKey,
  pointCoordOf,
  pointKeyOf,
  pointWireForm,
} from "./sketchTopology";

const POSITIONS: ConstraintPosition[] = ["Start", "End", "Center", "Midpoint"];

const P0: [number, number] = [1, 2];
const P1: [number, number] = [11, 22];
const CENTER: [number, number] = [5, 5];
const A_START: [number, number] = [30, 0];
const A_END: [number, number] = [0, 30];
const MID: [number, number] = [6, 12];

const ENTITIES: Record<SketchEntityType, SketchEntity> = {
  Point: { id: "pt", type: "Point", p0: P0 },
  Line: { id: "ln", type: "Line", p0: P0, p1: P1 },
  Circle: { id: "ci", type: "Circle", center: CENTER, radius: 30 },
  Ellipse: { id: "el", type: "Ellipse", center: CENTER, majorR: 8, minorR: 3, rotation: 0 },
  Arc: { id: "ar", type: "Arc", center: CENTER, radius: 30, start: A_START, end: A_END },
};

/** LITERAL COPY of `sketchWireMap.pointCoord` (private) — the lenient resolver. */
function legacyLenient(e: SketchEntity, position?: ConstraintPosition): [number, number] | null {
  switch (e.type) {
    case "Point":
      return e.p0 ?? null;
    case "Line":
      if (position === "End") return e.p1 ?? null;
      if (position === "Midpoint" && e.p0 && e.p1)
        return [(e.p0[0] + e.p1[0]) / 2, (e.p0[1] + e.p1[1]) / 2];
      return e.p0 ?? null;
    case "Circle":
    case "Ellipse":
      return e.center ?? null;
    case "Arc":
      if (position === "Start") return e.start ?? null;
      if (position === "End") return e.end ?? null;
      return e.center ?? null;
    default:
      return null;
  }
}

/**
 * LITERAL COPY of the strict resolver as it existed in BOTH
 * `badgeLayout.entityPointCoord` and `dimensionLineLayout.resolvePoint` before
 * SKETCH-V2 P0.5. Their bodies were behaviourally identical (verified cell by
 * cell against this table before the swap), so one copy documents both.
 *
 * Copied rather than imported ON PURPOSE: both call sites now DELEGATE to
 * `pointCoordOf`, so importing either would compare the new implementation with
 * itself and the migration evidence would silently evaporate.
 */
function legacyStrict(e: SketchEntity, position: ConstraintPosition): [number, number] | null {
  if (position === "Center" && e.center) return e.center;
  if (position === "Start") {
    if (e.type === "Arc" && e.start) return e.start;
    if (e.p0) return e.p0;
  }
  if (position === "End") {
    if (e.type === "Arc" && e.end) return e.end;
    if (e.p1) return e.p1;
  }
  if (position === "Midpoint" && e.p0 && e.p1)
    return [(e.p0[0] + e.p1[0]) / 2, (e.p0[1] + e.p1[1]) / 2];
  return null;
}

describe("canonicalRole", () => {
  it("aliases a free Point's Center onto Start — the one blessed leniency", () => {
    expect(canonicalRole("Point", "Center")).toBe("Start");
    expect(canonicalRole("Point", "Start")).toBe("Start");
  });

  it("rejects a position the entity does not have", () => {
    expect(canonicalRole("Point", "End")).toBeNull();
    expect(canonicalRole("Point", "Midpoint")).toBeNull();
    expect(canonicalRole("Line", "Center")).toBeNull();
    expect(canonicalRole("Circle", "Start")).toBeNull();
    expect(canonicalRole("Ellipse", "End")).toBeNull();
    expect(canonicalRole("Arc", "Midpoint")).toBeNull();
  });

  it("is idempotent", () => {
    for (const type of Object.keys(ENTITIES) as SketchEntityType[]) {
      for (const p of POSITIONS) {
        const once = canonicalRole(type, p);
        if (once) expect(canonicalRole(type, once)).toBe(once);
      }
    }
  });
});

describe("entityPointSlots / pointWireForm", () => {
  it("a Point has exactly one slot — Center is an alias, not a slot", () => {
    expect(entityPointSlots(ENTITIES.Point).map((s) => s.position)).toEqual(["Start"]);
  });

  it("a Line's Midpoint is derived and carries no wire point", () => {
    const mid = entityPointSlots(ENTITIES.Line).find((s) => s.position === "Midpoint")!;
    expect(mid.derived).toBe(true);
    expect(mid.wireForm).toBe("none");
  });

  it("an Arc's endpoints are owner+role, its centre is a real point id", () => {
    expect(pointWireForm("Arc", "Start")).toBe("arcRole");
    expect(pointWireForm("Arc", "End")).toBe("arcRole");
    expect(pointWireForm("Arc", "Center")).toBe("pointId");
  });

  it("only derived slots are non-marshallable, and only the Arc uses arcRole", () => {
    for (const e of Object.values(ENTITIES)) {
      for (const s of entityPointSlots(e)) {
        if (s.derived) expect(s.wireForm).toBe("none");
        if (s.wireForm === "arcRole") expect(e.type).toBe("Arc");
      }
    }
  });

  it("wire form follows the alias", () => {
    expect(pointWireForm("Point", "Center")).toBe("pointId");
    expect(pointWireForm("Point", "End")).toBeNull();
  });
});

describe("pointKeyOf / parsePointKey", () => {
  it("round-trips a frontend id", () => {
    const k = pointKeyOf("e12", "End");
    expect(k).toBe("e12.End");
    expect(parsePointKey(k)).toEqual({ entityId: "e12", position: "End" });
  });

  it("round-trips a hydrated backend uuid (no dots, hyphens are safe)", () => {
    const uuid = "3f2a91c4-7b0e-4d55-9a1e-0c8f2b6d4e77";
    expect(parsePointKey(pointKeyOf(uuid, "Center"))).toEqual({
      entityId: uuid,
      position: "Center",
    });
  });

  it("splits on the LAST dot, so a dotted id keeps its dots", () => {
    expect(parsePointKey("a.b.Start")).toEqual({ entityId: "a.b", position: "Start" });
  });

  it("rejects a non-address", () => {
    expect(parsePointKey("e1")).toBeNull(); // no dot
    expect(parsePointKey("e1.")).toBeNull(); // trailing dot
    expect(parsePointKey(".Start")).toBeNull(); // no entity id
    expect(parsePointKey("e1.start")).toBeNull(); // wire casing is not a frontend position
    expect(parsePointKey("e1.Middle")).toBeNull(); // unknown position
  });
});

describe("pointCoordOf — the (type x position) table", () => {
  /** null = no such point. Everything below is asserted cell by cell. */
  const EXPECTED: Record<SketchEntityType, Record<ConstraintPosition, [number, number] | null>> = {
    Point: { Start: P0, End: null, Center: P0, Midpoint: null },
    Line: { Start: P0, End: P1, Center: null, Midpoint: MID },
    Circle: { Start: null, End: null, Center: CENTER, Midpoint: null },
    Ellipse: { Start: null, End: null, Center: CENTER, Midpoint: null },
    Arc: { Start: A_START, End: A_END, Center: CENTER, Midpoint: null },
  };

  for (const type of Object.keys(ENTITIES) as SketchEntityType[]) {
    for (const position of POSITIONS) {
      it(`${type}.${position}`, () => {
        expect(pointCoordOf(ENTITIES[type], position)).toEqual(EXPECTED[type][position]);
      });
    }
  }

  it("NARROWS exactly the 10 cells the lenient resolver used to answer", () => {
    const narrowed: string[] = [];
    for (const type of Object.keys(ENTITIES) as SketchEntityType[]) {
      for (const position of POSITIONS) {
        const before = legacyLenient(ENTITIES[type], position);
        const after = pointCoordOf(ENTITIES[type], position);
        if (before !== null && after === null) narrowed.push(`${type}.${position}`);
      }
    }
    expect(narrowed.sort()).toEqual(
      [
        "Arc.Midpoint",
        "Circle.End",
        "Circle.Midpoint",
        "Circle.Start",
        "Ellipse.End",
        "Ellipse.Midpoint",
        "Ellipse.Start",
        "Line.Center",
        "Point.End",
        "Point.Midpoint",
      ].sort(),
    );
  });

  it("never CHANGES a non-null answer — narrowing only, no silent re-binding", () => {
    for (const type of Object.keys(ENTITIES) as SketchEntityType[]) {
      for (const position of POSITIONS) {
        const before = legacyLenient(ENTITIES[type], position);
        const after = pointCoordOf(ENTITIES[type], position);
        if (before !== null && after !== null) expect(after).toEqual(before);
      }
    }
  });

  it("WIDENS exactly one cell versus the strict twins — the Point.Center alias", () => {
    const widened: string[] = [];
    for (const type of Object.keys(ENTITIES) as SketchEntityType[]) {
      for (const position of POSITIONS) {
        const before = legacyStrict(ENTITIES[type], position);
        const after = pointCoordOf(ENTITIES[type], position);
        if (before === null && after !== null) widened.push(`${type}.${position}`);
        else expect(after, `${type}.${position}`).toEqual(before);
      }
    }
    expect(widened).toEqual(["Point.Center"]);
  });

  it("returns null when the entity is missing the backing coordinate field", () => {
    expect(pointCoordOf({ id: "l", type: "Line", p0: P0 }, "End")).toBeNull();
    expect(pointCoordOf({ id: "l", type: "Line", p0: P0 }, "Midpoint")).toBeNull();
    expect(pointCoordOf({ id: "a", type: "Arc", center: CENTER, radius: 3 }, "Start")).toBeNull();
    expect(pointCoordOf({ id: "c", type: "Circle", radius: 3 }, "Center")).toBeNull();
  });
});

describe("defaultPointRole — the absent-position path must not move", () => {
  it("matches the lenient resolver EXACTLY when no position is given", () => {
    // `sketchWireMap.pointCoord` is called with `positions[i]` which is
    // `undefined` for any constraint that omits the slot (SCHEMA §7.3: absent
    // means "this slot IS the point"). That path is legitimate and common —
    // narrowing it would silently drop hydrated `Fixed` constraints.
    for (const type of Object.keys(ENTITIES) as SketchEntityType[]) {
      const e = ENTITIES[type];
      expect(pointCoordOf(e, defaultPointRole(type)), type).toEqual(legacyLenient(e, undefined));
    }
  });

  it("a curve defaults to its defining point, a Point/Line to its start", () => {
    expect(defaultPointRole("Point")).toBe("Start");
    expect(defaultPointRole("Line")).toBe("Start");
    expect(defaultPointRole("Circle")).toBe("Center");
    expect(defaultPointRole("Ellipse")).toBe("Center");
    expect(defaultPointRole("Arc")).toBe("Center");
  });

  it("the default role is always a real slot", () => {
    for (const type of Object.keys(ENTITIES) as SketchEntityType[]) {
      expect(canonicalRole(type, defaultPointRole(type)), type).not.toBeNull();
    }
  });
});

describe("constraintMarshalBlocker — the loud-drop guard", () => {
  const byId = new Map(Object.values(ENTITIES).map((e) => [e.id, e]));

  it("lets Coincident reference an arc endpoint — the one kind whose wire shape carries it", () => {
    expect(
      constraintMarshalBlocker(
        { type: "Coincident", entities: ["ar", "ln"], positions: ["Start", "End"] },
        byId,
      ),
    ).toBeNull();
  });

  it("allows every OTHER kind on an arc endpoint too, since P3", () => {
    // Before P3 these all resolved null and `marshalUpsert` dropped them with no
    // error: the user applied a Tangent to an arc endpoint and nothing happened.
    for (const type of ["Tangent", "Fixed", "Distance", "HorizontalDistance", "Symmetric"]) {
      expect(
        constraintMarshalBlocker({ type, entities: ["ar", "ln"], positions: ["End", "Start"] }, byId),
        type,
      ).toBeNull();
    }
  });

  it("blocks a derived point (a Line's virtual midpoint) used as a POINT slot", () => {
    expect(
      constraintMarshalBlocker({ type: "Fixed", entities: ["ln"], positions: ["Midpoint"] }, byId),
    ).toBe("derivedPoint");
  });

  it("allows an entity-level slot — a whole line or circle is not a point ref", () => {
    expect(constraintMarshalBlocker({ type: "Horizontal", entities: ["ln"] }, byId)).toBeNull();
    // Midpoint's LINE operand is entity-level (no position), its point operand is real.
    expect(
      constraintMarshalBlocker(
        { type: "Midpoint", entities: ["pt", "ln"], positions: ["Start", undefined] },
        byId,
      ),
    ).toBeNull();
  });

  it("allows ordinary line endpoints and curve centres", () => {
    expect(
      constraintMarshalBlocker(
        { type: "Distance", entities: ["ln", "ci"], positions: ["Start", "Center"] },
        byId,
      ),
    ).toBeNull();
  });

  it("is silent about a MISSING entity — that is a different failure", () => {
    expect(
      constraintMarshalBlocker({ type: "Fixed", entities: ["ghost"], positions: ["Start"] }, byId),
    ).toBeNull();
  });

  it("an arc CENTRE is a real minted point, not an endpoint role", () => {
    expect(
      constraintMarshalBlocker({ type: "Fixed", entities: ["ar"], positions: ["Center"] }, byId),
    ).toBeNull();
  });
});

describe("coordOfPointKey", () => {
  const byId = new Map(Object.values(ENTITIES).map((e) => [e.id, e]));

  it("resolves through the key", () => {
    expect(coordOfPointKey("ar.End", byId)).toEqual(A_END);
    expect(coordOfPointKey("ln.Midpoint", byId)).toEqual(MID);
    expect(coordOfPointKey("pt.Center", byId)).toEqual(P0);
  });

  it("returns null for an unparseable key or an absent entity", () => {
    expect(coordOfPointKey("nope", byId)).toBeNull();
    expect(coordOfPointKey("ghost.Start", byId)).toBeNull();
  });
});
