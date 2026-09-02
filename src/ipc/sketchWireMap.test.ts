/*
 * sketchWireMap — the PURE frontend → Rust-typed sketch marshaller (F-WP9).
 * Deterministic ids (`uuid-N`) make the AddEntity/AddConstraint output assertable.
 */
import { beforeEach, describe, expect, it } from "vitest";
import {
  applySolvedCurves,
  applySolvedPositions,
  buildAddSketch,
  buildAddSketchOnDatum,
  buildDeleteSketch,
  cloneIdMap,
  createIdMap,
  frontendConflictingIds,
  frontendConstraintsFromDto,
  frontendEntitiesFromDto,
  frontendProjections,
  frontendSolvedCurves,
  frontendSolvedPositions,
  isDimensional,
  marshalUpsert,
  seedIdMapFromWire,
} from "./sketchWireMap";
import type { SketchConstraint, SketchEntity } from "./types";
import { __resetLogForTests, logSnapshot } from "@/debug/log";

let seq = 0;
const mint = () => `uuid-${++seq}`;
beforeEach(() => {
  seq = 0;
});

const line = (id: string, p0: [number, number], p1: [number, number]): SketchEntity => ({ id, type: "Line", p0, p1 });
const circle = (id: string, center: [number, number], radius: number): SketchEntity => ({ id, type: "Circle", center, radius });
const ellipse = (
  id: string,
  center: [number, number],
  majorR: number,
  minorR: number,
  rotation: number,
): SketchEntity => ({ id, type: "Ellipse", center, majorR, minorR, rotation });

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

  it("W0b: an arc-endpoint Coincident marshals as arc uuid + position", () => {
    const map = createIdMap("sk", "XY");
    const arcEntity: SketchEntity = { id: "e2", type: "Arc", center: [40, 0], radius: 5, start: [40, -5], end: [40, 5] };
    const ops = marshalUpsert(
      map,
      {
        entities: [line("e1", [0, 0], [40, 5]), arcEntity],
        // wall END welded to the arc's END.
        constraints: [{ id: "c1", type: "Coincident", entities: ["e1", "e2"], positions: ["End", "End"] }],
      },
      mint,
    );
    // e1: p uuid-1(Start), uuid-2(End), line uuid-3; e2: center uuid-4, arc uuid-5.
    const c = ops.find((o) => o.op === "addConstraint");
    expect(c).toEqual({
      op: "addConstraint",
      constraint: { kind: "coincident", id: "uuid-6", point1: "uuid-2", point2: "uuid-5", point2Position: "end" },
    });
    expect(map.constraint.has("c1")).toBe(true);
  });

  it("an arc endpoint on BOTH sides carries both position fields", () => {
    const map = createIdMap("sk", "XY");
    const a = (id: string): SketchEntity => ({ id, type: "Arc", center: [0, 0], radius: 5, start: [5, 0], end: [0, 5] });
    const ops = marshalUpsert(
      map,
      { entities: [a("e1"), a("e2")], constraints: [{ id: "c1", type: "Coincident", entities: ["e1", "e2"], positions: ["Start", "End"] }] },
      mint,
    );
    const c = ops.find((o) => o.op === "addConstraint");
    expect(c).toEqual({
      op: "addConstraint",
      constraint: { kind: "coincident", id: "uuid-5", point1: "uuid-2", point2: "uuid-4", point1Position: "start", point2Position: "end" },
    });
  });

  // SKETCH-V2 P3 flipped this. It used to assert the constraint was DROPPED —
  // the arc endpoint had no expressible wire form under any kind but Coincident,
  // so the user applied a Distance and nothing happened, silently. §7.3 now
  // generalizes the owner+role form to every point-taking slot.
  it("an arc Start/End is now marshalled under a kind OTHER than Coincident", () => {
    const map = createIdMap("sk", "XY");
    const arcEntity: SketchEntity = { id: "e1", type: "Arc", center: [0, 0], radius: 5, start: [5, 0], end: [0, 5] };
    const ops = marshalUpsert(
      map,
      { entities: [arcEntity, line("e2", [0, 0], [10, 0])], constraints: [{ id: "c1", type: "Distance", entities: ["e1", "e2"], positions: ["Start", "Start"], value: 4 }] },
      mint,
    );
    const c = ops.find((o) => o.op === "addConstraint");
    expect(c).toBeDefined();
    // The arc side rides as the ARC's uuid + a role; the line side is an
    // ordinary minted point and carries no role at all.
    expect(c).toMatchObject({
      op: "addConstraint",
      constraint: { kind: "distance", entity1Position: "start", value: { value: 4 } },
    });
    expect((c as { constraint: Record<string, unknown> }).constraint.entity2Position).toBeUndefined();
    expect(map.constraint.has("c1")).toBe(true);
  });

  it("a NON-arc point slot still emits no role (the wire shape is unchanged for it)", () => {
    const map = createIdMap("sk", "XY");
    const ops = marshalUpsert(
      map,
      {
        entities: [line("e1", [0, 0], [10, 0]), line("e2", [0, 5], [10, 5])],
        constraints: [{ id: "c1", type: "HorizontalDistance", entities: ["e1", "e2"], positions: ["Start", "End"], value: 4 }],
      },
      mint,
    );
    const c = ops.find((o) => o.op === "addConstraint") as { constraint: Record<string, unknown> };
    expect(c.constraint.kind).toBe("horizontalDistance");
    expect(c.constraint.point1Position).toBeUndefined();
    expect(c.constraint.point2Position).toBeUndefined();
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
    // `buildAddSketch` only ever emits the world arm — a real custom basis comes
    // from a datum or the backend's own `add_sketch_on_face`, so narrow before
    // reading `plane`.
    expect(custom.attachment.kind).toBe("world");
    if (custom.attachment.kind !== "world") throw new Error("expected a world attachment");
    expect(custom.attachment.plane).toBe("XY");
    // Canonical (non-standard) XY basis — must match Rust SketchPlane::xy().
    expect(custom.plane).toEqual({
      origin: [0, 0, 0],
      xAxis: [0, 1, 0],
      yAxis: [-1, 0, 0],
      normal: [0, 0, 1],
    });
  });

  it("buildAddSketchOnDatum pins the DATUM attachment wire shape + carries the frame verbatim", () => {
    // Rust `SketchAttachment` is internally tagged on "kind" with camelCase
    // variants + fields, so `Datum { datum: DatumPlaneId }` (a transparent UUID)
    // renders as {kind:"datum", datum:"<uuid>"}. A drift here is a hard serde
    // rejection at the command boundary, not a soft fallback — hence the pin.
    const plane = {
      origin: [0, 0, 10] as [number, number, number],
      xAxis: [0, 1, 0] as [number, number, number],
      yAxis: [-1, 0, 0] as [number, number, number],
      normal: [0, 0, 1] as [number, number, number],
    };
    expect(
      buildAddSketchOnDatum("sk-uuid", "Sketch 7", plane, "datum-uuid"),
    ).toEqual({
      cmd: "addSketch",
      sketch: {
        id: "sk-uuid",
        name: "Sketch 7",
        // The BACKEND-resolved datum frame, carried through untouched — no
        // re-derivation from basePlaneId + offset anywhere on this side.
        plane: { origin: [0, 0, 10], xAxis: [0, 1, 0], yAxis: [-1, 0, 0], normal: [0, 0, 1] },
        attachment: { kind: "datum", datum: "datum-uuid" },
      },
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

  it("REBASES: a prior session's frontend ids are dropped, never marshalled as removals", () => {
    // Session 1: the frontend authored `e1` (a line) and minted backend ids for it.
    const map = createIdMap("sk", "XY");
    marshalUpsert(map, { entities: [line("e1", [0, 0], [40, 0])], constraints: [] }, mint);
    expect(map.entity.has("e1")).toBe(true);

    // Session 2: `enter_sketch` returns that same geometry under its BACKEND ids.
    // Re-seeding must leave NO trace of `e1` — otherwise the next marshal reads it
    // as "mapped but absent from next" and emits removeEntity for the real uuid,
    // deleting everything the previous session drew.
    seedIdMapFromWire(map, wireEntities, wireConstraints);
    expect(map.entity.has("e1")).toBe(false);
    expect(map.point.has("e1.Start")).toBe(false);
    expect(map.backendSketchId).toBe("sk"); // session-independent fields survive

    const hydrated = frontendEntitiesFromDto(wireEntities);
    const ops = marshalUpsert(
      map,
      { entities: [...hydrated, line("e2", [80, 60], [120, 60])], constraints: [] },
      mint,
    );
    expect(ops.filter((o) => o.op === "removeEntity")).toEqual([]);
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

  it("W0b: keeps the wire's ARC-endpoint role while remapping the owned point", () => {
    // A welded slot on re-entry: slot 0 is a line's endpoint POINT (owner-derived
    // → l1/End), slot 1 is the ARC ITSELF with a role that exists ONLY in
    // `positions`. Dropping the latter would unweld every cap on reopen.
    const wire = [
      { id: "p1", type: "Point", at: [0, 0] as [number, number] },
      { id: "p2", type: "Point", at: [40, 0] as [number, number] },
      { id: "l1", type: "Line", p0Ref: "p1", p1Ref: "p2" },
      { id: "pc", type: "Point", at: [40, 5] as [number, number] },
      { id: "a1", type: "Arc", center: [40, 5] as [number, number], centerRef: "pc", radius: 5, startAngle: 0, endAngle: Math.PI },
    ];
    const [c] = frontendConstraintsFromDto(
      [{ id: "k1", type: "Coincident", entities: ["p2", "a1"], positions: ["", "End"] }],
      wire,
    );
    expect(c.entities).toEqual(["l1", "a1"]);
    expect(c.positions).toEqual(["End", "End"]);
  });

  it("wire positions stay INDEX-ALIGNED with entities when nothing is owned", () => {
    const [c] = frontendConstraintsFromDto([
      { id: "k1", type: "OnCurve", entities: ["p1", "a1"], positions: ["", "Start"] },
    ]);
    expect(c.positions).toEqual(["", "Start"]);
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
      else if (e.kind === "ellipse") entities.push({ id: e.id, type: "Ellipse", center: at.get(e.center), centerRef: e.center, majorR: e.majorR, minorR: e.minorR, rotation: e.rotation });
    } else if (op.op === "addConstraint") {
      const c = op.constraint;
      const type = PASCAL[c.kind];
      const rec: Record<string, unknown> = { id: c.id, type };
      if (c.kind === "coincident") {
        rec.entities = [c.point1, c.point2];
        // Mirrors Rust `wire_constraint`: `positions` rides ONLY when a slot names
        // an arc endpoint, and an `Arbitrary` slot is the EMPTY role.
        const role = (p?: "start" | "end") => (p === "start" ? "Start" : p === "end" ? "End" : "");
        if (c.point1Position || c.point2Position)
          rec.positions = [role(c.point1Position), role(c.point2Position)];
      }
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
        ellipse("el1", [-20, -30], 9, 4, 1.25),
      ],
      constraints: [
        { id: "k1", type: "Coincident", entities: ["l1", "l2"], positions: ["End", "Start"] },
        { id: "k2", type: "Horizontal", entities: ["l1"] },
        { id: "k3", type: "Angle", entities: ["l1", "l2"], value: 90 },
        { id: "k4", type: "Radius", entities: ["c1"], value: 3 },
        // W0b: a wall endpoint WELDED to an arc endpoint. Its `End` role lives
        // only in the wire's `positions`, so the hydration merge is what keeps the
        // weld from silently unbinding on re-entry (it would then re-marshal as a
        // brand-new constraint every reopen).
        { id: "k5", type: "Coincident", entities: ["l2", "a1"], positions: ["End", "Start"] },
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

    // No stray child points (2 lines + circle + arc + ellipse = 5 entities; no
    // endpoint/center dots).
    expect(entities).toHaveLength(5);
    // The Angle hydrated back to degrees.
    expect(constraints.find((c) => c.type === "Angle")?.value).toBeCloseTo(90, 10);
    // The arc-endpoint weld survived hydration WITH its roles: slot 0 remapped to
    // the OWNING line (owner-derived), slot 1 kept the arc + the wire-carried
    // Start. Hydrated constraints keep the BACKEND ids, so find it by shape.
    const [arcId, lineId] = [map1.entity.get("a1")!, map1.entity.get("l2")!];
    const weld = constraints.find((c) => c.type === "Coincident" && c.entities[1] === arcId)!;
    expect(weld.entities).toEqual([lineId, arcId]);
    expect(weld.positions).toEqual(["End", "Start"]);

    // 4. Re-marshal the hydrated arrays — a faithful reopen diffs to ZERO ops.
    expect(marshalUpsert(map2, { entities, constraints }, mint)).toEqual([]);
  });
});

// ── W3 P3: Ellipse marshals exactly like a Circle (one minted Center point) ──

describe("Ellipse — marshal / hydrate / seed", () => {
  it("mints a Center point then the core-serde `ellipse` AddEntity op", () => {
    const map = createIdMap("sk-uuid", "XY");
    const ops = marshalUpsert(
      map,
      { entities: [ellipse("e1", [3, 4], 10, 4, 1.5)], constraints: [] },
      mint,
    );
    expect(ops).toEqual([
      { op: "addEntity", entity: { kind: "point", id: "uuid-1", at: [3, 4] } },
      {
        op: "addEntity",
        entity: { kind: "ellipse", id: "uuid-2", center: "uuid-1", majorR: 10, minorR: 4, rotation: 1.5 },
      },
    ]);
    expect(map.entity.get("e1")).toBe("uuid-2");
    expect(map.point.get("e1.Center")).toBe("uuid-1");
  });

  it("defaults a missing rotation to 0 and carries the construction flag inline", () => {
    const map = createIdMap("sk-uuid", "XY");
    const e: SketchEntity = { id: "e1", type: "Ellipse", center: [0, 0], majorR: 5, minorR: 2, construction: true };
    const ops = marshalUpsert(map, { entities: [e], constraints: [] }, mint);
    expect(ops[1]).toEqual({
      op: "addEntity",
      entity: { kind: "ellipse", id: "uuid-2", center: "uuid-1", majorR: 5, minorR: 2, rotation: 0, construction: true },
    });
  });

  it("emits nothing for an ellipse missing a radius (never a half-formed op)", () => {
    const map = createIdMap("sk-uuid", "XY");
    const broken = { id: "e1", type: "Ellipse", center: [0, 0], majorR: 5 } as SketchEntity;
    expect(marshalUpsert(map, { entities: [broken], constraints: [] }, mint)).toEqual([]);
    expect(map.entity.has("e1")).toBe(false);
  });

  it("removes the ellipse AND its child centre point", () => {
    const map = createIdMap("sk-uuid", "XY");
    marshalUpsert(map, { entities: [ellipse("e1", [0, 0], 5, 2, 0)], constraints: [] }, mint);
    const ops = marshalUpsert(map, { entities: [], constraints: [] }, mint);
    expect(ops).toEqual([
      { op: "removeEntity", entity: "uuid-2" },
      { op: "removeEntity", entity: "uuid-1" },
    ]);
  });

  it("hydrates from the dto wire shape, keeping the centre point non-free-standing", () => {
    const wire = [
      { id: "p1", type: "Point", at: [3, 4] },
      { id: "el1", type: "Ellipse", center: [3, 4], centerRef: "p1", majorR: 10, minorR: 4, rotation: 1.5 },
    ];
    expect(frontendEntitiesFromDto(wire)).toEqual([
      { id: "el1", type: "Ellipse", center: [3, 4], majorR: 10, minorR: 4, rotation: 1.5, construction: undefined },
    ]);
  });

  it("drops an incomplete dto ellipse rather than hydrating a half entity", () => {
    expect(frontendEntitiesFromDto([{ id: "el1", type: "Ellipse", center: [0, 0], majorR: 3 }])).toEqual([]);
  });

  it("seeds `${id}.Center` from centerRef so the re-entry centre is draggable", () => {
    const map = createIdMap("sk", "XY");
    seedIdMapFromWire(
      map,
      [
        { id: "p1", type: "Point", at: [3, 4] },
        { id: "el1", type: "Ellipse", center: [3, 4], centerRef: "p1", majorR: 10, minorR: 4, rotation: 0 },
      ],
      [],
    );
    expect(map.entity.get("el1")).toBe("el1");
    expect(map.point.get("el1.Center")).toBe("p1");
    expect(map.entity.has("p1")).toBe(false); // owned child, not a free-standing entity
  });

  it("moves the centre on a solved Center position", () => {
    const moved = applySolvedPositions([ellipse("e1", [0, 0], 5, 2, 0.4)], { "e1.Center": [11, -7] });
    expect(moved[0].center).toEqual([11, -7]);
    expect(moved[0].majorR).toBe(5);
    expect(moved[0].rotation).toBe(0.4);
  });

  it("resolves a Fixed constraint's `at` from the ellipse centre", () => {
    const map = createIdMap("sk", "XY");
    const entities = [ellipse("e1", [8, 9], 5, 2, 0)];
    const ops = marshalUpsert(
      map,
      {
        entities,
        constraints: [{ id: "k1", type: "Fixed", entities: ["e1"], positions: ["Center"] }],
      },
      mint,
    );
    expect(ops[ops.length - 1]).toEqual({
      op: "addConstraint",
      constraint: { kind: "fixed", id: "uuid-3", point: "uuid-1", at: [8, 9] },
    });
  });
});

describe("frontendConflictingIds — reverse map.constraint (backend uuid → frontend id)", () => {
  it("maps known backend uuids to frontend ids and DROPS unknown", () => {
    const map = createIdMap("sk", "XY");
    map.constraint.set("c1", "uuid-c1");
    map.constraint.set("c2", "uuid-c2");
    // "uuid-x" is not in the map ⇒ dropped; order is preserved for the known ids.
    expect(frontendConflictingIds(map, ["uuid-c2", "uuid-x", "uuid-c1"])).toEqual(["c2", "c1"]);
  });

  it("returns [] for empty / undefined / all-unknown input", () => {
    const map = createIdMap("sk", "XY");
    map.constraint.set("c1", "uuid-c1");
    expect(frontendConflictingIds(map, undefined)).toEqual([]);
    expect(frontendConflictingIds(map, [])).toEqual([]);
    expect(frontendConflictingIds(map, ["nope"])).toEqual([]);
  });

  it("dedupes repeated ids", () => {
    const map = createIdMap("sk", "XY");
    map.constraint.set("c1", "uuid-c1");
    expect(frontendConflictingIds(map, ["uuid-c1", "uuid-c1"])).toEqual(["c1"]);
  });

  it("round-trips a re-entry sketch where frontend id == backend uuid", () => {
    // seedIdMapFromWire seeds map.constraint[raw.id] = raw.id (uuid == frontend id).
    const map = createIdMap("sk", "XY");
    seedIdMapFromWire(map, [], [{ id: "backend-uuid-1", type: "Horizontal", entities: ["e1"] }]);
    expect(frontendConflictingIds(map, ["backend-uuid-1"])).toEqual(["backend-uuid-1"]);
  });
});

// ── W1-B: construction flag transport (SetEntityConstruction) ─────────────────
//
// The id-only diff never noticed a flag change on an ALREADY-mapped entity, so
// flipping construction was silently lost on the wire. `entityConstruction` caches
// the last-SENT flag; these specs pin every touchpoint of that cache.

describe("marshalUpsert — construction flag", () => {
  const cline = (id: string, construction: boolean): SketchEntity => ({
    ...line(id, [0, 0], [40, 0]),
    construction,
  });

  it("carries the flag INLINE on the add (no separate op) and primes the cache", () => {
    const map = createIdMap("sk", "XY");
    const ops = marshalUpsert(map, { entities: [cline("e1", true)], constraints: [] }, mint);
    expect(ops.filter((o) => o.op === "setEntityConstruction")).toEqual([]);
    expect(ops[2]).toMatchObject({ op: "addEntity", entity: { kind: "line", construction: true } });
    expect(map.entityConstruction.get("e1")).toBe(true);
  });

  it("emits exactly ONE setEntityConstruction op naming the BACKEND uuid on a flip", () => {
    const map = createIdMap("sk", "XY");
    marshalUpsert(map, { entities: [line("e1", [0, 0], [40, 0])], constraints: [] }, mint);
    const backendId = map.entity.get("e1")!;

    const ops = marshalUpsert(map, { entities: [cline("e1", true)], constraints: [] }, mint);
    expect(ops).toEqual([{ op: "setEntityConstruction", entity: backendId, construction: true }]);
    expect(map.entityConstruction.get("e1")).toBe(true);
  });

  it("flips BACK to real geometry (construction: false)", () => {
    const map = createIdMap("sk", "XY");
    marshalUpsert(map, { entities: [cline("e1", true)], constraints: [] }, mint);
    const backendId = map.entity.get("e1")!;
    const ops = marshalUpsert(map, { entities: [line("e1", [0, 0], [40, 0])], constraints: [] }, mint);
    expect(ops).toEqual([{ op: "setEntityConstruction", entity: backendId, construction: false }]);
    expect(map.entityConstruction.get("e1")).toBe(false);
  });

  it("emits NOTHING when the flag is unchanged (either polarity, undefined == false)", () => {
    const map = createIdMap("sk", "XY");
    marshalUpsert(map, { entities: [cline("e1", true)], constraints: [] }, mint);
    expect(marshalUpsert(map, { entities: [cline("e1", true)], constraints: [] }, mint)).toEqual([]);

    const map2 = createIdMap("sk", "XY");
    marshalUpsert(map2, { entities: [line("e2", [0, 0], [40, 0])], constraints: [] }, mint);
    // `undefined` and an explicit `false` are the SAME state — no spurious op.
    expect(marshalUpsert(map2, { entities: [cline("e2", false)], constraints: [] }, mint)).toEqual([]);
  });

  it("flips only the entity that changed in a mixed batch", () => {
    const map = createIdMap("sk", "XY");
    const a = line("e1", [0, 0], [40, 0]);
    const b = circle("e2", [10, 10], 3);
    marshalUpsert(map, { entities: [a, b], constraints: [] }, mint);
    const ops = marshalUpsert(map, { entities: [a, { ...b, construction: true }], constraints: [] }, mint);
    expect(ops).toEqual([
      { op: "setEntityConstruction", entity: map.entity.get("e2")!, construction: true },
    ]);
  });

  it("drops the cache entry when the entity is removed (no stale flag on an id reuse)", () => {
    const map = createIdMap("sk", "XY");
    marshalUpsert(map, { entities: [cline("e1", true)], constraints: [] }, mint);
    expect(map.entityConstruction.get("e1")).toBe(true);

    marshalUpsert(map, { entities: [], constraints: [] }, mint);
    expect(map.entityConstruction.has("e1")).toBe(false);

    // Re-adding the SAME frontend id as REAL geometry must re-add inline, not flip.
    const ops = marshalUpsert(map, { entities: [line("e1", [0, 0], [40, 0])], constraints: [] }, mint);
    expect(ops.every((o) => o.op === "addEntity")).toBe(true);
    expect(map.entityConstruction.get("e1")).toBe(false);
  });

  it("re-entry: seedIdMapFromWire primes the flag, then a flip emits the right op", () => {
    const wire = [
      { id: "p1", type: "Point", at: [0, 0] },
      { id: "p2", type: "Point", at: [40, 0] },
      { id: "l1", type: "Line", p0Ref: "p1", p1Ref: "p2", construction: true },
      { id: "cc", type: "Circle", center: [10, 10], centerRef: "p3", radius: 3 },
      { id: "p3", type: "Point", at: [10, 10] },
    ];
    const map = createIdMap("sk", "XY");
    seedIdMapFromWire(map, wire, []);
    expect(map.entityConstruction.get("l1")).toBe(true);
    expect(map.entityConstruction.get("cc")).toBe(false);

    // The hydrated session carries the same flags ⇒ zero ops (no spurious flip).
    const entities = frontendEntitiesFromDto(wire);
    expect(marshalUpsert(map, { entities, constraints: [] }, mint)).toEqual([]);

    // Now flip the hydrated construction line back to real geometry.
    const flipped = entities.map((e) => (e.id === "l1" ? { ...e, construction: false } : e));
    expect(marshalUpsert(map, { entities: flipped, constraints: [] }, mint)).toEqual([
      { op: "setEntityConstruction", entity: "l1", construction: false },
    ]);
  });

  it("seedIdMapFromWire CLEARS a prior session's flags (no stale-map rebind)", () => {
    const map = createIdMap("sk", "XY");
    marshalUpsert(map, { entities: [cline("e1", true)], constraints: [] }, mint);
    seedIdMapFromWire(map, [{ id: "l9", type: "Line", p0Ref: "q1", p1Ref: "q2" }, { id: "q1", type: "Point", at: [0, 0] }, { id: "q2", type: "Point", at: [1, 1] }], []);
    expect(map.entityConstruction.has("e1")).toBe(false);
    expect(map.entityConstruction.get("l9")).toBe(false);
  });

  it("cloneIdMap copies the cache, so a REJECTED upsert leaves it at the last-SENT flag", () => {
    // Mirrors tauriClient.sketchUpsert: marshal against a clone, commit only on success.
    const live = createIdMap("sk", "XY");
    marshalUpsert(live, { entities: [line("e1", [0, 0], [40, 0])], constraints: [] }, mint);

    const rejected = cloneIdMap(live);
    marshalUpsert(rejected, { entities: [cline("e1", true)], constraints: [] }, mint);
    expect(rejected.entityConstruction.get("e1")).toBe(true);
    // The RPC threw ⇒ the clone is dropped; the live map must be untouched.
    expect(live.entityConstruction.get("e1")).toBe(false);

    // The retry therefore still emits the flip (nothing was silently swallowed).
    const retry = cloneIdMap(live);
    expect(marshalUpsert(retry, { entities: [cline("e1", true)], constraints: [] }, mint)).toEqual([
      { op: "setEntityConstruction", entity: live.entity.get("e1")!, construction: true },
    ]);
  });
});

// ── W0b: referenceLocked transport (SCHEMA §7.3) ──────────────────────────────
//
// Host-face projected geometry. NOTHING in the frontend authors it — it only
// ever arrives on the hydration wire — so these specs drive the read path and
// the two places the marshaller must refuse to author a mutating op against it.

describe("referenceLocked", () => {
  const wire = [
    { id: "p1", type: "Point", at: [0, 0] },
    { id: "p2", type: "Point", at: [40, 0] },
    { id: "lk", type: "Line", p0Ref: "p1", p1Ref: "p2", referenceLocked: true },
    { id: "fr", type: "Circle", center: [10, 10], centerRef: "p3", radius: 3 },
    { id: "p3", type: "Point", at: [10, 10] },
  ];

  it("hydration carries the flag onto the session, and its absence means false", () => {
    const entities = frontendEntitiesFromDto(wire);
    expect(entities.find((e) => e.id === "lk")!.referenceLocked).toBe(true);
    expect(entities.find((e) => e.id === "fr")!.referenceLocked).toBeUndefined();
  });

  it("seedIdMapFromWire latches the locked ids and clears a prior session's", () => {
    const map = createIdMap("sk", "XY");
    seedIdMapFromWire(map, wire, []);
    expect([...map.entityReferenceLocked]).toEqual(["lk"]);

    seedIdMapFromWire(map, [{ id: "q1", type: "Point", at: [0, 0] }], []);
    expect(map.entityReferenceLocked.size).toBe(0);
  });

  it("cloneIdMap copies the latch (a rejected upsert must not lose it)", () => {
    const map = createIdMap("sk", "XY");
    seedIdMapFromWire(map, wire, []);
    const clone = cloneIdMap(map);
    clone.entityReferenceLocked.delete("lk");
    expect([...map.entityReferenceLocked]).toEqual(["lk"]);
  });

  it("a hydrated locked sketch marshals to ZERO ops (no spurious flip or removal)", () => {
    const map = createIdMap("sk", "XY");
    seedIdMapFromWire(map, wire, []);
    expect(marshalUpsert(map, { entities: frontendEntitiesFromDto(wire), constraints: [] }, mint))
      .toEqual([]);
  });

  it("never authors a removal for a locked entity that fell out of the session", () => {
    const map = createIdMap("sk", "XY");
    seedIdMapFromWire(map, wire, []);
    // Drop EVERYTHING: the free circle goes (with its synthesized center child,
    // BUG-5), the locked line does not — endpoints included.
    const ops = marshalUpsert(map, { entities: [], constraints: [] }, mint);
    expect(ops).toEqual([
      { op: "removeEntity", entity: "fr" },
      { op: "removeEntity", entity: "p3" },
    ]);
    // …and the locked id stays mapped, so the next upsert is still coherent.
    expect(map.entity.get("lk")).toBe("lk");
    expect(map.point.get("lk.Start")).toBe("p1");
    expect(map.entity.has("fr")).toBe(false);
  });

  it("never authors a construction flip against a locked entity", () => {
    const map = createIdMap("sk", "XY");
    seedIdMapFromWire(map, wire, []);
    const flipped = frontendEntitiesFromDto(wire).map((e) =>
      e.id === "lk" ? { ...e, construction: true } : e,
    );
    expect(marshalUpsert(map, { entities: flipped, constraints: [] }, mint)).toEqual([]);
    // The FREE entity in the same batch still flips.
    const both = flipped.map((e) => (e.id === "fr" ? { ...e, construction: true } : e));
    expect(marshalUpsert(map, { entities: both, constraints: [] }, mint)).toEqual([
      { op: "setEntityConstruction", entity: "fr", construction: true },
    ]);
  });
});

// ── SP-2 W4: the `curves` channel (SCHEMA §7.4) ──────────────────────────────

describe("frontendSolvedCurves — re-key backend entity UUIDs", () => {
  it("maps backend entity UUIDs to frontend entity ids", () => {
    const map = createIdMap("sk", "XY");
    marshalUpsert(map, { entities: [circle("e1", [0, 0], 5)], constraints: [] }, mint);
    const uuid = map.entity.get("e1")!;
    expect(frontendSolvedCurves(map, { [uuid]: { radius: 9 } })).toEqual({ e1: { radius: 9 } });
  });

  it("drops keys not in the id-map, and returns {} for null/undefined", () => {
    const map = createIdMap("sk", "XY");
    marshalUpsert(map, { entities: [circle("e1", [0, 0], 5)], constraints: [] }, mint);
    expect(frontendSolvedCurves(map, { "not-a-mapped-uuid": { radius: 9 } })).toEqual({});
    expect(frontendSolvedCurves(map, null)).toEqual({});
    expect(frontendSolvedCurves(map, undefined)).toEqual({});
  });

  it("keys off map.entity, NOT map.point (a point uuid is not a curve)", () => {
    const map = createIdMap("sk", "XY");
    marshalUpsert(map, { entities: [circle("e1", [0, 0], 5)], constraints: [] }, mint);
    const centerUuid = map.point.get("e1.Center")!;
    expect(frontendSolvedCurves(map, { [centerUuid]: { radius: 9 } })).toEqual({});
  });
});

describe("frontendSolvedPositions — arc endpoint handles are expected, not unknown", () => {
  it("does NOT warn for `<uuid>.start`/`.end` keys (superseded by `curves`)", () => {
    __resetLogForTests();
    const map = createIdMap("sk", "XY");
    marshalUpsert(map, { entities: [line("e1", [0, 0], [40, 0])], constraints: [] }, mint);
    const out = frontendSolvedPositions(map, {
      "uuid-1": [5, 6],
      "arc-uuid.start": [1, 1],
      "arc-uuid.end": [2, 2],
    });
    expect(out).toEqual({ "e1.Start": [5, 6] }); // the handles still do not APPLY here
    expect(logSnapshot().filter((e) => e.level === "warn")).toEqual([]);
  });

  it("still warns for a genuinely unmapped point uuid", () => {
    __resetLogForTests();
    const map = createIdMap("sk", "XY");
    marshalUpsert(map, { entities: [line("e1", [0, 0], [40, 0])], constraints: [] }, mint);
    frontendSolvedPositions(map, { "orphan-uuid": [1, 1] });
    const warns = logSnapshot().filter((e) => e.level === "warn");
    expect(warns).toHaveLength(1);
    expect(warns[0].msg).toContain("1 unmapped point key(s)");
  });
});

describe("applySolvedCurves — reshape Circle/Arc from solved curve members", () => {
  const arc = (
    id: string,
    center: [number, number],
    radius: number,
    startAngle: number,
    endAngle: number,
  ): SketchEntity => {
    // Build it through the SAME derivation re-entry uses, so `start`/`end` are
    // exactly what the wire would have produced.
    const [built] = frontendEntitiesFromDto([{ id, type: "Arc", center, radius, startAngle, endAngle }]);
    return built;
  };

  it("applies a Circle radius", () => {
    const out = applySolvedCurves([circle("e1", [3, 4], 5)], { e1: { radius: 12.5 } });
    expect(out[0]).toMatchObject({ center: [3, 4], radius: 12.5 });
  });

  it("recomputes an Arc's start/end coords EXACTLY as re-entry derives them", () => {
    const before = arc("a1", [10, 10], 4, 0, Math.PI / 2);
    const out = applySolvedCurves([before], {
      a1: { radius: 7, startAngle: 0.3926, endAngle: 1.9634 },
    });
    // The oracle: hydrate the same (center, radius, angles) off the wire.
    const expected = arc("a1", [10, 10], 7, 0.3926, 1.9634);
    expect(out[0]).toEqual(expected);
    expect(out[0].center).toEqual([10, 10]); // the center is NOT a curve member
  });

  it("a radius-only report keeps the current angles (members are incremental)", () => {
    const before = arc("a1", [0, 0], 4, 0, Math.PI / 2);
    const out = applySolvedCurves([before], { a1: { radius: 10 } });
    expect(out[0]).toEqual(arc("a1", [0, 0], 10, 0, Math.PI / 2));
  });

  it("an angle-only report keeps the current radius", () => {
    const before = arc("a1", [0, 0], 4, 0, Math.PI / 2);
    const out = applySolvedCurves([before], { a1: { endAngle: Math.PI } });
    expect(out[0]).toEqual(arc("a1", [0, 0], 4, 0, Math.PI));
  });

  it("clamps a negative radius at 0 and drops non-finite members", () => {
    expect(applySolvedCurves([circle("e1", [0, 0], 5)], { e1: { radius: -3 } })[0].radius).toBe(0);
    const same = [circle("e1", [0, 0], 5)];
    expect(applySolvedCurves(same, { e1: { radius: Number.NaN } })).toBe(same);
    expect(applySolvedCurves(same, { e1: { radius: Number.POSITIVE_INFINITY } })).toBe(same);
  });

  it("returns the SAME array reference when nothing changed", () => {
    const entities = [circle("e1", [0, 0], 5), line("l1", [0, 0], [1, 1])];
    expect(applySolvedCurves(entities, {})).toBe(entities);
    expect(applySolvedCurves(entities, null)).toBe(entities);
    expect(applySolvedCurves(entities, { unknown: { radius: 9 } })).toBe(entities); // unknown id
    expect(applySolvedCurves(entities, { l1: { radius: 9 } })).toBe(entities); // non-curve
    expect(applySolvedCurves(entities, { e1: {} })).toBe(entities); // no members
  });

  it("leaves every other entity's object identity untouched", () => {
    const entities = [circle("e1", [0, 0], 5), line("l1", [0, 0], [1, 1])];
    const out = applySolvedCurves(entities, { e1: { radius: 6 } });
    expect(out).not.toBe(entities);
    expect(out[1]).toBe(entities[1]);
    expect(entities[0].radius).toBe(5); // input not mutated
  });

  it("round-trips with frontendSolvedCurves (client re-key → controller apply)", () => {
    const map = createIdMap("sk", "XY");
    const entities = [circle("e1", [0, 0], 5)];
    marshalUpsert(map, { entities, constraints: [] }, mint);
    const frontend = frontendSolvedCurves(map, { [map.entity.get("e1")!]: { radius: 8 } });
    expect(applySolvedCurves(entities, frontend)[0].radius).toBe(8);
  });
});

describe("frontendProjections (WP-P provenance re-key)", () => {
  const src = {
    sourceBodyId: "body_11111111-1111-1111-1111-111111111111",
    sourceElementId: "el-7",
    sourceKind: "edge" as const,
    sourceOrdinal: 0,
    projectedHash: "0123456789abcdef",
  };

  it("re-keys backend entity uuids to frontend ids through the id-map and drops unknown keys", () => {
    const map = createIdMap("sk-backend", "XY");
    map.entity.set("l1", "uuid-l1");
    expect(frontendProjections(map, { "uuid-l1": src, "uuid-ghost": src })).toEqual({ l1: src });
  });

  it("passes through without an id-map (never-entered sketch: frontend id == uuid)", () => {
    expect(frontendProjections(undefined, { "uuid-l1": src })).toEqual({ "uuid-l1": src });
  });

  it("keeps an absent map absent (Rust omits the key when empty)", () => {
    expect(frontendProjections(undefined, undefined)).toBeUndefined();
    expect(frontendProjections(createIdMap("sk", "XY"), null)).toBeUndefined();
  });
});
