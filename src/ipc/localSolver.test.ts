/*
 * localSolver preview lane — buildOpFromSession boolean pass-through (Wave 2). The
 * committed op must carry the drag params' booleanMode (default NewBody) + a
 * non-empty targetBodyId, so a Cut/Add previewed extrude materializes as a boolean
 * op through whichever `commit` dependency the client injects.
 */
import { describe, it, expect, vi } from "vitest";
import { createLocalSolverLane } from "./localSolver";
import type { ApplyOperationResult, OperationOp, PreviewResult, SketchConstraint, SketchEntity } from "./types";

type BackendPreview = {
  bodies: { bodyId: string; mesh: ArrayBuffer }[];
  changedBodies?: string[];
  deletedBodies?: string[];
  needsRepair?: unknown[];
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

function primeLane(lane: ReturnType<typeof createLocalSolverLane>): void {
  lane.cacheSketchPlane("sk", {
    kind: "XY",
    origin: [0, 0, 0],
    xAxis: [1, 0, 0],
    yAxis: [0, 1, 0],
    normal: [0, 0, 1],
  });
  lane.cacheFinishedRegions("sk", [
    {
      regionId: "r",
      outerLoop: [],
      holes: [],
      previewTriangles: {
        positions: [0, 0, 10, 0, 10, 10, 0, 10],
        indices: [0, 1, 2, 0, 2, 3],
        holesSubtracted: 0,
      },
    },
  ]);
}

function makeLane() {
  const commit = vi.fn(
    (_op: OperationOp): Promise<ApplyOperationResult> =>
      Promise.resolve({ revision: 1, changedBodies: [{ bodyId: "b1", meshKey: "b1#0" }], removedBodies: [], features: [] }),
  );
  const lane = createLocalSolverLane({ commit, latencyMs: () => 0 });
  primeLane(lane);
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

  it("rejects an unknown booleanMode instead of silently changing the operation", async () => {
    const { lane, commit } = makeLane();
    const s = await lane.beginPreview({ opType: "Extrude", sketchId: "sk", regionId: "r", params: { distance: 5 } });
    // PreviewParams is loosely typed on the wire; a stray value must not reach the op.
    lane.updatePreview(s.sessionId, { distance: 8, booleanMode: "Bogus" as never }, 1);
    await expect(lane.endPreview(s.sessionId, true)).rejects.toThrow(/Unsupported Extrude booleanMode/);
    expect(commit).not.toHaveBeenCalled();
  });

  it("passes each valid FeatureBooleanMode through verbatim (finding 14)", async () => {
    for (const mode of ["NewBody", "Add", "Cut", "Intersect"] as const) {
      const { lane, commit } = makeLane();
      const s = await lane.beginPreview({ opType: "Extrude", sketchId: "sk", regionId: "r", params: { distance: 5 } });
      lane.updatePreview(s.sessionId, { distance: 8, booleanMode: mode, targetBodyId: "b" }, 1);
      await lane.endPreview(s.sessionId, true);
      const op = commit.mock.calls[0][0];
      if (op.opType !== "Extrude") throw new Error("expected Extrude");
      expect(op.params.booleanMode).toBe(mode);
    }
  });

  it("rejects a stale explicit region id and lists the available identity", async () => {
    const { lane } = makeLane();
    await expect(
      lane.beginPreview({
        opType: "Extrude",
        sketchId: "sk",
        regionId: "missing",
        params: { distance: 5 },
      }),
    ).rejects.toThrow(/Unknown region missing.*available: r/);
  });
});

