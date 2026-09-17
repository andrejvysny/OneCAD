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
import { settingsStore } from "@/stores/settingsStore";

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
    onDocumentChanged: vi.fn(() => () => {}),
    getCurrentMeshPublication: vi.fn(() => null),
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

  function release(clientY: number, target: HTMLElement = container): void {
    target.dispatchEvent(
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

  it("a canvas drag released over chip chrome returns to armed without a behind-chip action", () => {
    press(yFor(40));
    move(yFor(55));
    expect(debug().phase).toBe("dragging");

    release(yFor(5), chip); // deliberately a different coordinate over UI

    expect(debug().phase).toBe("armed");
    expect(toolStore.getState().phase).toBe("armed");
    expect(engineMock.setExtrudeHandleHover).toHaveBeenLastCalledWith(false);
    expect(clientMock.endPreview).not.toHaveBeenCalled();
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

  // TODO.md SESSION 37 H3: the forced grab has no press to jitter around, so it is
  // past the drag threshold from its first frame — even a sample within DRAG_PX of
  // the (never-set) press coordinates drives the depth.
  it("forceExtrudeGrab counts as already past the drag threshold", () => {
    controller.forceExtrudeGrab();
    container.dispatchEvent(new MouseEvent("pointermove", { clientX: 1, clientY: 3, buttons: 1, bubbles: true }));
    expect(debug().depth).toBe(97);
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

  it("the arrow follows signed depth on one stable axis and stays two-way after grabbing", () => {
    controller.forceExtrudeGrab();

    move(yFor(40));
    let [origin, dir, mode] = handle();
    expect(origin[2]).toBeCloseTo(65, 6); // 25 + 40
    expect(dir[2]).toBeCloseTo(1, 6);
    expect(mode).toBe("twoWay");

    move(yFor(-40));
    [origin, dir, mode] = handle();
    expect(origin[2]).toBeCloseTo(-15, 6); // 25 - 40
    expect(dir[2]).toBeCloseTo(1, 6); // sign moves material; axis stays +normal
    expect(mode).toBe("twoWay");
  });

  it("depth 0 keeps the prepared +normal axis", () => {
    controller.forceExtrudeGrab();
    move(yFor(0));
    const [origin, dir] = handle();
    expect(origin[2]).toBeCloseTo(25, 6);
    expect(dir).toEqual([0, 0, 1]);
  });

  it("a typed depth moves the arrow exactly like a drag", () => {
    toolChipStore.getState().onValue?.(33);
    const [origin] = handle();
    expect(origin[2]).toBeCloseTo(58, 6); // 25 + 33
  });

  it("symmetric keeps two heads at the +|depth|/2 face the worker builds", () => {
    controller.forceExtrudeGrab();
    move(yFor(-18));
    toolChipStore.getState().onSymmetric?.(true);
    const [origin, dir, mode] = handle();
    // `depth` is the TOTAL span of a symmetric extrude (SCHEMA §7.3), half per side.
    expect(origin[2]).toBeCloseTo(34, 6); // 25 + |−18| / 2
    expect(dir[2]).toBeCloseTo(1, 6);
    expect(mode).toBe("twoWay");
  });

  it("symmetric endpoint motion changes Total span at twice physical displacement", () => {
    controller.forceExtrudeGrab();
    move(yFor(20));
    toolChipStore.getState().onSymmetric?.(true);

    move(yFor(25)); // positive endpoint moved outward 5 mm

    expect(debug().depth).toBeCloseTo(30, 6); // Total 20 + 2 × 5
  });
});

/*
 * SYMMETRIC + ZERO (TODO.md SESSION 37 H3, decision D-S2). The worker builds
 * `|distance|` for Symmetric, so the stored sign is encoding only: the physical
 * half-span follows the pointer 1:1 for EITHER sign, clamps at 0 (rebasing, so a
 * reversal acts at once) and keeps the sign. A zero depth has no effect and must
 * not be confirmable.
 *
 * The fake ray here is scaled: 10 px of pointer travel is 1 world unit along the
 * axis, so every step below clears the 4 px drag threshold.
 */
describe("ModelToolController extrude symmetric span and zero depth", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;

  /** clientY whose ray sits `world` units up the axis. */
  const yAt = (world: number): number => 100 - world * 10;

  function debug(): Record<string, unknown> {
    return (window as unknown as { __extrudePreview: Record<string, unknown> }).__extrudePreview;
  }

  beforeEach(async () => {
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
    selectionStore
      .getState()
      .set([{ kind: "sketchRegion", id: "r0-ref", sketchId: "sk", regionId: "r0" }]);
    engineMock = makeEngineMock();
    engineMock.screenRay.mockImplementation((_x: number, y: number) => ({
      origin: [0, 0, PLANE.origin[2] + (100 - y) / 10],
      dir: [1, 0, 0],
    }));
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

  function ptr(type: string, clientY: number): void {
    container.dispatchEvent(
      new MouseEvent(type, { clientX: 5, clientY, button: 0, buttons: type === "pointerup" ? 0 : 1, bubbles: true }),
    );
  }

  /** z of the newest arrow re-anchor (the sketch plane sits at z = 25). */
  const headZ = (): number => {
    const calls = engineMock.setExtrudeHandle.mock.calls;
    return (calls[calls.length - 1] as unknown as [number[]])[0][2];
  };

  const committed = (): boolean =>
    clientMock.applyOperation.mock.calls.length > 0 ||
    (clientMock.endPreview.mock.calls as unknown[][]).some((call) => call[1] === true);

  it("a NEGATIVE symmetric span grows with the pointer: stored −20, +1 along the normal ⇒ −22, head +1", () => {
    toolChipStore.getState().onValue?.(-20);
    toolChipStore.getState().onSymmetric?.(true);
    expect(headZ()).toBeCloseTo(35, 9); // 25 + |−20| / 2

    ptr("pointerdown", yAt(10));
    ptr("pointermove", yAt(11));

    expect(debug().depth).toBeCloseTo(-22, 9);
    expect(headZ()).toBeCloseTo(36, 9);
  });

  it("a positive symmetric span dragged below zero clamps at 0 with its sign kept, and reverses at once", () => {
    toolChipStore.getState().onValue?.(20);
    toolChipStore.getState().onSymmetric?.(true);

    ptr("pointerdown", yAt(10));
    ptr("pointermove", yAt(-5)); // 15 past the plane: the half-span would be −5
    expect(debug().depth).toBe(0);
    expect(headZ()).toBeCloseTo(25, 9);

    ptr("pointermove", yAt(-4)); // +1 back: no return journey through the overshoot
    expect(debug().depth).toBeCloseTo(2, 9);
  });

  it("a symmetric toggle mid-drag on a negative span does not jump the next move", () => {
    toolChipStore.getState().onValue?.(-20);
    ptr("pointerdown", yAt(-20));
    ptr("pointermove", yAt(-21));
    expect(debug().depth).toBeCloseTo(-21, 9);

    toolChipStore.getState().onSymmetric?.(true);
    expect(debug().depth).toBeCloseTo(-21, 9);
    const head = headZ();

    ptr("pointermove", yAt(-20)); // the head is now at +10.5; +1 moves it to +11.5
    expect(headZ()).toBeCloseTo(head + 1, 9);
    expect(debug().depth).toBeCloseTo(-23, 9);
  });

  it("depth 0 has no effect: invalid, the arrow stays, Enter and ✓ refuse; the next non-zero value clears it", async () => {
    const hides = engineMock.hideExtrudePreview.mock.calls.length;
    toolChipStore.getState().onValue?.(0);

    expect(toolChipStore.getState().validation).toMatchObject({ status: "invalid", message: "No effect: depth is 0" });
    expect(engineMock.hideExtrudePreview.mock.calls.length).toBe(hides);
    expect(headZ()).toBeCloseTo(25, 9);

    toolChipStore.getState().onConfirm?.();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    // …and a validation someone else cleared does not make it confirmable.
    toolChipStore.getState().clearValidation();
    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();
    expect(committed()).toBe(false);
    expect(toolChipStore.getState().validation.status).toBe("invalid");
    expect(debug().phase).toBe("armed");

    toolChipStore.getState().onValue?.(4);
    expect(toolChipStore.getState().validation).toEqual({ status: "valid" });
  });

  it("depth 0 asks the kernel for nothing and draws no candidate; the next non-zero value previews again", () => {
    // No pacing floor, so every request below is a send decision, not a timer race.
    (controller as unknown as { throttle: { setTrailingMs(ms: number): void } }).throttle.setTrailingMs(0);
    const deliver = clientMock.onPreviewResult.mock.calls[0][0] as (r: PreviewResult) => void;
    const sends = (): Array<[string, { distance?: number }, number]> =>
      clientMock.updatePreview.mock.calls as unknown as Array<[string, { distance?: number }, number]>;
    /** Answer the newest request with a candidate that replaces `body1`. */
    const answerNewest = (): void => {
      const [sessionId, , epoch] = sends()[sends().length - 1];
      deliver({ sessionId, epoch, bodyId: "preview", bodies: [], replacedBodyIds: ["body1"] });
    };
    const drawn = (): number =>
      engineMock.setPreviewReplacedBodyIds.mock.calls.filter((c) => (c[0] as string[]).length > 0).length;
    answerNewest(); // the arm's own request

    toolChipStore.getState().onValue?.(12);
    expect(sends()[sends().length - 1][1].distance).toBe(12);
    answerNewest();
    const drawnAt12 = drawn();
    expect(drawnAt12).toBeGreaterThan(0);

    const before = sends().length;
    toolChipStore.getState().onValue?.(0);
    toolChipStore.getState().onSymmetric?.(true); // still 0: still nothing to ask
    expect(sends()).toHaveLength(before);
    expect(engineMock.setPreviewReplacedBodyIds).toHaveBeenLastCalledWith([]); // the 12 mm candidate is gone

    toolChipStore.getState().onValue?.(4);
    expect(sends()[sends().length - 1][1].distance).toBe(4);
    toolChipStore.getState().onValue?.(6); // coalesced behind the in-flight 4
    toolChipStore.getState().onValue?.(0); // left for 0 with 4 in flight and 6 queued
    const queued = sends().length;
    answerNewest(); // 4's late answer is not drawn, and the queued 6 is not pumped
    expect(drawn()).toBe(drawnAt12);
    expect(sends()).toHaveLength(queued);
    expect(sends().some(([, params]) => params.distance === 0)).toBe(false);

    toolChipStore.getState().onValue?.(5);
    expect(sends()[sends().length - 1][1].distance).toBe(5);
  });

  it("depth 0 reads NO preview, not a pending one, even after a late answer", () => {
    (controller as unknown as { throttle: { setTrailingMs(ms: number): void } }).throttle.setTrailingMs(0);
    const deliver = clientMock.onPreviewResult.mock.calls[0][0] as (r: PreviewResult) => void;
    const sends = (): Array<[string, { distance?: number }, number]> =>
      clientMock.updatePreview.mock.calls as unknown as Array<[string, { distance?: number }, number]>;
    const answer = (i: number): void => {
      const [sessionId, , epoch] = sends()[i];
      deliver({ sessionId, epoch, bodyId: "preview", bodies: [] });
    };
    const lifecycle = (): string => toolChipStore.getState().previewLifecycle.status;
    answer(sends().length - 1);
    toolChipStore.getState().onValue?.(4); // in flight
    const inFlight = sends().length - 1;
    expect(lifecycle()).toBe("pending");

    toolChipStore.getState().onValue?.(0);
    expect(lifecycle()).toBe("none");
    answer(inFlight); // 4's answer lands at depth 0
    expect(lifecycle()).toBe("none");

    toolChipStore.getState().onValue?.(5);
    expect(lifecycle()).toBe("pending");
    answer(sends().length - 1);
    expect(lifecycle()).toBe("valid");
  });

  it("Escape restoring a drag to depth 0 asks the kernel for nothing", () => {
    (controller as unknown as { throttle: { setTrailingMs(ms: number): void } }).throttle.setTrailingMs(0);
    const deliver = clientMock.onPreviewResult.mock.calls[0][0] as (r: PreviewResult) => void;
    const sends = (): Array<[string, { distance?: number }, number]> =>
      clientMock.updatePreview.mock.calls as unknown as Array<[string, { distance?: number }, number]>;
    const answerNewest = (): void => {
      const [sessionId, , epoch] = sends()[sends().length - 1];
      deliver({ sessionId, epoch, bodyId: "preview", bodies: [] });
    };
    answerNewest();
    toolChipStore.getState().onValue?.(0);
    ptr("pointerdown", yAt(0));
    ptr("pointermove", yAt(5));
    expect(sends()[sends().length - 1][1].distance).toBe(5);
    answerNewest();

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));

    expect(debug().depth).toBe(0);
    expect(sends().some(([, params]) => params.distance === 0)).toBe(false);
  });

  it("W4: Flip at depth 0, or a typed −0, stores +0 — never a sign the depth cannot carry", () => {
    const depth = (): number => (controller as unknown as { extrude: { depth: number } }).extrude.depth;
    toolChipStore.getState().onValue?.(0);
    toolChipStore.getState().onFlip?.();
    expect(Object.is(depth(), 0)).toBe(true);
    expect(Object.is(toolChipStore.getState().value, 0)).toBe(true);

    toolChipStore.getState().onValue?.(-0);
    expect(Object.is(depth(), 0)).toBe(true);
    expect(Object.is(toolChipStore.getState().value, 0)).toBe(true);
  });

  it("W7: a one-sided drag snapping to 0 from the NEGATIVE side stores +0", () => {
    // A camera distance gives the drag a rounding quantum, so −0.2 snaps to (−)0.
    Object.assign(engineMock, { getCameraDistance: vi.fn(() => 100) });
    settingsStore.getState().setSnap("dimensionRound", true);
    toolChipStore.getState().onValue?.(-3);
    ptr("pointerdown", yAt(-3));
    ptr("pointermove", yAt(-0.2)); // snaps to the zero quantum from below

    expect(Object.is(debug().depth, 0)).toBe(true);
    expect(Object.is(toolChipStore.getState().value, 0)).toBe(true);
  });

  it("a symmetric drag that clamps at 0 is refused the same way", () => {
    toolChipStore.getState().onValue?.(6);
    toolChipStore.getState().onSymmetric?.(true);
    ptr("pointerdown", yAt(3));
    ptr("pointermove", yAt(-10));
    ptr("pointerup", yAt(-10));

    expect(debug().depth).toBe(0);
    expect(toolChipStore.getState().validation).toMatchObject({ status: "invalid", message: "No effect: depth is 0" });
  });

  it("Escape in the depth field reverts through the controller, restoring a zero depth's verdict", () => {
    toolChipStore.getState().onValue?.(0);
    toolChipStore.getState().onValue?.(7); // typed over it: valid while the edit lasts
    expect(toolChipStore.getState().validation.status).toBe("valid");

    const revert = toolChipStore.getState().onRevertValue;
    expect(revert).toBeTypeOf("function");
    revert?.(0);

    expect(toolChipStore.getState().value).toBe(0); // a typed depth does not republish the debug surface
    expect(toolChipStore.getState().validation).toMatchObject({ status: "invalid", message: "No effect: depth is 0" });
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
