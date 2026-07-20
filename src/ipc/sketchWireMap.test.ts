/*
 * sketchWireMap — the PURE frontend → Rust-typed sketch marshaller (F-WP9).
 * Deterministic ids (`uuid-N`) make the AddEntity/AddConstraint output assertable.
 */
import { beforeEach, describe, expect, it } from "vitest";
import {
  applySolvedPositions,
  buildAddSketch,
  buildDeleteSketch,
  createIdMap,
  frontendConstraintsFromDto,
  frontendEntitiesFromDto,
  frontendSolvedPositions,
  isDimensional,
  marshalUpsert,
  seedIdMapFromWire,
} from "./sketchWireMap";
import type { SketchConstraint, SketchEntity } from "./types";

let seq = 0;
const mint = () => `uuid-${++seq}`;
beforeEach(() => {
  seq = 0;
});

const line = (id: string, p0: [number, number], p1: [number, number]): SketchEntity => ({ id, type: "Line", p0, p1 });
const circle = (id: string, center: [number, number], radius: number): SketchEntity => ({ id, type: "Circle", center, radius });

describe("marshalUpsert — entities", () => {
  it("marshals a Line into two synthesized Points + a point-referenced Line", () => {
    const map = createIdMap("sk-uuid", "XY");
    const ops = marshalUpsert(map, { entities: [line("e1", [0, 0], [40, 0])], constraints: [] }, mint);
    expect(ops).toHaveLength(3);
    expect(ops[0]).toEqual({ op: "addEntity", entity: { kind: "point", id: "uuid-1", at: [0, 0] } });
    expect(ops[1]).toEqual({ op: "addEntity", entity: { kind: "point", id: "uuid-2", at: [40, 0] } });
    expect(ops[2]).toEqual({ op: "addEntity", entity: { kind: "line", id: "uuid-3", start: "uuid-1", end: "uuid-2" } });
    expect(map.entity.get("e1")).toBe("uuid-3");
    expect(map.point.get("e1.Start")).toBe("uuid-1");
    expect(map.point.get("e1.End")).toBe("uuid-2");
  });

  it("marshals a Circle into a center Point + a circle", () => {
    const map = createIdMap("sk", "XY");
    const ops = marshalUpsert(map, { entities: [circle("e1", [10, 10], 3)], constraints: [] }, mint);
    expect(ops[0]).toEqual({ op: "addEntity", entity: { kind: "point", id: "uuid-1", at: [10, 10] } });
    expect(ops[1]).toEqual({ op: "addEntity", entity: { kind: "circle", id: "uuid-2", center: "uuid-1", radius: 3 } });
  });

  it("emits only NEW entities on a second upsert (diff by id)", () => {
    const map = createIdMap("sk", "XY");
    marshalUpsert(map, { entities: [line("e1", [0, 0], [40, 0])], constraints: [] }, mint);
    const ops = marshalUpsert(map, { entities: [line("e1", [0, 0], [40, 0]), circle("e2", [5, 5], 2)], constraints: [] }, mint);
    expect(ops).toHaveLength(2); // e1 already mapped; only e2's point + circle
    expect(ops.every((o) => o.op === "addEntity")).toBe(true);
  });

  it("emits RemoveEntity when a mapped entity leaves the array", () => {
    const map = createIdMap("sk", "XY");
    marshalUpsert(map, { entities: [line("e1", [0, 0], [40, 0])], constraints: [] }, mint);
    const lineId = map.entity.get("e1")!;
    const ops = marshalUpsert(map, { entities: [], constraints: [] }, mint);
    expect(ops).toEqual([{ op: "removeEntity", entity: lineId }]);
    expect(map.entity.has("e1")).toBe(false);
    expect(map.point.has("e1.Start")).toBe(false);
  });
});

