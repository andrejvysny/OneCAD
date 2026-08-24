/*
 * MirrorBody `fuseWithOriginal` authoring (WP6 item 2, jsdom).
 *
 * The flag existed end to end on the wire (`MirrorBodyParams.fuseWithOriginal`,
 * `tauriCommandMap` §MirrorBody) but had no authoring surface: a fresh mirror
 * always committed `false` and only a re-edit could carry a stored `true`
 * through. These specs pin the new chip toggle against the COMMITTED params,
 * which is the only thing the worker ever sees — the chip's own aria-pressed is
 * covered in `ModelToolChips.test.tsx`.
 *
 * The product default is unchanged and is asserted here: a NEW mirror commits
 * non-fused unless the user says otherwise.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { ApplyOperationResult } from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";

const okResult = (): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#0" }],
  removedBodies: [],
});

// Mirrors ModelToolController.patternReedit.test.ts: `onToolChange` tears down
// every other tool before arming, so the fake needs each method those paths hit.
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

describe("ModelToolController — MirrorBody fuse authoring", () => {
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

  /** Arm a FRESH mirror on the seeded body through the real tool-change path. */
  async function armFreshMirror(): Promise<void> {
    selectionStore.setState({ selected: [{ kind: "body", id: "body1" }] });
    toolStore.getState().setTool("mirror");
    await flush();
    expect(toolChipStore.getState().kind).toBe("mirror");
  }

  /** Assert the ONE committed MirrorBody carried exactly these params. */
  function expectCommitted(params: Record<string, unknown>): void {
    expect(clientMock.applyOperation).toHaveBeenCalledTimes(1);
    expect(clientMock.applyOperation).toHaveBeenCalledWith(
      expect.objectContaining({
        opType: "MirrorBody",
        params: expect.objectContaining(params),
      }),
    );
  }

  beforeEach(() => {
    resetStores();
    container = document.createElement("div");
    document.body.appendChild(container);
    documentStore.setState({
      bodies: { body1: { id: "body1", name: "Body 1", visible: true } },
    });
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
  });

  // ── fresh authoring ─────────────────────────────────────────────────────────

  it("a fresh mirror arms NON-fused and commits fuseWithOriginal false", async () => {
    build();
    await armFreshMirror();

    expect(toolChipStore.getState().fuse).toBe(false);

    toolChipStore.getState().onConfirm?.();
    await flush();

    expectCommitted({ fuseWithOriginal: false });
  });

  it("toggling the fuse chip on a fresh mirror commits fuseWithOriginal true", async () => {
    build();
    await armFreshMirror();

    toolChipStore.getState().onFuse?.(true);
    expect(toolChipStore.getState().fuse).toBe(true);
    // The lifecycle line restates what the flag means, before the commit.
    expect(toolChipStore.getState().resultSummary).toBe("Mirror · fused into the source");

    toolChipStore.getState().onConfirm?.();
    await flush();

    expectCommitted({ fuseWithOriginal: true });
  });

  it("toggling fuse back off before ✓ commits false", async () => {
    build();
    await armFreshMirror();

    toolChipStore.getState().onFuse?.(true);
    toolChipStore.getState().onFuse?.(false);
    expect(toolChipStore.getState().fuse).toBe(false);
    expect(toolChipStore.getState().resultSummary).toBe("Mirror · 1 new body · source retained");

    toolChipStore.getState().onConfirm?.();
    await flush();

    expectCommitted({ fuseWithOriginal: false });
  });

  // ── re-edit ─────────────────────────────────────────────────────────────────

  const mirrorFeature = () => ({
    features: [
      {
        id: "feat-mi",
        kind: "boolean" as const,
        opType: "MirrorBody",
        label: "Mirror",
        valueText: "YZ",
        status: "ok" as const,
      },
    ],
  });

  it("a re-edit seeds the chip from the STORED fuseWithOriginal", async () => {
    build();
    clientMock.getOperationParams.mockResolvedValue({
      sourceBodyId: "body1",
      planePoint: [5, 10, 0],
      planeNormal: [1, 0, 0],
      fuseWithOriginal: true,
    });
    documentStore.setState(mirrorFeature());

    await controller.editMirrorFeature("feat-mi");
    await flush();

    expect(toolChipStore.getState().fuse).toBe(true);
  });

  it("a re-edit can turn a fused mirror OFF, and the commit carries false", async () => {
    build();
    clientMock.getOperationParams.mockResolvedValue({
      sourceBodyId: "body1",
      planePoint: [5, 10, 0],
      planeNormal: [1, 0, 0],
      fuseWithOriginal: true,
    });
    documentStore.setState(mirrorFeature());

    await controller.editMirrorFeature("feat-mi");
    await flush();

    toolChipStore.getState().onFuse?.(false);
    toolChipStore.getState().onConfirm?.();
    await flush();

    // The rest of the stored shape is untouched by the fuse edit.
    expectCommitted({ fuseWithOriginal: false, planePoint: [5, 10, 0] });
  });

  it("a re-edit can turn a non-fused mirror ON, and the commit carries true", async () => {
    build();
    clientMock.getOperationParams.mockResolvedValue({
      sourceBodyId: "body1",
      planePoint: [0, 0, 0],
      planeNormal: [0, 0, 1],
      fuseWithOriginal: false,
    });
    documentStore.setState(mirrorFeature());

    await controller.editMirrorFeature("feat-mi");
    await flush();
    expect(toolChipStore.getState().fuse).toBe(false);

    toolChipStore.getState().onFuse?.(true);
    toolChipStore.getState().onConfirm?.();
    await flush();

    expectCommitted({ fuseWithOriginal: true });
  });
});
