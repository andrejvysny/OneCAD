/*
 * Placement gizmo wiring (WP-B W2) — the pointer → FSM path.
 *
 * The math lives in `tools/preview/transformDrag` and the hit geometry in
 * `viewport/engine/TransformGizmo`; what is only checkable HERE is the contract
 * between them, and it has four parts that all fail silently if broken:
 *
 *   - the HANDLE decides mode + axis, and does it THROUGH the FSM, so the chip
 *     segments and the gizmo can never disagree (one writer);
 *   - a drag reports DIFFERENCES from the grab, so grabbing the middle of an
 *     arrow does not teleport the body to the cursor;
 *   - release keeps the placement ARMED (house pattern) — nothing reaches the
 *     timeline without Enter or the chip ✓;
 *   - a plane quad writes TWO components at once, which no chip gesture can do.
 *
 * The engine is mocked with a ray field that makes the projections exact:
 * `screenRay(x, y)` fires straight down −Z from world (x, −y), so a pointer move
 * of (dx, dy) is a world move of (dx, −dy) on the Z plane — screen-y grows
 * downward. Every expected number below is that identity, not a fitted constant.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type { ApplyOperationResult, OperationOp, TransformBodyParams } from "@/ipc/types";
import type { GizmoHit } from "@/viewport/engine/TransformGizmo";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";
import { buildBodyObjects, swap as swapMesh, __resetRegistryForTests } from "@/viewport/mesh/meshRegistry";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { makeBoxMesh } from "@/ipc/mockMeshes";

const BODY = "body1";

const okResult = (): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [{ bodyId: BODY, meshKey: `${BODY}#0` }],
  removedBodies: [],
});

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

/**
 * The engine seam. `hitTransformGizmo` is the switch the test throws to say
 * "the press landed on THIS handle"; `screenRay` is the camera.
 */
