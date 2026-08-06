/*
 * SKETCH-ON-FACE — HOST-BOOLEAN (jsdom): a modeling op driven off a FACE-HOSTED
 * sketch defaults to modifying its HOST body instead of spawning a new one, and
 * the extrude drag DIRECTION owns Add vs Cut until the chip overrides it.
 *
 * The wiring under test is entirely in the tool layer: the wire, the worker and
 * the shared op builder have carried `booleanMode` + `targetBodyId` since
 * MODEL-HARDEN W2 (`wire_contract`'s pocket/cut proofs). What changes here is
 * which defaults an arm opens with, so every assertion below reads either the
 * FSM's resolved state (the debug surface), the chip the user sees, or the exact
 * params that go out on the preview lane — never a mock's own bookkeeping.
 *
 * DEPTH CONTROL. `screenRay` is faked so a pointer move maps to an EXACT signed
 * depth: the ray runs perpendicular to the extrude axis, so `axisDepthFromRay`
 * collapses (`b = 0`, `denom = 1`) to the ray origin's height ABOVE THE SKETCH
 * PLANE. That makes "drag away from the body" vs "drag into it" a deterministic
 * input rather than a camera-dependent guess.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  FinishSketchResult,
  PreviewDraft,
  PreviewParams,
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
  normal: [0, 0, 1], // the host face's OUTWARD normal: +depth grows away from the solid
};

const R0: SketchRegion = {
  regionId: "r0",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [0, 0, 20, 0, 20, 20, 0, 20], indices: [0, 1, 2, 0, 2, 3] },
};

/** A sketch line at u=50 — a legal revolve axis for R0 (it crosses nothing). */
const AXIS_OK = {
  id: "axOk",
  type: "Line" as const,
  p0: [50, -50] as [number, number],
  p1: [50, 200] as [number, number],
};

const ok = (): ApplyOperationResult => ({
  revision: 1,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#1" }],
  removedBodies: [],
});

function makeSession(entities: SketchSession["entities"] = []): SketchSession {
  return { sketchId: "sk", plane: PLANE, entities, constraints: [], dof: 0, status: "FullyConstrained" };
}

