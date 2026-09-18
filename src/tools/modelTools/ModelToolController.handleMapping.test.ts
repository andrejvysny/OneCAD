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
  ClassifyResult,
  FinishSketchResult,
  PreviewDraft,
  SketchPlane,
  SketchRegion,
  SketchSession,
} from "@/ipc/types";
import { DragHandle, type DragHandleMode } from "@/viewport/engine/DragHandle";
import { worldPerPixel } from "@/viewport/engine/screenScale";
import {
  projectAxis,
  type FrozenMapping,
  type LinearHandlePath,
  type ValueWitness,
} from "@/tools/preview/handleProjection";
import { toolStore } from "@/stores/toolStore";
import { selectionStore, type EntityRef } from "@/stores/selectionStore";
import { documentStore } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { toolChipStore, MODEL_TOOL_CHIP_ID } from "@/stores/toolChipStore";
import { resetStores } from "@/test/resetStores";
import { makeBoxMesh, encodeMesh1 } from "@/ipc/mockMeshes";
import { parseMeshPayload } from "@/viewport/mesh/parseMeshPayload";
import * as meshRegistry from "@/viewport/mesh/meshRegistry";
import { faceFrame } from "@/tools/preview/alignSolve";

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
    showValueHandle: vi.fn((origin: number[], dir: number[]) => {
      handle.reset();
      handle.setAxis(vec(origin), vec(dir));
      handle.setVisible(true);
    }),
    showValueHandlePath: vi.fn((path: LinearHandlePath, valueMm: number) => {
      handle.reset();
      handle.setValuePath(path, valueMm);
      handle.setVisible(true);
    }),
    setValueHandleValue: vi.fn((valueMm: number) => handle.setValue(valueMm)),
    showScreenValueHandle: vi.fn((origin: number[], valueMm = 0) => {
      handle.reset();
      handle.setScreenProxy(vec(origin), valueMm);
      handle.setVisible(true);
    }),
    showValueWitness: vi.fn((_w: ValueWitness) => {}),
    hideValueWitness: vi.fn(),
    hideValueHandle: vi.fn(() => handle.setVisible(false)),
    valueHandleMapping: vi.fn(() =>
      handle.visible ? handle.computeMapping(state.camera, W, H, anchorScale()) : null,
    ),
    freezeValueHandle: vi.fn((mapping: FrozenMapping | null) => handle.freeze(mapping)),
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
  /** What `classifyElement` answers for the retained wall (H9's cylinder branch). */
  const classifyState: { result: ClassifyResult | null } = { result: null };
  /** What the worker reports as each prepared edge's anchor. `null` = none, so
   *  the attachment falls back to the polyline's arc-length midpoint. */
  const prepareState: { anchor: [number, number, number] | null } = { anchor: EDGE_MID };
  return {
    prepareState,
    classifyState,
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
          anchor: prepareState.anchor ? { worldPoint: prepareState.anchor } : undefined,
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
    getOperationParams: vi.fn(() => Promise.resolve<Record<string, unknown>>({})),
    classifyElement: vi.fn(() => Promise.resolve(classifyState.result)),
  };
}

type Debug = Record<string, unknown>;
const debug = (): Debug => (window as unknown as { __extrudePreview?: Debug }).__extrudePreview ?? {};
const chipValue = (): number => toolChipStore.getState().value;

