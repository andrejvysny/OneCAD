/*
 * EXTRUDE GESTURE HARDENING (jsdom) — the two invariants a MOVING arrow depends on.
 *
 * 1. A press on the CHIP is never a depth grab. The chip layer is a sibling of the
 *    canvas overlay inside the same container this controller listens on, so chip
 *    presses bubble into `onPointerDown`. The arrow draws with `depthTest: false`
 *    against a fat pick envelope, so a chip pixel CAN sit over it — and then ✓ or
 *    the value field would start a drag instead of doing their job.
 *
 * 2. Re-grabbing the arrow does not move the depth. The drag reports
 *    `startDepth + (raw - grabDepth)`, so the press's own offset along the axis
 *    cancels. Under the previous ABSOLUTE mapping this only held because the arrow
 *    never moved; once it travels with the prism, an absolute report would add an
 *    arrow-length to the depth on every re-grab (`commitExtrudeAtHandle` re-grabs
 *    on each `toPass` retry, so it would drift in the gate lane immediately).
 *
 * DEPTH CONTROL is the `hostBoolean` fake: `screenRay` runs perpendicular to the
 * extrude axis, so `axisDepthFromRay` collapses to the ray origin's height above
 * the sketch plane and a clientY maps to an EXACT signed depth.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  FinishSketchResult,
  PreviewDraft,
  PreviewResult,
  SketchPlane,
  SketchRegion,
  SketchSession,
} from "@/ipc/types";
import { toolStore } from "@/stores/toolStore";
import { selectionStore } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";

const PLANE: SketchPlane = {
  kind: "custom",
  origin: [0, 0, 25],
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

const ok = (): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#1" }],
  removedBodies: [],
});

/** clientY that projects to `depth` under the perpendicular-ray fake. */
const yFor = (depth: number): number => 100 - depth;

