import { beforeEach, describe, expect, it } from "vitest";
import { mockClient, resetMockDocument, setMockLatency } from "./mockClient";
import type { OperationOp } from "./types";
import { WORLD_AXIS } from "@/tools/preview/patternPreview";
import { operationToEditCommand } from "./tauriCommandMap";

const move = (x: number): Extract<OperationOp, { opType: "TransformBody" }> => ({
  opType: "TransformBody",
  inputs: [{ primary: { bodyId: "body1", kind: "body" } }],
  params: {
    targets: ["body1"],
    translate: [x, 0, 0],
    rotate: { center: [0, 0, 0], axis: WORLD_AXIS.Z, angleDeg: 0 },
    copy: false,
  },
});

describe("mock conditional failed-operation rollback", () => {
  beforeEach(() => {
    setMockLatency(0);
    resetMockDocument();
  });

  it("rolls back the exact single transaction once", async () => {
    const applied = await mockClient.applyOperation(move(10));
    expect(applied.rollbackToken).toBeDefined();
    const rolled = await mockClient.rollbackFailedOperation(applied.rollbackToken!);
    expect(rolled).toMatchObject({ rolledBack: true, reason: "rolledBack" });
    const duplicate = await mockClient.rollbackFailedOperation(applied.rollbackToken!);
    expect(duplicate).toMatchObject({ rolledBack: false, reason: "unknownOrConsumed" });
  });

  it("refuses after an interleaving authoring edit without undoing it", async () => {
    const failed = await mockClient.applyOperation(move(10));
    const newest = await mockClient.applyOperation(move(20));
    const before = newest.features.map((feature) => feature.id);
    const refused = await mockClient.rollbackFailedOperation(failed.rollbackToken!);
    expect(refused).toMatchObject({ rolledBack: false, reason: "tokenMismatch" });
    expect(refused.features.map((feature) => feature.id)).toEqual(before);
  });

  it("issues one receipt for a multi-operation undo step", async () => {
    const applied = await mockClient.applyOperations([move(10), move(20)]);
    const depth = applied.features.length;
    const rolled = await mockClient.rollbackFailedOperation(applied.rollbackToken!);
    expect(rolled.rolledBack).toBe(true);
    expect(rolled.features).toHaveLength(depth - 2);
    const redo = await mockClient.redo();
    expect(redo.features).toHaveLength(depth);
  });

  it("issues a receipt for direct updateOperationParams", async () => {
    const created = await mockClient.applyOperation(move(10));
    const featureId = created.features[created.features.length - 1].id;
    const update = operationToEditCommand({ ...move(20), featureId });
    const edited = await mockClient.applyEditCommand(update);
    expect(edited.rollbackToken).toBeDefined();
    const rolled = await mockClient.rollbackFailedOperation(edited.rollbackToken!);
    expect(rolled).toMatchObject({ rolledBack: true, reason: "rolledBack" });
  });

  it("does not borrow prior history when an update creates no undo item", async () => {
    await mockClient.applyOperation(move(10));
    const missing = operationToEditCommand({ ...move(20), featureId: "missing-record" });
    const result = await mockClient.applyEditCommand(missing);
    expect(result.rollbackToken).toBeUndefined();
  });
});
