/*
 * ONE MAPPING PER GESTURE (jsdom, real camera) — TODO.md SESSION 37 H3 / H5.
 *
 * The drag arrow's glyph is drawn from `handleProjection` (axis or vertical screen
 * proxy). These specs pin that the GESTURE maps the pointer the same way, chosen
 * once at the grab:
 *
 *  - R03: an extrude axis pointing at the camera is dragged as a vertical proxy at
 *    the anchor's own scale, never through a degenerate ray intersection;
 *  - the strategy and the metric are frozen for the whole gesture and released by
 *    every ending (release, cancel, abandon);
 *  - R06 / N1: an edge op drags along the tangent-REJECTED axis the glyph draws,
 *    at `p / pxPerWorld`, and both Fillet and Chamfer grow along the arrow with a
 *    rebasing floor;
 *  - the degraded proxies (fillet, shell) keep the scale they grabbed with.
 *
 * The engine double composes the real `DragHandle`, a real THREE camera and the
 * real `worldPerPixel`, so `valueHandleMapping` is the production classifier.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import * as THREE from "three";
import { ModelToolController, __setExactPreviewTimeoutForTests } from "./ModelToolController";
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
import { DragHandle, type DragHandleMode } from "@/viewport/engine/DragHandle";
import { worldPerPixel } from "@/viewport/engine/screenScale";
import { projectAxis, type HandleStrategy } from "@/tools/preview/handleProjection";
import { toolStore } from "@/stores/toolStore";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { toolChipStore } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";
import { makeBoxMesh } from "@/ipc/mockMeshes";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import * as meshRegistry from "@/viewport/mesh/meshRegistry";

const W = 800;
const H = 600;

const PLANE: SketchPlane = { kind: "XY", origin: [0, 0, 0], xAxis: [1, 0, 0], yAxis: [0, 1, 0], normal: [0, 0, 1] };

/** Centroid (10, 10, 0): the armed head sits at (10, 10, 10). */
const R0: SketchRegion = {
  regionId: "r0",
  outerLoop: [],
  holes: [],
  previewTriangles: { positions: [0, 0, 20, 0, 20, 20, 0, 20], indices: [0, 1, 2, 0, 2, 3] },
};

/** `e:0` of the mock box: mid (0,−30,−15), tangent +X, bisector (0,−√½,−√½). */
const EDGE: EntityRef = {
  kind: "edge",
  id: "body1#e:0",
  bodyId: "body1",
  topoKey: "e:0",
  elementId: "el-edge-0",
  anchor: { worldPoint: [0, -30, -15] },
};
const EDGE_MID: [number, number, number] = [0, -30, -15];
const EDGE_OUTWARD: [number, number, number] = [0, -Math.SQRT1_2, -Math.SQRT1_2];

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

function finish(camera: THREE.Camera): THREE.Camera {
  camera.updateMatrixWorld();
  (camera as THREE.PerspectiveCamera).updateProjectionMatrix();
  return camera;
}

/** Straight down −Z onto the XY plane: every Z-normal axis is end-on. 0.5 world/px. */
function topOrtho(): THREE.Camera {
  const camera = new THREE.OrthographicCamera(-200, 200, 150, -150, 0.1, 2000);
  camera.up.set(0, 1, 0);
  camera.position.set(10, 10, 500);
  camera.lookAt(10, 10, 0);
  return finish(camera);
}

function perspective(position: [number, number, number], target: [number, number, number], up: [number, number, number]): THREE.Camera {
  const camera = new THREE.PerspectiveCamera(60, W / H, 0.1, 5000);
  camera.up.set(...up);
  camera.position.set(...position);
  camera.lookAt(...target);
  return finish(camera);
}

const vec = (a: readonly number[]): THREE.Vector3 => new THREE.Vector3(a[0], a[1], a[2]);

