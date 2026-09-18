/*
 * MODEL-TOOL GESTURE OWNERSHIP (jsdom) — how a value drag ENDS.
 *
 * A drag has three honest endings and they must never be confused:
 *
 *   release  — pointerup (or a mouse sample proving the button is already up):
 *              keep the dragged value, back to armed;
 *   cancel   — pointercancel, an unexpected lostpointercapture, window blur or
 *              Escape: put the PRE-GRAB value back, back to armed, tool kept;
 *   abandon  — the tool is torn down (switch, dispose): drop the gesture, write
 *              nothing.
 *
 * Before this contract a missed pointerup left the tool "dragging" for good: hover
 * kept mutating the value, Enter was dead, wheel navigation stayed blocked, Escape
 * dropped the whole tool, and a second press re-grabbed on a new basis.
 *
 * Every kind is driven through the same table. `at(t)` is the pointer position for
 * a travel of `t` value units from the press, so a frame at `at(t)` reads
 * `valueAtPress + sign·t` for every kind (all drags are grab-relative).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
import type { ViewportEngine } from "@/viewport/engine/ViewportEngine";
import type { GizmoHit } from "@/viewport/engine/TransformGizmo";
import type { TransformBodyParams } from "@/ipc/types";
import type { CadClient } from "@/ipc/client";
import type {
  ApplyOperationResult,
  FinishSketchResult,
  OperationOp,
  PrepareOffsetFaceRequest,
  PrepareOffsetFaceResult,
  PreviewDraft,
  PreviewParams,
  PreviewResult,
  SketchPlane,
  SketchRegion,
  SketchSession,
} from "@/ipc/types";
import { toolStore, type ModelTool } from "@/stores/toolStore";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";
import { buildBodyObjects, swap as swapMesh, __resetRegistryForTests } from "@/viewport/mesh/meshRegistry";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import { makeBoxMesh } from "@/ipc/mockMeshes";
import { viewportDragActive } from "@/viewport/ViewportRoot";

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

/** A revolve axis line at u=50, clear of R0. */
const AXIS = { id: "ax", type: "Line" as const, p0: [50, -50] as [number, number], p1: [50, 200] as [number, number] };

const EDGES: EntityRef[] = [
  { kind: "edge", id: "body1#e:5", bodyId: "body1", topoKey: "e:5", elementId: "el-edge-5", anchor: { worldPoint: [1, 2, 3] } },
  { kind: "edge", id: "body1#e:9", bodyId: "body1", topoKey: "e:9", elementId: "el-edge-9", anchor: { worldPoint: [4, 5, 6] } },
];

const FACE: EntityRef = {
  kind: "face",
  id: "body1#f:2",
  bodyId: "body1",
  topoKey: "f:2",
  elementId: "el-face-2",
  anchor: { worldPoint: [0, 0, 10] },
};

const ok = (): ApplyOperationResult => ({
  revision: 2,
  features: [],
  changedBodies: [{ bodyId: "body1", meshKey: "body1#1" }],
  removedBodies: [],
});

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0));
const waitMs = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

type Debug = Record<string, unknown>;
const debug = (): Debug => (window as unknown as { __extrudePreview?: Debug }).__extrudePreview ?? {};

function makeEngineMock(grabbed: { value: GizmoHit | null }) {
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
    // Extrude: perpendicular to the +Z axis, so clientY maps to an exact depth.
    screenRay: vi.fn((_x: number, y: number): { origin: number[]; dir: number[] } => ({ origin: [0, 0, 100 - y], dir: [1, 0, 0] })),
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
    showGhostPreview: vi.fn(),
    hideGhostPreview: vi.fn(),
    hideValueHandle: vi.fn(),
    showValueHandle: vi.fn(),
    showScreenValueHandle: vi.fn(),
    showGhostPreviewMulti: vi.fn(),
    probePick: vi.fn(() => null),
    showTransformGizmo: vi.fn(),
    hideTransformGizmo: vi.fn(),
    setTransformGizmoHover: vi.fn(),
    setTransformGizmoActive: vi.fn(),
    hitTransformGizmo: vi.fn(() => grabbed.value),
  };
}

