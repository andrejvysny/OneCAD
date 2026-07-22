/*
 * localSolver preview lane — buildOpFromSession boolean pass-through (Wave 2). The
 * committed op must carry the drag params' booleanMode (default NewBody) + a
 * non-empty targetBodyId, so a Cut/Add previewed extrude materializes as a boolean
 * op through whichever `commit` dependency the client injects.
 */
import { describe, it, expect, vi } from "vitest";
import { createLocalSolverLane } from "./localSolver";
import type { ApplyOperationResult, OperationOp } from "./types";

function makeLane() {
  const commit = vi.fn(
    (_op: OperationOp): Promise<ApplyOperationResult> =>
      Promise.resolve({ revision: 1, changedBodies: [{ bodyId: "b1", meshKey: "b1#0" }], removedBodies: [], features: [] }),
  );
  const lane = createLocalSolverLane({ commit, latencyMs: () => 0 });
  return { lane, commit };
}

describe("localSolver buildOpFromSession boolean pass-through", () => {
  it("passes Cut + a non-empty targetBodyId through to the committed op", async () => {
    const { lane, commit } = makeLane();
    const s = await lane.beginPreview({ opType: "Extrude", sketchId: "sk", regionId: "r", params: { distance: 10 } });
    lane.updatePreview(s.sessionId, { distance: 20, booleanMode: "Cut", targetBodyId: "body7" }, 1);
    await lane.endPreview(s.sessionId, true);

    expect(commit).toHaveBeenCalledTimes(1);
    const op = commit.mock.calls[0][0];
    expect(op.opType).toBe("Extrude");
    if (op.opType !== "Extrude") throw new Error("expected Extrude");
    expect(op.params.booleanMode).toBe("Cut");
    expect(op.params.targetBodyId).toBe("body7");
    expect(op.params.distance).toBe(20);
  });

  it("defaults to NewBody + omits targetBodyId when none/empty was sent", async () => {
    const { lane, commit } = makeLane();
    const s = await lane.beginPreview({ opType: "Extrude", sketchId: "sk", regionId: "r", params: { distance: 5 } });
    // An empty-string target must NOT bind (would silently mis-target a body).
    lane.updatePreview(s.sessionId, { distance: 8, targetBodyId: "" }, 1);
    await lane.endPreview(s.sessionId, true);

    const op = commit.mock.calls[0][0];
    if (op.opType !== "Extrude") throw new Error("expected Extrude");
    expect(op.params.booleanMode).toBe("NewBody");
    expect(op.params.targetBodyId).toBeUndefined();
  });
});