function makeEngineMock() {
  return {
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    setRegionSelected: vi.fn(),
    hideRegionPick: vi.fn(),
    screenToPlaneOn: vi.fn((_p: SketchPlane, x: number, y: number) => ({ x, y })),
    setOrbitSuppressed: vi.fn(),
    planePixelWorld: vi.fn(() => 1),
    showExtrudePreview: vi.fn(),
    showExtrudePreviews: vi.fn(),
    setExtrudeDepth: vi.fn(),
    setPreviewTint: vi.fn(),
    setExtrudeHandleHover: vi.fn(),
    hitExtrudeHandle: vi.fn(() => false),
    // Depth = the ray origin's height above the sketch plane (see the header).
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

function makeClientMock(
  opts: { entities?: SketchSession["entities"]; stored?: Record<string, unknown> } = {},
  capture?: (cb: (r: PreviewResult) => void) => void,
) {
  let seq = 0;
  return {
    onPreviewResult: vi.fn((cb: (r: PreviewResult) => void) => {
      capture?.(cb);
      return () => {};
    }),
    finishSketch: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0] })),
    getSketchRegions: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0] })),
    getSketch: vi.fn(() => Promise.resolve(makeSession(opts.entities))),
    beginPreview: vi.fn((_d: PreviewDraft) =>
      Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` }),
    ),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(ok())),
    applyOperation: vi.fn(() => Promise.resolve(ok())),
    applyEditCommand: vi.fn(() => Promise.resolve(ok())),
    undo: vi.fn(() => Promise.resolve(ok())),
    getOperationParams: vi.fn(
      (): Promise<Record<string, unknown>> =>
        Promise.resolve(
          opts.stored ?? { profile: { sketchId: "sk", regionId: "r0" }, distance: { value: 20 } },
        ),
    ),
  };
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));
const waitMs = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

describe("ModelToolController host-boolean (sketch on a face defaults to modifying its host)", () => {
  let engineMock: ReturnType<typeof makeEngineMock>;
  let clientMock: ReturnType<typeof makeClientMock>;
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let previewCb: ((r: PreviewResult) => void) | null;

  function debug(): Record<string, unknown> {
    return (window as unknown as { __extrudePreview: Record<string, unknown> }).__extrudePreview;
  }

  function build(opts: Parameters<typeof makeClientMock>[0] = {}): void {
    engineMock = makeEngineMock();
    previewCb = null;
    clientMock = makeClientMock(opts, (cb) => {
      previewCb = cb;
    });
    controller = new ModelToolController({
      engine: engineMock as unknown as ViewportEngine,
      client: clientMock as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
      debug: true,
    });
  }

  /** Register sketch `sk`; `host` attaches it to a model face (else world-plane). */
  function seedSketch(host?: { bodyId: string; elementId: string }): void {
    documentStore.getState().addSketch({
      id: "sk",
      name: "Sketch On Face",
      visible: true,
      dof: 0,
      status: "ok",
      geometryToken: "sk:v1",
      ...(host ? { hostFace: host } : {}),
    });
  }

  beforeEach(() => {
    resetStores();
    __setExactPreviewTimeoutForTests(0);
    selectionStore.getState().set([
      { kind: "sketchRegion", id: "r0-ref", sketchId: "sk", regionId: "r0" },
    ]);
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    controller?.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
  });

  async function armExtrude(): Promise<void> {
    toolStore.getState().setTool("extrude");
    await flush();
  }

  /** Grab the extrude handle and drag to an exact signed depth. */
  function dragToDepth(depth: number): void {
    container.dispatchEvent(
      new MouseEvent("pointermove", { clientX: 5, clientY: 100 - depth, buttons: 1, bubbles: true }),
    );
  }

  /** Arm the revolve tool on the whole sketch (it starts from a sketch selection). */
  async function armRevolve(): Promise<void> {
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("revolve");
    await flush();
    await flush();
  }

  /** Click the u=50 axis candidate (identity screen→plane in the engine mock). */
  async function pickAxis(): Promise<void> {
    container.dispatchEvent(new MouseEvent("pointerdown", { clientX: 50, clientY: 50, button: 0, buttons: 1, bubbles: true }));
    container.dispatchEvent(new MouseEvent("pointerup", { clientX: 50, clientY: 50, button: 0, buttons: 0, bubbles: true }));
    await flush();
    await flush();
  }

  /** The params of the most recent lane push (what the kernel is previewing). */
  function lastParams(): PreviewParams {
    const calls = clientMock.updatePreview.mock.calls;
    return calls[calls.length - 1][1] as PreviewParams;
  }

  /**
   * Push the CURRENT params onto the wire: wait past the throttle's trailing floor,
   * then ack the in-flight epoch. Nothing else pumps the lane in this fake (the
   * throttle holds at ≤1 in flight and only a preview RESULT releases the slot),
   * so this is what makes `lastParams()` the newest params rather than the arm's.
   */
  async function settleLane(): Promise<void> {
    await waitMs(100);
    const calls = clientMock.updatePreview.mock.calls;
    const last = calls[calls.length - 1];
    previewCb?.({ sessionId: last[0] as string, epoch: last[2] as number, bodyId: "pb-1", bodies: [] });
  }

  // ── extrude ────────────────────────────────────────────────────────────────

  it("a FRESH arm off a face-hosted sketch opens on Add, bound to the host body", async () => {
    seedSketch({ bodyId: "body1", elementId: "el_top" });
    build();
    await armExtrude();

    expect(debug().phase).toBe("armed");
    expect(debug().booleanMode).toBe("Add");
    expect(debug().booleanTargetId).toBe("body1");
    // The chip the user sees opens on the SAME mode — not the old NewBody literal.
    expect(toolChipStore.getState().booleanMode).toBe("Add");
    expect(toolChipStore.getState().canBoolean).toBe(true);
    // …and the very first exact preview is already the fused candidate.
    expect(lastParams().booleanMode).toBe("Add");
    expect(lastParams().targetBodyId).toBe("body1");
  });

  it("dragging INTO the host flips to Cut live — chip, tint and previewed op all follow", async () => {
    seedSketch({ bodyId: "body1", elementId: "el_top" });
    build();
    await armExtrude();
    controller.forceExtrudeGrab();

    dragToDepth(-12); // into the solid
    expect(debug().booleanMode).toBe("Cut");
    expect(debug().depth).toBe(-12);
    expect(toolChipStore.getState().booleanMode).toBe("Cut");
    expect(engineMock.setPreviewTint).toHaveBeenLastCalledWith("cut");
    await settleLane();
    expect(lastParams().booleanMode).toBe("Cut"); // the kernel is previewing the pocket
    expect(lastParams().targetBodyId).toBe("body1");

    // …and back out again: the flip is live in both directions.
    dragToDepth(9);
    expect(debug().booleanMode).toBe("Add");
    expect(toolChipStore.getState().booleanMode).toBe("Add");
    expect(engineMock.setPreviewTint).toHaveBeenLastCalledWith("normal");
    await settleLane();
    expect(lastParams().booleanMode).toBe("Add");
  });

  it("a manual chip choice ends the auto lane — a later outward drag stays Cut", async () => {
    seedSketch({ bodyId: "body1", elementId: "el_top" });
    build();
    await armExtrude();

    toolChipStore.getState().onBooleanMode?.("Cut"); // the user overrides
    await flush();
    expect(debug().booleanMode).toBe("Cut");
    expect(debug().booleanTargetId).toBe("body1"); // the host stays bound

    controller.forceExtrudeGrab();
    dragToDepth(30); // outward — would have flipped to Add on the auto lane
    expect(debug().booleanMode).toBe("Cut");
    expect(toolChipStore.getState().booleanMode).toBe("Cut");
  });

  it("a NewBody override drops the host entirely and never returns for that session", async () => {
    seedSketch({ bodyId: "body1", elementId: "el_top" });
    build();
    await armExtrude();

    toolChipStore.getState().onBooleanMode?.("NewBody");
    await settleLane();
    expect(debug().booleanMode).toBe("NewBody");
    expect(debug().booleanTargetId).toBeNull();
    expect(lastParams().targetBodyId).toBeUndefined(); // the host is off the op entirely

    controller.forceExtrudeGrab();
    dragToDepth(-15);
    expect(debug().booleanMode).toBe("NewBody");
  });

  it("a WORLD-plane sketch is untouched: NewBody, no target, no flip", async () => {
    seedSketch(); // no hostFace
    build();
    await armExtrude();

    expect(debug().booleanMode).toBe("NewBody");
    expect(debug().booleanTargetId).toBeNull();
    expect(toolChipStore.getState().booleanMode).toBe("NewBody");

    controller.forceExtrudeGrab();
    dragToDepth(-20);
    expect(debug().booleanMode).toBe("NewBody");
  });

  it("a HIDDEN host body falls back to NewBody (never bind a body the user can't see)", async () => {
    seedSketch({ bodyId: "body1", elementId: "el_top" });
    documentStore.getState().setVisibility("body1", false);
    build();
    await armExtrude();

    expect(debug().booleanMode).toBe("NewBody");
    expect(debug().booleanTargetId).toBeNull();
  });

  it("a MISSING host body falls back to NewBody", async () => {
    seedSketch({ bodyId: "ghost", elementId: "el_top" });
    build();
    await armExtrude();

    expect(debug().booleanMode).toBe("NewBody");
    expect(debug().booleanTargetId).toBeNull();
  });

  it("a RE-EDIT is never re-defaulted: a stored Cut re-arms as Cut and commits as Cut", async () => {
    seedSketch({ bodyId: "body1", elementId: "el_top" });
    const stored = {
      profile: { sketchId: "sk", regionId: "r0" },
      distance: { value: 20 },
      booleanMode: "Cut",
      targetBodyId: "body1",
    };
    build({ stored });
    documentStore.setState({
      features: [{ id: "feat-ex", kind: "extrude", label: "Extrude", valueText: "20 mm", status: "ok" }],
    });
    controller.editExtrudeFeature("feat-ex");
    await flush();

    // The host default must not touch a re-edit — its commit deep-merges the STORED
    // params, so a fresh-arm default here would silently re-target the feature.
    expect(debug().booleanMode).toBe("NewBody");
    expect(debug().booleanTargetId).toBeNull();
    expect(toolChipStore.getState().showBooleanSegments).toBe(false);

    toolChipStore.getState().onValue?.(25);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    // WP-C3: the re-edit patch also carries the armed draft angle. This record
    // stores none, so the arm seeds 0 and the patch writes the wire default back
    // — the boolean mode + target still ride through untouched, which is what
    // this case is about.
    expect(clientMock.applyEditCommand).toHaveBeenCalledWith({
      cmd: "updateOperationParams",
      record: "feat-ex",
      op: {
        opType: "Extrude",
        params: { ...stored, distance: { value: 25 }, draftAngleDeg: { value: 0 } },
      },
    });
  });

  // ── revolve ────────────────────────────────────────────────────────────────

  it("a face-hosted REVOLVE arms on Add against the host, with no direction rule", async () => {
    seedSketch({ bodyId: "body1", elementId: "el_top" });
    build({ entities: [AXIS_OK] });
    await armRevolve();

    // The seed is live before the axis is even chosen, and survives the pick.
    expect(debug().revolvePhase).toBe("axisPick");
    expect(debug().booleanTargetId).toBe("body1");

    await pickAxis();

    expect(debug().revolvePhase).toBe("armed");
    expect(debug().booleanTargetId).toBe("body1");
    expect(toolChipStore.getState().booleanMode).toBe("Add");
    const draft = clientMock.beginPreview.mock.calls[0][0] as PreviewDraft;
    expect(draft.opType).toBe("Revolve");
    expect(draft.params.booleanMode).toBe("Add");
    expect(draft.params.targetBodyId).toBe("body1");
  });

  it("a world-plane REVOLVE is untouched: NewBody, no target", async () => {
    seedSketch();
    build({ entities: [AXIS_OK] });
    await armRevolve();
    await pickAxis();

    expect(debug().revolvePhase).toBe("armed");
    expect(debug().booleanTargetId).toBeNull();
    expect(toolChipStore.getState().booleanMode).toBe("NewBody");
    const draft = clientMock.beginPreview.mock.calls[0][0] as PreviewDraft;
    expect(draft.params.booleanMode).toBe("NewBody");
  });
});