function makeCameraEngine() {
  const root = new THREE.Group();
  const handle = new DragHandle({ root, invalidate: () => {} });
  const state = { camera: topOrtho(), planePixelWorld: 1 };
  const anchorScale = (): number => worldPerPixel(state.camera, handle.worldAnchor(), H);
  const engine = {
    showRegionPick: vi.fn(),
    setRegionHover: vi.fn(),
    setRegionSelected: vi.fn(),
    hideRegionPick: vi.fn(),
    screenToPlaneOn: vi.fn(),
    setOrbitSuppressed: vi.fn(),
    moveChip: vi.fn(),
    probeMaterial: vi.fn(() => null),
    planePixelWorld: vi.fn(() => state.planePixelWorld),
    setExtrudeDepth: vi.fn(),
    setPreviewTint: vi.fn(),
    setExtrudeHandleHover: vi.fn(),
    hitExtrudeHandle: vi.fn(() => true),
    isExtrudePreviewVisible: vi.fn(() => true),
    hideExtrudePreview: vi.fn(),
    setPreviewBody: vi.fn(),
    setPreviewReplacedBodyIds: vi.fn(),
    clearPreviewBody: vi.fn(),
    hideRevolvePreview: vi.fn(),
    hideGhostPreview: vi.fn(),
    showGhostPreviewMulti: vi.fn(),
    probePick: vi.fn(() => null),
    showExtrudePreviews: vi.fn((_p: SketchPlane, _profiles: unknown, centroid: number[], normal: number[]) => {
      handle.setAnchor(vec(centroid), vec(normal));
      handle.setVisible(true);
    }),
    setExtrudeHandle: vi.fn((origin: number[], dir: number[], mode: DragHandleMode) => {
      handle.setAxis(vec(origin), vec(dir), mode);
    }),
    showValueHandle: vi.fn((origin: number[], dir: number[], tangent?: number[]) => {
      handle.reset();
      handle.setAxis(vec(origin), vec(dir), "forward", tangent ? vec(tangent) : null);
      handle.setVisible(true);
    }),
    showScreenValueHandle: vi.fn((origin: number[]) => {
      handle.reset();
      handle.setScreenProxy(vec(origin));
      handle.setVisible(true);
    }),
    hideValueHandle: vi.fn(() => handle.setVisible(false)),
    valueHandleMapping: vi.fn(() =>
      handle.visible ? handle.computeMapping(state.camera, W, H, anchorScale()) : null,
    ),
    freezeValueHandleStrategy: vi.fn((strategy: HandleStrategy | null) => handle.freezeStrategy(strategy)),
    screenRay: vi.fn((x: number, y: number) => {
      const caster = new THREE.Raycaster();
      caster.setFromCamera(new THREE.Vector2((x / W) * 2 - 1, -(y / H) * 2 + 1), state.camera);
      return { origin: caster.ray.origin.toArray(), dir: caster.ray.direction.toArray() };
    }),
    projectPoint: vi.fn((world: number[]) => {
      const v = vec(world).project(state.camera);
      return { x: ((v.x + 1) / 2) * W, y: ((1 - v.y) / 2) * H };
    }),
  };
  return {
    engine,
    handle,
    state,
    anchorScale,
    /** One rendered frame: the glyph orients exactly as the engine's loop does. */
    render: () => handle.orient(state.camera, W, H, anchorScale()),
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
    getSketch: vi.fn(() =>
      Promise.resolve<SketchSession>({ sketchId: "sk", plane: PLANE, entities: [], constraints: [], dof: 0, status: "FullyConstrained" }),
    ),
    prepareEdgeOp: vi.fn((req: { pickedEdges: { topoKey?: string; elementId?: string }[] }) =>
      Promise.resolve({
        snapshotId: 1,
        targetBodyId: "body_body1",
        edges: req.pickedEdges.map((pick, i) => ({
          topoKey: pick.topoKey ?? `e:${i}`,
          elementId: pick.elementId ?? `el-${i}`,
          bodyId: "body_body1",
          kind: "edge" as const,
          picked: true,
          anchor: { worldPoint: EDGE_MID },
        })),
        refusal: null,
      }),
    ),
    analyzeEdgeOpRange: vi.fn(() => Promise.reject(new Error("no range in this lane"))),
    promoteSelection: vi.fn((bodyId: string, picks: { topoKey: string }[]) =>
      Promise.resolve(picks.map((p) => ({ topoKey: p.topoKey, elementId: `el-${p.topoKey}`, kind: "face", bodyId }))),
    ),
    beginPreview: vi.fn((_d: PreviewDraft) => Promise.resolve({ sessionId: `pv-${++seq}`, previewBodyId: `pb-${seq}` })),
    updatePreview: vi.fn(),
    endPreview: vi.fn(() => Promise.resolve(ok())),
    applyOperation: vi.fn(() => Promise.resolve(ok())),
    applyEditCommand: vi.fn(() => Promise.resolve(ok())),
    getOperationParams: vi.fn(() => Promise.resolve({})),
  };
}

type Debug = Record<string, unknown>;
const debug = (): Debug => (window as unknown as { __extrudePreview?: Debug }).__extrudePreview ?? {};
const chipValue = (): number => toolChipStore.getState().value;