describe("marshalUpsert — constraints", () => {
  it("Horizontal references the line entity id", () => {
    const map = createIdMap("sk", "XY");
    const ops = marshalUpsert(
      map,
      { entities: [line("e1", [0, 0], [40, 0])], constraints: [{ id: "c1", type: "Horizontal", entities: ["e1"] }] },
      mint,
    );
    const c = ops.find((o) => o.op === "addConstraint");
    expect(c).toEqual({ op: "addConstraint", constraint: { kind: "horizontal", id: "uuid-4", line: "uuid-3" } });
  });

  it("Coincident resolves endpoint POINT ids via the positions selectors", () => {
    const map = createIdMap("sk", "XY");
    const constraints: SketchConstraint[] = [
      { id: "c1", type: "Coincident", entities: ["e1", "e2"], positions: ["End", "Start"] },
    ];
    const ops = marshalUpsert(
      map,
      { entities: [line("e1", [0, 0], [40, 0]), line("e2", [40, 0], [40, 40])], constraints },
      mint,
    );
    // e1: p uuid-1(Start), uuid-2(End), line uuid-3; e2: p uuid-4(Start), uuid-5(End), line uuid-6.
    const c = ops.find((o) => o.op === "addConstraint");
    expect(c).toEqual({ op: "addConstraint", constraint: { kind: "coincident", id: "uuid-7", point1: "uuid-2", point2: "uuid-4" } });
  });

  it("emits SetDimension when a dimensional value changes in place", () => {
    const map = createIdMap("sk", "XY");
    const radius = (v: number): SketchConstraint => ({ id: "c1", type: "Radius", entities: ["e1"], value: v });
    marshalUpsert(map, { entities: [circle("e1", [0, 0], 5)], constraints: [radius(5)] }, mint);
    const cId = map.constraint.get("c1")!;
    const ops = marshalUpsert(map, { entities: [circle("e1", [0, 0], 5)], constraints: [radius(8)] }, mint);
    expect(ops).toEqual([{ op: "setDimension", constraint: cId, value: { value: 8 } }]);
  });

  it("skips an unmappable constraint (arc-endpoint coincidence) without throwing", () => {
    const map = createIdMap("sk", "XY");
    const arc: SketchEntity = { id: "e1", type: "Arc", center: [0, 0], radius: 5, start: [5, 0], end: [0, 5] };
    // Coincident on the arc START has no Rust point id (arc references only center).
    const ops = marshalUpsert(
      map,
      { entities: [arc], constraints: [{ id: "c1", type: "Coincident", entities: ["e1", "e1"], positions: ["Start", "End"] }] },
      mint,
    );
    expect(ops.some((o) => o.op === "addConstraint")).toBe(false);
    expect(map.constraint.has("c1")).toBe(false);
  });
});

describe("buildAddSketch / frontendEntitiesFromDto / isDimensional", () => {
  it("builds a minimal world-plane AddSketch w/ REQUIRED plane basis (custom → XY)", () => {
    // Rust SketchData has NO serde default for `plane` — omitting it fails
    // deserialization before the command handler runs (the picker-flow bug).
    expect(buildAddSketch("id-1", "Sketch 1", "XZ")).toEqual({
      cmd: "addSketch",
      sketch: {
        id: "id-1",
        name: "Sketch 1",
        plane: { origin: [0, 0, 0], xAxis: [0, 1, 0], yAxis: [0, 0, 1], normal: [1, 0, 0] },
        attachment: { kind: "world", plane: "XZ" },
      },
    });
    const custom = buildAddSketch("id-1", "S", "custom").sketch;
    expect(custom.attachment.plane).toBe("XY");
    // Canonical (non-standard) XY basis — must match Rust SketchPlane::xy().
    expect(custom.plane).toEqual({
      origin: [0, 0, 0],
      xAxis: [0, 1, 0],
      yAxis: [-1, 0, 0],
      normal: [0, 0, 1],
    });
  });

  it("reverse-maps worker-wire entities to the frontend inlined form; [] for empty", () => {
    expect(frontendEntitiesFromDto([])).toEqual([]);
    const wire = [
      { id: "p1", type: "Point", at: [0, 0] },
      { id: "p2", type: "Point", at: [40, 0] },
      { id: "l1", type: "Line", p0Ref: "p1", p1Ref: "p2" },
      { id: "c1", type: "Circle", center: [10, 10], radius: 3 },
    ];
    const fe = frontendEntitiesFromDto(wire);
    expect(fe.find((e) => e.id === "l1")).toMatchObject({ type: "Line", p0: [0, 0], p1: [40, 0] });
    expect(fe.find((e) => e.id === "c1")).toMatchObject({ type: "Circle", center: [10, 10], radius: 3 });
  });

  it("classifies dimensional constraint types", () => {
    expect(isDimensional("Radius")).toBe(true);
    expect(isDimensional("Distance")).toBe(true);
    expect(isDimensional("Horizontal")).toBe(false);
    expect(isDimensional("Coincident")).toBe(false);
  });

  it("builds a DeleteSketch EditCommand with a bare SketchId string on `sketch`", () => {
    // Rust `EditCommand::DeleteSketch { sketch: SketchId }` — internally tagged
    // `cmd`, camelCase; SketchId serde is transparent over a uuid → a bare string.
    expect(buildDeleteSketch("11111111-2222-3333-4444-555555555555")).toEqual({
      cmd: "deleteSketch",
      sketch: "11111111-2222-3333-4444-555555555555",
    });
  });
});

