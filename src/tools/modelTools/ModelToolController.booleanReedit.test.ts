/*
 * Boolean re-edit honesty (jsdom, TRUST wave). A committed Boolean CONSUMES its
 * tool body, so a re-edit can only swap the OPERATION — there is no body to
 * re-pick. These specs pin that contract:
 *   - the commit is an `UpdateOperationParams` that preserves the stored
 *     targetBodyId/toolBodyId VERBATIM and changes only `operation` (never a
 *     second `applyOperation`, which would append a whole new boolean);
 *   - a RETIRED tool body still arms — chip-only, no body highlight, no re-pick;
 *   - a missing operation/target/tool, or a non-Boolean row, refuses with a
 *     sticky hint instead of guessing.
 *
 * The fixtures carry the REAL projection shape: `dto.rs feature_kind` folds the
 * pattern/mirror ops into kind `boolean`, so the guard is on `opType`.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { ApplyOperationResult } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { documentStore } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";

const okResult = (): ApplyOperationResult => ({
  revision: 7,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#0" }],
  removedBodies: [],
});

/** `onToolChange` tears every OTHER tool down before arming, so the engine fake
 *  needs every method those cancel paths touch. */
function makeEngineMock() {
  return {
    showGhostPreview: vi.fn(),
    hideGhostPreview: vi.fn(),
    hideValueHandle: vi.fn(),
    showValueHandle: vi.fn(),
    showGhostPreviewMulti: vi.fn(),
    probePick: vi.fn(() => null),
    hideExtrudePreview: vi.fn(),
    clearPreviewBody: vi.fn(),
    setPreviewTint: vi.fn(),
    setOrbitSuppressed: vi.fn(),
    setExtrudeHandle: vi.fn(),
    moveChip: vi.fn(),
    probeMaterial: vi.fn(() => null),
    hideRevolvePreview: vi.fn(),
    hideRegionPick: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => false),
  };
}

/** The params a committed Union stored (`get_operation_params` shape). */
const STORED_UNION = {
  operation: "Union",
  targetBodyId: "body1",
  toolBodyId: "body2-retired",
};

