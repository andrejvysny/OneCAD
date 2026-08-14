/*
 * Modeling interaction contract — SUCCESS-SELECTION probe (U0 red evidence).
 *
 * The frozen contract declares `successSelection: "allChildOutputs"` for
 * boolean: after a commit the user is left holding what the operation actually
 * produced, which for a split result is every deterministic child output.
 *
 * Production ignores the result entirely and always selects the stored target
 * (`ModelToolController.ts:7454` re-edit, `:7536` fresh) — and does so
 * unconditionally, so a FAILED commit also moves the selection. RED until U1
 * routes selection through the classifier and U5's policy.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";

import type { CadClient } from "@/ipc/client";
import type { ApplyOperationResult } from "@/ipc/types";
import { documentStore } from "@/stores/documentStore";
import { selectionStore } from "@/stores/selectionStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";

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

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

const STORED_CUT = { operation: "Cut", targetBodyId: "body1", toolBodyId: "body2-retired" };

describe("modeling interaction contract — boolean success selection", () => {
  let container: HTMLDivElement;
  let controller: ModelToolController;

  function build(result: ApplyOperationResult): void {
    documentStore.setState({
      bodies: {
        body1: { id: "body1", name: "Body 1", visible: true },
        body3: { id: "body3", name: "Body 3", visible: true },
        body4: { id: "body4", name: "Body 4", visible: true },
      },
      features: [
        { id: "feat-bool", kind: "boolean", opType: "Boolean", label: "Cut", valueText: "", status: "ok" },
      ],
    });
    controller = new ModelToolController({
      engine: makeEngineMock() as unknown as ViewportEngine,
      client: {
        onPreviewResult: vi.fn(() => () => {}),
        applyOperation: vi.fn(() => Promise.resolve(result)),
        applyEditCommand: vi.fn(() => Promise.resolve(result)),
        getOperationParams: vi.fn(() => Promise.resolve({ ...STORED_CUT })),
      } as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
    });
  }

  async function commit(): Promise<void> {
    await controller.editBooleanFeature("feat-bool");
    await flush();
    const chip = toolChipStore.getState();
    (chip.onConfirm)?.();
    await flush();
    await flush();
  }

  beforeEach(() => {
    resetStores();
    __setExactPreviewTimeoutForTests(0);
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  it("a split Cut selects every child output, not the stored target", async () => {
    build({
      revision: 9,
      features: [],
      changedBodies: [
        { bodyId: "body3", meshKey: "body3#0" },
        { bodyId: "body4", meshKey: "body4#0" },
      ],
      removedBodies: [],
      terminal: "published",
    });
    selectionStore.setState({ selected: [{ kind: "body", id: "body1" }] });

    await commit();

    expect(selectionStore.getState().selected.map((r) => r.id).sort()).toEqual(["body3", "body4"]);
  });

  it("a FAILED commit leaves the selection where the user put it", async () => {
    build({
      revision: 9,
      features: [],
      changedBodies: [{ bodyId: "body3", meshKey: "body3#0" }],
      removedBodies: [],
      terminal: "failed",
      errorMessage: "kernel refused",
    });
    selectionStore.setState({ selected: [{ kind: "body", id: "body4" }] });

    await commit();

    expect(selectionStore.getState().selected.map((r) => r.id)).toEqual(["body4"]);
  });
});
