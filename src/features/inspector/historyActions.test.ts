/*
 * historyActions — the history-row + rebind edit dispatch (M4b). Verifies each
 * action sends the RIGHT raw EditCommand through the client (rollback/suppress/
 * delete command mapping) and reflects the result in the stores.
 */
import { beforeEach, describe, it, expect, vi } from "vitest";
import { suppressFeature, rollToIndex, deleteFeature, rebindCandidate } from "./historyActions";
import { toFeatureMeta } from "@/ipc/projectionHydration";
import { mockClient } from "@/ipc/mockClient";
import { documentStore } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { resetStores } from "@/test/resetStores";
import type { FeatureRecord, NeedsRepairItem, ResolveCandidate } from "@/ipc/types";

beforeEach(() => resetStores());

describe("toFeatureMeta", () => {
  it("carries statusMessage and suppressed through (the shared mapping historyActions and ModelToolController both use)", () => {
    const record: FeatureRecord = {
      id: "f9",
      kind: "revolve",
      opType: "Revolve",
      label: "Revolve",
      valueText: "90°",
      status: "error",
      statusMessage: "revolve axis lineId not found",
      suppressed: true,
    };
    expect(toFeatureMeta(record)).toEqual({
      id: "f9",
      kind: "revolve",
      opType: "Revolve",
      label: "Revolve",
      valueText: "90°",
      status: "error",
      statusMessage: "revolve axis lineId not found",
      suppressed: true,
    });
  });
});

describe("historyActions — command mapping", () => {
  it("suppressFeature cascades when SUPPRESSING + dims from the RESULT projection (no overlay)", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    await suppressFeature("f3", true);
    expect(apply.mock.calls[0][0]).toEqual({
      cmd: "setOperationSuppression",
      record: "f3",
      suppressed: true,
      cascade: true,
    });
    // The dim state IS the projection: the backend-authoritative `suppressed` flag
    // hydrated from the returned features array, not a frontend overlay.
    expect(documentStore.getState().features.find((f) => f.id === "f3")?.suppressed).toBe(true);
    apply.mockRestore();
  });

  it("suppressFeature does NOT cascade when UN-suppressing (never resurrects deliberately-suppressed dependents)", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    await suppressFeature("f3", true);
    await suppressFeature("f3", false);
    expect(apply.mock.calls[1][0]).toEqual({
      cmd: "setOperationSuppression",
      record: "f3",
      suppressed: false,
      cascade: false,
    });
    expect(documentStore.getState().features.find((f) => f.id === "f3")?.suppressed).toBe(false);
    apply.mockRestore();
  });

  it("a FAILED suppress leaves the projection untouched (nothing optimistic to revert)", async () => {
    const apply = vi
      .spyOn(mockClient, "applyEditCommand")
      .mockRejectedValue(new Error("regen refused"));
    await suppressFeature("f3", true);
    expect(documentStore.getState().features.find((f) => f.id === "f3")?.suppressed).toBeUndefined();
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
    apply.mockRestore();
  });

  it("rollToIndex sends SetRollback with cursor = index + 1 (applied op count)", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    await rollToIndex(2);
    expect(apply.mock.calls[0][0]).toEqual({ cmd: "setRollback", cursor: 3 });
    apply.mockRestore();
  });

  it("hydrates the feature timeline with the authored opType (re-edit routing)", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand").mockResolvedValue({
      revision: 9,
      features: [
        { id: "f-ch", kind: "fillet", opType: "Chamfer", label: "Chamfer", valueText: "1.0 mm", status: "ok" },
      ],
      changedBodies: [],
      removedBodies: [],
    });
    await rollToIndex(0);
    expect(documentStore.getState().features[0].opType).toBe("Chamfer");
    apply.mockRestore();
  });

  it("deleteFeature sends RemoveOperation and drops the feature from the store", async () => {
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    expect(documentStore.getState().features.some((f) => f.id === "f3")).toBe(true);
    await deleteFeature("f3");
    expect(apply.mock.calls[0][0]).toEqual({ cmd: "removeOperation", record: "f3" });
    expect(documentStore.getState().features.some((f) => f.id === "f3")).toBe(false);
    apply.mockRestore();
  });
});

describe("historyActions — rebind flow (promote → EditOperationInput)", () => {
  const item: NeedsRepairItem = {
    opId: "f3",
    refId: "f3.input1",
    reason: "ambiguous",
    candidateCount: 2,
  };
  const candidate: ResolveCandidate = {
    topoKey: "e:7",
    score: 0.9,
    margin: 0.02,
    worldPos: [4, 5, 6],
    summary: "linear edge",
  };

  it("promotes the candidate then sends EditOperationInput at the parsed slot", async () => {
    const promote = vi.spyOn(mockClient, "promoteSelection");
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    const ok = await rebindCandidate(item, candidate);
    expect(ok).toBe(true);
    // (a) promotion: topoKey + worldPos anchor on the (single) seed body.
    expect(promote.mock.calls[0][0]).toBe("body1");
    expect(promote.mock.calls[0][1]).toEqual([{ topoKey: "e:7", anchor: { worldPoint: [4, 5, 6] } }]);
    // (c) rebind: EditOperationInput{FilletEdges{index:1}} carrying a typed edge ref
    // (primary edge + anchor worldPoint) — the minted elementId + worldPos.
    const cmd = apply.mock.calls[0][0];
    expect(cmd).toMatchObject({
      cmd: "editOperationInput",
      record: "f3",
      path: { path: "filletEdges", index: 1 },
    });
    expect((cmd as { reference: { element: { primary: { kind: string } } } }).reference.element.primary.kind).toBe(
      "edge",
    );
    promote.mockRestore();
    apply.mockRestore();
  });

  it("refuses with a sticky ambiguity hint when >1 body and none/many are selected (never guesses)", async () => {
    documentStore.setState({
      bodies: {
        body1: { id: "body1", name: "Body 1", visible: true },
        body2: { id: "body2", name: "Body 2", visible: true },
      },
    });
    selectionStore.getState().clear(); // no explicit user statement
    const promote = vi.spyOn(mockClient, "promoteSelection");
    const ok = await rebindCandidate(item, candidate);
    expect(ok).toBe(false);
    expect(promote).not.toHaveBeenCalled();
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toBe(
      "Cannot repair: operated body is ambiguous (2 bodies). Select the body this feature operated on first.",
    );
    promote.mockRestore();
  });

  it("honors an explicit single-body selection as the operated body when >1 body exists", async () => {
    documentStore.setState({
      bodies: {
        body1: { id: "body1", name: "Body 1", visible: true },
        body2: { id: "body2", name: "Body 2", visible: true },
      },
    });
    selectionStore.getState().set([{ kind: "body", id: "body2" }]);
    const promote = vi.spyOn(mockClient, "promoteSelection");
    const apply = vi.spyOn(mockClient, "applyEditCommand");
    const ok = await rebindCandidate(item, candidate);
    expect(ok).toBe(true);
    expect(promote.mock.calls[0][0]).toBe("body2"); // the explicitly-selected body, not a guess
    promote.mockRestore();
    apply.mockRestore();
  });
});