function makeClientMock(capture: (cb: (r: PreviewResult) => void) => void) {
  let seq = 0;
  return {
    onPreviewResult: vi.fn((cb: (r: PreviewResult) => void) => {
      capture(cb);
      return () => {};
    }),
    onDocumentChanged: vi.fn(() => () => {}),
    getCurrentMeshPublication: vi.fn(() => null),
    finishSketch: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0] })),
    getSketchRegions: vi.fn((): Promise<FinishSketchResult> => Promise.resolve({ regions: [R0] })),
    getSketch: vi.fn(() =>
      Promise.resolve<SketchSession>({
        sketchId: "sk",
        plane: PLANE,
        entities: [AXIS],
        constraints: [],
        dof: 0,
        status: "FullyConstrained",
      }),
    ),
    prepareEdgeOp: vi.fn(() =>
      Promise.resolve({
        snapshotId: 1,
        targetBodyId: "body_body1",
        edges: EDGES.map((edge) => ({
          topoKey: edge.topoKey ?? "",
          elementId: edge.elementId,
          bodyId: "body_body1",
          kind: "edge" as const,
          picked: true,
          anchor: edge.anchor,
        })),
        refusal: null,
      }),
    ),
    prepareOffsetFace: vi.fn(
      (req: PrepareOffsetFaceRequest): Promise<PrepareOffsetFaceResult> =>
        Promise.resolve({
          snapshotId: 1,
          targetBodyId: req.pickedFaces[0]?.bodyId ?? "",
          faces: req.pickedFaces.map((p) => ({
            topoKey: p.topoKey ?? "f:2",
            picked: true,
            anchor: { worldPoint: [0, 0, 10] as [number, number, number] },
          })),
          currentDims: {},
          refusal: null,
        }),
    ),
    // H10 resolves an offset's display attachment through the same classification
    // the shell wall uses; `null` is the ordinary answer for an unclassifiable
    // face and is what these gesture specs want (they arm on a planar frame).
    classifyElement: vi.fn(() => Promise.resolve(null)),
    promoteSelection: vi.fn((bodyId: string, picks: { topoKey: string }[]) =>
      Promise.resolve(
        picks.map((p) => ({
          topoKey: p.topoKey,
          elementId: `el-${p.topoKey.replace(":", "")}`,
          kind: p.topoKey.startsWith("e:") ? ("edge" as const) : ("face" as const),
          bodyId,
        })),
      ),
    ),
    canFoldTransform: vi.fn(() => Promise.resolve<string | null>(null)),
    analyzeEdgeOpRange: vi.fn((_req: { mode: string }) => Promise.reject(new Error("no range in this lane"))),
    beginPreview: vi.fn((_d: PreviewDraft) =>
      Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` }),
    ),
    updatePreview: vi.fn((_id: string, _params: PreviewParams, _epoch: number) => {}),
    endPreview: vi.fn((_id: string, _commit: boolean) => Promise.resolve(ok())),
    applyOperation: vi.fn(() => Promise.resolve(ok())),
    applyEditCommand: vi.fn(() => Promise.resolve(ok())),
    undo: vi.fn(() => Promise.resolve(ok())),
    getOperationParams: vi.fn(() => Promise.resolve({})),
  };
}

type Kind = "extrude" | "fillet" | "shell" | "offsetFace" | "revolve" | "transform";

interface Harness {
  engine: ReturnType<typeof makeEngineMock>;
  client: ReturnType<typeof makeClientMock>;
  grabbed: { value: GizmoHit | null };
}

interface Driver {
  kind: Kind;
  tool: ModelTool;
  arm(h: Harness): Promise<void>;
  /** Pointer position for a travel of `t` value units from a press at `at(0)`. */
  at(t: number): { x: number; y: number };
  sign: 1 | -1;
  /** The FSM's own resting phase, or "dragging" while a gesture owns it. */
  fsmPhase(): unknown;
  /** Called before every press (the gizmo needs to be told which handle is hit). */
  beforePress?(h: Harness): void;
}

const DRIVERS: Driver[] = [
  {
    kind: "extrude",
    tool: "extrude",
    async arm() {
      documentStore.getState().addSketch({
        id: "sk",
        name: "Sketch",
        visible: true,
        dof: 0,
        status: "ok",
        geometryToken: "sk:v1",
      });
      selectionStore.getState().set([{ kind: "sketchRegion", id: "r0-ref", sketchId: "sk", regionId: "r0" }]);
      toolStore.getState().setTool("extrude");
      await flush();
      await flush();
    },
    at: (t) => ({ x: 5, y: 100 - t }),
    sign: 1,
    fsmPhase: () => debug().phase,
  },
  {
    kind: "fillet",
    tool: "fillet",
    async arm() {
      selectionStore.getState().set(EDGES);
      toolStore.getState().setTool("fillet");
      await flush();
      await flush();
    },
    at: (t) => ({ x: 10, y: 100 - t }),
    sign: 1,
    fsmPhase: () => debug().filletPhase,
  },
  {
    kind: "shell",
    tool: "shell",
    async arm() {
      selectionStore.getState().set([FACE]);
      toolStore.getState().setTool("shell");
      await flush();
      await flush();
    },
    at: (t) => ({ x: 10, y: 100 - t }),
    sign: 1,
    fsmPhase: () => debug().shellPhase,
  },
  {
    kind: "offsetFace",
    tool: "offsetFace",
    async arm() {
      selectionStore.getState().set([FACE]);
      toolStore.getState().setTool("offsetFace");
      await flush();
      await flush();
      await flush();
    },
    at: (t) => ({ x: 10, y: 100 - t }),
    sign: 1,
    fsmPhase: () => debug().offsetFacePhase,
  },
  {
    kind: "revolve",
    tool: "revolve",
    async arm() {
      documentStore.getState().addSketch({
        id: "sk",
        name: "Sketch",
        visible: true,
        dof: 0,
        status: "ok",
        geometryToken: "sk:v1",
      });
      selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
      toolStore.getState().setTool("revolve");
      await flush();
      await flush();
      // Identity screen→plane: this click takes the u=50 axis line.
      const at = { clientX: 50, clientY: 50, button: 0, bubbles: true };
      container.dispatchEvent(new MouseEvent("pointerdown", { ...at, buttons: 1 }));
      container.dispatchEvent(new MouseEvent("pointerup", { ...at, buttons: 0 }));
      await flush();
      await flush();
    },
    // 0.75°/px, leftward reduces the angle: travel t = t degrees below the press.
    at: (t) => ({ x: 200 - t / 0.75, y: 100 }),
    sign: -1,
    fsmPhase: () => debug().revolvePhase,
  },
  {
    kind: "transform",
    tool: "transform",
    async arm(h) {
      swapMesh("body1", buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body1", 1));
      // Straight down −Z from world (x, −y): a screen delta IS a world delta.
      h.engine.screenRay.mockImplementation((x: number, y: number) => ({ origin: [x, -y, 1000], dir: [0, 0, -1] }));
      selectionStore.getState().set([{ kind: "body", id: "body1" }]);
      toolStore.getState().setTool("transform");
      await flush();
    },
    at: (t) => ({ x: t, y: 0 }),
    sign: 1,
    fsmPhase: () => (debug().transformGrab ? "dragging" : debug().transformPhase),
    beforePress: (h) => {
      h.grabbed.value = { kind: "axis", axis: "X" };
    },
  },
];

let container: HTMLDivElement;

describe.each(DRIVERS)("model-tool gesture lifecycle — $kind", (d) => {
  let h: Harness;
  let controller: ModelToolController;
  let shortcutCancel: (e: KeyboardEvent) => void;
  let shortcutCancels = 0;

  beforeEach(async () => {
    resetStores();
    __resetRegistryForTests();
    __setExactPreviewTimeoutForTests(0);
    container = document.createElement("div");
    document.body.appendChild(container);
    const grabbed = { value: null as GizmoHit | null };
    const engine = makeEngineMock(grabbed);
    const client = makeClientMock(() => {});
    h = { engine, client, grabbed };
    controller = new ModelToolController({
      engine: engine as unknown as ViewportEngine,
      client: client as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
      debug: true,
    });
    // Stand-in for `useShortcuts`' Esc ladder (window, BUBBLE phase): the first
    // rung drops the active tool. Anything the controller swallows never reaches it.
    shortcutCancels = 0;
    shortcutCancel = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      shortcutCancels++;
      if (toolStore.getState().modelTool !== "select") toolStore.getState().setTool("select");
    };
    window.addEventListener("keydown", shortcutCancel);
    await d.arm(h);
    expect(toolStore.getState().modelTool).toBe(d.tool);
    expect(toolStore.getState().phase).toBe("armed");
  });

  afterEach(() => {
    window.removeEventListener("keydown", shortcutCancel);
    controller.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
    __resetRegistryForTests();
  });

  interface PtrInit {
    pointerId?: number;
    pointerType?: string;
    buttons?: number;
    isPrimary?: boolean;
    target?: EventTarget;
  }

  function ptr(type: string, t: number, init: PtrInit = {}): void {
    const { x, y } = d.at(t);
    const target = init.target ?? container;
    target.dispatchEvent(
      new PointerEvent(type, {
        clientX: x,
        clientY: y,
        button: 0,
        buttons: init.buttons ?? (type === "pointerup" || type === "pointercancel" ? 0 : 1),
        pointerId: init.pointerId ?? 1,
        pointerType: init.pointerType ?? "mouse",
        isPrimary: init.isPrimary ?? false,
        bubbles: true,
        cancelable: true,
      }),
    );
  }

  function press(t = 0, init: PtrInit = {}): void {
    d.beforePress?.(h);
    ptr("pointerdown", t, init);
  }

  const chipValue = (): number => toolChipStore.getState().value as number;

  /** A completed gesture first, so the pre-grab value is not the arm default. */
  function primeValue(): number {
    const armed = chipValue();
    press(0);
    ptr("pointermove", 10);
    ptr("pointerup", 10);
    expect(toolStore.getState().phase).toBe("armed");
    const primed = chipValue();
    expect(primed).toBeCloseTo(armed + d.sign * 10, 6);
    return primed;
  }

  function expectArmedAt(value: number): void {
    expect(chipValue()).toBeCloseTo(value, 6);
    expect(d.fsmPhase()).toBe("armed");
    expect(toolStore.getState().phase).toBe("armed");
    expect(toolStore.getState().modelTool).toBe(d.tool);
  }

  it("pointercancel mid-drag restores the pre-grab value and keeps the tool armed", () => {
    const pre = primeValue();
    press(0);
    ptr("pointermove", 20);
    expect(chipValue()).toBeCloseTo(pre + d.sign * 20, 6);
    expect(toolStore.getState().phase).toBe("dragging");

    ptr("pointercancel", 20);

    expectArmedAt(pre);
    // The gesture is over: a later hover sample moves nothing.
    ptr("pointermove", 30, { buttons: 0 });
    expectArmedAt(pre);
  });

  it("lostpointercapture AFTER a pointerup keeps the dragged value", async () => {
    const pre = primeValue();
    press(0);
    ptr("pointermove", 20);
    ptr("pointerup", 20);
    ptr("lostpointercapture", 20);
    await flush();
    expectArmedAt(pre + d.sign * 20);
  });

  it("lostpointercapture delivered BEFORE the pointerup it belongs to keeps the dragged value", async () => {
    const pre = primeValue();
    press(0);
    ptr("pointermove", 20);
    ptr("lostpointercapture", 20);
    ptr("pointerup", 20);
    await flush();
    expectArmedAt(pre + d.sign * 20);
  });

  it("an unexpected lostpointercapture mid-drag restores", async () => {
    const pre = primeValue();
    press(0);
    ptr("pointermove", 20);
    ptr("lostpointercapture", 20);
    await flush();
    expectArmedAt(pre);
  });

  it("window blur mid-drag restores", () => {
    const pre = primeValue();
    press(0);
    ptr("pointermove", 20);
    window.dispatchEvent(new Event("blur"));
    expectArmedAt(pre);
  });

  it("Escape mid-drag restores and is swallowed; the NEXT Escape still cancels the tool", () => {
    const pre = primeValue();
    press(0);
    ptr("pointermove", 20);

    const esc = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    document.body.dispatchEvent(esc);

    expectArmedAt(pre);
    expect(esc.defaultPrevented).toBe(true);
    expect(shortcutCancels).toBe(0); // propagation stopped before the shortcut lane

    document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    expect(shortcutCancels).toBe(1);
    expect(toolStore.getState().modelTool).toBe("select");
    expect(toolStore.getState().phase).toBe("idle");
  });

  it("a foreign pointer can neither move, end, nor cancel the gesture", async () => {
    const pre = primeValue();
    const touch = { pointerType: "touch" };
    press(0, { ...touch, pointerId: 11 });
    ptr("pointermove", 10, { ...touch, pointerId: 11 });
    expect(chipValue()).toBeCloseTo(pre + d.sign * 10, 6);

    press(25, { ...touch, pointerId: 12 });
    ptr("pointermove", 30, { ...touch, pointerId: 12 });
    ptr("pointerup", 30, { ...touch, pointerId: 12 });
    ptr("pointercancel", 30, { ...touch, pointerId: 12 });
    ptr("lostpointercapture", 30, { ...touch, pointerId: 12 });
    await flush();
    expect(chipValue()).toBeCloseTo(pre + d.sign * 10, 6);
    expect(toolStore.getState().phase).toBe("dragging");

    ptr("pointermove", 15, { ...touch, pointerId: 11 });
    ptr("pointerup", 15, { ...touch, pointerId: 11 });
    expectArmedAt(pre + d.sign * 15);
  });

  it("a second press mid-drag is ignored — no re-grab on a new basis", () => {
    const pre = primeValue();
    press(0);
    ptr("pointermove", 10);
    press(25); // same pointer; must not re-capture the grab basis
    ptr("pointermove", 20);
    expect(chipValue()).toBeCloseTo(pre + d.sign * 20, 6);
    ptr("pointerup", 20);
    expectArmedAt(pre + d.sign * 20);
  });

  it("a mouse move with the button already up releases at the last applied value", () => {
    const pre = primeValue();
    press(0);
    ptr("pointermove", 10);
    // The pointerup was lost (focus change, OS gesture…): this sample proves it.
    ptr("pointermove", 20, { buttons: 0 });
    expectArmedAt(pre + d.sign * 10);
    ptr("pointermove", 30, { buttons: 0 });
    expectArmedAt(pre + d.sign * 10);
  });

  it("Enter, the chip ✓ and type-to-enter are all refused mid-drag", async () => {
    primeValue();
    press(0);
    ptr("pointermove", 20);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", cancelable: true }));
    toolChipStore.getState().onConfirm?.();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "5", cancelable: true }));
    await flush();
    await flush();

    expect(h.client.applyOperation).not.toHaveBeenCalled();
    expect(h.client.applyEditCommand).not.toHaveBeenCalled();
    expect(h.client.endPreview).not.toHaveBeenCalledWith(expect.anything(), true);
    expect(toolChipStore.getState().primaryEntry).toBeNull();
    expect(toolStore.getState().phase).toBe("dragging");
    expect(toolStore.getState().modelTool).toBe(d.tool);
  });

  it("a tool switch mid-drag abandons the gesture without restoring anything", async () => {
    primeValue();
    press(0);
    ptr("pointermove", 20);
    // Restore-only side effects: the restore request (sent at once, throttle or
    // not) and each kind's re-sync of its on-screen value.
    const restoreEffects = (): number[] => [
      h.client.updatePreview.mock.calls.length,
      h.engine.setExtrudeDepth.mock.calls.length,
      h.engine.setRevolveAngle.mock.calls.length,
      h.engine.showScreenValueHandle.mock.calls.length,
      h.engine.showTransformGizmo.mock.calls.length,
    ];
    const before = restoreEffects();

    toolStore.getState().setTool("select");
    expect(restoreEffects()).toEqual(before);
    expect(toolStore.getState().phase).toBe("idle");

    ptr("pointercancel", 20);
    window.dispatchEvent(new Event("blur"));
    ptr("pointermove", 30);
    ptr("pointerup", 30);
    await flush();
    expect(restoreEffects()).toEqual(before);
    expect(toolStore.getState().phase).toBe("idle");
    expect(toolChipStore.getState().kind).toBe("none");
  });

  it("a pen hover after a lost contact releases at the last applied value", () => {
    const pre = primeValue();
    const pen = { pointerType: "pen", pointerId: 31 };
    press(0, pen);
    ptr("pointermove", 10, pen);
    ptr("pointermove", 20, { ...pen, buttons: 0 }); // lifted: hovering, not touching
    expectArmedAt(pre + d.sign * 10);
    ptr("pointermove", 30, { ...pen, buttons: 0 });
    expectArmedAt(pre + d.sign * 10);
  });

  it("a new PRIMARY touch proves the old contact lifted: the stale gesture releases and the press grabs", () => {
    const pre = primeValue();
    const touch = (pointerId: number) => ({ pointerType: "touch", pointerId, isPrimary: true });
    press(0, touch(21));
    ptr("pointermove", 10, touch(21));
    // 21's pointerup never reached us; the next contact is a new primary touch.
    press(0, touch(22));
    expect(chipValue()).toBeCloseTo(pre + d.sign * 10, 6);
    ptr("pointermove", 5, touch(22));
    expect(chipValue()).toBeCloseTo(pre + d.sign * 15, 6);
    ptr("pointerup", 5, touch(22));
    expectArmedAt(pre + d.sign * 15);
  });

  it("a runtime session change mid-drag abandons the gesture", () => {
    const pre = primeValue();
    press(0);
    ptr("pointermove", 20);
    documentStore.setState({ runtimeSession: "rs-replaced" });
    expect(toolStore.getState().phase).not.toBe("dragging");
    ptr("pointermove", 30);
    ptr("pointercancel", 30); // nothing left to restore either
    expect(chipValue()).toBeCloseTo(pre + d.sign * 20, 6);
    expect(viewportDragActive()).toBe(false);
  });

  it("the wheel gate follows gesture ownership, not a phase a same-tool setTool rewrites", () => {
    primeValue();
    expect(viewportDragActive()).toBe(false);
    press(0);
    ptr("pointermove", 20);
    expect(viewportDragActive()).toBe(true);
    toolStore.getState().setTool(d.tool); // e.g. the tool's own shortcut, pressed mid-drag
    expect(toolStore.getState().phase).toBe("armed");
    expect(viewportDragActive()).toBe(true);
    ptr("pointerup", 20);
    expect(viewportDragActive()).toBe(false);
  });

  it("dispose mid-drag leaves no toolStore 'dragging' behind", () => {
    primeValue();
    press(0);
    ptr("pointermove", 20);
    expect(toolStore.getState().phase).toBe("dragging");
    controller.dispose();
    expect(toolStore.getState().phase).not.toBe("dragging");
    expect(viewportDragActive()).toBe(false);
  });
});

// ── kind-specific endings ─────────────────────────────────────────────────────

describe("model-tool gesture lifecycle — specifics", () => {
  let h: Harness;
  let controller: ModelToolController;
  let previewCb: ((r: PreviewResult) => void) | null;

  beforeEach(() => {
    resetStores();
    __resetRegistryForTests();
    __setExactPreviewTimeoutForTests(0);
    container = document.createElement("div");
    document.body.appendChild(container);
    const grabbed = { value: null as GizmoHit | null };
    const engine = makeEngineMock(grabbed);
    previewCb = null;
    const client = makeClientMock((cb) => {
      previewCb = cb;
    });
    h = { engine, client, grabbed };
    controller = new ModelToolController({
      engine: engine as unknown as ViewportEngine,
      client: client as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
      debug: true,
    });
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    __setExactPreviewTimeoutForTests(4000);
    __resetRegistryForTests();
  });

  const driver = (kind: Kind): Driver => DRIVERS.find((x) => x.kind === kind) as Driver;

  interface PtrInit {
    buttons?: number;
    altKey?: boolean;
    pointerId?: number;
    pointerType?: string;
  }

  function ptr(d: Driver, type: string, t: number, init: PtrInit = {}): void {
    const { x, y } = d.at(t);
    container.dispatchEvent(
      new PointerEvent(type, {
        clientX: x,
        clientY: y,
        button: 0,
        buttons: init.buttons ?? (type === "pointerup" || type === "pointercancel" ? 0 : 1),
        pointerId: init.pointerId ?? 1,
        pointerType: init.pointerType ?? "mouse",
        altKey: init.altKey,
        bubbles: true,
        cancelable: true,
      }),
    );
  }

  function lastUpdate(): { sessionId: string; params: PreviewParams; epoch: number } {
    const calls = h.client.updatePreview.mock.calls;
    const last = calls[calls.length - 1];
    return { sessionId: last[0], params: last[1], epoch: last[2] };
  }

  const chipValue = (): number => toolChipStore.getState().value as number;

  it("fillet: an explicit type flip mid-drag ends the gesture; a later pointercancel restores nothing", async () => {
    const d = driver("fillet");
    await d.arm(h);
    const armed = chipValue();
    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 20);
    expect(chipValue()).toBeCloseTo(armed + 20, 6);

    toolChipStore.getState().onEdgeOp?.("Chamfer");
    await flush();
    expect(debug().edgeOpKind).toBe("Chamfer");
    // The grab basis belonged to the Fillet number line: the flip must not leave
    // a gesture (or a "dragging" phase) that nothing can end.
    expect(debug().filletPhase).toBe("armed");
    expect(toolStore.getState().phase).toBe("armed");

    ptr(d, "pointercancel", 20);
    window.dispatchEvent(new Event("blur"));
    expect(debug().edgeOpKind).toBe("Chamfer");
    expect(chipValue()).toBeCloseTo(armed + 20, 6);
    expect(toolStore.getState().phase).toBe("armed");
  });

  it("restore sends the pre-grab value at once under a fresh epoch; stale drag results cannot re-apply", async () => {
    const d = driver("fillet");
    await d.arm(h);
    const armed = chipValue();
    const first = lastUpdate();
    previewCb?.({ sessionId: first.sessionId, epoch: first.epoch, bodyId: "preview", bodies: [], replacedBodyIds: [] });

    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 20);
    await waitMs(200); // past the drag trailing floor: the dragged value is on the wire
    const dragged = lastUpdate();
    expect(dragged.params.radius).toBeCloseTo(armed + 20, 6);
    ptr(d, "pointermove", 25); // coalesced behind the in-flight drag epoch

    ptr(d, "pointercancel", 25);

    const restored = lastUpdate();
    expect(restored.epoch).toBeGreaterThan(dragged.epoch);
    expect(restored.params.radius).toBeCloseTo(armed, 6);

    // The in-flight answer for the cancelled value lands late — as a failure…
    previewCb?.({
      sessionId: dragged.sessionId,
      epoch: dragged.epoch,
      bodyId: "preview",
      error: { kind: "opFailed", message: "radius too large", structural: false },
    });
    expect(viewportStore.getState().statusHint?.severity).not.toBe("error");
    expect(debug().previewError ?? null).toBeNull();
    // …or as a candidate: neither may claim the scene.
    h.engine.setPreviewReplacedBodyIds.mockClear();
    previewCb?.({ sessionId: dragged.sessionId, epoch: dragged.epoch, bodyId: "preview", bodies: [], replacedBodyIds: ["body1"] });
    expect(h.engine.setPreviewReplacedBodyIds).not.toHaveBeenCalled();

    // The trailing frame for the cancelled drag never goes out.
    await waitMs(260);
    const after = h.client.updatePreview.mock.calls.slice(
      h.client.updatePreview.mock.calls.findIndex((c) => c[2] === restored.epoch),
    );
    for (const call of after) expect((call[1] as PreviewParams).radius).toBeCloseTo(armed, 6);

    // The restore's own answer is applied normally.
    previewCb?.({ sessionId: restored.sessionId, epoch: restored.epoch, bodyId: "preview", bodies: [], replacedBodyIds: ["body1"] });
    expect(h.engine.setPreviewReplacedBodyIds).toHaveBeenCalledWith(["body1"]);
  });

  it("a SECONDARY session's stale answer cannot re-apply the cancelled value either", async () => {
    // Secondary sessions never touch the throttle, so the epoch floor is their only guard.
    const R1: SketchRegion = {
      regionId: "r1",
      outerLoop: [],
      holes: [],
      previewTriangles: { positions: [100, 100, 140, 100, 140, 140, 100, 140], indices: [0, 1, 2, 0, 2, 3] },
    };
    h.client.getSketchRegions.mockImplementation(() => Promise.resolve({ regions: [R0, R1] }));
    h.client.finishSketch.mockImplementation(() => Promise.resolve({ regions: [R0, R1] }));
    documentStore.getState().addSketch({ id: "sk", name: "Sketch", visible: true, dof: 0, status: "ok", geometryToken: "sk:v1" });
    selectionStore.getState().set([{ kind: "sketch", id: "sk" }]);
    toolStore.getState().setTool("revolve");
    await flush();
    await flush();
    const click = (x: number, y: number): void => {
      container.dispatchEvent(new MouseEvent("pointerdown", { clientX: x, clientY: y, button: 0, buttons: 1, bubbles: true }));
      container.dispatchEvent(new MouseEvent("pointerup", { clientX: x, clientY: y, button: 0, buttons: 0, bubbles: true }));
    };
    click(10, 10);
    await flush();
    click(130, 110);
    await flush();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    await flush();
    click(50, 50); // the axis
    await flush();
    await flush();
    const opened = h.client.updatePreview.mock.calls.map((c) => c[0]);
    expect(opened).toEqual(["pv-1", "pv-2"]);
    for (const sessionId of opened) {
      previewCb?.({ sessionId, epoch: 1, bodyId: "preview", bodies: [], replacedBodyIds: [] });
    }

    const d = driver("revolve");
    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 20);
    await waitMs(200); // the dragged angle is on the wire for BOTH sessions
    const dragged = lastUpdate();
    expect(dragged.params.angleDeg).toBe(340);

    ptr(d, "pointercancel", 20);
    expect(lastUpdate().params.angleDeg).toBe(360);
    expect(lastUpdate().epoch).toBeGreaterThan(dragged.epoch);

    previewCb?.({
      sessionId: "pv-2",
      epoch: dragged.epoch,
      bodyId: "preview",
      error: { kind: "opFailed", message: "self-intersecting at 340°", structural: false },
    });
    expect(debug().previewError ?? null).toBeNull();
    expect(viewportStore.getState().statusHint?.severity).not.toBe("error");
  });

  it("restore clears a failure the cancelled value earned", async () => {
    const d = driver("fillet");
    await d.arm(h);
    const first = lastUpdate();
    previewCb?.({ sessionId: first.sessionId, epoch: first.epoch, bodyId: "preview", bodies: [], replacedBodyIds: [] });
    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 40);
    await waitMs(200);
    const dragged = lastUpdate();
    previewCb?.({
      sessionId: dragged.sessionId,
      epoch: dragged.epoch,
      bodyId: "preview",
      error: { kind: "opFailed", message: "radius too large", structural: false },
    });
    expect(viewportStore.getState().statusHint?.severity).toBe("error");

    ptr(d, "pointercancel", 40);

    expect(debug().previewError ?? null).toBeNull();
    expect(viewportStore.getState().statusHint?.severity).not.toBe("error");
  });

  it("extrude: cancel restores the auto boolean mode the drag flipped, and its tint", async () => {
    const d = driver("extrude");
    documentStore.setState({ bodies: { b1: { id: "b1", name: "Body 1", visible: true } } });
    h.engine.probeMaterial.mockImplementation((_o: number[], dir: number[]) =>
      dir[2] < 0 ? { bodyId: "b1", gap: 0, inside: true } : null,
    );
    await d.arm(h);
    const mode = debug().booleanMode;
    const depth = debug().depth as number;
    expect(debug().booleanAuto).toBe(true);

    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", -(depth + 12)); // push through the sketch plane into the body
    expect(h.engine.setPreviewTint).toHaveBeenLastCalledWith("cut");

    ptr(d, "pointercancel", -(depth + 12));

    expect(debug().booleanMode).toBe(mode);
    expect(debug().booleanAuto).toBe(true);
    expect(debug().depth).toBeCloseTo(depth, 6);
    expect(h.engine.setPreviewTint).toHaveBeenLastCalledWith("normal");
    expect(h.engine.setExtrudeDepth).toHaveBeenLastCalledWith(depth, false);
  });

  it("extrude: an Alt symmetric toggle made during the drag is undone by Escape", async () => {
    const d = driver("extrude");
    await d.arm(h);
    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 6);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Alt" }));
    expect(toolChipStore.getState().symmetric).toBe(true);

    document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));

    expect(debug().symmetric).toBe(false);
    expect(toolChipStore.getState().symmetric).toBe(false);
  });

  it("revolve: an armed press that never became a drag is cleared by pointercancel and blur", async () => {
    const d = driver("revolve");
    await d.arm(h);
    const armed = chipValue();

    ptr(d, "pointerdown", 0);
    ptr(d, "pointercancel", 0);
    container.dispatchEvent(new MouseEvent("pointermove", { ...toClient(d.at(20)), buttons: 1, bubbles: true }));
    expect(debug().revolvePhase).toBe("armed");
    expect(chipValue()).toBe(armed);

    ptr(d, "pointerdown", 0);
    window.dispatchEvent(new Event("blur"));
    container.dispatchEvent(new MouseEvent("pointermove", { ...toClient(d.at(20)), buttons: 1, bubbles: true }));
    expect(debug().revolvePhase).toBe("armed");
    expect(chipValue()).toBe(armed);
  });

  it("revolve: a mouse move with the button up never promotes a stale armed press", async () => {
    const d = driver("revolve");
    await d.arm(h);
    const armed = chipValue();
    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 20, { buttons: 0 });
    expect(debug().revolvePhase).toBe("armed");
    expect(toolStore.getState().phase).toBe("armed");
    expect(chipValue()).toBe(armed);
  });

  it("transform: entering Align mid-drag abandons the gesture without stranding 'dragging'", async () => {
    const d = driver("transform");
    await d.arm(h);
    d.beforePress?.(h);
    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 20);
    expect(toolStore.getState().phase).toBe("dragging");

    toolChipStore.getState().onAlign?.(); // hides the gizmo: a teardown of the drag

    expect(debug().transformGrab).toBeNull();
    expect(toolStore.getState().phase).toBe("armed");
    ptr(d, "pointercancel", 20); // nothing left to cancel or restore
    expect(debug().transformTranslate).toEqual([20, 0, 0]);
  });

  it("a capture loss still pending at dispose leaves no timer behind", async () => {
    const d = driver("transform"); // no preview lane, so no trailing timers to account for
    await d.arm(h);
    vi.useFakeTimers();
    try {
      d.beforePress?.(h);
      ptr(d, "pointerdown", 0);
      ptr(d, "pointermove", 20);
      expect(vi.getTimerCount()).toBe(0);
      ptr(d, "lostpointercapture", 20);
      expect(vi.getTimerCount()).toBe(1);
      controller.dispose();
      expect(vi.getTimerCount()).toBe(0);
      expect(toolStore.getState().phase).not.toBe("dragging");
    } finally {
      vi.useRealTimers();
    }
  });

  it("transform: cancel restores the mode, axis, angle and copy flag a grab re-typed", async () => {
    const d = driver("transform");
    await d.arm(h);
    expect(debug().transformMode).toBe("move");
    expect(debug().transformCopy).toBe(false);

    // Grab the Z ring with Alt: the grab alone re-types to Rotate/Z and turns copy on.
    h.grabbed.value = { kind: "ring", axis: "Z" };
    container.dispatchEvent(
      new PointerEvent("pointerdown", { clientX: 100, clientY: 0, button: 0, buttons: 1, pointerId: 1, pointerType: "mouse", altKey: true, bubbles: true }),
    );
    container.dispatchEvent(
      new PointerEvent("pointermove", { clientX: 0, clientY: -100, button: 0, buttons: 1, pointerId: 1, pointerType: "mouse", bubbles: true }),
    );
    expect(debug().transformMode).toBe("rotate");
    expect(debug().transformAngleDeg).toBe(90);
    expect(debug().transformCopy).toBe(true);

    window.dispatchEvent(new Event("blur"));

    expect(debug().transformMode).toBe("move");
    expect(debug().transformAxis).toBe("X");
    expect(debug().transformAngleDeg).toBe(0);
    expect(debug().transformCopy).toBe(false);
    expect(debug().transformGrab).toBeNull();
    expect(h.engine.setTransformGizmoActive).toHaveBeenLastCalledWith(null);
    expect(toolChipStore.getState().transformMode).toBe("move");
    expect(toolChipStore.getState().copy).toBe(false);
  });

  // ── arms and re-arms landing under a live gesture (H1b) ─────────────────────

  const touch = (pointerId: number): PtrInit => ({ pointerType: "touch", pointerId });

  const TRANSFORM_RECORD = { id: "t1", opType: "TransformBody", kind: "boolean", name: "Move", valueText: "" };
  const transformStored = (): Promise<Record<string, unknown>> =>
    Promise.resolve({
      targets: ["body1"],
      translate: [100, 0, 0],
      rotate: { center: [0, 0, 0], axis: [0, 0, 1], angleDeg: 0 },
    });

  it("transform: a re-edit landing mid-drag ends the gesture; the old drag never writes into the new arm", async () => {
    const d = driver("transform");
    await d.arm(h);
    d.beforePress?.(h);
    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 20);
    expect(debug().transformTranslate).toEqual([20, 0, 0]);
    documentStore.setState({ features: [TRANSFORM_RECORD] as never });
    h.client.getOperationParams.mockImplementation(transformStored as never);

    await controller.editTransformFeature("t1");

    expect(debug().transformTranslate).toEqual([100, 0, 0]);
    expect(toolStore.getState().phase).toBe("armed");
    expect(viewportDragActive()).toBe(false);
    ptr(d, "pointermove", 30);
    ptr(d, "pointerup", 30);
    expect(debug().transformTranslate).toEqual([100, 0, 0]);
    expect(chipValue()).toBe(100);

    toolChipStore.getState().onConfirm?.();
    await flush();
    const op = (h.client.applyOperation.mock.calls as unknown as [OperationOp][])[0][0];
    expect((op.params as TransformBodyParams).translate).toEqual([100, 0, 0]);
  });

  it("transform: once a re-edit request supersedes the arm, the live drag stops writing before the record lands", async () => {
    const d = driver("transform");
    await d.arm(h);
    d.beforePress?.(h);
    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 20);
    documentStore.setState({ features: [TRANSFORM_RECORD] as never });
    let land!: () => void;
    h.client.getOperationParams.mockImplementation((() =>
      new Promise((resolve) => {
        land = () => resolve(transformStored());
      })) as never);

    const edit = controller.editTransformFeature("t1");
    ptr(d, "pointermove", 30); // the arm this drag belongs to is already superseded
    expect(chipValue()).toBe(20);
    expect(toolStore.getState().phase).not.toBe("dragging");

    land();
    await edit;
    expect(debug().transformTranslate).toEqual([100, 0, 0]);
  });

  it("offsetFace: a re-edit landing mid-drag ends the gesture, so its own ✓ is not refused as a drag", async () => {
    const d = driver("offsetFace");
    await d.arm(h);
    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 20);
    documentStore.setState({
      features: [{ id: "of1", opType: "OffsetFace", kind: "fillet", name: "Offset", valueText: "7 mm", primaryValue: 7 }] as never,
    });
    h.client.getOperationParams.mockImplementation((() =>
      Promise.resolve({
        faces: [{ primary: { bodyId: "body1", elementId: "el-f2", kind: "face" } }],
        distance: { value: 7 },
        distanceType: "Offset",
        chainTangentFaces: true,
        targetBodyId: "body1",
      })) as never);

    await controller.editOffsetFaceFeature("of1");

    expect(chipValue()).toBe(7);
    expect(toolStore.getState().phase).toBe("armed");
    expect(viewportDragActive()).toBe(false);
    ptr(d, "pointermove", 40);
    expect(chipValue()).toBe(7);
    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();
    expect(h.client.applyEditCommand).toHaveBeenCalledTimes(1);
  });

  it("shell: a re-edit landing mid-drag ends the gesture; the old grab basis never writes the new arm's value", async () => {
    const d = driver("shell");
    await d.arm(h);
    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 20);
    documentStore.setState({
      features: [{ id: "sh1", opType: "Shell", kind: "fillet", name: "Shell", valueText: "5 mm" }] as never,
    });
    h.client.getOperationParams.mockImplementation((() =>
      Promise.resolve({ thickness: { value: 5 }, openFaces: ["el-face-2"], targetBodyId: "body1" })) as never);

    await controller.editShellFeature("sh1");

    expect(chipValue()).toBe(5);
    ptr(d, "pointermove", 30);
    expect(chipValue()).toBe(5);
    ptr(d, "pointerup", 30);
    expect(chipValue()).toBe(5);
    expect(toolStore.getState().phase).toBe("armed");
  });

  it("offsetFace: a grab while a re-arm handshake is in flight is refused, so nothing jumps under the pointer", async () => {
    const d = driver("offsetFace");
    await d.arm(h);
    const armed = chipValue();
    const plain = h.client.prepareOffsetFace.getMockImplementation() as (
      req: PrepareOffsetFaceRequest,
    ) => Promise<PrepareOffsetFaceResult>;
    let land!: () => void;
    h.client.prepareOffsetFace.mockImplementationOnce(
      (req) => new Promise((resolve) => {
        land = () => resolve(plain(req));
      }),
    );

    toolChipStore.getState().onChainTangent?.(false);
    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 10);
    expect(toolStore.getState().phase).not.toBe("dragging");
    expect(viewportDragActive()).toBe(false);
    expect(chipValue()).toBe(armed);

    land();
    await flush();
    await flush();
    await flush();
    expect(debug().offsetChainTangent).toBe(false);
    ptr(d, "pointermove", 30);
    ptr(d, "pointerup", 30);
    expect(chipValue()).toBe(armed);

    // The landed arm grabs as usual.
    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 10);
    expect(chipValue()).toBeCloseTo(armed + 10, 6);
    ptr(d, "pointerup", 10);
  });

  it("offsetFace: a re-arm requested mid-drag ends the drag at its value, and a refused handshake keeps that value", async () => {
    const d = driver("offsetFace");
    await d.arm(h);
    const armed = chipValue();
    h.client.prepareOffsetFace.mockImplementationOnce(() =>
      Promise.resolve({
        snapshotId: 1,
        targetBodyId: "",
        faces: [],
        currentDims: {},
        refusal: { code: "chainMismatch", message: "no", faces: [] },
      }),
    );
    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 20);

    toolChipStore.getState().onChainTangent?.(false);
    await flush();
    await flush();

    expect(chipValue()).toBeCloseTo(armed + 20, 6);
    expect(debug().offsetFacePhase).toBe("armed");
    expect(toolStore.getState().phase).toBe("armed");
    ptr(d, "pointermove", 35);
    ptr(d, "pointerup", 35);
    expect(chipValue()).toBeCloseTo(armed + 20, 6);
  });

  it("extrude: a grab before the arm's preview sessions open is refused", async () => {
    const plain = h.client.beginPreview.getMockImplementation() as (
      draft: PreviewDraft,
    ) => Promise<{ sessionId: string; previewBodyId: string }>;
    let open!: () => void;
    const gate = new Promise<void>((resolve) => {
      open = resolve;
    });
    h.client.beginPreview.mockImplementation(async (draft) => {
      await gate;
      return plain(draft);
    });
    const d = driver("extrude");
    await d.arm(h);
    expect(h.client.beginPreview).toHaveBeenCalled();

    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 10);
    expect(toolStore.getState().phase).not.toBe("dragging");
    expect(h.engine.setExtrudeDepth).not.toHaveBeenCalledWith(20, false);

    open();
    for (let i = 0; i < 4; i++) await flush();
    ptr(d, "pointerup", 10);
    expect(chipValue()).toBe(10);

    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 10);
    expect(chipValue()).toBe(20);
    ptr(d, "pointerup", 10);
  });

  it("revolve: only the pointer that pressed may promote the armed press into a drag", async () => {
    const d = driver("revolve");
    await d.arm(h);
    const armed = chipValue();
    ptr(d, "pointerdown", 0, touch(11));
    ptr(d, "pointermove", 20, touch(12)); // another finger moves
    expect(debug().revolvePhase).toBe("armed");
    expect(chipValue()).toBe(armed);

    ptr(d, "pointermove", 20, touch(11));
    expect(debug().revolvePhase).toBe("dragging");
    expect(chipValue()).toBe(armed - 20);
    ptr(d, "pointerup", 20, touch(11));
  });

  it("restore keeps a refusal the PRE-GRAB value earned until the restore's own answer lands", async () => {
    const d = driver("fillet");
    await d.arm(h);
    const first = lastUpdate();
    previewCb?.({
      sessionId: first.sessionId,
      epoch: first.epoch,
      bodyId: "preview",
      error: { kind: "opFailed", message: "radius too large", structural: false },
    });
    expect(viewportStore.getState().statusHint?.severity).toBe("error");

    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 10);
    ptr(d, "pointercancel", 10);

    const restored = lastUpdate();
    expect(restored.epoch).toBeGreaterThan(first.epoch);
    expect(debug().previewError).toMatchObject({ message: "radius too large" });
    expect(viewportStore.getState().statusHint).toMatchObject({
      severity: "error",
      message: expect.stringContaining("radius too large"),
    });
    expect(toolChipStore.getState().previewLifecycle.status).toBe("invalid");

    previewCb?.({ sessionId: restored.sessionId, epoch: restored.epoch, bodyId: "preview", bodies: [], replacedBodyIds: [] });
    expect(debug().previewError ?? null).toBeNull();
    expect(viewportStore.getState().statusHint?.severity).not.toBe("error");
    expect(toolChipStore.getState().previewLifecycle.status).toBe("valid");
  });

  it("fillet: an explicit type flip — even mid-drag — re-measures the range for the new type", async () => {
    const d = driver("fillet");
    await d.arm(h);
    expect(h.client.analyzeEdgeOpRange).toHaveBeenCalledTimes(1);
    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 20);

    toolChipStore.getState().onEdgeOp?.("Chamfer");
    await flush();

    expect(h.client.analyzeEdgeOpRange).toHaveBeenCalledTimes(2);
    expect(h.client.analyzeEdgeOpRange.mock.calls[1][0]).toMatchObject({ mode: "Chamfer" });
  });

  it("fillet re-edit: Enter and ✓ are refused mid-drag (the edit commit has no phase check of its own)", async () => {
    selectionStore.getState().set([]);
    documentStore.setState({
      features: [{ id: "f1", kind: "fillet", opType: "Fillet", name: "Fillet", valueText: "3 mm" }] as never,
    });
    h.client.getOperationParams.mockImplementation((() =>
      Promise.resolve({
        radius: { value: 3 },
        edgeIds: ["el-edge-5"],
        edges: [{ primary: { bodyId: "body1", elementId: "el-edge-5", kind: "edge" } }],
      })) as never);
    await controller.editEdgeOpFeature("f1");
    await flush();
    const d = driver("fillet");
    ptr(d, "pointerdown", 0);
    ptr(d, "pointermove", 10);
    expect(toolStore.getState().phase).toBe("dragging");

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", cancelable: true }));
    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();
    expect(h.client.applyEditCommand).not.toHaveBeenCalled();

    // Control: the same ✓ commits once the drag is over.
    ptr(d, "pointerup", 10);
    toolChipStore.getState().onConfirm?.();
    await flush();
    await flush();
    expect(h.client.applyEditCommand).toHaveBeenCalledTimes(1);
  });
});

function toClient(p: { x: number; y: number }): { clientX: number; clientY: number; button: number } {
  return { clientX: p.x, clientY: p.y, button: 0 };
}