function makeEngineMock() {
  return {
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    setRegionSelected: vi.fn(),
    hideRegionPick: vi.fn(),
    screenToPlaneOn: vi.fn((_p: SketchPlane, x: number, y: number) => ({ x, y })),
    setOrbitSuppressed: vi.fn(),
    setExtrudeHandle: vi.fn(),
    moveChip: vi.fn(),
    probeMaterial: vi.fn(
      (_o: number[], _dir: number[]): { bodyId: string; gap: number; inside: boolean } | null => null,
    ),
    planePixelWorld: vi.fn(() => 1),
    showExtrudePreview: vi.fn(),
    showExtrudePreviews: vi.fn(),
    setExtrudeDepth: vi.fn(),
    setPreviewTint: vi.fn(),
    setExtrudeHandleHover: vi.fn(),
    hitExtrudeHandle: vi.fn(() => true),
    screenRay: vi.fn((_x: number, y: number) => ({
      origin: [0, 0, PLANE.origin[2] + (100 - y)],
      dir: [1, 0, 0],
    })),
    hideExtrudePreview: vi.fn(),
    isExtrudePreviewVisible: vi.fn(() => true),
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
    onPreviewResult: vi.fn((_cb: (r: PreviewResult) => void) => () => {}),
    finishSketch: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0] })),
    getSketchRegions: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0] })),
    getSketch: vi.fn(() =>
      Promise.resolve<SketchSession>({
        sketchId: "sk",
        plane: PLANE,
        entities: [],
        constraints: [],
        dof: 0,
        status: "FullyConstrained",
      }),
    ),
    beginPreview: vi.fn((_d: PreviewDraft) =>
      Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` }),
    ),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(ok())),
    applyOperation: vi.fn(() => Promise.resolve(ok())),
    applyEditCommand: vi.fn(() => Promise.resolve(ok())),
    undo: vi.fn(() => Promise.resolve(ok())),
    getOperationParams: vi.fn(() => Promise.resolve({})),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));

describe("ModelToolController extrude gesture (chip exclusion + grab-relative depth)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let chip: HTMLDivElement;
  let controller: ModelToolController;

  function debug(): Record<string, unknown> {
    return (window as unknown as { __extrudePreview: Record<string, unknown> }).__extrudePreview;
  }

  beforeEach(async () => {
    resetStores();
    __setExactPreviewTimeoutForTests(0);
    container = document.createElement("div");
    document.body.appendChild(container);
    // The chip layer, as the engine builds it: a child of the same container.
    chip = document.createElement("div");
    chip.dataset.testid = "model-tool-chip";
    container.appendChild(chip);

    documentStore.getState().addSketch({
      id: "sk",
      name: "Sketch",
      visible: true,
      dof: 0,
      status: "ok",
      geometryToken: "sk:v1",
    });
    selectionStore
      .getState()
      .set([{ kind: "sketchRegion", id: "r0-ref", sketchId: "sk", regionId: "r0" }]);

    engineMock = makeEngineMock();
    clientMock = makeClientMock();
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
      debug: true,
    });
    toolStore.getState().setTool("extrude");
    await flush();
    expect(debug().phase).toBe("armed");
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  /** Press at `clientY`, optionally from inside the chip. */
  function press(clientY: number, target: HTMLElement = container): void {
    target.dispatchEvent(
      new MouseEvent("pointerdown", {
        clientX: 5,
        clientY,
        button: 0,
        buttons: 1,
        bubbles: true,
      }),
    );
  }

  function move(clientY: number): void {
    container.dispatchEvent(
      new MouseEvent("pointermove", { clientX: 5, clientY, buttons: 1, bubbles: true }),
    );
  }

  function release(clientY: number): void {
    container.dispatchEvent(
      new MouseEvent("pointerup", { clientX: 5, clientY, button: 0, buttons: 0, bubbles: true }),
    );
  }

  // ── 1. the chip is never a depth grab ──────────────────────────────────────

  it("a press on the chip does NOT start a depth drag, even over the handle", () => {
    // The handle reports a hit at every pixel — the exclusion is the only thing
    // that can keep this press from being swallowed.
    press(yFor(40), chip);
    expect(debug().phase).toBe("armed");
    expect(toolStore.getState().phase).not.toBe("dragging");
    expect(engineMock.setExtrudeHandleHover).not.toHaveBeenCalledWith(true);
  });

  it("a press on a chip BUTTON is excluded too (the ✓ stays clickable)", () => {
    const confirm = document.createElement("button");
    chip.appendChild(confirm);
    press(yFor(40), confirm);
    expect(debug().phase).toBe("armed");
  });

  it("a press on the viewport still starts the drag", () => {
    press(yFor(40));
    expect(debug().phase).toBe("dragging");
    expect(engineMock.setExtrudeHandleHover).toHaveBeenCalledWith(true);
  });

  // ── 2. re-grabbing does not move the depth ─────────────────────────────────

  it("depth is grab-RELATIVE: a press away from the head does not jump the value", () => {
    const armed = debug().depth as number;
    // Press 30 units up-axis from where the depth actually is — i.e. mid-shaft.
    press(yFor(armed + 30));
    move(yFor(armed + 30));
    expect(debug().depth).toBeCloseTo(armed, 6);

    // …and travel from there is still 1:1.
    move(yFor(armed + 42));
    expect(debug().depth).toBeCloseTo(armed + 12, 6);
    release(yFor(armed + 42));
  });

  it("N grab/release cycles at the same pixel leave the depth where it was", () => {
    const start = debug().depth as number;
    for (let i = 0; i < 5; i++) {
      const at = yFor((debug().depth as number) + 30); // always grab mid-shaft
      press(at);
      move(at);
      release(at);
      expect(debug().phase).toBe("armed");
    }
    expect(debug().depth).toBeCloseTo(start, 6);
  });

  it("forceExtrudeGrab keeps the ABSOLUTE mapping the gate helpers drive", () => {
    controller.forceExtrudeGrab();
    move(yFor(-12));
    expect(debug().depth).toBe(-12);
  });

  // ── 3. the arrow travels with the operation ────────────────────────────────

  /** `[origin, dir, mode, destructive]` of the newest arrow re-anchor. */
  function handle(): [number[], number[], string, boolean] {
    const calls = engineMock.setExtrudeHandle.mock.calls;
    return calls[calls.length - 1] as unknown as [number[], number[], string, boolean];
  }

  it("a fresh arm anchors the arrow at the armed depth, drawn TWO-WAY", () => {
    const [origin, , mode] = handle();
    // The sketch plane sits at z=25 and its normal is +Z, so the head's z is the
    // plane plus the depth.
    expect(origin[2]).toBeCloseTo(25 + (debug().depth as number), 6);
    expect(mode).toBe("twoWay");
  });

  it("the arrow follows the depth and flips with its sign, single-headed once grabbed", () => {
    controller.forceExtrudeGrab();

    move(yFor(40));
    let [origin, dir, mode] = handle();
    expect(origin[2]).toBeCloseTo(65, 6); // 25 + 40
    expect(dir[2]).toBeCloseTo(1, 6);
    expect(mode).toBe("forward");

    move(yFor(-40));
    [origin, dir, mode] = handle();
    expect(origin[2]).toBeCloseTo(-15, 6); // 25 - 40
    expect(dir[2]).toBeCloseTo(-1, 6); // points where the material is going
    expect(mode).toBe("forward");
  });

  it("depth 0 hands the handle a zero direction (it holds its own orientation)", () => {
    controller.forceExtrudeGrab();
    move(yFor(0));
    const [origin, dir] = handle();
    expect(origin[2]).toBeCloseTo(25, 6);
    expect(dir).toEqual([0, 0, 0]);
  });

  it("a typed depth moves the arrow exactly like a drag", () => {
    toolChipStore.getState().onValue?.(33);
    const [origin] = handle();
    expect(origin[2]).toBeCloseTo(58, 6); // 25 + 33
  });

  it("symmetric keeps ONE head, at the +|depth| face", () => {
    controller.forceExtrudeGrab();
    move(yFor(-18));
    toolChipStore.getState().onSymmetric?.(true);
    const [origin, dir, mode] = handle();
    expect(origin[2]).toBeCloseTo(43, 6); // 25 + |−18|
    expect(dir[2]).toBeCloseTo(1, 6);
    expect(mode).toBe("forward");
  });
});

/*
 * DIRECTION DECIDES THE OPERATION, for a sketch that is NOT face-hosted.
 *
 * The lane used to require `SketchDto.hostFace`; a sketch on a datum plane
 * slicing through a body opened on NewBody and offered a text hint. Here the
 * engine's ray probe is the seam, so these cases are exercised without a
 * tessellated scene.
 */
describe("ModelToolController auto-boolean from the material probe", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;

  function debug(): Record<string, unknown> {
    return (window as unknown as { __extrudePreview: Record<string, unknown> }).__extrudePreview;
  }

  /** Arm with a probe that reports `hit` for rays pointing along −Z and `null` up. */
  async function armWithMaterialBelow(hit: { bodyId: string; gap: number; inside: boolean }) {
    engineMock.probeMaterial = vi.fn((_o: number[], dir: number[]) => (dir[2] < 0 ? hit : null));
    toolStore.getState().setTool("extrude");
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
    documentStore.setState({ bodies: { b1: { id: "b1", name: "Body 1", visible: true } } });
    selectionStore
      .getState()
      .set([{ kind: "sketchRegion", id: "r0-ref", sketchId: "sk", regionId: "r0" }]);
    engineMock = makeEngineMock();
    clientMock = makeClientMock();
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
      debug: true,
    });
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  function move(clientY: number): void {
    container.dispatchEvent(
      new MouseEvent("pointermove", { clientX: 5, clientY, buttons: 1, bubbles: true }),
    );
  }

  it("a sketch lying ON a body cuts downward and joins upward — no hostFace needed", async () => {
    await armWithMaterialBelow({ bodyId: "b1", gap: 0, inside: true });
    expect(debug().booleanAuto).toBe(true);

    controller.forceExtrudeGrab();
    move(yFor(-12)); // into the solid
    expect(debug().booleanMode).toBe("Cut");
    expect(debug().booleanTargetId).toBe("b1");
    expect(engineMock.setPreviewTint).toHaveBeenLastCalledWith("cut");

    move(yFor(12)); // away from it
    expect(debug().booleanMode).toBe("Add");
    expect(debug().booleanTargetId).toBe("b1");
    expect(engineMock.setPreviewTint).toHaveBeenLastCalledWith("normal");
  });

  it("a body 10 BELOW is only joined once the prism reaches it", async () => {
    await armWithMaterialBelow({ bodyId: "b1", gap: 10, inside: false });
    controller.forceExtrudeGrab();

    move(yFor(-4)); // still in flight
    expect(debug().booleanMode).toBe("NewBody");
    expect(debug().booleanTargetId).toBeNull();

    move(yFor(-25)); // through it
    expect(debug().booleanMode).toBe("Add");
    expect(debug().booleanTargetId).toBe("b1");
  });

  it("a HIDDEN body is never bound — the probe hit is dropped by visibility", async () => {
    documentStore.getState().setVisibility("b1", false);
    await armWithMaterialBelow({ bodyId: "b1", gap: 0, inside: true });
    expect(debug().booleanAuto).toBe(false);

    controller.forceExtrudeGrab();
    move(yFor(-12));
    expect(debug().booleanMode).toBe("NewBody");
  });

  it("nothing either way stays NewBody, exactly as before", async () => {
    engineMock.probeMaterial = vi.fn(() => null);
    toolStore.getState().setTool("extrude");
    await flush();
    expect(debug().booleanAuto).toBe(false);
    controller.forceExtrudeGrab();
    move(yFor(-30));
    expect(debug().booleanMode).toBe("NewBody");
  });

  it("does not STROBE across the zero crossing", async () => {
    await armWithMaterialBelow({ bodyId: "b1", gap: 0, inside: true });
    controller.forceExtrudeGrab();
    move(yFor(-5));
    const tintsBefore = engineMock.setPreviewTint.mock.calls.length;

    // 60 frames hovering on the crossing: depth 0 HOLDS, so this must not re-resolve.
    for (let i = 0; i < 60; i++) move(yFor(i % 2 === 0 ? 0 : -0.0001));

    expect(debug().booleanMode).toBe("Cut");
    expect(engineMock.setPreviewTint.mock.calls.length).toBe(tintsBefore);
  });

  it("a manual override still ends the lane for the rest of the session", async () => {
    await armWithMaterialBelow({ bodyId: "b1", gap: 0, inside: true });
    toolChipStore.getState().onBooleanMode?.("NewBody");
    await flush();

    controller.forceExtrudeGrab();
    move(yFor(-20)); // would have resolved to Cut on the auto lane
    expect(debug().booleanMode).toBe("NewBody");
  });
});