function makeClientMock() {
  let seq = 0;
  return {
    onPreviewResult: vi.fn(() => () => {}),
    applyOperation: vi.fn(() => Promise.resolve(okResult())),
    applyEditCommand: vi.fn(() => Promise.resolve(okResult())),
    getOperationParams: vi.fn(() => Promise.resolve({ ...STORED_UNION }) as Promise<Record<string, unknown>>),
    // A FRESH boolean commit now routes through the kernel-preview lane (mirrors
    // extrude); a RE-EDIT never opens one, so these three are unused by every spec
    // above except the trailing "tool switch mid-edit" one.
    beginPreview: vi.fn(() =>
      Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` }),
    ),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(okResult())),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

const booleanFeature = (id = "feat-bool") => ({
  id,
  kind: "boolean" as const,
  opType: "Boolean",
  label: "Union",
  valueText: "",
  status: "ok" as const,
});

describe("ModelToolController boolean re-edit (operation swap only)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;

  function build(): void {
    engineMock = makeEngineMock();
    clientMock = makeClientMock();
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
    });
  }

  beforeEach(() => {
    resetStores();
    // A FRESH boolean commit waits for the exact-preview barrier (mirrors extrude);
    // these specs assert MUTATION SEQUENCING, not the barrier itself, so collapse
    // it to one macrotask.
    __setExactPreviewTimeoutForTests(0);
    container = document.createElement("div");
    document.body.appendChild(container);
    // Only the TARGET survives a committed boolean — the tool body is consumed.
    documentStore.setState({
      bodies: { body1: { id: "body1", name: "Body 1", visible: true } },
      features: [booleanFeature()],
    });
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  it("arms chip-only on a RETIRED tool body: no body highlight, no re-pick offered", async () => {
    build();
    selectionStore.getState().clear();

    await controller.editBooleanFeature("feat-bool");
    await flush();

    expect(toolStore.getState().modelTool).toBe("boolean");
    expect(toolStore.getState().phase).toBe("armed");
    // The chip is the whole affordance, seeded with the STORED operation.
    expect(toolChipStore.getState().kind).toBe("booleanOp");
    expect(toolChipStore.getState().op).toBe("Union");
    // The consumed tool body is not in the document, so nothing is highlighted —
    // a body selection here would imply a re-pick that cannot exist.
    expect(selectionStore.getState().selected).toEqual([]);
    expect(viewportStore.getState().statusHint?.message).toBe("Change the boolean operation · Enter or ✓ to confirm");
  });

  it("Apply sends UpdateOperationParams with the stored bodies VERBATIM and only `operation` swapped", async () => {
    build();

    await controller.editBooleanFeature("feat-bool");
    await flush();
    toolChipStore.getState().onOp?.("Cut"); // chip segment: Union → Cut
    toolChipStore.getState().onConfirm?.();
    await flush();

    expect(clientMock.applyOperation).not.toHaveBeenCalled(); // never appends a 2nd boolean
    expect(clientMock.applyEditCommand).toHaveBeenCalledWith({
      cmd: "updateOperationParams",
      record: "feat-bool",
      op: {
        opType: "Boolean",
        params: { operation: "Cut", targetBodyId: "body1", toolBodyId: "body2-retired" },
      },
    });
    // Settling back to `select` clears the transient hint (onToolChange), so the
    // observable end state is the tool, not the message.
    expect(toolStore.getState().modelTool).toBe("select");
  });

  it("Apply without touching the chip re-sends the SAME operation (idempotent, still preserves the bodies)", async () => {
    build();

    await controller.editBooleanFeature("feat-bool");
    await flush();
    toolChipStore.getState().onConfirm?.();
    await flush();

    expect(clientMock.applyEditCommand).toHaveBeenCalledWith(
      expect.objectContaining({
        cmd: "updateOperationParams",
        record: "feat-bool",
        op: {
          opType: "Boolean",
          params: { operation: "Union", targetBodyId: "body1", toolBodyId: "body2-retired" },
        },
      }),
    );
  });

  it("highlights BOTH bodies when the tool body somehow still exists", async () => {
    build();
    documentStore.setState({
      bodies: {
        body1: { id: "body1", name: "Body 1", visible: true },
        "body2-retired": { id: "body2-retired", name: "Body 2", visible: true },
      },
    });

    await controller.editBooleanFeature("feat-bool");
    await flush();

    expect(selectionStore.getState().selected).toEqual([
      { kind: "body", id: "body1" },
      { kind: "body", id: "body2-retired" },
    ]);
  });

  it("refuses (sticky hint, never arms) when the stored params carry no tool body", async () => {
    build();
    clientMock.getOperationParams.mockResolvedValue({ operation: "Union", targetBodyId: "body1" });

    await controller.editBooleanFeature("feat-bool");
    await flush();

    expect(toolStore.getState().modelTool).not.toBe("boolean");
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("stored operation or bodies are missing");
  });

  it("refuses when the stored operation is not one the chip offers (never defaults to Union)", async () => {
    build();
    clientMock.getOperationParams.mockResolvedValue({
      operation: "Subtract", // not a BooleanOperation
      targetBodyId: "body1",
      toolBodyId: "body2-retired",
    });

    await controller.editBooleanFeature("feat-bool");
    await flush();

    expect(toolStore.getState().modelTool).not.toBe("boolean");
    expect(viewportStore.getState().statusHint?.severity).toBe("error");
  });

  it("ignores a LinearPattern row that merely shares the folded `boolean` kind", async () => {
    build();
    documentStore.setState({
      features: [
        { id: "feat-lp", kind: "boolean", opType: "LinearPattern", label: "Linear Pattern", valueText: "×3", status: "ok" },
      ],
    });

    await controller.editBooleanFeature("feat-lp");
    await flush();

    expect(clientMock.getOperationParams).not.toHaveBeenCalled();
    expect(toolStore.getState().modelTool).not.toBe("boolean");
  });

  it("a tool switch mid-edit drops the edit id, so the next boolean commit is a FRESH kernel-preview commit, not an edit", async () => {
    build();

    await controller.editBooleanFeature("feat-bool");
    await flush();
    toolStore.getState().setTool("select"); // cancelBoolean clears the edit context
    await flush();

    // Re-arm a FRESH boolean on two live bodies and commit it.
    documentStore.setState({
      bodies: {
        body1: { id: "body1", name: "Body 1", visible: true },
        body3: { id: "body3", name: "Body 3", visible: true },
      },
    });
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);
    toolStore.getState().setTool("boolean");
    controller.forceBooleanPick("body3");
    await flush(); // let the pick's beginPreview open the lane session

    expect(clientMock.beginPreview).toHaveBeenCalledWith({
      opType: "Boolean",
      inputs: [
        { primary: { bodyId: "body1", kind: "body" } },
        { primary: { bodyId: "body3", kind: "body" } },
      ],
      params: { operation: "Union", targetBodyId: "body1", toolBodyId: "body3" },
    });

    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush(); // exact-preview barrier (collapsed to one macrotask) + endPreview

    // A FRESH commit routes through the SAME lane session as its preview — never a
    // second `applyOperation` (which the old direct-commit path used) and never an
    // `applyEditCommand` against the stale (now-cleared) edit feature id.
    expect(clientMock.applyOperation).not.toHaveBeenCalled();
    expect(clientMock.applyEditCommand).not.toHaveBeenCalled();
    expect(clientMock.endPreview).toHaveBeenCalledWith(expect.any(String), true);
  });
});
