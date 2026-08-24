import { describe, it, expect, beforeEach } from "vitest";
import {
  commitDimensionConstraint,
  deleteConstraints,
  deleteEntities,
  editConstraintValue,
  rejectConflictHint,
  setEntitiesConstruction,
  undoSketch,
} from "./sketchService";
import { mockClient, resetMockSketches } from "@/ipc/mockClient";
import { planeFor } from "@/ipc/mockSketch";
import { detectRegions } from "@/ipc/mockRegions";
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

  // Same reject-on-conflict round trip as above, but through the REAL mock lane
  // (mockClient → localSolver.sketchUpsert → mockSketch.solveSketch), not a fake
  // client — proves detectConflicts is actually wired end to end, not just unit
  // tested in isolation.
  it("names the pre-existing Distance in the reject hint on the mock lane", async () => {
    seedSession([{ id: "d1", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 40 }]);
    const clashing: SketchConstraint = {
      id: "d2",
      type: "Distance",
      entities: ["e1", "e1"],
      positions: ["Start", "End"],
      value: 120,
    };
    const { rejected, hint } = await commitDimensionConstraint(mockClient, clashing);
    expect(rejected).toBe(true);
    expect(hint).toBe("Dimension removed — conflicts with Distance (d1)");
    const s = sketchStore.getState().session!;
    expect(s.constraints.map((c) => c.id)).toEqual(["d1"]); // d2 dropped; d1 survives
    expect(sketchStore.getState().conflictingIds).toEqual([]); // the restore solve is clean
  });
});

// ── rejectConflictHint — self-blame fix (adversarial-review item 5) ───────────

describe("rejectConflictHint", () => {
  const named: SketchConstraint = { id: "c14", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 40 };

  it("names the OTHER conflicting constraint when one is present", () => {
    expect(rejectConflictHint([named], ["c14", "d1"], "d1", "Dimension")).toBe(
      "Dimension removed — conflicts with Distance (c14)",
    );
  });

  it("falls back to the generic sentence when `conflicting` is empty", () => {
    expect(rejectConflictHint([], [], "d1", "Dimension")).toBe(
      "Dimension removed — it would over-constrain the sketch",
    );
    expect(rejectConflictHint([], undefined, "d1", "Constraint")).toBe(
      "Constraint removed — it would over-constrain the sketch",
    );
  });

  // The bug: the OLD `?? ids[0]` fallback re-picked the authored id itself when
  // it was the ONLY entry, naming the just-removed dimension as its own clash
  // ("Dimension removed — conflicts with Distance (c14)" where c14 IS d1).
  it("falls back to the generic sentence when the ONLY conflicting id is the authored one", () => {
    expect(rejectConflictHint([named], ["d1"], "d1", "Dimension")).toBe(
      "Dimension removed — it would over-constrain the sketch",
    );
  });
});

// Same self-blame case, through the real reject path (mirrors the fake-client
// shape of "a rejected applied constraint names the clashing constraint" above).
describe("commitDimensionConstraint — reject hint never blames the authored dimension", () => {
  beforeEach(() => {
    resetMockSketches();
    sketchStore.getState().reset();
  });

  it("uses the generic sentence when the solve's only conflicting id is the new dimension", async () => {
    seedSession([
      { id: "c1", type: "Coincident", entities: ["e1", "e1"], positions: ["Start", "End"] },
    ]);
    const dim: SketchConstraint = { id: "c2", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 40 };
    const client = {
      sketchUpsert: (() => {
        let n = 0;
        return () => {
          n += 1;
          return Promise.resolve(
            n === 1
              ? { sketchId: "sk-dim", sketchRevision: 2, dof: 0, status: "Conflicting" as const, conflicting: ["c2"], solvedPositions: {} }
              : { sketchId: "sk-dim", sketchRevision: 3, dof: 0, status: "FullyConstrained" as const, conflicting: [], solvedPositions: {} },
          );
        };
      })(),
    } as unknown as CadClient;
    const { rejected, hint } = await commitDimensionConstraint(client, dim);
    expect(rejected).toBe(true);
    expect(hint).toBe("Dimension removed — it would over-constrain the sketch");
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
    expect(viewportStore.getState().statusHint?.message).toContain("boom");
  });

  // ── L3 guard: locked reference geometry (SKETCH-ON-FACE W2) ──────────────

  it("SKIPS locked entities but still deletes the user's own, silently", async () => {
    // A marquee that swept up the projected boundary alongside the user's line
    // should still delete the line — refusing the whole action would be worse.
    const locked: SketchEntity = { ...lineB, referenceLocked: true };
    seed([lineA, locked], []);
    viewportStore.getState().setStatusHint(null);
    await deleteEntities(mockClient, ["e1", "e2"]);
    const s = sketchStore.getState().session!;
    expect(s.entities.map((e) => e.id)).toEqual(["e2"]); // the locked one survives
    expect(viewportStore.getState().statusHint).toBeNull(); // partial ⇒ no complaint
  });

  it("refuses with a NAMED hint when the target set was ENTIRELY locked", async () => {
    const locked: SketchEntity = { ...lineA, referenceLocked: true };
    seed([locked], [{ id: "c1", type: "Horizontal", entities: ["e1"] }]);
    const before = sketchStore.getState().session;
    await deleteEntities(mockClient, ["e1"]);
    expect(sketchStore.getState().session).toBe(before); // untouched
    expect(viewportStore.getState().statusHint?.message).toMatch(/Reference geometry is locked/);
  });

  it("stays silent when the ids matched nothing at all (not a lock refusal)", async () => {
    seed([{ ...lineA, referenceLocked: true }], []);
    viewportStore.getState().setStatusHint(null);
    await deleteEntities(mockClient, ["nope"]);
    expect(viewportStore.getState().statusHint).toBeNull();
  });
});

