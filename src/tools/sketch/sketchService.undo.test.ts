/*
 * Sketch-scoped undo/redo (C2) — pushes on confirmed success, coalesces consecutive
 * edits of one constraint, caps at 50, and a fresh push clears the redo stack. Uses
 * the in-memory mockClient (identity solve) with a directly-seeded session, matching
 * sketchService.test.ts.
 */
import { describe, it, expect, beforeEach } from "vitest";
import {
  deleteEntities,
  editConstraintValue,
  redoSketch,
  undoSketch,
} from "./sketchService";
import { mockClient, resetMockSketches } from "@/ipc/mockClient";
import { planeFor } from "@/ipc/mockSketch";
import { sketchStore } from "@/stores/sketchStore";
import type { SketchConstraint, SketchEntity, SketchSession } from "@/ipc/types";

const line: SketchEntity = { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] };
const circle: SketchEntity = { id: "e2", type: "Circle", center: [80, 0], radius: 10 };

function seed(entities: SketchEntity[], constraints: SketchConstraint[]): SketchSession {
  const s: SketchSession = {
    sketchId: "sk-undo",
    plane: planeFor("XY"),
    entities,
    constraints,
    dof: 0,
    status: "UnderConstrained",
  };
  sketchStore.getState().setSession(s);
  sketchStore.getState().clearSketchUndo();
  return s;
}

const ids = () => sketchStore.getState().session!.entities.map((e) => e.id);

describe("undoSketch / redoSketch — revert and restore", () => {
  beforeEach(() => {
    resetMockSketches();
    sketchStore.getState().reset();
  });

  it("undo reverts entities + constraints; redo restores", async () => {
    seed([line, circle], []);
    await deleteEntities(mockClient, ["e2"]);
    expect(ids()).toEqual(["e1"]); // circle gone

    await undoSketch(mockClient);
    expect(ids()).toEqual(["e1", "e2"]); // restored

    await redoSketch(mockClient);
    expect(ids()).toEqual(["e1"]); // re-applied
  });

  it("undo is a no-op when the stack is empty", async () => {
    const s = seed([line], []);
    await undoSketch(mockClient);
    expect(sketchStore.getState().session).toBe(s); // untouched
  });

  it("a fresh push clears the redo stack", async () => {
    seed([line, circle], []);
    await deleteEntities(mockClient, ["e2"]); // undoStack: 1
    await undoSketch(mockClient); // redoStack: 1
    expect(sketchStore.getState().redoStack).toHaveLength(1);
    // A new mutation pushes → redo is invalidated.
    await deleteEntities(mockClient, ["e1"]);
    expect(sketchStore.getState().redoStack).toHaveLength(0);
  });
});

describe("editConstraintValue — undo coalescing", () => {
  beforeEach(() => {
    resetMockSketches();
    sketchStore.getState().reset();
  });

  const distance: SketchConstraint = {
    id: "c1",
    type: "Distance",
    entities: ["e1", "e1"],
    positions: ["Start", "End"],
    value: 40,
  };

  it("coalesces consecutive edits of the SAME constraint into one undo entry", async () => {
    seed([line], [distance]);
    await editConstraintValue(mockClient, "c1", 50);
    await editConstraintValue(mockClient, "c1", 60);
    expect(sketchStore.getState().undoStack).toHaveLength(1); // coalesced

    // Undo returns to the value BEFORE the run of edits (40), not the intermediate 50.
    await undoSketch(mockClient);
    const c = sketchStore.getState().session!.constraints.find((x) => x.id === "c1")!;
    expect(c.value).toBe(40);
  });

  it("does NOT coalesce across an intervening undo (redo-reset breaks the run)", async () => {
    seed([line], [distance]);
    await editConstraintValue(mockClient, "c1", 50); // push #1
    await undoSketch(mockClient); // lastUndoPush → null
    await editConstraintValue(mockClient, "c1", 70); // not coalesced → push again
    expect(sketchStore.getState().undoStack).toHaveLength(1);
  });
});

describe("undo history cap (50)", () => {
  beforeEach(() => sketchStore.getState().reset());

  it("keeps only the newest 50 snapshots (oldest sliced off)", () => {
    for (let i = 0; i < 60; i++) {
      sketchStore.getState().pushUndoSnapshot({ entities: [], constraints: [{ id: `c${i}`, type: "Horizontal", entities: ["e1"] }] });
    }
    const stack = sketchStore.getState().undoStack;
    expect(stack).toHaveLength(50);
    // Entries 0..9 were sliced; the oldest retained is c10, the newest c59.
    expect(stack[0].constraints[0].id).toBe("c10");
    expect(stack[49].constraints[0].id).toBe("c59");
  });
});
