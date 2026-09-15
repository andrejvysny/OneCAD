/*
 * Extrude cluster controls (jsdom — 2026-09-14 UX review T2 + T3).
 *
 * T2: the ⇔ toggle carried `aria-label="Symmetric"` and a NATIVE `title`, and
 * neither reached a sighted user — no visible label, and the native tooltip does
 * not render in the app's WebView. It now shows the word "Symmetric" and uses the
 * app `Tooltip` (450 ms dwell).
 *
 * T3: dragging the depth handle back through zero was the ONLY way to reverse an
 * extrude, and nothing said so. There is now a "Flip" button; the controller
 * consumes it ONCE into the signed depth, so a later drag through zero is not
 * negated a second time and the arrow and the number agree.
 *
 * The chip probe in `modelingInteraction.golden.test.tsx` greps for a SINGLE
 * `/cancel|✕/i` button, which is why the new control is named "Flip".
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DirectionFlipButton, SymmetricToggle } from "./ExtrudeChipControls";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "@/tools/modelTools/ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  FinishSketchResult,
  PreviewDraft,
  SketchPlane,
  SketchRegion,
  SketchSession,
} from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";

describe("SymmetricToggle (T2)", () => {
  it("shows the word Symmetric beside the glyph", () => {
    render(<SymmetricToggle pressed={false} onToggle={() => {}} />);
    const button = screen.getByTestId("chip-symmetric");
    expect(button).toHaveTextContent("Symmetric");
    expect(button).toHaveAttribute("aria-pressed", "false");
  });

  it("uses the app tooltip, not the native title the WebView never renders", async () => {
    const user = userEvent.setup();
    render(<SymmetricToggle pressed={false} onToggle={() => {}} />);
    const button = screen.getByTestId("chip-symmetric");
    expect(button).not.toHaveAttribute("title");
    await user.hover(button);
    expect(await screen.findByRole("tooltip", {}, { timeout: 3000 })).toHaveTextContent(
      "Symmetric (hold Alt while dragging)",
    );
  });

  it("toggles on click", () => {
    const onToggle = vi.fn();
    render(<SymmetricToggle pressed onToggle={onToggle} />);
    fireEvent.click(screen.getByTestId("chip-symmetric"));
    expect(onToggle).toHaveBeenCalledTimes(1);
  });
});

describe("DirectionFlipButton (T3)", () => {
  it("is labelled Flip — never cancel / ✕, which the chip probe owns", () => {
    render(<DirectionFlipButton onFlip={() => {}} />);
    const button = screen.getByTestId("chip-flip");
    expect(button).toHaveTextContent("Flip");
    expect(button).toHaveAttribute("aria-label", "Flip direction");
    expect(screen.queryByRole("button", { name: /cancel|✕/i })).not.toBeInTheDocument();
  });

  it("calls onFlip once per click", () => {
    const onFlip = vi.fn();
    render(<DirectionFlipButton onFlip={onFlip} />);
    fireEvent.click(screen.getByTestId("chip-flip"));
    expect(onFlip).toHaveBeenCalledTimes(1);
  });
});

// ── the controller half of T3: the flip is consumed ONCE ─────────────────────

const PLANE: SketchPlane = {
  kind: "XY",
  origin: [0, 0, 0],
  xAxis: [1, 0, 0],
  yAxis: [0, 1, 0],
  normal: [0, 0, 1],
};

const R0: SketchRegion = {
  regionId: "r0",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [0, 0, 20, 0, 20, 20, 0, 20], indices: [0, 1, 2, 0, 2, 3] },
};

const okResult = (): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [{ bodyId: "b1", meshKey: "b1#0" }],
  removedBodies: [],
});

function makeSession(): SketchSession {
  return { sketchId: "sk", plane: PLANE, entities: [], constraints: [], dof: 0, status: "FullyConstrained" };
}

function makeEngineMock() {
  return {
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    hideRegionPick: vi.fn(),
    setRegionSelected: vi.fn(),
    screenToPlaneOn: vi.fn((_p: SketchPlane, x: number, y: number) => ({ x, y })),
    setOrbitSuppressed: vi.fn(),
    setExtrudeHandle: vi.fn(),
    moveChip: vi.fn(),
    probeMaterial: vi.fn(() => null),
    planePixelWorld: vi.fn(() => 1),
    showExtrudePreview: vi.fn(),
    showExtrudePreviews: vi.fn(),
    setExtrudeDepth: vi.fn(),
    setPreviewTint: vi.fn(),
    setExtrudeHandleHover: vi.fn(),
    hitExtrudeHandle: vi.fn(() => false),
    screenRay: vi.fn(() => ({ origin: [0, 0, 100] as const, dir: [0, 0, -1] as const })),
    hideExtrudePreview: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => false),
    setPreviewBody: vi.fn(),
    setPreviewReplacedBodyIds: vi.fn(),
    clearPreviewBody: vi.fn(),
    showRevolveAxisCandidates: vi.fn(),
    setRevolveAxisHover: vi.fn(),
    showRevolvePreview: vi.fn(),
    setRevolveAngle: vi.fn(),
    hideRevolvePreview: vi.fn(),
    hideGhostPreview: vi.fn(),
    hideValueHandle: vi.fn(),
    showValueHandle: vi.fn(),
    showGhostPreviewMulti: vi.fn(),
    probePick: vi.fn(() => null),
  };
}

function makeClientMock() {
  let seq = 0;
  return {
    onPreviewResult: vi.fn(() => () => {}),
    onDocumentChanged: vi.fn(() => () => {}),
    getCurrentMeshPublication: vi.fn(() => null),
    finishSketch: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0] })),
    getSketchRegions: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0] })),
    getSketch: vi.fn(() => Promise.resolve(makeSession())),
    beginPreview: vi.fn((_d: PreviewDraft) =>
      Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` }),
    ),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(okResult())),
    applyOperation: vi.fn(() => Promise.resolve(okResult())),
    applyEditCommand: vi.fn(() => Promise.resolve(okResult())),
    getOperationParams: vi.fn(() => Promise.resolve({})),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("Flip negates the armed extrude depth exactly once (T3)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;

  async function armExtrude(): Promise<void> {
    engineMock = makeEngineMock();
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: makeClientMock() as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
    });
    selectionStore.getState().set([
      { kind: "sketchRegion", id: "r0-ref", sketchId: "sk", regionId: "r0" },
    ]);
    toolStore.getState().setTool("extrude");
    await flush();
    await flush();
  }

  beforeEach(() => {
    resetStores();
    __setExactPreviewTimeoutForTests(0);
    container = document.createElement("div");
    document.body.appendChild(container);
    documentStore.getState().addSketch({
      id: "sk",
      name: "Sketch",
      visible: true,
      dof: 0,
      status: "ok",
      geometryToken: "sk:v1",
    });
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  it("publishes the negated depth to the chip and the preview", async () => {
    await armExtrude();
    const armed = toolChipStore.getState().value;
    expect(armed).toBeGreaterThan(0);

    toolChipStore.getState().onFlip?.();
    expect(toolChipStore.getState().value).toBe(-armed);
    expect(engineMock.setExtrudeDepth).toHaveBeenLastCalledWith(-armed, false);
  });

  it("flips back on a second press (an involution, not a latched flag)", async () => {
    await armExtrude();
    const armed = toolChipStore.getState().value;
    toolChipStore.getState().onFlip?.();
    toolChipStore.getState().onFlip?.();
    expect(toolChipStore.getState().value).toBe(armed);
  });

  it("does not negate a later drag a second time", async () => {
    await armExtrude();
    toolChipStore.getState().onFlip?.();
    expect(toolChipStore.getState().value).toBeLessThan(0);

    // The parallel-ray fallback in `axisDepthFromRay` projects this fake ray onto
    // the normal axis at +100; `forceExtrudeGrab` zeroes the grab basis, so the
    // drag reports that projection verbatim.
    controller.forceExtrudeGrab();
    container.dispatchEvent(
      new MouseEvent("pointermove", { clientX: 5, clientY: 5, buttons: 1, bubbles: true }),
    );
    expect(toolChipStore.getState().value).toBe(100);
  });
});
