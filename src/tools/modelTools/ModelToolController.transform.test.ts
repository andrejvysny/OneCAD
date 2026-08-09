/*
 * Placement tool wiring (WP-B W1) — the FOLD DECISION and the commit it produces.
 *
 * The decision has no visible surface of its own: fold and append look identical
 * until the history row count is read, and by then a wrong choice has already
 * written a second record (or, worse, silently re-pointed an existing one at
 * bodies it never covered). So it is pinned here, at the seam:
 *
 *   - `canFoldTransform` is the backend's answer and is authoritative for
 *     "may this fold at all";
 *   - the frontend adds ONE condition on top — the stored targets must equal the
 *     current selection — because the query is asked per body and a partial match
 *     would silently widen the record's scope;
 *   - a fold commits `featureId`, a fresh arm commits none.
 *
 * Re-edit seeding is held to the same standard as the pattern/mirror re-edits:
 * the targets come from the STORED params, never the selection, and an
 * unrepresentable stored shape refuses loudly instead of guessing.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { ApplyOperationResult, OperationOp } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { documentStore } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { viewportStore } from "@/stores/viewportStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";
import { buildBodyObjects, swap as swapMesh, __resetRegistryForTests } from "@/viewport/mesh/meshRegistry";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { makeBoxMesh } from "@/ipc/mockMeshes";

const okResult = (): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#0" }],
  removedBodies: [],
});

/** Every method `onToolChange`'s teardown sweep + the placement ghost touch. */
function makeEngineMock() {
  return {
    showGhostPreview: vi.fn(),
    showGhostPreviewMulti: vi.fn(),
    hideGhostPreview: vi.fn(),
    hideValueHandle: vi.fn(),
    showValueHandle: vi.fn(),
    setPreviewReplacedBodyIds: vi.fn(),
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
    // Placement gizmo (WP-B W2) — armed/teardown paths touch all four.
    showTransformGizmo: vi.fn(),
    hideTransformGizmo: vi.fn(),
    setTransformGizmoHover: vi.fn(),
    setTransformGizmoActive: vi.fn(),
    hitTransformGizmo: vi.fn(() => null),
  };
}

function makeClientMock() {
  return {
    onPreviewResult: vi.fn(() => () => {}),
    applyOperation: vi.fn(() => Promise.resolve(okResult())),
    getOperationParams: vi.fn(() => Promise.resolve({})),
    canFoldTransform: vi.fn(() => Promise.resolve<string | null>(null)),
  };
}

/** The stored wire params of a `body1` move of 30mm along X, pivot at the origin. */
const STORED_MOVE_30 = {
  targets: ["body1"],
  translate: [{ value: 30 }, { value: 0 }, { value: 0 }],
  rotate: { center: [0, 0, 0], axis: [0, 0, 1], angleDeg: { value: 0 } },
  copy: false,
};

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

/** The status hint's message (the store carries `{message, severity, sticky}`). */
function hintText(): string | null {
  const hint = viewportStore.getState().statusHint as { message?: string } | string | null;
  return typeof hint === "string" ? hint : (hint?.message ?? null);
}

/**
 * Register a real registry entry for `bodyId` so the ghost path has geometry to
 * clone. `getEntry` returning undefined is an honest early-out (nothing ingested
 * ⇒ nothing on screen to hide either), so the hide/restore behaviour can only be
 * exercised against a body the registry actually knows.
 */
function registerBody(bodyId: string, blob: ArrayBuffer): void {
  swapMesh(bodyId, buildBodyObjects(parseMeshPayload(blob), bodyId, 1));
}