function makeEngineMock() {
  return {
    grabbed: null as GizmoHit | null,
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
    hitExtrudeHandle: vi.fn(() => false),
    showTransformGizmo: vi.fn(),
    hideTransformGizmo: vi.fn(),
    setTransformGizmoHover: vi.fn(),
    setTransformGizmoActive: vi.fn(),
    hitTransformGizmo(this: { grabbed: GizmoHit | null }): GizmoHit | null {
      return this.grabbed;
    },
    // Straight down −Z from world (x, −y): a screen delta IS a world delta.
    screenRay: (x: number, y: number) => ({ origin: [x, -y, 1000], dir: [0, 0, -1] }),
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

describe("ModelToolController placement gizmo (drag)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;

  beforeEach(() => {
    resetStores();
    __resetRegistryForTests();
    // A real registry entry so the ghost has geometry AND the pivot is real:
    // the demo box is centred on the origin, so the gizmo starts at (0,0,0).
    swapMesh(BODY, buildBodyObjects(parseMeshPayload(makeBoxMesh()), BODY, 1));
    container = document.createElement("div");
    document.body.appendChild(container);
    engineMock = makeEngineMock();
    clientMock = makeClientMock();
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
    });
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    resetStores();
    __resetRegistryForTests();
  });

  async function armTransform(): Promise<void> {
    selectionStore.getState().set([{ kind: "body", id: BODY }]);
    toolStore.getState().setTool("transform");
    await flush();
  }

  function pointer(type: string, x: number, y: number, mods: MouseEventInit = {}): void {
    container.dispatchEvent(
      new MouseEvent(type, { clientX: x, clientY: y, button: 0, bubbles: true, ...mods }),
    );
  }

  /** Grab `hit` at (0,0), drag to (x,y), release. `mods` ride BOTH down and move. */
  function drag(hit: GizmoHit, x: number, y: number, mods: MouseEventInit = {}): void {
    engineMock.grabbed = hit;
    pointer("pointerdown", 0, 0, mods);
    pointer("pointermove", x, y, mods);
    pointer("pointerup", x, y, mods);
    engineMock.grabbed = null;
  }

  const chip = () => toolChipStore.getState();
  const params = (): TransformBodyParams => {
    const calls = clientMock.applyOperation.mock.calls as unknown as OperationOp[][];
    return calls[0][0].params as TransformBodyParams;
  };

  it("shows the gizmo at the frozen pivot the moment the tool arms", async () => {
    await armTransform();
    expect(engineMock.showTransformGizmo).toHaveBeenCalledWith([0, 0, 0]);
  });

  it("an arrow drag writes ONE component and leaves the others alone", async () => {
    await armTransform();
    drag({ kind: "axis", axis: "X" }, 30, 12);

    expect(chip().transformMode).toBe("move");
    expect(chip().axis).toBe("X");
    expect(chip().value).toBe(30); // the screen dx, verbatim
    // The perpendicular screen motion must NOT leak into Y.
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    expect(params().translate).toEqual([30, 0, 0]);
  });

  it("measures from the GRAB, so grabbing mid-arrow does not teleport the body", async () => {
    await armTransform();
    // Press 200px away from the gizmo origin, then move only 25px further.
    engineMock.grabbed = { kind: "axis", axis: "X" };
    pointer("pointerdown", 200, 0);
    pointer("pointermove", 225, 0);
    pointer("pointerup", 225, 0);
    expect(chip().value).toBe(25); // the DELTA, not the absolute 225
  });

  it("a plane quad drag writes TWO components in one gesture", async () => {
    await armTransform();
    // The Z quad translates in XY. Screen y grows downward ⇒ world dy = −12.
    drag({ kind: "plane", axis: "Z" }, 30, 12);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    expect(params().translate).toEqual([30, -12, 0]);
  });

  it("a plane grab keeps the chip on an IN-PLANE axis (never the quad's normal)", async () => {
    await armTransform();
    // Standing choice is X, which lies in the Z quad's plane — so it survives…
    drag({ kind: "plane", axis: "Z" }, 10, 0);
    expect(chip().axis).toBe("X");
    // …but grabbing the X quad (which cannot move X at all) must move off it.
    drag({ kind: "plane", axis: "X" }, 10, 0);
    expect(chip().axis).toBe("Y");
  });

  it("a ring drag re-types the placement to Rotate about THAT axis", async () => {
    await armTransform();
    // Grab at world (100,0) and release at world (0,100): a +90° sweep about Z.
    engineMock.grabbed = { kind: "ring", axis: "Z" };
    pointer("pointerdown", 100, 0);
    pointer("pointermove", 0, -100);
    pointer("pointerup", 0, -100);

    expect(chip().transformMode).toBe("rotate");
    expect(chip().axis).toBe("Z");
    expect(chip().value).toBe(90);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    expect(params().rotate.angleDeg).toBe(90);
    expect(params().rotate.axis).toEqual([0, 0, 1]);
    expect(params().translate).toEqual([0, 0, 0]); // a rotation is not a move
  });

  it("snaps a dragged distance to 1mm", async () => {
    await armTransform();
    drag({ kind: "axis", axis: "X" }, 30.44, 0);
    expect(chip().value).toBe(30);
  });

  it("Shift selects the 0.1mm fine tier", async () => {
    await armTransform();
    drag({ kind: "axis", axis: "X" }, 30.44, 0, { shiftKey: true });
    expect(chip().value).toBe(30.4);
  });

  /**
   * The one place a drag deliberately differs from the chip. TYPING replaces the
   * addressed component (W1: a placement is one cumulative record, and a second
   * typed value is the new absolute). DRAGGING continues from where the body now
   * is, because the handle the user grabbed is at the moved position — a drag
   * that reset to zero first would jump the body backwards on the first frame.
   */
  it("a second drag CONTINUES from the placed position (unlike a typed value)", async () => {
    await armTransform();
    drag({ kind: "axis", axis: "X" }, 30, 0);
    drag({ kind: "axis", axis: "X" }, 20, 0);
    expect(chip().value).toBe(50);
  });

  it("release KEEPS the placement armed — nothing reaches the timeline", async () => {
    await armTransform();
    drag({ kind: "axis", axis: "X" }, 30, 0);

    expect(toolStore.getState().phase).toBe("armed");
    expect(chip().kind).toBe("transform");
    expect(clientMock.applyOperation).not.toHaveBeenCalled();
  });

  it("suppresses orbit for the gesture by owning the press, and only then", async () => {
    await armTransform();
    // A press that MISSES every handle is not a drag: the phase stays armed and
    // the gizmo never goes active, leaving the gesture to the camera.
    engineMock.grabbed = null;
    pointer("pointerdown", 5, 5);
    pointer("pointermove", 60, 60);
    expect(toolStore.getState().phase).toBe("armed");
    expect(chip().value).toBe(0);
    expect(engineMock.setTransformGizmoActive).not.toHaveBeenCalledWith(
      expect.objectContaining({ kind: "axis" }),
    );
  });

  it("Alt at grab turns on copy, and the chip shows it", async () => {
    await armTransform();
    expect(chip().copy).toBe(false);
    drag({ kind: "axis", axis: "X" }, 20, 0, { altKey: true });

    expect(chip().copy).toBe(true);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    expect(params().copy).toBe(true);
  });

  it("keeps the SOURCES visible while a copy is ghosted (that IS the difference)", async () => {
    await armTransform();
    drag({ kind: "axis", axis: "X" }, 20, 0, { altKey: true });
    // `setPreviewReplacedBodyIds` is what hides a moved body's original; a copy
    // must never call it with the targets.
    for (const call of engineMock.setPreviewReplacedBodyIds.mock.calls) {
      expect(call[0]).not.toContain(BODY);
    }
  });

  it("moves the gizmo WITH the body, so the next grab starts where the ghost is", async () => {
    await armTransform();
    drag({ kind: "axis", axis: "X" }, 30, 0);
    const poses = engineMock.showTransformGizmo.mock.calls as unknown as [number[]][];
    expect(poses[poses.length - 1][0]).toEqual([30, 0, 0]);
  });

  it("a degenerate view makes the drag INERT rather than throwing the body away", async () => {
    await armTransform();
    // Camera looking straight down the X arm: the projection is refused, so the
    // placement must hold its previous value — not jump, and never NaN.
    engineMock.screenRay = (x: number) => ({ origin: [x, 0, 0], dir: [1, 0, 0] });
    drag({ kind: "axis", axis: "X" }, 40, 0);
    expect(chip().value).toBe(0);
  });

  it("drops the gizmo on commit and on a tool switch", async () => {
    await armTransform();
    drag({ kind: "axis", axis: "X" }, 30, 0);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    expect(engineMock.hideTransformGizmo).toHaveBeenCalled();

    engineMock.hideTransformGizmo.mockClear();
    await armTransform();
    toolStore.getState().setTool("select");
    expect(engineMock.hideTransformGizmo).toHaveBeenCalled();
  });
});
