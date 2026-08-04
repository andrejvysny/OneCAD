/*
 * H10 dependency view — `mockClient.featureDependencies`, the mock's honest
 * derivation from `featureTouched` (see `mockFeatureDependencies` in
 * `mockClient.ts`): a body-producer chain built the same way the core
 * `DependencyGraph` builds one from `OperationRecord::outputs`, just recovered
 * from the mock's per-feature body-touch log instead of a real regen.
 *
 * `TransformBody` (Move) is used to build the chain — it is the one mock op that
 * touches a body with NO sketch/region setup, so the chain is exact and cheap.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { mockClient, resetMockDocument, resetMockSketches, setMockLatency } from "./mockClient";
import { WORLD_AXIS } from "@/tools/preview/patternPreview";
import type { FeatureRecord, OperationOp } from "./types";

function moveOp(targets: string[], translate: [number, number, number]): OperationOp {
  return {
    opType: "TransformBody",
    inputs: targets.map((bodyId) => ({ primary: { bodyId, kind: "body" as const } })),
    params: {
      targets,
      translate,
      rotate: { center: [0, 0, 0], axis: WORLD_AXIS.Z, angleDeg: 0 },
      copy: false,
    },
  };
}

const lastRow = (rows: FeatureRecord[]): FeatureRecord => rows[rows.length - 1];

describe("mockClient.featureDependencies", () => {
  beforeEach(() => {
    setMockLatency(0);
    resetMockDocument();
    resetMockSketches();
  });

  it("a chain of ops on the SAME body forms real producer-chain edges (both directions, transitive)", async () => {
    const r1 = await mockClient.applyOperation(moveOp(["body1"], [10, 0, 0]));
    const m1 = lastRow(r1.features).id;
    const r2 = await mockClient.applyOperation(moveOp(["body1"], [0, 10, 0]));
    const m2 = lastRow(r2.features).id;
    const r3 = await mockClient.applyOperation(moveOp(["body1"], [0, 0, 10]));
    const m3 = lastRow(r3.features).id;

    // Direct neighbours.
    await expect(mockClient.featureDependencies(m2)).resolves.toEqual({
      upstream: [m1],
      downstream: [m3],
    });

    // M1's downstream is the FULL transitive closure {M2, M3}, never itself.
    const deps1 = await mockClient.featureDependencies(m1);
    expect(deps1.upstream).toEqual([]);
    expect(new Set(deps1.downstream)).toEqual(new Set([m2, m3]));
    expect(deps1.downstream).not.toContain(m1);

    // M3's upstream is the full transitive closure {M1, M2}; nothing downstream.
    const deps3 = await mockClient.featureDependencies(m3);
    expect(deps3.downstream).toEqual([]);
    expect(new Set(deps3.upstream)).toEqual(new Set([m1, m2]));
  });

  it("rejects an unknown record", async () => {
    await expect(mockClient.featureDependencies("not-a-real-id")).rejects.toThrow(/unknown record/);
  });

  it("HONEST LIMIT: a seeded base-fixture feature has no tracked lineage, so it answers empty rather than fabricating a chain", async () => {
    // f1..f5 are MOCK_BASE_FEATURES — seeded directly, never through `mutateOp`,
    // so `featureTouched` has no entry for them.
    await expect(mockClient.featureDependencies("f3")).resolves.toEqual({
      upstream: [],
      downstream: [],
    });
  });
});