describe("ModelToolController placement (TransformBody)", () => {
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

  /** Select bodies, arm the tool, and let the async fold query settle. */
  async function armOn(...bodyIds: string[]): Promise<void> {
    selectionStore.getState().set(bodyIds.map((id) => ({ kind: "body", id })));
    toolStore.getState().setTool("transform");
    await flush();
  }

  /** The op of the most recent `applyOperation` call. */
  const lastOp = (): OperationOp => {
    const calls = clientMock.applyOperation.mock.calls as unknown as OperationOp[][];
    return calls[calls.length - 1][0];
  };

  beforeEach(() => {
    resetStores();
    __resetRegistryForTests();
    container = document.createElement("div");
    document.body.appendChild(container);
    documentStore.setState({
      bodies: {
        body1: { id: "body1", name: "Body 1", visible: true },
        body2: { id: "body2", name: "Body 2", visible: true },
      },
    });
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
  });

  // ── arming ────────────────────────────────────────────────────────────────

  it("asks for a body when nothing is selected, and does not arm", async () => {
    build();
    await armOn();
    expect(hintText()).toBe("Select a body to move");
    expect(toolChipStore.getState().kind).toBe("none");
    expect(clientMock.canFoldTransform).not.toHaveBeenCalled();
  });

  it("arms the Move·X·0 cluster on a body selection", async () => {
    build();
    await armOn("body1");
    const chip = toolChipStore.getState();
    expect(chip.kind).toBe("transform");
    expect(chip.transformMode).toBe("move");
    expect(chip.axis).toBe("X");
    expect(chip.value).toBe(0);
    expect(toolStore.getState().phase).toBe("armed");
  });

  it("queries the fold lineage for the FIRST target only", async () => {
    build();
    await armOn("body2", "body1");
    expect(clientMock.canFoldTransform).toHaveBeenCalledTimes(1);
    expect(clientMock.canFoldTransform).toHaveBeenCalledWith("body2");
  });

  // ── the fold decision ─────────────────────────────────────────────────────

  it("FOLDS into the named record: seeds its params and commits with its featureId", async () => {
    build();
    clientMock.canFoldTransform.mockResolvedValue("rec-move");
    clientMock.getOperationParams.mockResolvedValue(STORED_MOVE_30);

    await armOn("body1");
    // The chip opens on the STORED cumulative value, not on a bare 0.
    expect(toolChipStore.getState().value).toBe(30);

    toolChipStore.getState().onValue!(50);
    toolChipStore.getState().onConfirm!();
    await flush();

    const op = lastOp();
    expect(op.featureId).toBe("rec-move"); // re-edit, NOT a second record
    expect(op.opType).toBe("TransformBody");
    if (op.opType !== "TransformBody") throw new Error("unreachable");
    expect(op.params.translate).toEqual([50, 0, 0]); // replaces, never accumulates
  });

  it("APPENDS when the backend says the body has no foldable record", async () => {
    build();
    await armOn("body1");
    toolChipStore.getState().onValue!(30);
    toolChipStore.getState().onConfirm!();
    await flush();
    expect(lastOp().featureId).toBeUndefined();
  });

  it("APPENDS when the fold candidate's targets are not the current selection", async () => {
    // The query is per-BODY: body1 alone was moved before, but the user has now
    // selected body1 + body2. Folding would silently start moving body2 too.
    build();
    clientMock.canFoldTransform.mockResolvedValue("rec-move");
    clientMock.getOperationParams.mockResolvedValue(STORED_MOVE_30);

    await armOn("body1", "body2");
    toolChipStore.getState().onValue!(30);
    toolChipStore.getState().onConfirm!();
    await flush();

    const op = lastOp();
    expect(op.featureId).toBeUndefined();
    if (op.opType !== "TransformBody") throw new Error("unreachable");
    expect(op.params.targets).toEqual(["body1", "body2"]);
  });

  it("APPENDS when the fold candidate's stored params cannot be read", async () => {
    build();
    clientMock.canFoldTransform.mockResolvedValue("rec-move");
    clientMock.getOperationParams.mockRejectedValue(new Error("gone"));
    await armOn("body1");
    toolChipStore.getState().onValue!(30);
    toolChipStore.getState().onConfirm!();
    await flush();
    expect(lastOp().featureId).toBeUndefined();
  });

  it("APPENDS when the lineage query itself fails — never blocks the gesture", async () => {
    build();
    clientMock.canFoldTransform.mockRejectedValue(new Error("no document"));
    await armOn("body1");
    expect(toolChipStore.getState().kind).toBe("transform");
    toolChipStore.getState().onValue!(30);
    toolChipStore.getState().onConfirm!();
    await flush();
    expect(lastOp().featureId).toBeUndefined();
  });

  // ── commit shape ──────────────────────────────────────────────────────────

  it("commits inputs[] mirroring params.targets in order (SCHEMA §7.3)", async () => {
    build();
    await armOn("body2", "body1");
    toolChipStore.getState().onValue!(10);
    toolChipStore.getState().onConfirm!();
    await flush();

    const op = lastOp();
    if (op.opType !== "TransformBody") throw new Error("unreachable");
    expect(op.params.targets).toEqual(["body2", "body1"]);
    expect(op.inputs).toEqual([
      { primary: { bodyId: "body2", kind: "body" } },
      { primary: { bodyId: "body1", kind: "body" } },
    ]);
  });

  it("a rotation commits angleDeg about the picked world axis with a zero translate", async () => {
    build();
    await armOn("body1");
    toolChipStore.getState().onTransformMode!("rotate");
    toolChipStore.getState().onAxis!("Z");
    toolChipStore.getState().onValue!(90);
    toolChipStore.getState().onConfirm!();
    await flush();

    const op = lastOp();
    if (op.opType !== "TransformBody") throw new Error("unreachable");
    expect(op.params.translate).toEqual([0, 0, 0]);
    expect(op.params.rotate.axis).toEqual([0, 0, 1]);
    expect(op.params.rotate.angleDeg).toBe(90);
  });

  it("resets to select with a hint naming what changed, and selects the placed bodies", async () => {
    build();
    await armOn("body1");
    toolChipStore.getState().onValue!(30);
    toolChipStore.getState().onConfirm!();
    await flush();

    expect(toolStore.getState().modelTool).toBe("select");
    expect(hintText()).toBe("Moved");
    expect(selectionStore.getState().selected).toEqual([{ kind: "body", id: "body1" }]);
    expect(toolChipStore.getState().kind).toBe("none");
  });

  it("a REFUSED commit stays armed with the reason — the typed numbers survive", async () => {
    build();
    clientMock.applyOperation.mockRejectedValue(new Error("overlapping solids"));
    await armOn("body1");
    toolChipStore.getState().onValue!(30);
    toolChipStore.getState().onConfirm!();
    await flush();

    expect(toolStore.getState().modelTool).toBe("transform");
    expect(toolChipStore.getState().kind).toBe("transform");
    expect(toolChipStore.getState().value).toBe(30);
    expect(hintText()).toBe("Move failed: overlapping solids");
  });

  // ── the ghost ─────────────────────────────────────────────────────────────

  it("hides the SOURCE bodies only once the placement is non-identity, and restores on cancel", async () => {
    build();
    registerBody("body1", makeBoxMesh());
    await armOn("body1");
    // A zero placement shows nothing — there is no motion to preview yet.
    expect(engineMock.setPreviewReplacedBodyIds).not.toHaveBeenCalled();

    toolChipStore.getState().onValue!(30);
    expect(engineMock.setPreviewReplacedBodyIds).toHaveBeenCalledWith(["body1"]);

    toolChipStore.getState().onCancel!();
    expect(engineMock.setPreviewReplacedBodyIds).toHaveBeenLastCalledWith([]);
  });

  it("never touches the preview lane's hidden-body snapshot when no placement was armed", () => {
    build();
    toolStore.getState().setTool("fillet"); // runs the full cancel sweep
    expect(engineMock.setPreviewReplacedBodyIds).not.toHaveBeenCalled();
  });

  // ── re-edit from history ──────────────────────────────────────────────────

  it("editTransformFeature seeds targets + the full stored placement from the RECORD", async () => {
    build();
    clientMock.getOperationParams.mockResolvedValue({
      targets: ["body2"],
      translate: [{ value: 0 }, { value: 0 }, { value: 0 }],
      rotate: { center: [5, 6, 7], axis: [0, 1, 0], angleDeg: { value: 45 } },
      copy: false,
    });
    documentStore.setState({
      features: [
        { id: "feat-mv", kind: "boolean", opType: "TransformBody", label: "Move", valueText: "45.0°", status: "ok" },
      ],
    });
    // A DIFFERENT body is selected — the re-edit must ignore it entirely.
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);

    await controller.editTransformFeature("feat-mv");
    await flush();

    const chip = toolChipStore.getState();
    expect(chip.kind).toBe("transform");
    expect(chip.transformMode).toBe("rotate"); // opened on the component that carries the placement
    expect(chip.axis).toBe("Y");
    expect(chip.value).toBe(45);

    chip.onValue!(90);
    chip.onConfirm!();
    await flush();

    const op = lastOp();
    expect(op.featureId).toBe("feat-mv"); // the SAME row
    if (op.opType !== "TransformBody") throw new Error("unreachable");
    expect(op.params.targets).toEqual(["body2"]);
    expect(op.params.rotate.angleDeg).toBe(90);
    // The FROZEN pivot survives the re-edit rather than being re-derived.
    expect(op.params.rotate.center).toEqual([5, 6, 7]);
  });

  /**
   * A stored rotation about an OFF-AXIS vector was refused outright in W1: no
   * chip could show it, so arming would have let a ✓ rewrite it to a world axis
   * the user never chose. WP-B W2.5 makes such a record a normal frontend
   * product (an align flushes two arbitrary faces about their normals' cross
   * product), so the vector is now carried VERBATIM instead — which closes the
   * same data-loss hole without also breaking the fold rule for aligned bodies.
   */
  it("editTransformFeature carries a stored NON-world rotation axis through unchanged", async () => {
    build();
    clientMock.getOperationParams.mockResolvedValue({
      targets: ["body1"],
      translate: [{ value: 0 }, { value: 0 }, { value: 0 }],
      rotate: { center: [0, 0, 0], axis: [0.6, 0, 0.8], angleDeg: { value: 30 } },
      copy: false,
    });
    documentStore.setState({
      features: [
        { id: "feat-mv", kind: "boolean", opType: "TransformBody", label: "Move", valueText: "30.0°", status: "ok" },
      ],
    });

    await controller.editTransformFeature("feat-mv");
    await flush();
    expect(toolStore.getState().modelTool).toBe("transform");

    toolChipStore.getState().onConfirm?.();
    await flush();
    const op = lastOp();
    if (op.opType !== "TransformBody") throw new Error("unreachable");
    expect(op.featureId).toBe("feat-mv");
    expect(op.params.rotate.axis).toEqual([0.6, 0, 0.8]); // NOT snapped to Z
    expect(op.params.rotate.angleDeg).toBe(30);
  });

  it("editTransformFeature still refuses a stored axis that is not a direction at all", async () => {
    build();
    clientMock.getOperationParams.mockResolvedValue({
      targets: ["body1"],
      translate: [{ value: 0 }, { value: 0 }, { value: 0 }],
      rotate: { center: [0, 0, 0], axis: [0, 0, 0], angleDeg: { value: 30 } },
      copy: false,
    });
    documentStore.setState({
      features: [
        { id: "feat-mv", kind: "boolean", opType: "TransformBody", label: "Move", valueText: "30.0°", status: "ok" },
      ],
    });

    await controller.editTransformFeature("feat-mv");
    await flush();

    expect(hintText()).toBe("Cannot re-edit move: stored placement has no usable rotation axis");
    expect(toolChipStore.getState().kind).toBe("none");
    expect(toolStore.getState().modelTool).not.toBe("transform");
  });

  it("editTransformFeature ignores a row that is not a TransformBody (opType-gated, not kind)", async () => {
    build();
    documentStore.setState({
      features: [
        { id: "feat-mir", kind: "boolean", opType: "MirrorBody", label: "Mirror", valueText: "", status: "ok" },
      ],
    });
    await controller.editTransformFeature("feat-mir");
    await flush();
    expect(clientMock.getOperationParams).not.toHaveBeenCalled();
    expect(toolChipStore.getState().kind).toBe("none");
  });

  it("the tool's own fresh arm cannot land after a re-edit and clobber the record", async () => {
    build();
    clientMock.canFoldTransform.mockResolvedValue("rec-other");
    clientMock.getOperationParams.mockResolvedValue({
      targets: ["body2"],
      translate: [{ value: 12 }, { value: 0 }, { value: 0 }],
      rotate: { center: [0, 0, 0], axis: [0, 0, 1], angleDeg: { value: 0 } },
      copy: false,
    });
    documentStore.setState({
      features: [
        { id: "feat-mv", kind: "boolean", opType: "TransformBody", label: "Move", valueText: "Δ(12.0, 0.0, 0.0)", status: "ok" },
      ],
    });
    selectionStore.getState().set([{ kind: "body", id: "body1" }]);

    await controller.editTransformFeature("feat-mv");
    await flush();
    await flush(); // let the superseded fresh arm's query resolve too

    toolChipStore.getState().onConfirm!();
    await flush();
    expect(lastOp().featureId).toBe("feat-mv");
  });
});
