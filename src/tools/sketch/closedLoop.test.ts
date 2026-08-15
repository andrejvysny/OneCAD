/*
 * closedLoop — CHARACTERIZATION pin, written before the topology migration
 * touches this module (SKETCH-V2 P0).
 *
 * `findClosedLoops` is today the fourth independent re-implementation of vertex
 * adjacency in the frontend: it welds endpoints by QUANTIZED COORDINATE
 * (`key()`, EPS 1e-6) and builds its own adjacency map + degree count, because
 * no shared point topology exists. P4 replaces that with graph lookups. These
 * tests pin the behaviour that must survive that swap — in particular the
 * degree->2 branch exclusion, the `chain.length >= 3` closure minimum, and the
 * `entities.length + 1` walk guard.
 *
 * Two pinned behaviours are LIMITATIONS, not contracts, and are labelled as
 * such: a two-entity closed profile (line + arc) is not detected, and
 * `loopCentroid` assumes every entity is stored in walk order. Both are
 * expected to change under shared topology; they are pinned so the change is
 * visible and deliberate rather than silent.
 */
import { describe, it, expect } from "vitest";
import type { SketchEntity } from "@/ipc/types";
import { findClosedLoops, loopCentroid } from "./closedLoop";

const line = (id: string, p0: [number, number], p1: [number, number]): SketchEntity => ({
  id,
  type: "Line",
  p0,
  p1,
});
const arc = (
  id: string,
  center: [number, number],
  radius: number,
  start: [number, number],
  end: [number, number],
): SketchEntity => ({ id, type: "Arc", center, radius, start, end });
const circle = (id: string, center: [number, number], radius: number): SketchEntity => ({
  id,
  type: "Circle",
  center,
  radius,
});
const point = (id: string, p0: [number, number]): SketchEntity => ({ id, type: "Point", p0 });

/** The canonical 10×10 rectangle, every line stored head-to-tail. */
const rect = (): SketchEntity[] => [
  line("l1", [0, 0], [10, 0]),
  line("l2", [10, 0], [10, 10]),
  line("l3", [10, 10], [0, 10]),
  line("l4", [0, 10], [0, 0]),
];

const ids = (e: SketchEntity[]) => new Map(e.map((x) => [x.id, x]));

describe("findClosedLoops — closure", () => {
  it("a rectangle is one loop of four, in walk order", () => {
    const loops = findClosedLoops(rect());
    expect(loops).toHaveLength(1);
    expect(loops[0].entityIds).toEqual(["l1", "l2", "l3", "l4"]);
  });

  it("a triangle closes — three entities is the minimum", () => {
    const loops = findClosedLoops([
      line("a", [0, 0], [10, 0]),
      line("b", [10, 0], [5, 8]),
      line("c", [5, 8], [0, 0]),
    ]);
    expect(loops).toHaveLength(1);
    expect(loops[0].entityIds).toHaveLength(3);
  });

  it("an open chain yields no loop", () => {
    expect(findClosedLoops([line("a", [0, 0], [10, 0]), line("b", [10, 0], [10, 10])])).toEqual([]);
  });

  it("two disjoint closed loops are both found", () => {
    const far = rect().map((e) => ({
      ...e,
      id: `f${e.id}`,
      p0: [e.p0![0] + 100, e.p0![1]] as [number, number],
      p1: [e.p1![0] + 100, e.p1![1]] as [number, number],
    }));
    const loops = findClosedLoops([...rect(), ...far]);
    expect(loops).toHaveLength(2);
    expect(loops.map((l) => l.entityIds.length)).toEqual([4, 4]);
  });

  it("an open chain preceding a closed loop does not consume it (failed chains release)", () => {
    // The open chain walks first, marks its entities visited, fails to close and
    // must release them — otherwise a later start could be starved.
    const loops = findClosedLoops([
      line("open1", [-50, 0], [-40, 0]),
      line("open2", [-40, 0], [-40, 10]),
      ...rect(),
    ]);
    expect(loops).toHaveLength(1);
    expect(loops[0].entityIds).toEqual(["l1", "l2", "l3", "l4"]);
  });
});

