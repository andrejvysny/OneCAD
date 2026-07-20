import { describe, it, expect, beforeEach } from "vitest";
import { commitDimensionConstraint, deleteConstraints, deleteEntities } from "./sketchService";
import { mockClient, resetMockSketches } from "@/ipc/mockClient";
import { planeFor } from "@/ipc/mockSketch";
import { sketchStore } from "@/stores/sketchStore";
import { viewportStore } from "@/stores/viewportStore";
import type { CadClient } from "@/ipc/client";
import type { SketchConstraint, SketchEntity, SketchSession } from "@/ipc/types";

const line: SketchEntity = { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] };

function seedSession(constraints: SketchConstraint[]): SketchSession {
  const s: SketchSession = {
    sketchId: "sk-dim",
    plane: planeFor("XY"),
    entities: [line],
    constraints,
    dof: 0,
    status: "UnderConstrained",
  };
  sketchStore.getState().setSession(s);
  return s;
}

const distance: SketchConstraint = {
  id: "d1",
  type: "Distance",
  entities: ["e1", "e1"],
  positions: ["Start", "End"],
  value: 40,
};

describe("commitDimensionConstraint — solver round-trip + reject-on-conflict", () => {
  beforeEach(() => {
    resetMockSketches();
    sketchStore.getState().reset();
  });

  it("accepts a dimension that keeps the sketch solvable (under-constrained)", async () => {
    // One Coincident removes 2 of the line's 4 DOF; a Distance removes 1 more ⇒ 1 DOF.
    seedSession([{ id: "c1", type: "Coincident", entities: ["e1", "e1"], positions: ["Start", "End"] }]);
    const { rejected } = await commitDimensionConstraint(mockClient, distance);
    expect(rejected).toBe(false);
    const s = sketchStore.getState().session!;
    expect(s.constraints.some((c) => c.id === "d1")).toBe(true);
    expect(s.dof).toBe(1);
    expect(s.status).toBe("UnderConstrained");
  });

  it("rejects + auto-undoes a dimension that over-constrains the sketch", async () => {
    // Two Coincidents remove all 4 DOF (fully constrained); a Distance ⇒ −1 ⇒ over.
    seedSession([
      { id: "c1", type: "Coincident", entities: ["e1", "e1"], positions: ["Start", "End"] },
      { id: "c2", type: "Coincident", entities: ["e1", "e1"], positions: ["End", "Start"] },
    ]);
    const { rejected } = await commitDimensionConstraint(mockClient, distance);
    expect(rejected).toBe(true);
    const s = sketchStore.getState().session!;
    // The dimension was removed; the sketch reverts to its prior (fully) state.
    expect(s.constraints.some((c) => c.id === "d1")).toBe(false);
    expect(s.constraints).toHaveLength(2);
    expect(s.status).toBe("FullyConstrained");
    expect(s.dof).toBe(0);
  });

  it("is a no-op with no active session", async () => {
    const { rejected } = await commitDimensionConstraint(mockClient, distance);
    expect(rejected).toBe(false);
    expect(sketchStore.getState().session).toBeNull();
  });
});

// ── deletion (entities + constraints, mirroring the Rust cascade) ─────────────

const lineA: SketchEntity = { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] };
const lineB: SketchEntity = { id: "e2", type: "Line", p0: [40, 0], p1: [40, 40] };
const circle: SketchEntity = { id: "e3", type: "Circle", center: [80, 0], radius: 10 };

function seed(entities: SketchEntity[], constraints: SketchConstraint[]): void {
  sketchStore.getState().setSession({
    sketchId: "sk-del",
    plane: planeFor("XY"),
    entities,
    constraints,
    dof: 0,
    status: "UnderConstrained",
  });
}

