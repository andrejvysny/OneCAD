/*
 * Pattern/Mirror/Shell re-edit honesty (jsdom). `editLinearPatternFeature` /
 * `editCircularPatternFeature` / `editMirrorFeature` used to guess the source
 * body (selected-else-first) and seed only count/angle, silently resetting the
 * axis/spacing/plane on every re-edit. These specs prove the fix: the source
 * body comes from the STORED `sourceBodyId` (never a guess), the FULL stored
 * shape survives the re-arm, and an unresolvable body/axis/plane refuses with a
 * sticky hint instead of falling back.
 *
 * EVERY fixture here carries the REAL backend projection shape, not the mock's
 * old invented kinds: `dto.rs feature_kind` folds LinearPattern/CircularPattern/
 * MirrorBody → `boolean` and Shell → `fillet`, so a re-edit entry point that
 * gates on `kind` is dead on the Tauri lane. The guards gate on `opType`.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { ApplyOperationResult } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";

const okResult = (): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#0" }],
  removedBodies: [],
});

// `onToolChange` unconditionally tears down every OTHER tool's transient state
// (cancelPreview/cancelFillet/cancelBoolean/cancelRevolve/cancelShell) before
// arming the new one, so the engine fake needs every method those paths touch —
// mirrors ModelToolController.commit.test.ts's fuller engine mock.
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

function makeClientMock() {
  return {
    onPreviewResult: vi.fn(() => () => {}),
    applyOperation: vi.fn(() => Promise.resolve(okResult())),
    getOperationParams: vi.fn(() => Promise.resolve({})),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("ModelToolController pattern/mirror re-edit honesty", () => {
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
    container = document.createElement("div");
    document.body.appendChild(container);
    documentStore.setState({
      bodies: {
        body1: { id: "body1", name: "Body 1", visible: true },
      },
    });
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
  });

  // ── linear pattern ──────────────────────────────────────────────────────────

  it("editLinearPatternFeature seeds axis + spacing + count from the stored params (no reset to defaults)", async () => {
    build();
    clientMock.getOperationParams.mockResolvedValue({
      sourceBodyId: "body1",
      direction: [0, 1, 0], // Y — NOT the FSM default (X)
      spacing: { value: 33 },
      count: 7,
      fuseResult: true,
    });
    documentStore.setState({
      features: [{ id: "feat-lp", kind: "boolean", opType: "LinearPattern", label: "Linear Pattern", valueText: "×7", status: "ok" }],
    });

    await controller.editLinearPatternFeature("feat-lp");
    await flush();

    expect(toolStore.getState().modelTool).toBe("linearPattern");
    expect(toolChipStore.getState().kind).toBe("linearPattern");
    expect(toolChipStore.getState().axis).toBe("Y"); // survived, not reset to X
    expect(toolChipStore.getState().value).toBe(33); // spacing survived
    expect(toolChipStore.getState().count).toBe(7);
  });

  it("editLinearPatternFeature refuses when the stored sourceBodyId is missing/foreign (no fallback)", async () => {
    build();
    clientMock.getOperationParams.mockResolvedValue({
      sourceBodyId: "body-deleted",
      direction: [1, 0, 0],
      spacing: { value: 20 },
      count: 3,
    });
    documentStore.setState({
      features: [{ id: "feat-lp", kind: "boolean", opType: "LinearPattern", label: "Linear Pattern", valueText: "×3", status: "ok" }],
    });

    await controller.editLinearPatternFeature("feat-lp");
    await flush();

    expect(toolStore.getState().modelTool).not.toBe("linearPattern"); // never armed
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("source body is missing");
  });

  it("editLinearPatternFeature refuses when the stored direction is not a world axis (never snaps to X)", async () => {
    build();
    clientMock.getOperationParams.mockResolvedValue({
      sourceBodyId: "body1",
      direction: [1, 1, 0], // not X/Y/Z
      spacing: { value: 20 },
      count: 3,
    });
    documentStore.setState({
      features: [{ id: "feat-lp", kind: "boolean", opType: "LinearPattern", label: "Linear Pattern", valueText: "×3", status: "ok" }],
    });

    await controller.editLinearPatternFeature("feat-lp");
    await flush();

    expect(toolStore.getState().modelTool).not.toBe("linearPattern");
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("not a world axis");
  });

  // ── circular pattern ────────────────────────────────────────────────────────

  it("editCircularPatternFeature seeds axis + angle + count from the stored params", async () => {
    build();
    clientMock.getOperationParams.mockResolvedValue({
      sourceBodyId: "body1",
      axisOrigin: [0, 0, 0],
      axisDirection: [1, 0, 0], // X — NOT the FSM default (Z)
      angleDeg: { value: 180 },
      count: 5,
      fuseResult: true,
    });
    documentStore.setState({
      features: [
        { id: "feat-cp", kind: "boolean", opType: "CircularPattern", label: "Circular Pattern", valueText: "×5", status: "ok" },
      ],
    });

    await controller.editCircularPatternFeature("feat-cp");
    await flush();

    expect(toolStore.getState().modelTool).toBe("circularPattern");
    expect(toolChipStore.getState().axis).toBe("X"); // survived, not reset to Z
    expect(toolChipStore.getState().value).toBe(180); // angle survived
    expect(toolChipStore.getState().count).toBe(5);
  });

  it("editCircularPatternFeature refuses when the stored source body no longer exists", async () => {
    build();
    clientMock.getOperationParams.mockResolvedValue({
      axisOrigin: [0, 0, 0],
      axisDirection: [0, 0, 1],
      angleDeg: { value: 360 },
      count: 4,
      // sourceBodyId absent entirely
    });
    documentStore.setState({
      features: [
        { id: "feat-cp", kind: "boolean", opType: "CircularPattern", label: "Circular Pattern", valueText: "×4", status: "ok" },
      ],
    });

    await controller.editCircularPatternFeature("feat-cp");
    await flush();

    expect(toolStore.getState().modelTool).not.toBe("circularPattern");
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("source body is missing");
  });

  // ── mirror ───────────────────────────────────────────────────────────────────

  it("editMirrorFeature seeds the plane from the stored planeNormal", async () => {
    build();
    clientMock.getOperationParams.mockResolvedValue({
      sourceBodyId: "body1",
      planePoint: [0, 0, 0],
      planeNormal: [1, 0, 0], // YZ — NOT the FSM default (XY)
      fuseWithOriginal: false,
    });
    documentStore.setState({
      features: [{ id: "feat-mi", kind: "boolean", opType: "MirrorBody", label: "Mirror", valueText: "YZ", status: "ok" }],
    });

    await controller.editMirrorFeature("feat-mi");
    await flush();

    expect(toolStore.getState().modelTool).toBe("mirror");
    expect(toolChipStore.getState().plane).toBe("YZ"); // survived, not reset to XY
  });

  it("editMirrorFeature refuses when the stored planeNormal is not a world plane", async () => {
    build();
    clientMock.getOperationParams.mockResolvedValue({
      sourceBodyId: "body1",
      planePoint: [0, 0, 0],
      planeNormal: [1, 1, 1],
      fuseWithOriginal: false,
    });
    documentStore.setState({
      features: [{ id: "feat-mi", kind: "boolean", opType: "MirrorBody", label: "Mirror", valueText: "?", status: "ok" }],
    });

    await controller.editMirrorFeature("feat-mi");
    await flush();

    expect(toolStore.getState().modelTool).not.toBe("mirror");
    const hint = viewportStore.getState().statusHint;
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toContain("not a world plane");
  });

  // ── shell ────────────────────────────────────────────────────────────────────

  it("editShellFeature arms the shell tool on the REAL projection shape (kind fillet + opType Shell)", async () => {
    build();
    clientMock.getOperationParams.mockResolvedValue({
      thickness: { value: 3.5 },
      openFaces: ["e1"],
      targetBodyId: "body1",
    });
    documentStore.setState({
      features: [{ id: "feat-sh", kind: "fillet", opType: "Shell", label: "Shell", valueText: "3.5 mm", status: "ok" }],
    });

    await controller.editShellFeature("feat-sh");
    await flush();

    expect(toolStore.getState().modelTool).toBe("shell");
    expect(toolChipStore.getState().value).toBe(3.5);
  });

  it("editShellFeature ignores a fillet feature that merely shares the folded `fillet` kind", async () => {
    build();
    documentStore.setState({
      features: [{ id: "feat-fi", kind: "fillet", opType: "Fillet", label: "Fillet", valueText: "2.0 mm", status: "ok" }],
    });

    await controller.editShellFeature("feat-fi");
    await flush();

    expect(toolStore.getState().modelTool).not.toBe("shell");
  });

  // ── committed shape (proves the seeded values round-trip into the commit) ────

  it("committing a re-edited linear pattern sends the SEEDED axis/spacing, not the FSM default", async () => {
    build();
    clientMock.getOperationParams.mockResolvedValue({
      sourceBodyId: "body1",
      direction: [0, 1, 0],
      spacing: { value: 33 },
      count: 7,
    });
    documentStore.setState({
      features: [{ id: "feat-lp", kind: "boolean", opType: "LinearPattern", label: "Linear Pattern", valueText: "×7", status: "ok" }],
    });

    await controller.editLinearPatternFeature("feat-lp");
    await flush();
    toolChipStore.getState().onApply?.();
    await flush();

    expect(clientMock.applyOperation).toHaveBeenCalledWith(
      expect.objectContaining({
        opType: "LinearPattern",
        featureId: "feat-lp",
        params: expect.objectContaining({
          sourceBodyId: "body1",
          direction: [0, 1, 0],
          spacing: 33,
          count: 7,
        }),
      }),
    );
  });

  it("the pattern commit's done hint SURVIVES its reset to select", async () => {
    // Hint-clobber regression: `commitPattern` published the done hint and THEN
    // called setTool("select"), whose synchronous sweep cleared it.
    build();
    clientMock.getOperationParams.mockResolvedValue({
      sourceBodyId: "body1",
      direction: [0, 1, 0],
      spacing: { value: 33 },
      count: 7,
    });
    documentStore.setState({
      features: [{ id: "feat-lp", kind: "boolean", opType: "LinearPattern", label: "Linear Pattern", valueText: "×7", status: "ok" }],
    });

    await controller.editLinearPatternFeature("feat-lp");
    await flush();
    toolChipStore.getState().onApply?.();
    await flush();

    expect(toolStore.getState().modelTool).toBe("select");
    expect(viewportStore.getState().statusHint?.message).toBe("Linear pattern ×7");
  });
});