describe("value-handle mapping is chosen at the grab and owns the gesture", () => {
  let container: HTMLDivElement;
  let controller: ModelToolController;
  let cam: ReturnType<typeof makeCameraEngine>;
  let client: ReturnType<typeof makeClientMock>;

  beforeEach(() => {
    resetStores();
    meshRegistry.disposeAll();
    meshRegistry.__resetRegistryForTests();
    __setExactPreviewTimeoutForTests(0);
    container = document.createElement("div");
    document.body.appendChild(container);
    cam = makeCameraEngine();
    client = makeClientMock();
    controller = new ModelToolController({
      engine: cam.engine as unknown as ViewportEngine,
      client: client as unknown as CadClient,
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

    expect(cam.engine.freezeValueHandle).toHaveBeenCalledWith(expect.objectContaining({ kind: "proxy" }));
    expect(Number.isFinite(debug().depth as number)).toBe(true);
    expect(debug().depth).toBeCloseTo(start + 20 * scale, 9);
    expect(viewportStore.getState().statusHint?.message).toBe("Depth: drag vertically");

    pointer("pointerup", 400, 280);
    expect(cam.engine.freezeValueHandle).toHaveBeenLastCalledWith(null);
    expect(viewportStore.getState().statusHint?.message).toBe(armHint);
  });

  it("a near-parallel perspective ray is finite: the ill-conditioned axis maps as the proxy", async () => {
    cam.state.camera = perspective([13, 10, 400], [10, 10, 10], [0, 1, 0]);
    await armExtrude();
    const start = debug().depth as number;
    const scale = cam.anchorScale();

    pointer("pointerdown", 400, 300);
    pointer("pointermove", 400, 280);

    expect(cam.engine.freezeValueHandle).toHaveBeenCalledWith(expect.objectContaining({ kind: "proxy" }));
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
    expect(cam.engine.valueHandleMapping()?.kind).toBe("world"); // …what a FRESH grab would take
    cam.render();
    expect(cam.handle.mapping()?.kind).toBe("proxy");

    pointer("pointermove", 400, 280);
    expect(debug().depth).toBeCloseTo(start + 20 * scale, 9);
    expect(cam.engine.freezeValueHandle).not.toHaveBeenCalledWith(expect.objectContaining({ kind: "world" }));
  });

  it.each(["release", "cancel", "abandon"] as const)("%s unfreezes the handle strategy", async (ending) => {
    await armExtrude();
    pointer("pointerdown", 400, 300);
    pointer("pointermove", 400, 280);
    expect(cam.engine.freezeValueHandle).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "proxy" }));

    if (ending === "release") pointer("pointerup", 400, 280);
    else if (ending === "cancel") container.dispatchEvent(new MouseEvent("pointercancel", { bubbles: true }));
    else toolStore.getState().setTool("select");

    expect(toolStore.getState().gestureLive).toBe(false);
    expect(cam.engine.freezeValueHandle).toHaveBeenLastCalledWith(null);
  });

  // ── edge op (H8: β = 0, the exact inverse, one representative edge) ──────────

  function registerBox(): void {
    meshRegistry.swap("body1", meshRegistry.buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body1", 1));
  }

  /** Oblique perspective: the raw and the tangent-rejected screen axes differ by ~15.7°. */
  const OBLIQUE = (): THREE.Camera => perspective([120, -260, 40], [0, 0, 0], [0, 0, 1]);

  type WorldMapping = Extract<FrozenMapping, { kind: "world" }>;

  /** The witness the controller last published (no `Array.at` under this lib target). */
  const lastWitness = (): ValueWitness => {
    const calls = cam.engine.showValueWitness.mock.calls;
    return calls[calls.length - 1][0];
  };

  async function armObliqueFillet(): Promise<WorldMapping> {
    cam.state.camera = OBLIQUE();
    registerBox();
    await arm("fillet", [EDGE]);
    expect(debug().edgeOpAxisSource).toBe("bisector");
    cam.render();
    const mapping = cam.handle.mapping();
    expect(mapping?.kind).toBe("world");
    return mapping as WorldMapping;
  }

  /*
   * β = 0 (docs/design/astra/modeling-handle-attachment.md §3). The glyph draws,
   * and the drag maps against, the TRUE projected derivative of the point that
   * moves. Rejecting the projected edge tangent produced a direction H(q) cannot
   * follow: full rejection at a 1° screen angle multiplies the gain by 57.2987.
   */
  it("a single-edge fillet maps along the RAW projected outward axis, tangent and all", async () => {
    const mapping = await armObliqueFillet();
    expect(cam.engine.showValueHandle).not.toHaveBeenCalled();
    const [path] = cam.engine.showValueHandlePath.mock.calls[0];
    expect(path.point0Mm).toEqual(EDGE_MID);

    const viewProj = new THREE.Matrix4().multiplyMatrices(
      cam.state.camera.projectionMatrix,
      cam.state.camera.matrixWorldInverse,
    );
    const raw = projectAxis(viewProj.elements, EDGE_MID, EDGE_OUTWARD, W, H)!;
    expect(mapping.direction[0]).toBeCloseTo(raw.direction[0], 9);
    expect(mapping.direction[1]).toBeCloseTo(raw.direction[1], 9);

    // The case only means something if the retired rejection WOULD have moved it.
    const tangent = projectAxis(viewProj.elements, EDGE_MID, [1, 0, 0], W, H)!;
    const dot = raw.derivative[0] * tangent.direction[0] + raw.derivative[1] * tangent.direction[1];
    const rejected = [raw.derivative[0] - dot * tangent.direction[0], raw.derivative[1] - dot * tangent.direction[1]];
    const rLen = Math.hypot(rejected[0], rejected[1]);
    const degrees =
      (Math.acos(raw.direction[0] * (rejected[0] / rLen) + raw.direction[1] * (rejected[1] / rLen)) * 180) / Math.PI;
    expect(degrees).toBeGreaterThanOrEqual(10);
  });

  /*
   * The EXACT perspective inverse. `p / g` is only the tangent of the real
   * mapping at p = 0; under perspective the handle's own depth changes as it
   * moves, and the frozen `k` is what accounts for it.
   */
  it("inverts the frozen mapping exactly: Δq = p / (g − k·p), not p / g", async () => {
    const mapping = await armObliqueFillet();
    expect(mapping.kPerMm).not.toBe(0);
    const p = 30;
    const exact = 2 + p / (mapping.g0PxPerMm - mapping.kPerMm * p);
    const constantGain = 2 + p / mapping.g0PxPerMm;
    expect(Math.abs(exact - constantGain)).toBeGreaterThan(0.1); // the two really differ here

    pointer("pointerdown", 400, 300);
    pointer("pointermove", 400 + p * mapping.direction[0], 300 + p * mapping.direction[1]);

    expect(chipValue()).toBeCloseTo(exact, 6);
  });

  /*
   * THE HANDLE MOVES (§5). H(q) = E + q·b, so growing the radius has to carry
   * the arrow with it — before H8 it stayed pinned at the arm point forever.
   */
  it("moves the arrow to H(q) as the radius changes, and the witness with it", async () => {
    const mapping = await armObliqueFillet();
    // The arm seats it at H(2), the default radius — already off the edge.
    const armed = cam.handle.worldAnchor();
    for (let i = 0; i < 3; i++) expect(armed.getComponent(i)).toBeCloseTo(EDGE_MID[i] + 2 * EDGE_OUTWARD[i], 9);

    const p = 30;
    pointer("pointerdown", 400, 300);
    pointer("pointermove", 400 + p * mapping.direction[0], 300 + p * mapping.direction[1]);
    const radius = chipValue();
    expect(radius).toBeGreaterThan(2);

    const at = cam.handle.worldAnchor();
    for (let i = 0; i < 3; i++) {
      expect(at.getComponent(i)).toBeCloseTo(EDGE_MID[i] + radius * EDGE_OUTWARD[i], 6);
    }
    const witness = lastWitness();
    expect(witness.fromMm).toEqual(EDGE_MID);
    expect(witness.toMm[1]).toBeCloseTo(EDGE_MID[1] + radius * EDGE_OUTWARD[1], 6);
  });

  it("labels the witness a PARAMETER construction, never a measured blend", async () => {
    await armObliqueFillet();
    const witness = lastWitness();
    expect(witness.meaning).toBe("parameterConstruction");
    expect(witness.label).toBe("Radius parameter");

    toolChipStore.getState().onEdgeOp?.("Chamfer");
    await flush();
    const chamfer = lastWitness();
    expect(chamfer.label).toBe("Chamfer distance parameter");
  });

  it("a Chamfer GROWS along the displayed arrow (D-S3), exactly like a Fillet", async () => {
    await armObliqueFillet();
    toolChipStore.getState().onEdgeOp?.("Chamfer");
    await flush();
    expect(debug().edgeOpKind).toBe("Chamfer");
    const start = chipValue();
    // The flip reseeds the size, which MOVES the handle: re-read the mapping at
    // where it actually sits now.
    cam.render();
    const mapping = cam.handle.mapping() as WorldMapping;
    expect(mapping.kind).toBe("world");

    const p = 20;
    pointer("pointerdown", 400, 300);
    pointer("pointermove", 400 + p * mapping.direction[0], 300 + p * mapping.direction[1]);

    expect(chipValue()).toBeCloseTo(start + p / (mapping.g0PxPerMm - mapping.kPerMm * p), 6);
  });

  it("an edge-op drag past the floor rebases: the first reversing move increases the size", async () => {
    const mapping = await armObliqueFillet();
    pointer("pointerdown", 400, 300);
    pointer("pointermove", 400 - 200 * mapping.direction[0], 300 - 200 * mapping.direction[1]);
    expect(chipValue()).toBe(0.1);

    pointer("pointermove", 400 - 199 * mapping.direction[0], 300 - 199 * mapping.direction[1]);
    expect(chipValue()).toBeGreaterThan(0.1);
  });

  /*
   * ONE REPRESENTATIVE EDGE (§5, "Four-edge tangent chain"). Averaging a closed
   * chain's outward directions cancels them, so the whole arm used to degrade to
   * a screen proxy; averaging its midpoints puts the anchor where no edge runs.
   */
  it("a four-edge tangent chain attaches to ONE edge, never to the chain's mean", async () => {
    const R = 20;
    const ring = (from: number): [number, number, number][] =>
      [0, 45, 90].map((d) => {
        const a = ((from + d) * Math.PI) / 180;
        return [R * Math.cos(a), R * Math.sin(a), 20];
      });
    // A flat square patch at z = 20 with a quarter-circle rim edge per quadrant:
    // the bbox is symmetric, so the T2 tier resolves outward radially.
    meshRegistry.swap(
      "body1",
      meshRegistry.buildBodyObjects(
        parseMeshPayload(
          encodeMesh1({
            positions: [-R, -R, 20, R, -R, 20, R, R, 20, -R, R, 20],
            faces: [{ triangles: [[0, 1, 2], [0, 2, 3]], id: "f:0" }],
            edges: [0, 90, 180, 270].map((from, i) => ({ id: `e:${i}`, points: ring(from) })),
          }),
        ),
        "body1",
        1,
      ),
    );
    cam.state.camera = perspective([0, 0, 300], [0, 0, 20], [0, 1, 0]);
    client.prepareState.anchor = null;
    // Four picks, no pick anchors: each edge's E is its own arc-length midpoint.
    await arm(
      "fillet",
      [0, 1, 2, 3].map((i) => ({
        kind: "edge" as const,
        id: `body1#e:${i}`,
        bodyId: "body1",
        topoKey: `e:${i}`,
        elementId: `el-edge-${i}`,
      })),
    );

    // Not degraded — the mean of the four outward directions would have cancelled.
    expect(debug().edgeOpAxisSource).not.toBe("screen");
    const [path] = cam.engine.showValueHandlePath.mock.calls[0];
    expect(path.point0Mm[0]).toBeCloseTo(14.142136, 5);
    expect(path.point0Mm[1]).toBeCloseTo(14.142136, 5);
    expect(path.point0Mm[2]).toBeCloseTo(20, 9);

    // §6: q = 2 mm puts the handle at (15.5563, 15.5563, 20).
    const at = cam.handle.worldAnchor();
    expect(at.x).toBeCloseTo(15.556349, 5);
    expect(at.y).toBeCloseTo(15.556349, 5);

    const witness = lastWitness();
    expect(witness.label).toBe("Radius parameter · shared by 4 edges");
  });

  /*
   * A CONCAVE edge (§7). The parameter construction is the same segment and the
   * same label: the blend midpoint's displacement reverses sign with convexity,
   * so no claim about it may be made from what the frontend has.
   */
  it("keeps the parameter label on a concave edge and makes no blend claim", async () => {
    registerBox();
    cam.state.camera = OBLIQUE();
    // The box's own edges are convex; the point is that the label is a property
    // of the CONSTRUCTION, not of a convexity test the frontend never ran.
    await arm("fillet", [EDGE]);
    const witness = lastWitness();
    expect(witness.meaning).toBe("parameterConstruction");
    // The handle travels +q·b — the true convex blend midpoint moves the OTHER
    // way by −(√2−1)·q·b, which is exactly why this is not labelled a contact.
    const travel = [0, 1, 2].map((i) => witness.toMm[i] - witness.fromMm[i]);
    const along = travel.reduce((a, v, i) => a + v * EDGE_OUTWARD[i], 0);
    expect(along).toBeCloseTo(2, 9);
  });

  /*
   * THE VIEW LIMIT (§2.9, §3). Exhausting the frozen valid interval HOLDS the
   * value and says why; it never switches to another mapping mid-gesture, and
   * the first reversing sample moves the value again.
   */
  it("holds at the frozen view limit, says so, and rebases on the way back", async () => {
    const mapping = await armObliqueFillet();
    const armHint = viewportStore.getState().statusHint?.message;
    // The interval's px limit, from the mapping's own coefficients.
    const [lo] = mapping.validDeltaMm;
    expect(Number.isFinite(lo)).toBe(true);
    const pLimit = (mapping.g0PxPerMm * lo) / (1 + mapping.kPerMm * lo);

    pointer("pointerdown", 400, 300);
    const move = (p: number): void =>
      pointer("pointermove", 400 + p * mapping.direction[0], 300 + p * mapping.direction[1]);

    move(pLimit * 1.5); // well past the limit
    expect(viewportStore.getState().statusHint?.message).toBe(
      "View limits this drag; release and reposition the view.",
    );
    const held = chipValue();

    move(pLimit * 3); // further out still: the value HOLDS
    expect(chipValue()).toBe(held);

    move(0); // back inside: the hint clears and the value tracks again
    expect(viewportStore.getState().statusHint?.message).toBe(armHint);
    expect(chipValue()).not.toBe(held);
  });


  /*
   * The §8 DIAGNOSTICS, and the bottom two rungs of the fallback ladder. An
   * end-on world axis is still an honest attachment — it just cannot be dragged
   * along, so the gesture says which way it CAN be dragged instead.
   */
  it("says why an end-on edge axis drags vertically instead", async () => {
    registerBox();
    // Look down the edge's own outward direction, tilted 2° off: the projection
    // exists but Cq ≈ 0.035, well under τ = 0.08.
    const tilt = (2 * Math.PI) / 180;
    const dir: [number, number, number] = [
      Math.sin(tilt),
      Math.cos(tilt) * EDGE_OUTWARD[1],
      Math.cos(tilt) * EDGE_OUTWARD[2],
    ];
    cam.state.camera = perspective(
      [EDGE_MID[0] + 250 * dir[0], EDGE_MID[1] + 250 * dir[1], EDGE_MID[2] + 250 * dir[2]],
      EDGE_MID,
      [0, 0, 1],
    );
    await arm("fillet", [EDGE]);
    expect(debug().edgeOpAxisSource).toBe("bisector"); // a real attachment, not a degraded arm

    pointer("pointerdown", 400, 300);
    expect(viewportStore.getState().statusHint?.message).toBe("Radius: drag vertically — axis nearly end-on.");
    const scale = cam.anchorScale();
    pointer("pointermove", 400, 290); // 10 px UP
    expect(chipValue()).toBeCloseTo(2 + 10 * scale, 9);
  });

  it("with no usable depth the drag is inert and the value must be typed", async () => {
    registerBox();
    cam.state.camera = OBLIQUE();
    await arm("fillet", [EDGE]);
    cam.engine.valueHandleMapping.mockReturnValue({ kind: "disabled", q0Mm: 2, reason: "noScale" });

    pointer("pointerdown", 400, 300);
    expect(viewportStore.getState().statusHint?.message).toBe(
      "Radius: type a value — handle attachment unavailable.",
    );
    pointer("pointermove", 400, 100);
    expect(chipValue()).toBe(2); // no fabricated gain
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
    // No mesh registered, so no rim neighbour resolves: the picked anchor proxy.
    await arm("shell", [FACE]);
    expect(debug().shellAttachment).toBe("pick");
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

  // ── shell: the retained-wall attachment (H9, §5 "Shell") ────────────────────

  /** A 40 mm cube spanning [0,40]³ — the derivation's open box. */
  function registerOpenBox(): void {
    meshRegistry.swap(
      "body1",
      meshRegistry.buildBodyObjects(parseMeshPayload(makeBoxMesh(40, 40, 40, 0, [20, 20, 20])), "body1", 1),
    );
  }

  /** The lid: f:4 is the +Z face, picked nearer its +X rim than any other. */
  const LID: EntityRef = {
    kind: "face",
    id: "body1#f:4",
    bodyId: "body1",
    topoKey: "f:4",
    anchor: { worldPoint: [30, 20, 40] },
  };

  /**
   * Derivation camera B: orthographic, eye `100(1,1,1)/√3`, right `(-1,1,0)/√2`,
   * up `(-1,-1,2)/√6`, view height 100 mm.
   */
  function cameraB(width: number, height: number): THREE.Camera {
    const halfH = 50;
    const e = 100 / Math.sqrt(3);
    const camera = new THREE.OrthographicCamera(-(halfH * width) / height, (halfH * width) / height, halfH, -halfH, 0.1, 2000);
    camera.up.set(-1 / Math.sqrt(6), -1 / Math.sqrt(6), 2 / Math.sqrt(6));
    camera.position.set(e, e, e);
    camera.lookAt(0, 0, 0);
    return finish(camera);
  }

  /** Derivation camera O: orthographic down −Z, view height 100 mm. */
  function cameraO(width: number, height: number): THREE.Camera {
    const halfH = 50;
    const camera = new THREE.OrthographicCamera(-(halfH * width) / height, (halfH * width) / height, halfH, -halfH, 0.1, 2000);
    camera.up.set(0, 1, 0);
    camera.position.set(0, 0, 100);
    camera.lookAt(0, 0, 0);
    return finish(camera);
  }

  const viewProjOf = (camera: THREE.Camera): number[] =>
    new THREE.Matrix4().multiplyMatrices(camera.projectionMatrix, camera.matrixWorldInverse).elements;

  /**
   * A faceted tube: a CURVED retained side wall `f:0` and the cap `f:1` the shell
   * removes, sharing the top rim edge `e:0`. `faceFrame` refuses the side wall
   * (15° per facet), so the attachment can only come from `classifyElement`.
   */
  function registerTube(radius: number, segments = 24): void {
    const positions: number[] = [];
    const normals: number[] = [];
    const angle = (i: number): number => (i / segments) * Math.PI * 2;
    const side: [number, number, number][] = [];
    for (let i = 0; i <= segments; i++) {
      const c = Math.cos(angle(i));
      const s = Math.sin(angle(i));
      positions.push(radius * c, radius * s, 0, radius * c, radius * s, 40);
      normals.push(c, s, 0, c, s, 0);
    }
    for (let i = 0; i < segments; i++) {
      side.push([i * 2, (i + 1) * 2, (i + 1) * 2 + 1], [i * 2, (i + 1) * 2 + 1, i * 2 + 1]);
    }
    const capBase = positions.length / 3;
    positions.push(0, 0, 40);
    normals.push(0, 0, 1);
    const rim: [number, number, number][] = [];
    for (let i = 0; i <= segments; i++) {
      const p: [number, number, number] = [radius * Math.cos(angle(i)), radius * Math.sin(angle(i)), 40];
      positions.push(...p);
      normals.push(0, 0, 1);
      rim.push(p);
    }
    const cap: [number, number, number][] = [];
    for (let i = 0; i < segments; i++) cap.push([capBase, capBase + 1 + i, capBase + 2 + i]);
    meshRegistry.swap(
      "body1",
      meshRegistry.buildBodyObjects(
        parseMeshPayload(
          encodeMesh1({
            positions,
            normals,
            faces: [
              { triangles: side, id: "f:0" },
              { triangles: cap, id: "f:1" },
            ],
            edges: [{ id: "e:0", points: rim }],
          }),
        ),
        "body1",
        1,
      ),
    );
  }

  /**
   * The removed cap, picked ON its rim: `edgePolylinePoint` then returns that
   * polyline VERTEX, so the representative point is the analytic one rather than
   * a chord interior the faceting chose.
   */
  const tubeCap = (radius: number): EntityRef => ({
    kind: "face",
    id: "body1#f:1",
    bodyId: "body1",
    topoKey: "f:1",
    anchor: { worldPoint: [radius, 0, 40] },
  });

  const cylinderFrame = (radius: number, sidedness?: "pin" | "hole"): ClassifyResult => ({
    kind: "face",
    surfaceType: "cylinder",
    curveType: "",
    frame: { origin: [0, 0, 0], normal: null, axis: [0, 0, 1], radius, ...(sidedness ? { sidedness } : {}) },
  });

  /** The chip anchor the controller last published to the overlay driver. */
  const lastChipAnchor = (): { world: number[]; axisFrom?: number[] } => {
    const calls = cam.engine.moveChip.mock.calls;
    const last = calls[calls.length - 1];
    expect(last[0]).toBe(MODEL_TOOL_CHIP_ID);
    return { world: last[1], axisFrom: last[2] };
  };

  /*
   * R07. The armed shell used to show a vertical proxy at
   * `faces[0]?.anchor?.worldPoint ?? [0,0,0]` — the WORLD ORIGIN on a re-edit —
   * and the removed lid's own normal describes the cavity's depth, not the wall's
   * thickness. The open box has enough evidence for a real attachment: a retained
   * side wall, its rim point, and its outward normal.
   */
  it("attaches the thickness to a retained SIDE wall, not to the removed lid", async () => {
    registerOpenBox();
    cam.state.camera = cameraB(W, H);
    await arm("shell", [LID]);

    expect(debug().shellAttachment).toBe("wall");
    const [path, seeded] = cam.engine.showValueHandlePath.mock.calls[0];
    expect(seeded).toBe(2);
    // The rim point nearest the pick, on the +X wall: E = (40, 20, 40), n = +X.
    expect(path.point0Mm).toEqual([40, 20, 40]);
    expect(path.dPointDValue).toEqual([-1, 0, 0]);
    // NOT the floor (its inward direction is +Z) and NOT the lid (−Z).
    expect(path.dPointDValue[2]).toBe(0);

    // §6: t = 2 mm puts the handle at (38, 20, 40).
    const at = cam.handle.worldAnchor();
    expect([at.x, at.y, at.z]).toEqual([38, 20, 40]);
  });

  /*
   * §6 "Open-box Shell", camera B at 1000 × 1000 px: the drawn construction
   * displaces (14.1421, −8.16497) px over t = 2 mm, a 16.3299 px segment.
   */
  it("reproduces the derivation's camera-B screen displacement exactly", () => {
    const projected = projectAxis(viewProjOf(cameraB(1000, 1000)), [40, 20, 40], [-1, 0, 0], 1000, 1000)!;
    expect(projected.derivative[0] * 2).toBeCloseTo(14.142136, 5);
    expect(projected.derivative[1] * 2).toBeCloseTo(-8.164966, 5);
    expect(projected.pxPerWorld * 2).toBeCloseTo(16.329932, 5);
  });

  it("labels the segment a TARGET construction and never a measured reference", async () => {
    registerOpenBox();
    cam.state.camera = cameraB(W, H);
    await arm("shell", [LID]);
    const witness = lastWitness();
    expect(witness.meaning).toBe("targetConstruction");
    expect(witness.meaning).not.toBe("measuredReference");
    expect(witness.label).toBe("Thickness target t — construction");
    expect(witness.fromMm).toEqual([40, 20, 40]);
    expect(witness.toMm).toEqual([38, 20, 40]);
  });

  it("says the drag measures a construction when it grabs a real wall", async () => {
    registerOpenBox();
    cam.state.camera = cameraB(W, H);
    await arm("shell", [LID]);
    pointer("pointerdown", 400, 300);
    expect(viewportStore.getState().statusHint?.message).toBe("Thickness target — construction.");
  });

  /*
   * AMBIGUITY IS REFUSED (§7). Two candidate walls 0.05 mm apart both pass the
   * 0.069282 mm adjacency test on a 40 mm body at fine LOD, so the geometric
   * attachment is dropped rather than resolved by picking one.
   */
  it("falls back with the diagnostic when two walls match the rim inside tolerance", async () => {
    const quad = (base: number): [number, number, number][] => [
      [base, base + 1, base + 2],
      [base, base + 2, base + 3],
    ];
    const lid = [0, 0, 40, 40, 0, 40, 40, 40, 40, 0, 40, 40];
    const wall = [40, 0, 40, 40, 40, 40, 40, 40, 0, 40, 0, 0];
    const impostor = wall.map((v, i) => (i % 3 === 0 ? v + 0.05 : v));
    const floor = [0, 0, 0, 40, 0, 0, 40, 40, 0, 0, 40, 0];
    meshRegistry.swap(
      "body1",
      meshRegistry.buildBodyObjects(
        parseMeshPayload(
          encodeMesh1({
            positions: [...lid, ...wall, ...impostor, ...floor],
            faces: [0, 4, 8, 12].map((base, i) => ({ triangles: quad(base), id: `f:${i}` })),
            edges: [{ id: "e:0", points: [[40, 0, 40], [40, 40, 40]] }],
            lod: 2,
          }),
        ),
        "body1",
        1,
      ),
    );
    cam.state.camera = cameraB(W, H);
    await arm("shell", [{ kind: "face", id: "body1#f:0", bodyId: "body1", topoKey: "f:0", anchor: { worldPoint: [30, 20, 40] } }]);

    expect(debug().shellAttachment).not.toBe("wall");
    expect(cam.engine.showValueHandlePath).not.toHaveBeenCalled();
    expect(cam.engine.showScreenValueHandle).toHaveBeenCalled();
    pointer("pointerdown", 400, 300);
    expect(viewportStore.getState().statusHint?.message).toBe(
      "Thickness: drag vertically or type — wall attachment unavailable.",
    );
  });

  /*
   * §6 "Curved Shell wall": cylinder R = 10, axis Z, E = (10,0,40), σ = +1,
   * t = 2 ⇒ H = (8,0,40) and 20 px inward under camera O.
   */
  it("attaches to a CLASSIFIED cylindrical wall and offsets it by σ", async () => {
    registerTube(10);
    client.classifyState.result = cylinderFrame(10, "pin");
    cam.state.camera = cameraO(W, H);
    await arm("shell", [tubeCap(10)]);
    await flush();

    expect(debug().shellAttachment).toBe("wall");
    const [path] = cam.engine.showValueHandlePath.mock.calls[0];
    expect(path.point0Mm[0]).toBeCloseTo(10, 9);
    expect(path.point0Mm[2]).toBeCloseTo(40, 9);
    expect(path.dPointDValue[0]).toBeCloseTo(-1, 9);
    const at = cam.handle.worldAnchor();
    expect(at.x).toBeCloseTo(8, 9);
    expect(at.z).toBeCloseTo(40, 9);

    const projected = projectAxis(viewProjOf(cameraO(1000, 1000)), [10, 0, 40], [-1, 0, 0], 1000, 1000)!;
    expect(projected.derivative[0] * 2).toBeCloseTo(-20, 6);
    expect(projected.derivative[1] * 2).toBeCloseTo(0, 6);
  });

  /*
   * ABSENT SIDEDNESS IS NOT A SIDE (types.ts: "absent means not measured"). σ
   * decides which way the wall's inner surface runs, so guessing it would run
   * the drag backwards on half the models it guessed wrong about.
   */
  it("falls back rather than assuming σ when sidedness was not measured", async () => {
    registerTube(10);
    client.classifyState.result = cylinderFrame(10);
    cam.state.camera = cameraO(W, H);
    await arm("shell", [tubeCap(10)]);
    await flush();

    expect(debug().shellAttachment).not.toBe("wall");
    expect(cam.engine.showValueHandlePath).not.toHaveBeenCalled();
    pointer("pointerdown", 400, 300);
    expect(viewportStore.getState().statusHint?.message).toBe(
      "Thickness: drag vertically or type — wall attachment unavailable.",
    );
  });

  /*
   * A NON-POSITIVE predicted radius cannot be depicted as an inner wall (§5):
   * a 1 mm pin shelled 2 mm has consumed itself. The labelled parameter control
   * stays and the existing preview/domain validation refuses the operation.
   */
  it("draws no inner wall when the predicted radius is not positive", async () => {
    registerTube(1);
    client.classifyState.result = cylinderFrame(1, "pin");
    cam.state.camera = cameraO(W, H);
    await arm("shell", [tubeCap(1)]);
    await flush();

    expect(cam.engine.showValueHandlePath).not.toHaveBeenCalled();
    expect(cam.engine.showScreenValueHandle).toHaveBeenCalled();
    expect(cam.engine.showValueWitness).not.toHaveBeenCalled();
    expect(chipValue()).toBe(2); // the number is still there to type against
  });

  /*
   * THE RE-EDIT (R07). It arms with no picked faces at all, and the origin is
   * never the answer: the stored open faces re-resolve the same wall, and when
   * they cannot the anchor is the BODY's own, not (0,0,0).
   */
  it("re-resolves the retained wall from the stored open faces", async () => {
    registerOpenBox();
    cam.state.camera = cameraB(W, H);
    documentStore.setState({
      features: [{ id: "sh-1", kind: "fillet", label: "Shell", valueText: "2.0 mm", status: "ok", opType: "Shell" }],
    });
    client.getOperationParams.mockResolvedValue({ targetBodyId: "body1", openFaces: ["f:4"], thickness: 2 });
    await controller.editShellFeature("sh-1");
    await flush();
    await flush();

    expect(debug().shellAttachment).toBe("wall");
    const [path] = cam.engine.showValueHandlePath.mock.calls[0];
    expect(path.point0Mm).toEqual([40, 20, 40]);
  });

  it("anchors a re-edit with no predecessor evidence on the BODY, never the origin", async () => {
    registerOpenBox();
    cam.state.camera = cameraB(W, H);
    documentStore.setState({
      features: [{ id: "sh-1", kind: "fillet", label: "Shell", valueText: "2.0 mm", status: "ok", opType: "Shell" }],
    });
    client.getOperationParams.mockResolvedValue({ targetBodyId: "body1", openFaces: ["el_gone"], thickness: 2 });
    await controller.editShellFeature("sh-1");
    await flush();
    await flush();

    expect(debug().shellAttachment).toBe("body");
    const [origin] = cam.engine.showScreenValueHandle.mock.calls[0];
    expect(origin).toEqual([20, 20, 20]); // the body's own centre, not [0,0,0]
    pointer("pointerdown", 400, 300);
    expect(viewportStore.getState().statusHint?.message).toBe(
      "Thickness: drag vertically or type — wall attachment unavailable.",
    );
  });

  /*
   * The bottom of the ladder (§3 "No usable depth"). With no body on screen and
   * no pick there is nothing to attach to, and the honest answer is to DISABLE
   * the drag — not to draw an affordance at the world origin, which is what the
   * old `?? [0,0,0]` did.
   */
  it("disables the drag rather than drawing a handle at the origin", async () => {
    cam.state.camera = cameraB(W, H);
    await arm("shell", [{ kind: "face", id: "ghost#f:0", bodyId: "ghost", topoKey: "f:0" }]);

    expect(debug().shellAttachment).toBe("none");
    expect(cam.engine.showValueHandlePath).not.toHaveBeenCalled();
    expect(cam.engine.showScreenValueHandle).not.toHaveBeenCalled();
    expect(cam.engine.hideValueHandle).toHaveBeenCalled();
  });

  // ── the label rides the handle (H9 item 6) ──────────────────────────────────

  /*
   * The chip anchored at `E + 1·b` — one millimetre along the construction —
   * while the handle rides `H(q)`, so the label trailed the arrow for every
   * value but exactly 1 mm. Both now read the same point.
   */
  it("anchors the edge-op label at H(q), and keeps it there as the value moves", async () => {
    const mapping = await armObliqueFillet();
    const armed = lastChipAnchor();
    for (let i = 0; i < 3; i++) {
      expect(armed.world[i]).toBeCloseTo(EDGE_MID[i] + 2 * EDGE_OUTWARD[i], 9);
    }
    expect(armed.axisFrom).toEqual(EDGE_MID);

    const p = 30;
    pointer("pointerdown", 400, 300);
    pointer("pointermove", 400 + p * mapping.direction[0], 300 + p * mapping.direction[1]);
    const radius = chipValue();
    expect(radius).toBeGreaterThan(2);

    const moved = lastChipAnchor();
    for (let i = 0; i < 3; i++) {
      expect(moved.world[i]).toBeCloseTo(EDGE_MID[i] + radius * EDGE_OUTWARD[i], 6);
    }
  });

  it("anchors the shell label at H(t), and keeps it there as the thickness moves", async () => {
    registerOpenBox();
    cam.state.camera = cameraB(W, H);
    await arm("shell", [LID]);
    expect(lastChipAnchor().world).toEqual([38, 20, 40]);

    toolChipStore.getState().onValue?.(5);
    expect(chipValue()).toBe(5);
    expect(lastChipAnchor().world).toEqual([35, 20, 40]);
  });

  /*
   * H10 — THE TWO REMAINING `[0,0,0]` ATTACHMENTS (audit findings N2/N3).
   *
   * A Fillet/Chamfer RE-EDIT published its chip at the world origin and installed
   * no value handle at all, while its own hint said "drag or type". An OffsetFace
   * re-edit did the same. Both now walk the §5 ladder the fresh arms walk, and
   * the hint tells the truth on every rung — including the bottom one, where the
   * honest answer is that there is nothing to drag.
   *
   * A fillet CONSUMES its edge, so the geometric rung fails often. That is fine;
   * what must not happen is a control drawn on a point the model has no
   * relationship to.
   */

  /**
   * The same box, but its EDGE id table carries minted ElementIds (MESH1
   * `IDS_HAVE_ELEMENTIDS`, mesh_format.md §2) — the only publication shape in
   * which a re-edit's stored refs, which carry no snapshot TopoKey, can name an
   * edge at all.
   */
  function registerBoxWithEdgeElementIds(): void {
    const view = parseMeshPayload(makeBoxMesh());
    const ids = Array.from({ length: view.edgeCount }, (_, i) => `el-edge-${i}`);
    const encoder = new TextEncoder();
    const offsets = new Uint32Array(view.edgeCount + 1);
    let at = 0;
    ids.forEach((id, i) => {
      offsets[i] = at;
      at += encoder.encode(id).length;
    });
    offsets[view.edgeCount] = at;
    const patched = {
      ...view,
      idsHaveElementIds: true,
      edgeIdChars: encoder.encode(ids.join("")),
      edgeIdOffsets: offsets,
    };
    meshRegistry.swap("body1", meshRegistry.buildBodyObjects(patched, "body1", 1));
  }

  /** A box centred at (20,20,20), so its own centre is NOT the world origin. */
  function registerOffsetBox(): void {
    meshRegistry.swap(
      "body1",
      meshRegistry.buildBodyObjects(
        parseMeshPayload(makeBoxMesh(40, 40, 40, 0, [20, 20, 20])),
        "body1",
        1,
      ),
    );
  }

  const FILLET_ROW = [
    { id: "fi-1", kind: "fillet" as const, opType: "Fillet", label: "Fillet", valueText: "2.0 mm", status: "ok" as const },
  ];
  const storedEdge = (elementId: string): Record<string, unknown> => ({
    radius: { value: 2 },
    edgeIds: [elementId],
    edges: [{ primary: { bodyId: "body1", elementId, kind: "edge" } }],
  });

  it("re-resolves the edge op's representative edge from the record's own refs", async () => {
    registerBoxWithEdgeElementIds();
    cam.state.camera = OBLIQUE();
    documentStore.setState({ features: FILLET_ROW });
    client.getOperationParams.mockResolvedValue(storedEdge("el-edge-0"));
    await controller.editEdgeOpFeature("fi-1", "Fillet");
    await flush();
    await flush();

    expect(debug().edgeOpAttachment).toBe("edge");
    const [path] = cam.engine.showValueHandlePath.mock.calls[0];
    expect(path.point0Mm).toEqual(EDGE_MID);
    for (let i = 0; i < 3; i++) expect(path.dPointDValue[i]).toBeCloseTo(EDGE_OUTWARD[i], 9);
    // The chip rides H(q), not the origin.
    expect(lastChipAnchor().world[1]).toBeCloseTo(EDGE_MID[1] + 2 * EDGE_OUTWARD[1], 6);
    expect(viewportStore.getState().statusHint?.message).toBe(
      "Edit fillet radius — drag or type, Enter to apply",
    );
  });

  it("anchors a re-edit whose edge is GONE on the body, never the origin", async () => {
    // The ordinary case: the fillet consumed the edge it was built on, so the
    // record's ElementId names nothing in today's publication.
    registerOffsetBox();
    cam.state.camera = OBLIQUE();
    documentStore.setState({ features: FILLET_ROW });
    client.getOperationParams.mockResolvedValue(storedEdge("el-edge-gone"));
    await controller.editEdgeOpFeature("fi-1", "Fillet");
    await flush();
    await flush();

    expect(debug().edgeOpAttachment).toBe("body");
    expect(cam.engine.showValueHandlePath).not.toHaveBeenCalled();
    const [origin] = cam.engine.showScreenValueHandle.mock.calls[0];
    expect(origin).toEqual([20, 20, 20]); // the body's own centre, not [0,0,0]
    expect(viewportStore.getState().statusHint?.message).toBe(
      "Edit fillet radius — drag vertically or type, Enter to apply",
    );
  });

  it("disables the edge-op re-edit's drag when nothing attributable is left", async () => {
    cam.state.camera = OBLIQUE(); // no mesh registered at all
    documentStore.setState({ features: FILLET_ROW });
    client.getOperationParams.mockResolvedValue(storedEdge("el-edge-gone"));
    await controller.editEdgeOpFeature("fi-1", "Fillet");
    await flush();
    await flush();

    expect(debug().edgeOpAttachment).toBe("none");
    expect(cam.engine.showValueHandlePath).not.toHaveBeenCalled();
    expect(cam.engine.showScreenValueHandle).not.toHaveBeenCalled();
    expect(cam.engine.hideValueHandle).toHaveBeenCalled();
    // …and the hint stops promising a drag that does not exist.
    expect(viewportStore.getState().statusHint?.message).toBe(
      "Edit fillet radius — type a value, Enter to apply",
    );
  });

  it("a re-edited CHAIN still says how many edges share the one value", async () => {
    // `filletEdges` is empty on a re-edit, so a count taken from the live picks
    // labelled a four-edge chain as a single edge. The count comes from whatever
    // the attachment was resolved from — here, the record's own edges.
    registerBoxWithEdgeElementIds();
    cam.state.camera = OBLIQUE();
    documentStore.setState({ features: FILLET_ROW });
    client.getOperationParams.mockResolvedValue({
      radius: { value: 2 },
      edgeIds: ["el-edge-0", "el-edge-1", "el-edge-2", "el-edge-3"],
      edges: ["el-edge-0", "el-edge-1", "el-edge-2", "el-edge-3"].map((elementId) => ({
        primary: { bodyId: "body1", elementId, kind: "edge" },
      })),
    });
    await controller.editEdgeOpFeature("fi-1", "Fillet");
    await flush();
    await flush();

    expect(debug().edgeOpAttachment).toBe("edge");
    expect(lastWitness().label).toContain("shared by 4 edges");
  });

  it("anchors an OffsetFace re-edit on the record's body, never the origin", async () => {
    registerOffsetBox();
    cam.state.camera = OBLIQUE();
    documentStore.setState({
      features: [
        { id: "of-1", kind: "fillet", opType: "OffsetFace", label: "Offset face", valueText: "2.0 mm", status: "ok" },
      ],
    });
    client.getOperationParams.mockResolvedValue({
      targetBodyId: "body1",
      distance: { value: 2 },
      distanceType: "Offset",
      faces: [{ primary: { bodyId: "body1", elementId: "el-face-gone", kind: "face" } }],
    });
    await controller.editOffsetFaceFeature("of-1");
    await flush();
    await flush();

    expect(debug().offsetAttachment).toBe("body");
    const [origin] = cam.engine.showScreenValueHandle.mock.calls[0];
    expect(origin).toEqual([20, 20, 20]);
    expect(viewportStore.getState().statusHint?.message).toBe(
      "Edit offset distance — drag visible distance control or type, Enter to apply",
    );
  });

  it("an OffsetFace re-edit whose face SURVIVES gets a real path from the record", async () => {
    // Face id table carrying ElementIds: the record's own frozen ref names a face
    // in today's publication, so the re-edit attaches to it instead of degrading.
    const view = parseMeshPayload(makeBoxMesh(40, 40, 40, 0, [20, 20, 20]));
    const ids = Array.from({ length: view.faceCount }, (_, i) => `el-face-${i}`);
    const encoder = new TextEncoder();
    const offsets = new Uint32Array(view.faceCount + 1);
    let at = 0;
    ids.forEach((id, i) => {
      offsets[i] = at;
      at += encoder.encode(id).length;
    });
    offsets[view.faceCount] = at;
    const patched = {
      ...view,
      idsHaveElementIds: true,
      faceIdChars: encoder.encode(ids.join("")),
      faceIdOffsets: offsets,
    };
    meshRegistry.swap("body1", meshRegistry.buildBodyObjects(patched, "body1", 1));
    const expected = faceFrame(patched, 2)!;

    cam.state.camera = OBLIQUE();
    documentStore.setState({
      features: [
        { id: "of-1", kind: "fillet", opType: "OffsetFace", label: "Offset face", valueText: "2.0 mm", status: "ok" },
      ],
    });
    client.getOperationParams.mockResolvedValue({
      targetBodyId: "body1",
      distance: { value: 2 },
      distanceType: "Offset",
      faces: [{ primary: { bodyId: "body1", elementId: "el-face-2", kind: "face" } }],
    });
    await controller.editOffsetFaceFeature("of-1");
    await flush();
    await flush();

    expect(debug().offsetAttachment).toBe("plane");
    const [path] = cam.engine.showValueHandlePath.mock.calls[0];
    expect(path.point0Mm).toEqual(expected.center);
    for (let i = 0; i < 3; i++) expect(path.dPointDValue[i]).toBeCloseTo(expected.normal[i], 9);
    expect(viewportStore.getState().statusHint?.message).toBe(
      "Edit offset distance — drag the arrow or type, Enter to apply",
    );
  });

});