describe("deleteEntities — cascade the referencing constraints", () => {
  beforeEach(() => {
    resetMockSketches();
    sketchStore.getState().reset();
  });

  it("drops the entity + every constraint referencing it; unrelated survive", async () => {
    seed(
      [lineA, lineB, circle],
      [
        { id: "c1", type: "Radius", entities: ["e3"], value: 10 }, // refs e3 (doomed)
        { id: "c2", type: "Coincident", entities: ["e1", "e3"], positions: ["Start", "Center"] }, // refs e3
        { id: "c3", type: "Horizontal", entities: ["e1"] }, // unrelated
      ],
    );
    await deleteEntities(mockClient, ["e3"]);
    const s = sketchStore.getState().session!;
    expect(s.entities.map((e) => e.id)).toEqual(["e1", "e2"]);
    expect(s.constraints.map((c) => c.id)).toEqual(["c3"]);
    // e1(4)+e2(4) free − 1 (Horizontal) = 7 dof.
    expect(s.dof).toBe(7);
    expect(s.status).toBe("UnderConstrained");
  });

  it("drops a child-point-ref constraint (line Start↔End Distance) with its line", async () => {
    seed(
      [lineA, lineB],
      [
        // A self-referential Distance on e1 via {entityId:e1, position} refs — the
        // entities-array form (["e1","e1"]) is what the cascade predicate matches.
        { id: "c1", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 40 },
        { id: "c2", type: "Horizontal", entities: ["e2"] }, // unrelated
      ],
    );
    await deleteEntities(mockClient, ["e1"]);
    const s = sketchStore.getState().session!;
    expect(s.entities.map((e) => e.id)).toEqual(["e2"]);
    expect(s.constraints.map((c) => c.id)).toEqual(["c2"]);
  });

  it("is a no-op on empty ids", async () => {
    seed([lineA], [{ id: "c1", type: "Horizontal", entities: ["e1"] }]);
    await deleteEntities(mockClient, []);
    const s = sketchStore.getState().session!;
    expect(s.entities).toHaveLength(1);
    expect(s.constraints).toHaveLength(1);
  });

  it("is a no-op when no live id matches", async () => {
    seed([lineA], []);
    const before = sketchStore.getState().session;
    await deleteEntities(mockClient, ["nope"]);
    expect(sketchStore.getState().session).toBe(before); // same reference: untouched
  });

  it("is a no-op with no active session", async () => {
    await deleteEntities(mockClient, ["e1"]);
    expect(sketchStore.getState().session).toBeNull();
  });

  it("surfaces a status hint and leaves state intact on upsert failure", async () => {
    seed([lineA, lineB], []);
    const before = sketchStore.getState().session;
    const failing = {
      sketchUpsert: () => Promise.reject(new Error("boom")),
    } as unknown as CadClient;
    await deleteEntities(failing, ["e1"]);
    expect(sketchStore.getState().session).toBe(before); // unchanged
    expect(viewportStore.getState().statusHint).toContain("boom");
  });
});

describe("deleteConstraints — constraints only, entities untouched", () => {
  beforeEach(() => {
    resetMockSketches();
    sketchStore.getState().reset();
  });

  it("removes the named constraints and re-solves (dof rises)", async () => {
    seed(
      [lineA],
      [
        { id: "c1", type: "Coincident", entities: ["e1", "e1"], positions: ["Start", "End"] }, // removes 2
        { id: "c2", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 40 }, // removes 1
      ],
    );
    // Before: 4 free − 3 removed = 1 dof. After dropping c2: 4 − 2 = 2 dof.
    await deleteConstraints(mockClient, ["c2"]);
    const s = sketchStore.getState().session!;
    expect(s.entities.map((e) => e.id)).toEqual(["e1"]); // entities untouched
    expect(s.constraints.map((c) => c.id)).toEqual(["c1"]);
    expect(s.dof).toBe(2);
  });

  it("is a no-op on empty ids", async () => {
    seed([lineA], [{ id: "c1", type: "Horizontal", entities: ["e1"] }]);
    await deleteConstraints(mockClient, []);
    expect(sketchStore.getState().session!.constraints).toHaveLength(1);
  });

  it("is a no-op when no constraint id matches", async () => {
    seed([lineA], [{ id: "c1", type: "Horizontal", entities: ["e1"] }]);
    const before = sketchStore.getState().session;
    await deleteConstraints(mockClient, ["nope"]);
    expect(sketchStore.getState().session).toBe(before);
  });
});
