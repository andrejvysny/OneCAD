/*
 * MOCK ↔ CORE ROLLBACK PARITY (HISTORY-HARDEN H7b / M8).
 *
 * The mock lane is what the whole e2e rollback surface runs against, so its
 * cursor semantics have to be the CORE's semantics — a mock that "rolls back" by
 * bumping a revision (what `setRollback` used to do) makes every spec above it
 * vacuous. The two rules asserted here are transcribed from
 * `src-tauri/crates/onecad-core/src/history/timeline.rs`:
 *
 *   - `insert_at_cursor` (timeline.rs:174-185): `index = min(cursor, len)`; the
 *     record is inserted THERE, `cursor = index + 1`, and the inserted node plus
 *     every later step become Dirty. (Itself a transcription of OneCAD-CPP
 *     `AddOperationCommand::execute`, AddOperationCommand.cpp:33-37.)
 *   - `set_cursor(k)` (timeline.rs:198-210): k is clamped to `[0, len]`; steps
 *     that leave the applied prefix become drafts, and rolling forward again
 *     promotes them back.
 *
 * The walk below is the sequence the e2e specs drive: add a, add b, roll to 1,
 * add c, roll to end.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { mockClient, resetMockDocument, resetMockSketches, setMockLatency } from "./mockClient";
import { rollbackToCursorCommand } from "./tauriCommandMap";
import type { ApplyOperationResult, OperationOp, SketchEntity } from "./types";

const CIRCLE: SketchEntity = { id: "e1", type: "Circle", center: [0, 0], radius: 10 };

/** The seeded demo timeline (`MOCK_BASE_FEATURES`) every case starts from. */
const BASE = 5;

async function seedRegion(sketchId: string): Promise<string> {
  await mockClient.enterSketch({ newOnPlane: "XY", sketchId });
  await mockClient.sketchUpsert(sketchId, [CIRCLE], []);
  const fin = await mockClient.finishSketch(sketchId);
  return fin.regions[0].regionId;
}

function extrudeOp(sketchId: string, regionId: string, distance: number): OperationOp {
  return { opType: "Extrude", sketchId, regionId, params: { distance } };
}

/** Author one extrude and return its committed result. */
async function addExtrude(name: string, distance: number): Promise<ApplyOperationResult> {
  const regionId = await seedRegion(name);
  return mockClient.applyOperation(extrudeOp(name, regionId, distance));
}

const rollTo = (cursor: number): Promise<ApplyOperationResult> =>
  mockClient.applyEditCommand(rollbackToCursorCommand(cursor));

describe("mock rollback cursor (core parity)", () => {
  beforeEach(() => {
    setMockLatency(0);
    resetMockDocument();
    resetMockSketches();
  });

  it("every result reports the cursor and the timeline length", async () => {
    const res = await addExtrude("skA", 10);
    // A frontier commit leaves the timeline fully applied.
    expect(res.totalOps).toBe(BASE + 1);
    expect(res.appliedOps).toBe(BASE + 1);
  });

  it("set_cursor clamps to [0, len] and rolls back / forward again", async () => {
    await addExtrude("skA", 10);
    const back = await rollTo(2);
    expect(back.appliedOps).toBe(2);
    expect(back.totalOps).toBe(BASE + 1);

    // Past the end clamps to len (this is how "roll to end" is expressed).
    const forward = await rollTo(999);
    expect(forward.appliedOps).toBe(BASE + 1);

    // Below zero clamps to 0 — nothing applied, the whole timeline is a draft.
    const zero = await rollTo(-3);
    expect(zero.appliedOps).toBe(0);
  });

  it("rolling back MASKS the bodies the rolled-back records made, and forward restores them", async () => {
    const a = await addExtrude("skA", 10);
    const bodyA = a.changedBodies[0].bodyId;

    // Roll to just before the extrude: its body leaves the document.
    const back = await rollTo(BASE);
    expect(back.removedBodies).toContain(bodyA);
    expect(back.changedBodies.map((b) => b.bodyId)).not.toContain(bodyA);

    const forward = await rollTo(BASE + 1);
    expect(forward.changedBodies.map((b) => b.bodyId)).toContain(bodyA);
    expect(forward.removedBodies).not.toContain(bodyA);
  });

  it("add a · add b · roll to 1 · add c · roll to end — the core's insert-at-cursor walk", async () => {
    const a = await addExtrude("skA", 10);
    const b = await addExtrude("skB", 20);
    const order = (res: ApplyOperationResult): string[] => res.features.map((f) => f.id);
    const idA = order(a)[BASE];
    const idB = order(b)[BASE + 1];
    expect(b.appliedOps).toBe(BASE + 2);

    // Roll the cursor back INTO the seeded prefix (applied = 1).
    const rolled = await rollTo(1);
    expect(rolled.appliedOps).toBe(1);
    expect(order(rolled)).toHaveLength(BASE + 2); // rollback never removes records

    // Author c while rolled back: timeline.rs `insert_at_cursor` puts it AT index
    // 1 (not at the tail) and advances the cursor past it.
    const c = await addExtrude("skC", 30);
    const idC = order(c)[1];
    expect(c.appliedOps).toBe(2);
    expect(c.totalOps).toBe(BASE + 3);
    expect(order(c).indexOf(idC)).toBe(1);
    // …and the pre-existing records keep their relative order behind it.
    expect(order(c).indexOf(idA)).toBeGreaterThan(1);
    expect(order(c).indexOf(idA)).toBeLessThan(order(c).indexOf(idB));

    // Everything after the insert is Dirty (pending regen), the prefix is not.
    const statuses = c.features.map((f) => f.status);
    expect(statuses[0]).toBe("ok");
    expect(statuses.slice(2).every((s) => s === "dirty")).toBe(true);

    // Roll to end restores the full timeline.
    const end = await rollTo(c.totalOps!);
    expect(end.appliedOps).toBe(BASE + 3);
    expect(end.appliedOps).toBe(end.totalOps);
  });

  it("deleting a record inside the applied prefix shortens the cursor with it", async () => {
    await addExtrude("skA", 10);
    const rolled = await rollTo(3);
    expect(rolled.appliedOps).toBe(3);
    // f1 is inside the applied prefix (index 0) — removing it leaves 2 applied.
    const del = await mockClient.applyEditCommand({ cmd: "removeOperation", record: "f1" });
    expect(del.appliedOps).toBe(2);
    expect(del.totalOps).toBe(BASE);
  });

  it("undo restores the cursor a rollback moved", async () => {
    await addExtrude("skA", 10);
    await rollTo(2);
    const undone = await mockClient.undo();
    expect(undone.appliedOps).toBe(BASE + 1);
    expect(undone.appliedOps).toBe(undone.totalOps);
  });
});