describe("value-handle mapping is chosen at the grab and owns the gesture", () => {
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let cam: ReturnType<typeof makeCameraEngine>;

  beforeEach(() => {
    resetStores();
    meshRegistry.disposeAll();
    meshRegistry.__resetRegistryForTests();
    __setExactPreviewTimeoutForTests(0);
    container = document.createElement("div");
    document.body.appendChild(container);
    cam = makeCameraEngine();
    controller = new ModelToolController({
      engine: cam.engine as unknown as ViewportEngine,
      client: makeClientMock() as unknown as CadClient,
      container,
      onBodyLoaded: () => () => {},
      debug: true,
    });
  });

  afterEach(() => {
    controller.dispose();
    container.remove();
    meshRegistry.disposeAll();
    meshRegistry.__resetRegistryForTests();
    __setExactPreviewTimeoutForTests(4000);
  });

  function pointer(type: string, x: number, y: number): void {
    container.dispatchEvent(new MouseEvent(type, { clientX: x, clientY: y, button: 0, buttons: type === "pointerup" ? 0 : 1, bubbles: true }));
  }

  async function armExtrude(): Promise<void> {
    documentStore.getState().addSketch({ id: "sk", name: "Sketch", visible: true, dof: 0, status: "ok", geometryToken: "sk:v1" });
    selectionStore.getState().set([{ kind: "sketchRegion", id: "r0-ref", sketchId: "sk", regionId: "r0" }]);
    toolStore.getState().setTool("extrude");
    await flush();
    await flush();
    expect(debug().phase).toBe("armed");
  }

  async function arm(tool: "fillet" | "shell", refs: EntityRef[]): Promise<void> {
    selectionStore.getState().set(refs);
    toolStore.getState().setTool(tool);
    await flush();
    await flush();
  }

  // ── extrude (R03) ────────────────────────────────────────────────────────────

  it("an end-on axis under an orthographic top view drags as a vertical proxy: Δdepth = dy · worldPerPx", async () => {
    await armExtrude();
    const armHint = viewportStore.getState().statusHint?.message;
    const start = debug().depth as number;
    const scale = cam.anchorScale();
    expect(scale).toBeCloseTo(0.5, 12);

    pointer("pointerdown", 400, 300);
    pointer("pointermove", 400, 280); // 20 px UP

    expect(cam.engine.freezeValueHandleStrategy).toHaveBeenCalledWith("screenProxy");
    expect(Number.isFinite(debug().depth as number)).toBe(true);
    expect(debug().depth).toBeCloseTo(start + 20 * scale, 9);
    expect(viewportStore.getState().statusHint?.message).toBe("Depth: drag vertically");

    pointer("pointerup", 400, 280);
    expect(cam.engine.freezeValueHandleStrategy).toHaveBeenLastCalledWith(null);
    expect(viewportStore.getState().statusHint?.message).toBe(armHint);
  });

  it("a near-parallel perspective ray is finite: the ill-conditioned axis maps as the proxy", async () => {
    cam.state.camera = perspective([13, 10, 400], [10, 10, 10], [0, 1, 0]);
    await armExtrude();
    const start = debug().depth as number;
    const scale = cam.anchorScale();

    pointer("pointerdown", 400, 300);
    pointer("pointermove", 400, 280);

    expect(cam.engine.freezeValueHandleStrategy).toHaveBeenCalledWith("screenProxy");
    expect(Number.isFinite(debug().depth as number)).toBe(true);
    expect(debug().depth).toBeCloseTo(start + 20 * scale, 9);
  });

  it("the strategy and its metric survive a mid-drag camera change (glyph included)", async () => {
    await armExtrude();
    const start = debug().depth as number;
    const scale = cam.anchorScale();
    pointer("pointerdown", 400, 300);
    pointer("pointermove", 400, 290);

    cam.state.camera = perspective([300, -300, 100], [10, 10, 10], [0, 0, 1]); // side-on: a well-conditioned axis
    expect(cam.engine.valueHandleMapping()?.strategy).toBe("axis"); // …what a FRESH grab would take
    cam.render();
    expect(cam.handle.mapping()?.strategy).toBe("screenProxy");

    pointer("pointermove", 400, 280);
    expect(debug().depth).toBeCloseTo(start + 20 * scale, 9);
    expect(cam.engine.freezeValueHandleStrategy).not.toHaveBeenCalledWith("axis");
  });

  it.each(["release", "cancel", "abandon"] as const)("%s unfreezes the handle strategy", async (ending) => {
    await armExtrude();
    pointer("pointerdown", 400, 300);
    pointer("pointermove", 400, 280);
    expect(cam.engine.freezeValueHandleStrategy).toHaveBeenLastCalledWith("screenProxy");

    if (ending === "release") pointer("pointerup", 400, 280);
    else if (ending === "cancel") container.dispatchEvent(new MouseEvent("pointercancel", { bubbles: true }));
    else toolStore.getState().setTool("select");

    expect(toolStore.getState().gestureLive).toBe(false);
    expect(cam.engine.freezeValueHandleStrategy).toHaveBeenLastCalledWith(null);
  });

  // ── edge op (R06 axis, N1) ───────────────────────────────────────────────────

  function registerBox(): void {
    meshRegistry.swap("body1", meshRegistry.buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body1", 1));
  }

  /** Oblique perspective where the raw and the tangent-rejected screen axes differ by ~15.7°. */
  const OBLIQUE = (): THREE.Camera => perspective([120, -260, 40], [0, 0, 0], [0, 0, 1]);

  async function armObliqueFillet(): Promise<{ direction: readonly [number, number]; pxPerWorld: number }> {
    cam.state.camera = OBLIQUE();
    registerBox();
    await arm("fillet", [EDGE]);
    expect(debug().edgeOpAxisSource).toBe("bisector");
    cam.render();
    const mapping = cam.handle.mapping();
    expect(mapping?.strategy).toBe("axis");
    return { direction: mapping!.direction, pxPerWorld: mapping!.pxPerWorld! };
  }

  it("a single-edge fillet drags along the glyph's tangent-rejected axis at p / pxPerWorld", async () => {
    const { direction, pxPerWorld } = await armObliqueFillet();
    const [, , tangent] = cam.engine.showValueHandle.mock.calls[0];
    expect(tangent).toEqual([1, 0, 0]);

    const viewProj = new THREE.Matrix4().multiplyMatrices(cam.state.camera.projectionMatrix, cam.state.camera.matrixWorldInverse);
    const raw = projectAxis(viewProj.elements, EDGE_MID, EDGE_OUTWARD, W, H)!;
    const degrees = (Math.acos(raw.direction[0] * direction[0] + raw.direction[1] * direction[1]) * 180) / Math.PI;
    expect(degrees).toBeGreaterThanOrEqual(10);

    pointer("pointerdown", 400, 300);
    pointer("pointermove", 400 + 30 * direction[0], 300 + 30 * direction[1]);

    expect(chipValue()).toBeCloseTo(2 + 30 / pxPerWorld, 6);
  });

  it("a Chamfer GROWS along the displayed arrow (D-S3), exactly like a Fillet", async () => {
    const { direction, pxPerWorld } = await armObliqueFillet();
    toolChipStore.getState().onEdgeOp?.("Chamfer");
    await flush();
    expect(debug().edgeOpKind).toBe("Chamfer");
    const start = chipValue();

    pointer("pointerdown", 400, 300);
    pointer("pointermove", 400 + 20 * direction[0], 300 + 20 * direction[1]);

    expect(chipValue()).toBeCloseTo(start + 20 / pxPerWorld, 6);
  });

  it("an edge-op drag past the floor rebases: the first reversing move increases the size", async () => {
    const { direction, pxPerWorld } = await armObliqueFillet();
    pointer("pointerdown", 400, 300);
    pointer("pointermove", 400 - 200 * direction[0], 300 - 200 * direction[1]);
    expect(chipValue()).toBe(0.1);

    pointer("pointermove", 400 - 199 * direction[0], 300 - 199 * direction[1]);
    expect(chipValue()).toBeCloseTo(0.1 + 1 / pxPerWorld, 6);
  });

  // ── degraded proxies keep the scale they grabbed with ───────────────────────

  it("a degraded fillet proxy keeps the anchor scale it grabbed with, whatever the orbit metric does", async () => {
    cam.state.camera = perspective([0, -300, 200], [0, 0, 0], [0, 0, 1]);
    cam.state.planePixelWorld = 7; // the orbit-distance metric the proxy used to read per move
    await arm("fillet", [{ ...EDGE, id: "body1#e:9", topoKey: "e:9" }]); // no mesh ⇒ degraded
    expect(debug().edgeOpAxisSource).toBe("screen");
    const scale = cam.anchorScale();

    pointer("pointerdown", 400, 300);
    pointer("pointermove", 400, 290);
    expect(chipValue()).toBeCloseTo(2 + 10 * scale, 9);

    cam.state.camera = perspective([0, -30, 20], [0, 0, 0], [0, 0, 1]); // zoomed right in
    cam.state.planePixelWorld = 3;
    pointer("pointermove", 400, 280);
    expect(chipValue()).toBeCloseTo(2 + 20 * scale, 9);
  });

  it("a shell proxy keeps its grab scale and rebases at the floor", async () => {
    cam.state.camera = perspective([0, -300, 200], [0, 0, 0], [0, 0, 1]);
    cam.state.planePixelWorld = 7;
    await arm("shell", [FACE]);
    const scale = cam.anchorScale();

    pointer("pointerdown", 400, 300);
    pointer("pointermove", 400, 290);
    expect(chipValue()).toBeCloseTo(2 + 10 * scale, 9);

    cam.state.planePixelWorld = 3;
    pointer("pointermove", 400, 300 + 400); // far below the floor
    expect(chipValue()).toBe(0.1);
    pointer("pointermove", 400, 300 + 399);
    expect(chipValue()).toBeCloseTo(0.1 + scale, 9);
  });
});