// ── F-WP9: solvedPositions reverse map (backend point UUID → frontend geometry) ─

describe("frontendSolvedPositions — re-key backend point UUIDs", () => {
  it("maps backend point UUIDs to frontend entityId.Position keys", () => {
    const map = createIdMap("sk", "XY");
    marshalUpsert(map, { entities: [line("e1", [0, 0], [40, 0])], constraints: [] }, mint);
    // uuid-1 = e1.Start, uuid-2 = e1.End (from the Line marshal above).
    const out = frontendSolvedPositions(map, {
      "uuid-1": [5, 6],
      "uuid-2": [7, 8],
    });
    expect(out).toEqual({ "e1.Start": [5, 6], "e1.End": [7, 8] });
  });

  it("silently drops keys not in the id-map (re-entry / stale)", () => {
    const map = createIdMap("sk", "XY");
    marshalUpsert(map, { entities: [line("e1", [0, 0], [40, 0])], constraints: [] }, mint);
    const out = frontendSolvedPositions(map, { "not-a-mapped-uuid": [1, 1] });
    expect(out).toEqual({});
  });

  it("returns {} for null/undefined positions", () => {
    const map = createIdMap("sk", "XY");
    expect(frontendSolvedPositions(map, null)).toEqual({});
    expect(frontendSolvedPositions(map, undefined)).toEqual({});
  });
});

// ── Re-entry hydration: seed the id-map from the enter_sketch wire ────────────