// ── conflictingIds ownership (SCHEMA §7.4) ────────────────────────────────────

describe("conflictingIds — solve write-back REPLACES, exit CLEARS, reject-hint names", () => {
  beforeEach(() => {
    resetMockSketches();
    sketchStore.getState().reset();
  });

  it("a solve write-back REPLACES the store's conflictingIds from the result", async () => {
    seedSession([{ id: "c1", type: "Coincident", entities: ["e1", "e1"], positions: ["Start", "End"] }]);
    // A fake client whose solve reports c1 in conflict.
    const client = {
      sketchUpsert: () =>
        Promise.resolve({
          sketchId: "sk-dim",
          sketchRevision: 2,
          dof: 1,
          status: "UnderConstrained" as const,
          conflicting: ["c1"],
          solvedPositions: {},
        }),
    } as unknown as CadClient;
    await editConstraintValue(client, "c1", 5);
    expect(sketchStore.getState().conflictingIds).toEqual(["c1"]);
  });

  it("a clean solve REPLACES conflictingIds back to []", async () => {
    seedSession([{ id: "c1", type: "Coincident", entities: ["e1", "e1"], positions: ["Start", "End"] }]);
    sketchStore.getState().setConflicting(["c1"]); // stale set from a prior solve
    // The mock lane always reports conflicting: [] — the write-back must clear it.
    await editConstraintValue(mockClient, "c1", 5);
    expect(sketchStore.getState().conflictingIds).toEqual([]);
  });

  it("a refused EDIT reverts the value and surfaces an error hint (A2 fix)", async () => {
    seedSession([
      { id: "c1", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 40 },
    ]);
    const client = {
      // Edit solve → Conflicting blaming only the edited dim itself (the mock
      // lane's can't-drive refusal shape); restore solve → clean.
      sketchUpsert: (() => {
        let n = 0;
        return () => {
          n += 1;
          return Promise.resolve(
            n === 1
              ? { sketchId: "sk-dim", sketchRevision: 2, dof: 0, status: "Conflicting" as const, conflicting: ["c1"], solvedPositions: {} }
              : { sketchId: "sk-dim", sketchRevision: 3, dof: 2, status: "UnderConstrained" as const, conflicting: [], solvedPositions: {} },
          );
        };
      })(),
    } as unknown as CadClient;
    await editConstraintValue(client, "c1", 99);
    const s = sketchStore.getState().session;
    expect(s?.constraints.find((c) => c.id === "c1")?.value).toBe(40); // reverted, not 99
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
    expect(viewportStore.getState().statusHint?.message).toContain("Dimension edit reverted");
    expect(sketchStore.getState().conflictingIds).toEqual([]); // restore cleared it
  });

  it("setSession(null) (exit / dispose) CLEARS conflictingIds", () => {
    seedSession([]);
    sketchStore.getState().setConflicting(["c1", "c2"]);
    sketchStore.getState().setSession(null);
    expect(sketchStore.getState().conflictingIds).toEqual([]);
  });

  it("a rejected applied constraint names the clashing constraint in the hint", async () => {
    // Two Coincidents fully constrain the line; a third geometric constraint that the
    // fake client rejects (Conflicting) with c1 as the conflicting id.
    seedSession([
      { id: "c1", type: "Coincident", entities: ["e1", "e1"], positions: ["Start", "End"] },
    ]);
    const client = {
      // First call (with the new constraint) → Conflicting, blames c1; restore → clean.
      sketchUpsert: (() => {
        let n = 0;
        return () => {
          n += 1;
          return Promise.resolve(
            n === 1
              ? { sketchId: "sk-dim", sketchRevision: 2, dof: 0, status: "Conflicting" as const, conflicting: ["c1"], solvedPositions: {} }
              : { sketchId: "sk-dim", sketchRevision: 3, dof: 0, status: "FullyConstrained" as const, conflicting: [], solvedPositions: {} },
          );
        };
      })(),
    } as unknown as CadClient;
    const dim: SketchConstraint = { id: "c2", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 40 };
    const { rejected, hint } = await commitDimensionConstraint(client, dim);
    expect(rejected).toBe(true);
    expect(hint).toBe("Dimension removed — conflicts with Coincident (c1)");
    // The restore solve cleared the conflicting set.
    expect(sketchStore.getState().conflictingIds).toEqual([]);
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

// ── W1-B: construction flip ───────────────────────────────────────────────────

describe("setEntitiesConstruction — flip the construction flag", () => {
  beforeEach(() => {
    resetMockSketches();
    sketchStore.getState().reset();
  });

  it("flips the named entities, leaves the rest and the constraints alone", async () => {
    seed([lineA, lineB, circle], [{ id: "c1", type: "Horizontal", entities: ["e1"] }]);
    await setEntitiesConstruction(mockClient, ["e1", "e3"], true);
    const s = sketchStore.getState().session!;
    expect(s.entities.map((e) => !!e.construction)).toEqual([true, false, true]);
    expect(s.constraints.map((c) => c.id)).toEqual(["c1"]);
  });

  it("pushes ONE undo snapshot carrying the pre-flip flags", async () => {
    seed([lineA], []);
    await setEntitiesConstruction(mockClient, ["e1"], true);
    const undo = sketchStore.getState().undoStack;
    expect(undo).toHaveLength(1);
    expect(undo[0].entities[0].construction).toBeFalsy();
    expect(sketchStore.getState().lastUndoPush).toEqual({ kind: "construction" });
  });

  it("undo restores the pre-flip flag", async () => {
    seed([lineA], []);
    await setEntitiesConstruction(mockClient, ["e1"], true);
    await undoSketch(mockClient);
    expect(sketchStore.getState().session!.entities[0].construction).toBeFalsy();
  });

  it("construction geometry is still SOLVED (dof unchanged) but leaves regions", async () => {
    // A closed square: 4 lines ⇒ one region until an edge turns construction.
    const sq: SketchEntity[] = [
      { id: "s1", type: "Line", p0: [0, 0], p1: [10, 0] },
      { id: "s2", type: "Line", p0: [10, 0], p1: [10, 10] },
      { id: "s3", type: "Line", p0: [10, 10], p1: [0, 10] },
      { id: "s4", type: "Line", p0: [0, 10], p1: [0, 0] },
    ];
    seed(sq, []);
    const dofBefore = sketchStore.getState().session!.entities.length * 4;
    await setEntitiesConstruction(mockClient, ["s1"], true);
    const s = sketchStore.getState().session!;
    expect(s.dof).toBe(dofBefore); // still 4 free dof per line — the solver sees it
    expect(detectRegions(s.entities)).toHaveLength(0); // but it no longer closes a loop
  });

  it("is a no-op on empty ids / no session / already at the target", async () => {
    await setEntitiesConstruction(mockClient, ["e1"], true); // no session
    expect(sketchStore.getState().session).toBeNull();

    seed([{ ...lineA, construction: true }], []);
    const before = sketchStore.getState().session;
    await setEntitiesConstruction(mockClient, [], true);
    await setEntitiesConstruction(mockClient, ["nope"], true);
    await setEntitiesConstruction(mockClient, ["e1"], true); // already construction
    expect(sketchStore.getState().session).toBe(before); // same reference: untouched
    expect(sketchStore.getState().undoStack).toHaveLength(0);
  });

  it("surfaces a status hint and leaves state intact on upsert failure", async () => {
    seed([lineA], []);
    const before = sketchStore.getState().session;
    const failing = { sketchUpsert: () => Promise.reject(new Error("boom")) } as unknown as CadClient;
    await setEntitiesConstruction(failing, ["e1"], true);
    expect(sketchStore.getState().session).toBe(before);
    expect(viewportStore.getState().statusHint?.message).toContain("Sketch construction failed");
    expect(sketchStore.getState().undoStack).toHaveLength(0);
  });
});
