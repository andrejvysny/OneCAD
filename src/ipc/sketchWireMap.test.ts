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

  it("emits RemoveEntity for a leaving entity AND its synthesized child points", () => {
    const map = createIdMap("sk", "XY");
    marshalUpsert(map, { entities: [line("e1", [0, 0], [40, 0])], constraints: [] }, mint);
    const lineId = map.entity.get("e1")!; // uuid-3
    const startId = map.point.get("e1.Start")!; // uuid-1
    const endId = map.point.get("e1.End")!; // uuid-2
    const ops = marshalUpsert(map, { entities: [], constraints: [] }, mint);
    // BUG-5: the line AND its two endpoint points are removed (no orphan dots).
    expect(ops).toEqual([
      { op: "removeEntity", entity: lineId },
      { op: "removeEntity", entity: startId },
      { op: "removeEntity", entity: endId },
    ]);
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

  it("BUG-5: owns line endpoints / circle centers, hydrates only free-standing Points", () => {
    const wire = [
      { id: "p1", type: "Point", at: [0, 0] }, // l1 endpoint (owned)
      { id: "p2", type: "Point", at: [40, 0] }, // l1 endpoint (owned)
      { id: "pc", type: "Point", at: [10, 10] }, // circle center (owned, via centerRef)
      { id: "pf", type: "Point", at: [99, 99] }, // genuinely free-standing
      { id: "l1", type: "Line", p0Ref: "p1", p1Ref: "p2" },
      { id: "c1", type: "Circle", center: [10, 10], centerRef: "pc", radius: 3 },
    ];
    const fe = frontendEntitiesFromDto(wire);
    const ids = fe.map((e) => e.id).sort();
    // Only the line, the circle, and the ONE free point survive — no stray dots.
    expect(ids).toEqual(["c1", "l1", "pf"]);
    expect(fe.find((e) => e.id === "pf")).toMatchObject({ type: "Point", p0: [99, 99] });
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
    // BUG-5: p1/p2 are l1's endpoints (owned child points) — NOT free-standing
    // entities, so they are seeded ONLY under their owner's `${l1}.Start/.End` keys.
    expect(map.entity.has("p1")).toBe(false);
    expect(map.entity.has("p2")).toBe(false);
    // Line endpoints resolve to the backend point ids (p0Ref/p1Ref).
    expect(map.point.get("l1.Start")).toBe("p1");
    expect(map.point.get("l1.End")).toBe("p2");
    // An owned child point is NOT keyed under its own id (only via the owner).
    expect(map.point.has("p1.Start")).toBe(false);
    expect(map.point.has("p1.Center")).toBe(false);
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

// ── BUG-2: Angle canonicalization (UI degrees ↔ wire radians) at all 3 sites ──

const arc = (id: string, center: [number, number], radius: number, start: [number, number], end: [number, number]): SketchEntity => ({ id, type: "Arc", center, radius, start, end });
const HALF_PI = Math.PI / 2;
const QUARTER_PI = Math.PI / 4;

describe("angle units — degrees UI, radians wire", () => {
  const twoLines: SketchEntity[] = [line("l1", [0, 0], [40, 0]), line("l2", [40, 0], [40, 40])];
  const angle = (v: number): SketchConstraint => ({ id: "ca", type: "Angle", entities: ["l1", "l2"], value: v });

  it("SITE 1 — toWireConstraint emits an Angle in RADIANS (90° → π/2)", () => {
    const map = createIdMap("sk", "XY");
    const ops = marshalUpsert(map, { entities: twoLines, constraints: [angle(90)] }, mint);
    const add = ops.find((o) => o.op === "addConstraint");
    expect(add).toMatchObject({ constraint: { kind: "angle" } });
    // @ts-expect-error narrow to the angle wire shape for the value assertion
    expect(add.constraint.value.value).toBeCloseTo(HALF_PI, 12);
  });

  it("SITE 2 — the SetDimension diff converts an in-place Angle edit (45° → π/4)", () => {
    const map = createIdMap("sk", "XY");
    marshalUpsert(map, { entities: twoLines, constraints: [angle(90)] }, mint);
    const ops = marshalUpsert(map, { entities: twoLines, constraints: [angle(45)] }, mint);
    expect(ops).toHaveLength(1);
    expect(ops[0].op).toBe("setDimension");
    // @ts-expect-error setDimension carries a WireScalar value
    expect(ops[0].value.value).toBeCloseTo(QUARTER_PI, 12);
  });

  it("SITE 3 — frontendConstraintsFromDto converts a wire Angle to DEGREES (π/2 → 90°)", () => {
    const [c] = frontendConstraintsFromDto([
      { id: "ca", type: "Angle", entities: ["l1", "l2"], value: HALF_PI },
    ]);
    expect(c.value).toBeCloseTo(90, 10);
  });

  it("a non-angular dimension (Radius mm) passes through both directions unchanged", () => {
    const map = createIdMap("sk", "XY");
    const ops = marshalUpsert(map, { entities: [circle("c1", [0, 0], 5)], constraints: [{ id: "cr", type: "Radius", entities: ["c1"], value: 5 }] }, mint);
    // @ts-expect-error radius wire value
    expect(ops.find((o) => o.op === "addConstraint").constraint.value.value).toBe(5);
    const [c] = frontendConstraintsFromDto([{ id: "cr", type: "Radius", entities: ["c1"], value: 5 }]);
    expect(c.value).toBe(5);
  });

  it("seedIdMapFromWire caches the Angle in the UI domain (no spurious SetDimension)", () => {
    const map = createIdMap("sk-backend", "XY");
    const wireEntities = [
      { id: "l1", type: "Line", p0Ref: "p1", p1Ref: "p2" },
      { id: "l2", type: "Line", p0Ref: "p2", p1Ref: "p3" },
      { id: "p1", type: "Point", at: [0, 0] },
      { id: "p2", type: "Point", at: [40, 0] },
      { id: "p3", type: "Point", at: [40, 40] },
    ];
    const wireConstraints = [{ id: "ca", type: "Angle", entities: ["l1", "l2"], value: HALF_PI }];
    seedIdMapFromWire(map, wireEntities, wireConstraints);
    // The cache holds DEGREES, so re-marshalling the hydrated (degrees) value is a no-op.
    expect(map.constraintValue.get("ca")).toBeCloseTo(90, 10);
    const constraints = frontendConstraintsFromDto(wireConstraints, wireEntities);
    const entities = frontendEntitiesFromDto(wireEntities);
    expect(marshalUpsert(map, { entities, constraints }, mint)).toEqual([]);
  });
});

// ── BUG-5: circle/arc center ownership on re-entry (centerRef) + constraint remap ─

describe("re-entry ownership — centerRef seeds .Center, constraints remap to owner", () => {
  it("seeds a circle/arc `.Center` from centerRef (center becomes draggable)", () => {
    const map = createIdMap("sk", "XY");
    const wire = [
      { id: "pc", type: "Point", at: [10, 10] },
      { id: "pa", type: "Point", at: [60, 10] },
      { id: "c1", type: "Circle", center: [10, 10], centerRef: "pc", radius: 3 },
      { id: "a1", type: "Arc", center: [60, 10], centerRef: "pa", radius: 5, startAngle: 0, endAngle: 1 },
    ];
    seedIdMapFromWire(map, wire, []);
    // The center points are owned → not entities; `.Center` resolves to the backend uuid.
    expect(map.entity.has("pc")).toBe(false);
    expect(map.point.get("c1.Center")).toBe("pc");
    expect(map.point.get("a1.Center")).toBe("pa");
    // A gesture-style lookup (tauriClient) finds the backend center point to drag.
    expect(map.point.get("c1.Center") ?? map.entity.get("c1.Center") ?? "c1.Center").toBe("pc");
  });

  it("remaps a wire Coincident (point uuids) to owner entity + position", () => {
    // Each line owns its OWN endpoints (marshaller shape); the shared corner is two
    // distinct points joined by Coincident (l1.End == l2.Start).
    const wire = [
      { id: "p1", type: "Point", at: [0, 0] },
      { id: "p2", type: "Point", at: [40, 0] }, // l1.End
      { id: "p3", type: "Point", at: [40, 0] }, // l2.Start (coincident with p2)
      { id: "p4", type: "Point", at: [40, 40] },
      { id: "l1", type: "Line", p0Ref: "p1", p1Ref: "p2" },
      { id: "l2", type: "Line", p0Ref: "p3", p1Ref: "p4" },
    ];
    const [c] = frontendConstraintsFromDto(
      [{ id: "k1", type: "Coincident", entities: ["p2", "p3"] }],
      wire,
    );
    expect(c.entities).toEqual(["l1", "l2"]);
    expect(c.positions).toEqual(["End", "Start"]);
  });

  it("without entities, constraint refs pass through verbatim (backward compatible)", () => {
    const [c] = frontendConstraintsFromDto([{ id: "k1", type: "Coincident", entities: ["p1", "p2"] }]);
    expect(c.entities).toEqual(["p1", "p2"]);
    expect(c.positions).toBeUndefined();
  });
});

// ── Full hydration round-trip (BUG-5 + BUG-2): fresh → wire → hydrate → 0 diff ──

const PASCAL: Record<string, string> = {
  coincident: "Coincident", horizontal: "Horizontal", vertical: "Vertical",
  midpoint: "Midpoint", parallel: "Parallel", perpendicular: "Perpendicular",
  equal: "Equal", distance: "Distance", horizontalDistance: "HorizontalDistance",
  verticalDistance: "VerticalDistance", angle: "Angle", radius: "Radius", diameter: "Diameter",
};

/** Rebuild the `enter_sketch` wire (entities + constraints) from marshalled ops —
 *  mirrors the Rust `wire_entity`/`wire_constraint`: lines carry p0Ref/p1Ref,
 *  circle/arc inline center coords + a `centerRef`, constraints carry uuid refs. */
function wireFromOps(ops: ReturnType<typeof marshalUpsert>) {
  const at = new Map<string, [number, number]>();
  const entities: unknown[] = [];
  const constraints: unknown[] = [];
  for (const op of ops) {
    if (op.op === "addEntity") {
      const e = op.entity;
      if (e.kind === "point") { at.set(e.id, e.at); entities.push({ id: e.id, type: "Point", at: e.at }); }
      else if (e.kind === "line") entities.push({ id: e.id, type: "Line", p0Ref: e.start, p1Ref: e.end });
      else if (e.kind === "circle") entities.push({ id: e.id, type: "Circle", center: at.get(e.center), centerRef: e.center, radius: e.radius });
      else if (e.kind === "arc") entities.push({ id: e.id, type: "Arc", center: at.get(e.center), centerRef: e.center, radius: e.radius, startAngle: e.startAngle, endAngle: e.endAngle });
    } else if (op.op === "addConstraint") {
      const c = op.constraint;
      const type = PASCAL[c.kind];
      const rec: Record<string, unknown> = { id: c.id, type };
      if (c.kind === "coincident") rec.entities = [c.point1, c.point2];
      else if (c.kind === "horizontal" || c.kind === "vertical") rec.entities = [c.line];
      else if (c.kind === "angle") { rec.entities = [c.line1, c.line2]; rec.value = c.value.value; }
      else if (c.kind === "radius" || c.kind === "diameter") { rec.entities = [c.entity]; rec.value = c.value.value; }
      else if (c.kind === "distance") { rec.entities = [c.entity1, c.entity2]; rec.value = c.value.value; }
      else if (c.kind === "horizontalDistance" || c.kind === "verticalDistance") { rec.entities = [c.point1, c.point2]; rec.value = c.value.value; }
      constraints.push(rec);
    }
  }
  return { entities, constraints };
}

describe("hydration round-trip — marshal fresh → wire → hydrate → re-marshal is 0 ops", () => {
  it("line + circle + arc + Coincident/Horizontal/Angle/Radius diffs to nothing", () => {
    const fresh = {
      entities: [
        line("l1", [0, 0], [40, 0]),
        line("l2", [40, 0], [40, 40]),
        circle("c1", [10, 20], 3),
        arc("a1", [60, 10], 5, [65, 10], [60, 15]),
      ],
      constraints: [
        { id: "k1", type: "Coincident", entities: ["l1", "l2"], positions: ["End", "Start"] },
        { id: "k2", type: "Horizontal", entities: ["l1"] },
        { id: "k3", type: "Angle", entities: ["l1", "l2"], value: 90 },
        { id: "k4", type: "Radius", entities: ["c1"], value: 3 },
      ] as SketchConstraint[],
    };
    // 1. Marshal the fresh sketch (mints backend uuids into map1).
    const map1 = createIdMap("sk-backend", "XY");
    const ops = marshalUpsert(map1, fresh, mint);
    expect(ops.length).toBeGreaterThan(0);

    // 2. Simulate the enter_sketch wire coming back with those uuids.
    const wire = wireFromOps(ops);

    // 3. Hydrate into the frontend session (fresh map2, as after a reopen).
    const map2 = createIdMap("sk-backend", "XY");
    const entities = frontendEntitiesFromDto(wire.entities);
    const constraints = frontendConstraintsFromDto(wire.constraints, wire.entities);
    seedIdMapFromWire(map2, wire.entities, wire.constraints);

    // No stray child points (2 lines + circle + arc = 4 entities; no endpoint/center dots).
    expect(entities).toHaveLength(4);
    // The Angle hydrated back to degrees.
    expect(constraints.find((c) => c.type === "Angle")?.value).toBeCloseTo(90, 10);

    // 4. Re-marshal the hydrated arrays — a faithful reopen diffs to ZERO ops.
    expect(marshalUpsert(map2, { entities, constraints }, mint)).toEqual([]);
  });
});