describe("localSolver exact backend preview lifecycle", () => {
  function backendLane(
    previewOp: (op: OperationOp, sketchId: string | undefined) => Promise<BackendPreview>,
    commit = vi.fn(
      (_op: OperationOp): Promise<ApplyOperationResult> =>
        Promise.resolve({
          revision: 1,
          changedBodies: [{ bodyId: "b1", meshKey: "b1#0" }],
          removedBodies: [],
          features: [],
        }),
    ),
  ) {
    const lane = createLocalSolverLane({ commit, previewOp, latencyMs: () => 0 });
    primeLane(lane);
    return { lane, commit };
  }

  it("keeps one request in flight and dispatches only the newest queued snapshot", async () => {
    const first = deferred<BackendPreview>();
    const second = deferred<BackendPreview>();
    const previewOp = vi
      .fn<(op: OperationOp, sketchId: string | undefined) => Promise<BackendPreview>>()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    const { lane } = backendLane(previewOp);
    const session = await lane.beginPreview({
      opType: "Extrude",
      sketchId: "sk",
      regionId: "r",
      params: { distance: 1 },
    });

    lane.updatePreview(session.sessionId, { distance: 10 }, 1);
    lane.updatePreview(session.sessionId, { distance: 20 }, 2);
    lane.updatePreview(session.sessionId, { distance: 30 }, 3);
    expect(previewOp).toHaveBeenCalledTimes(1);

    first.resolve({ bodies: [{ bodyId: "b1", mesh: new ArrayBuffer(1) }] });
    await flush();
    expect(previewOp).toHaveBeenCalledTimes(2);
    const newest = previewOp.mock.calls[1][0];
    expect(newest.opType).toBe("Extrude");
    if (newest.opType !== "Extrude") throw new Error("expected Extrude");
    expect(newest.params.distance).toBe(30);

    second.resolve({ bodies: [{ bodyId: "b1", mesh: new ArrayBuffer(1) }] });
    await flush();
    expect(previewOp).toHaveBeenCalledTimes(2);
    await lane.endPreview(session.sessionId, false);
  });

  /**
   * The controller now holds a COMMIT BARRIER: it pushes the final immutable params
   * as a fresh epoch, waits for that epoch's exact candidate, and only then ends the
   * session. The lane's half of that contract is asserted here — the final
   * updatePreview must publish a result under the SAME epoch, and the subsequent
   * commit must carry byte-identical params + the preview's stable opId.
   */
  it("publishes the final epoch's exact candidate, then commits those same params", async () => {
    const pending = deferred<BackendPreview>();
    const previewOp = vi.fn(
      (_op: OperationOp, _sketchId: string | undefined): Promise<BackendPreview> =>
        pending.promise,
    );
    const { lane, commit } = backendLane(previewOp);
    const results: PreviewResult[] = [];
    lane.onPreviewResult((r) => results.push(r));
    const session = await lane.beginPreview({
      opType: "Extrude",
      sketchId: "sk",
      regionId: "r",
      params: { distance: 1 },
    });
    const finalParams = {
      distance: 42,
      draftAngleDeg: 3,
      extrudeMode: "Blind" as const,
      booleanMode: "Cut" as const,
      targetBodyId: "target",
      twoDirections: false,
      extrudeMode2: "Blind" as const,
      distance2: 0,
    };
    lane.updatePreview(session.sessionId, finalParams, 7);
    expect(commit).not.toHaveBeenCalled(); // nothing commits off the update alone

    // The barrier's release: the final epoch's exact candidate lands first.
    pending.resolve({ bodies: [{ bodyId: "target", mesh: new ArrayBuffer(1) }] });
    await flush();
    const exact = results.find((r) => r.epoch === 7 && !r.committed);
    expect(exact).toBeDefined();
    expect(exact?.error).toBeUndefined();

    await lane.endPreview(session.sessionId, true);

    expect(commit).toHaveBeenCalledTimes(1);
    const previewCandidate = previewOp.mock.calls[0][0];
    const committed = commit.mock.calls[0][0];
    // The Rust side has a nil-UUID fallback for a missing opId — the preview
    // candidate must always carry a real minted id so that path stays unreachable.
    expect(previewCandidate.opId).toBeTruthy();
    expect(previewCandidate.opId).not.toBe("00000000-0000-0000-0000-000000000000");
    expect(committed.opId).toBe(previewCandidate.opId);
    expect(committed.params).toEqual(previewCandidate.params);
  });

  it("replaces full params so switching to NewBody drops stale target fields", async () => {
    const { lane, commit } = makeLane();
    const session = await lane.beginPreview({
      opType: "Extrude",
      sketchId: "sk",
      regionId: "r",
      params: { distance: 1, booleanMode: "Cut", targetBodyId: "old-target" },
    });
    lane.updatePreview(
      session.sessionId,
      {
        distance: 9,
        booleanMode: "NewBody",
        extrudeMode: "Blind",
        twoDirections: false,
      },
      2,
    );
    await lane.endPreview(session.sessionId, true);
    const op = commit.mock.calls[0][0];
    if (op.opType !== "Extrude") throw new Error("expected Extrude");
    expect(op.params.targetBodyId).toBeUndefined();
    expect(op.params.booleanMode).toBe("NewBody");
  });

  it("surfaces NeedsRepair as a structural failure with original evidence", async () => {
    const evidence = [{ refId: "op.input0", reason: "ambiguous" }];
    const { lane } = backendLane(async () => ({ bodies: [], needsRepair: evidence }));
    const seen: PreviewResult[] = [];
    lane.onPreviewResult((result) => seen.push(result));
    const session = await lane.beginPreview({
      opType: "Extrude",
      sketchId: "sk",
      regionId: "r",
      params: { distance: 5 },
    });
    lane.updatePreview(session.sessionId, { distance: 6 }, 1);
    await flush();

    expect(seen[0]?.error).toMatchObject({
      kind: "needsRepair",
      structural: true,
      evidence,
    });
    expect(seen[0]?.error?.message).toContain("op.input0");
  });

  it("accepts a full-deletion Cut and preserves every split preview body", async () => {
    const results: BackendPreview[] = [
      { bodies: [], deletedBodies: ["target"] },
      {
        bodies: [
          { bodyId: "split-a", mesh: new ArrayBuffer(1) },
          { bodyId: "split-b", mesh: new ArrayBuffer(1) },
        ],
        changedBodies: ["split-a", "split-b"],
        deletedBodies: ["target"],
      },
    ];
    const { lane } = backendLane(async () => results.shift()!);
    const seen: PreviewResult[] = [];
    lane.onPreviewResult((result) => seen.push(result));

    const deleted = await lane.beginPreview({
      opType: "Extrude",
      sketchId: "sk",
      regionId: "r",
      params: { distance: 5 },
    });
    lane.updatePreview(
      deleted.sessionId,
      { distance: 5, booleanMode: "Cut", targetBodyId: "target" },
      1,
    );
    await flush();
    expect(seen[0]).toMatchObject({
      bodies: [],
      replacedBodyIds: ["target"],
    });
    expect(seen[0]?.error).toBeUndefined();
    await lane.endPreview(deleted.sessionId, false);

    const split = await lane.beginPreview({
      opType: "Extrude",
      sketchId: "sk",
      regionId: "r",
      params: { distance: 5 },
    });
    lane.updatePreview(
      split.sessionId,
      { distance: 5, booleanMode: "Cut", targetBodyId: "target" },
      2,
    );
    await flush();
    expect(seen[1]?.bodies?.map((body) => body.bodyId)).toEqual(["split-a", "split-b"]);
    expect(seen[1]?.replacedBodyIds).toEqual(["split-a", "split-b", "target"]);
  });

  it("does not run exact PreviewOp for a feature re-edit", async () => {
    const previewOp = vi.fn(async (): Promise<BackendPreview> => ({ bodies: [] }));
    const { lane } = backendLane(previewOp);
    const session = await lane.beginPreview({
      opType: "Extrude",
      sketchId: "sk",
      regionId: "r",
      params: { distance: 5, featureId: "feature-1" },
    });
    // Even a malformed caller that omits featureId from a later full snapshot
    // cannot accidentally enable exact-on-head preview or whole-param commit.
    lane.updatePreview(session.sessionId, { distance: 8 }, 1);
    await flush();
    expect(previewOp).not.toHaveBeenCalled();
    await expect(lane.endPreview(session.sessionId, true)).rejects.toThrow(/scalar update/);
  });
});

describe("localSolver sketchUpsert — deterministic conflict detection (mock lane)", () => {
  const rectLine: SketchEntity = { id: "e1", type: "Line", p0: [0, 0], p1: [40, 0] };

  it("surfaces Conflicting + the clashing ids for a duplicate-incompatible dimension", async () => {
    const { lane } = makeLane();
    const constraints: SketchConstraint[] = [
      { id: "d1", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 40 },
      { id: "d2", type: "Distance", entities: ["e1", "e1"], positions: ["Start", "End"], value: 120 },
    ];
    const result = await lane.sketchUpsert("sk-conflict", [rectLine], constraints);
    expect(result.status).toBe("Conflicting");
    expect(result.conflicting?.slice().sort()).toEqual(["d1", "d2"]);
  });

  it("reports conflicting: [] and the dof-derived status when nothing clashes", async () => {
    const { lane } = makeLane();
    const result = await lane.sketchUpsert("sk-clean", [rectLine], []);
    expect(result.status).toBe("UnderConstrained");
    expect(result.conflicting).toEqual([]);
  });
});