describe("seedIdMapFromWire — hydrate the id-map on re-entry", () => {
  // The worker-wire shape `enter_sketch` returns (Point + p0Ref/p1Ref line, inlined
  // circle center); wire ids ARE the authoritative backend ids (mapped 1:1).
  const wireEntities = [
    { id: "p1", type: "Point", at: [0, 0] },
    { id: "p2", type: "Point", at: [40, 0] },
    { id: "l1", type: "Line", p0Ref: "p1", p1Ref: "p2" },
    { id: "cc", type: "Circle", center: [10, 10], radius: 3 },
  ];
  const wireConstraints = [
    { id: "k1", type: "Coincident", entities: ["p1", "p2"] },
    { id: "k2", type: "Radius", entities: ["cc"], value: 3 },
  ];

  it("maps entity/point/constraint ids 1:1 mirroring addEntityOps' key scheme", () => {
    const map = createIdMap("sk-backend", "XY");
    seedIdMapFromWire(map, wireEntities, wireConstraints);
    // Entities map to themselves (the wire id IS the backend id).
    expect(map.entity.get("l1")).toBe("l1");
    expect(map.entity.get("cc")).toBe("cc");
    expect(map.entity.get("p1")).toBe("p1");
    // Line endpoints resolve to the backend point ids (p0Ref/p1Ref).
    expect(map.point.get("l1.Start")).toBe("p1");
    expect(map.point.get("l1.End")).toBe("p2");
    // A Point entity keys under BOTH Start + Center → its own id (addEntityOps).
    expect(map.point.get("p1.Start")).toBe("p1");
    expect(map.point.get("p1.Center")).toBe("p1");
    // Constraints map 1:1 + their value primes the SetDimension cache.
    expect(map.constraint.get("k1")).toBe("k1");
    expect(map.constraint.get("k2")).toBe("k2");
    expect(map.constraintValue.get("k2")).toBe(3);
  });

  it("makes a re-marshal of the SAME hydrated arrays emit ZERO ops", () => {
    const map = createIdMap("sk-backend", "XY");
    seedIdMapFromWire(map, wireEntities, wireConstraints);
    const entities = frontendEntitiesFromDto(wireEntities);
    const constraints = frontendConstraintsFromDto(wireConstraints);
    expect(marshalUpsert(map, { entities, constraints }, mint)).toEqual([]);
  });

  it("marshals ONLY a newly-added entity after seeding (hydrated geometry untouched)", () => {
    const map = createIdMap("sk", "XY");
    seedIdMapFromWire(map, wireEntities, []);
    const entities = [...frontendEntitiesFromDto(wireEntities), circle("e9", [5, 5], 2)];
    const ops = marshalUpsert(map, { entities, constraints: [] }, mint);
    // Only e9's synthesized center point + the circle — nothing for p1/p2/l1/cc.
    expect(ops).toEqual([
      { op: "addEntity", entity: { kind: "point", id: "uuid-1", at: [5, 5] } },
      { op: "addEntity", entity: { kind: "circle", id: "uuid-2", center: "uuid-1", radius: 2 } },
    ]);
  });

  it("seeds point-ref keys so a gesture-style lookup finds the backend point id", () => {
    const map = createIdMap("sk", "XY");
    seedIdMapFromWire(map, wireEntities, []);
    // beginGesture resolves "l1.Start" → the backend point uuid (mirrors tauriClient).
    const dragPoint = map.point.get("l1.Start") ?? map.entity.get("l1.Start") ?? "l1.Start";
    expect(dragPoint).toBe("p1");
  });

  it("is a no-op for a fresh sketch (empty wire) and tolerates non-arrays", () => {
    const map = createIdMap("sk", "XY");
    seedIdMapFromWire(map, [], []);
    seedIdMapFromWire(map, null, undefined);
    expect(map.entity.size).toBe(0);
    expect(map.constraint.size).toBe(0);
  });
});

describe("applySolvedPositions — move geometry per entity kind", () => {
  it("moves a Line's endpoints (Start → p0, End → p1)", () => {
    const entities: SketchEntity[] = [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }];
    const out = applySolvedPositions(entities, { "e1.Start": [5, 6], "e1.End": [7, 8] });
    expect(out[0]).toMatchObject({ id: "e1", type: "Line", p0: [5, 6], p1: [7, 8] });
    // Immutable: a new array + entity, source untouched.
    expect(out).not.toBe(entities);
    expect(entities[0].p0).toEqual([0, 0]);
  });

  it("moves a Circle's center", () => {
    const entities: SketchEntity[] = [{ id: "e2", type: "Circle", center: [10, 10], radius: 3 }];
    const out = applySolvedPositions(entities, { "e2.Center": [12, 9] });
    expect(out[0]).toMatchObject({ type: "Circle", center: [12, 9], radius: 3 });
  });

  it("moves a Point's p0 (Start or Center key)", () => {
    const entities: SketchEntity[] = [{ id: "e3", type: "Point", p0: [1, 1] }];
    expect(applySolvedPositions(entities, { "e3.Center": [4, 5] })[0]).toMatchObject({ p0: [4, 5] });
  });

  it("returns the SAME array reference when nothing moved (no churn)", () => {
    const entities: SketchEntity[] = [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }];
    expect(applySolvedPositions(entities, {})).toBe(entities);
    // Positions for an unknown entity id are ignored.
    expect(applySolvedPositions(entities, { "eX.Start": [1, 2] })).toBe(entities);
  });

  it("round-trips with frontendSolvedPositions (client re-key → controller apply)", () => {
    const map = createIdMap("sk", "XY");
    marshalUpsert(map, { entities: [line("e1", [0, 0], [40, 0])], constraints: [] }, mint);
    const frontend = frontendSolvedPositions(map, { "uuid-1": [5, 6], "uuid-2": [7, 8] });
    const entities: SketchEntity[] = [{ id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] }];
    expect(applySolvedPositions(entities, frontend)[0]).toMatchObject({ p0: [5, 6], p1: [7, 8] });
  });
});
