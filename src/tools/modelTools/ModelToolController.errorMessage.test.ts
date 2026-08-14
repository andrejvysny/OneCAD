/*
 * WP0 red test — resolved `ApplyOperationResult.errorMessage` must NOT be treated
 * as success.
 *
 * The controller's `applyResult()` ignores the `errorMessage` field and hydrates
 * bodies/features regardless. Several commit paths then announce a success hint.
 * This test arms a linear pattern, commits, and mocks `applyOperation` to resolve
 * with `errorMessage` set. The expected behavior: NO success hint, tool stays armed.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { ApplyOperationResult } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";

const errorResult = (): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#0" }],
  removedBodies: [],
  errorMessage: "pattern self-intersects",
});

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
    applyOperation: vi.fn(() => Promise.resolve(errorResult())),
    getOperationParams: vi.fn(() => Promise.resolve({})),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("ModelToolController errorMessage WP0", () => {
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

  it("does not treat a resolved errorMessage as success", async () => {
    build();
    selectionStore.setState({ selected: [{ kind: "body", id: "body1" }] });

    // Arm the linear pattern tool via keyboard shortcut simulation.
    toolStore.getState().setTool("linearPattern");
    await flush();

    expect(toolChipStore.getState().kind).toBe("linearPattern");

    // Commit through the chip's onApply handler.
    toolChipStore.getState().onConfirm?.();
    await flush();

    // The commit should NOT have announced a success hint.
    const hint = viewportStore.getState().statusHint;
    expect(hint?.message).not.toMatch(/Linear pattern/);
    expect(hint?.severity).toBe("error");
    expect(hint?.message).toMatch(/pattern self-intersects|failed/i);
  });
});