describe("findClosedLoops — the chain.length >= 3 minimum", () => {
  it("two lines sharing BOTH endpoints do not close (a 2-gon is rejected)", () => {
    expect(findClosedLoops([line("a", [0, 0], [10, 0]), line("b", [0, 0], [10, 0])])).toEqual([]);
  });

  it("LIMITATION: a line + arc closed profile is not detected (only 2 entities)", () => {
    // A D-shape is a genuine closed profile, but the walk needs >= 3 entities.
    // Shared topology should make this expressible; pinned so the fix is visible.
    expect(
      findClosedLoops([line("l", [0, 0], [10, 0]), arc("a", [5, 0], 5, [10, 0], [0, 0])]),
    ).toEqual([]);
  });

  it("an arc closes a three-entity loop", () => {
    const loops = findClosedLoops([
      line("l1", [0, 0], [10, 0]),
      line("l2", [10, 0], [10, 5]),
      arc("a1", [5, 5], 5, [10, 5], [0, 0]),
    ]);
    expect(loops).toHaveLength(1);
    expect(loops[0].entityIds).toHaveLength(3);
  });
});

describe("findClosedLoops — branch exclusion (degree > 2)", () => {
  it("a T-junction excludes every entity touching it", () => {
    expect(
      findClosedLoops([
        line("a", [0, 0], [10, 0]),
        line("b", [10, 0], [20, 0]),
        line("c", [10, 0], [10, 10]),
      ]),
    ).toEqual([]);
  });

  it("a rectangle with a tail is fully excluded — the tail makes a corner degree 3", () => {
    // No entity through a degree-3 vertex has a unique next hop, so the two
    // rectangle sides meeting the tail are excluded too, and the loop is lost.
    expect(findClosedLoops([...rect(), line("tail", [0, 0], [-10, -10])])).toEqual([]);
  });

  it("a loop whose vertices all stay degree 2 survives a nearby unrelated chain", () => {
    const loops = findClosedLoops([...rect(), line("t", [50, 50], [60, 60])]);
    expect(loops).toHaveLength(1);
    expect(loops[0].entityIds).toEqual(["l1", "l2", "l3", "l4"]);
  });
});

describe("findClosedLoops — entity eligibility", () => {
  it("only Line and Arc participate; Circle, Point and Ellipse are ignored", () => {
    const loops = findClosedLoops([
      ...rect(),
      circle("c1", [5, 5], 2),
      point("p1", [0, 0]),
      { id: "el1", type: "Ellipse", center: [5, 5], majorR: 3, minorR: 1, rotation: 0 },
    ]);
    expect(loops).toHaveLength(1);
    expect(loops[0].entityIds).toEqual(["l1", "l2", "l3", "l4"]);
  });

  it("a Circle alone is not a loop (it has no chain endpoints)", () => {
    expect(findClosedLoops([circle("c1", [0, 0], 5)])).toEqual([]);
  });

  it("a Line missing an endpoint field is skipped", () => {
    expect(findClosedLoops([{ id: "l1", type: "Line", p0: [0, 0] }])).toEqual([]);
  });

  it("an Arc missing start/end is skipped even though it has a center", () => {
    expect(findClosedLoops([{ id: "a1", type: "Arc", center: [0, 0], radius: 5 }])).toEqual([]);
  });
});

describe("findClosedLoops — coordinate welding tolerance (EPS 1e-6 quantization)", () => {
  it("corners off by 1e-9 still weld into one loop", () => {
    const loops = findClosedLoops([
      line("l1", [0, 0], [10, 0]),
      line("l2", [10 + 1e-9, 0], [10, 10]),
      line("l3", [10, 10], [0, 10]),
      line("l4", [0, 10], [0, 0]),
    ]);
    expect(loops).toHaveLength(1);
  });

  it("corners off by 1e-3 do not weld — the chain stays open", () => {
    expect(
      findClosedLoops([
        line("l1", [0, 0], [10, 0]),
        line("l2", [10 + 1e-3, 0], [10, 10]),
        line("l3", [10, 10], [0, 10]),
        line("l4", [0, 10], [0, 0]),
      ]),
    ).toEqual([]);
  });
});

describe("loopCentroid", () => {
  it("a rectangle loop centroids at its middle", () => {
    const entities = rect();
    const [loop] = findClosedLoops(entities);
    expect(loopCentroid(loop, ids(entities))).toEqual([5, 5]);
  });

  it("returns null when an entity id is absent from the map", () => {
    const [loop] = findClosedLoops(rect());
    expect(loopCentroid(loop, ids([]))).toBeNull();
  });

  it("returns null for a ring of fewer than three points", () => {
    expect(loopCentroid({ entityIds: ["p1"] }, ids([point("p1", [0, 0])]))).toBeNull();
  });
});
