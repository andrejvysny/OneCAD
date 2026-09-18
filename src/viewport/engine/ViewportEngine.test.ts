/*
 * Engine init/dispose + render-on-demand smoke test.
 *
 * jsdom has no real WebGL, so the renderer is mocked and rAF is driven manually.
 * This verifies lifecycle (StrictMode-safe idempotent init/dispose) and the
 * on-demand contract (a frame renders only when dirty; idle renders nothing).
 * The actual GPU output is only verifiable in-browser (see README.md).
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const mocks = vi.hoisted(() => {
  const renderer = {
    domElement: null as unknown as HTMLCanvasElement,
    render: vi.fn(),
    setSize: vi.fn(),
    setPixelRatio: vi.fn(),
    setClearColor: vi.fn(),
    dispose: vi.fn(),
  };
  const handleDispose = vi.fn();
  const createRenderer = vi.fn(async () => ({
    renderer,
    isWebGPU: false,
    dispose: handleDispose,
  }));
  return { renderer, handleDispose, createRenderer };
});

vi.mock("./renderer", () => ({ createRenderer: mocks.createRenderer }));

import { ViewportEngine } from "./ViewportEngine";
import * as THREE from "three";
import { makeBoxMesh } from "@/ipc/mockMeshes";
import {
  buildBodyObjects,
  swap,
  disposeAll,
  __resetRegistryForTests,
} from "../mesh/meshRegistry";
import { buildBodyObject } from "./BodyObject";
import { BodyMaterialLibrary } from "./bodyMaterials";
import { parseMeshPayload } from "../mesh/parseMeshPayload";
import type { ProbeCandidateFilter } from "./Picker";
import type { CadOrbitControls, FrameViewRequest } from "./CadOrbitControls";
import type { DragHandle } from "./DragHandle";

let rafCbs: FrameRequestCallback[] = [];
function flushFrame(t = 16): void {
  const cbs = rafCbs;
  rafCbs = [];
  for (const cb of cbs) cb(t);
}

beforeEach(() => {
  rafCbs = [];
  mocks.createRenderer.mockClear();
  mocks.handleDispose.mockClear();
  mocks.renderer.render.mockClear();
  mocks.renderer.dispose.mockClear();
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
    rafCbs.push(cb);
    return rafCbs.length;
  });
  vi.stubGlobal("cancelAnimationFrame", () => {});
});

afterEach(() => vi.unstubAllGlobals());

function newDom() {
  return {
    canvas: document.createElement("div"), // engine creates its own <canvas>
    overlay: document.createElement("div"),
  };
}

it("passes overlap probes through without mutating picker state", () => {
  const engine = new ViewportEngine();
  const filter: ProbeCandidateFilter = { kinds: ["body"], includeBodyIds: ["body-a"] };
  const probeCandidates = vi.fn(() => []);
  (engine as unknown as { picker: { probeCandidates: typeof probeCandidates } }).picker = {
    probeCandidates,
  };
  expect(engine.probeCandidates(12, 34, filter)).toEqual([]);
  expect(probeCandidates).toHaveBeenCalledWith(12, 34, filter);
});

describe("ViewportEngine lifecycle", () => {
  it("initializes with the mocked renderer and renders on demand", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});

    expect(mocks.createRenderer).toHaveBeenCalledTimes(1);
    expect(engine.frameCount).toBe(0); // nothing rendered until a frame runs
    expect(rafCbs.length).toBe(1); // a frame is scheduled (dirty)

    flushFrame();
    expect(engine.frameCount).toBe(1);
    expect(mocks.renderer.render).toHaveBeenCalled();

    // Idle: nothing scheduled, nothing renders.
    expect(rafCbs.length).toBe(0);
    flushFrame();
    expect(engine.frameCount).toBe(1);

    // invalidate() re-schedules exactly one frame.
    engine.invalidate();
    expect(rafCbs.length).toBe(1);
    flushFrame();
    expect(engine.frameCount).toBe(2);

    engine.dispose();
  });

  it("reports completion only after an actual renderer submission", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    const completed = vi.fn();
    const unsubscribe = engine.onAfterRender(completed);

    expect(completed).not.toHaveBeenCalled();
    flushFrame();
    expect(mocks.renderer.render).toHaveBeenCalledTimes(1);
    expect(completed).toHaveBeenCalledTimes(1);

    unsubscribe();
    engine.invalidate();
    flushFrame();
    expect(completed).toHaveBeenCalledTimes(1);
    engine.dispose();
  });

  it("waits for asynchronous WebGPU-style render completion", async () => {
    let complete!: () => void;
    mocks.renderer.render.mockImplementationOnce(
      () => new Promise<void>((resolve) => { complete = resolve; }),
    );
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    const completed = vi.fn();
    engine.onAfterRender(completed);

    flushFrame();
    expect(completed).not.toHaveBeenCalled();
    complete();
    await Promise.resolve();
    expect(completed).toHaveBeenCalledTimes(1);
    engine.dispose();
  });

  it("dispose() releases the renderer and is idempotent", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});

    engine.dispose();
    expect(mocks.handleDispose).toHaveBeenCalledTimes(1);
    engine.dispose();
    expect(mocks.handleDispose).toHaveBeenCalledTimes(1);
  });

  it("dispose() racing an in-flight init still releases the renderer", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    const pending = engine.init(canvas, overlay, {});
    engine.dispose(); // before init resolves
    await pending;
    expect(mocks.handleDispose).toHaveBeenCalledTimes(1);
    expect(rafCbs.length).toBe(0); // disposed → nothing scheduled
  });

  it("setProjection swaps the camera and re-renders", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    flushFrame();
    const before = engine.frameCount;

    engine.setProjection("ortho");
    expect(rafCbs.length).toBe(1);
    flushFrame();
    expect(engine.frameCount).toBe(before + 1);

    engine.dispose();
  });

  it("planePixelWorld() scales with orthographic zoom (snap threshold sizing)", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    engine.setProjection("ortho");

    const ortho = (engine as any).rig.ortho as THREE.OrthographicCamera;
    ortho.zoom = 1;
    ortho.updateProjectionMatrix();
    const atZoom1 = engine.planePixelWorld();

    // Zooming IN (zoom > 1) must shrink world-units-per-pixel — a stale value
    // here inflates the sketch snap threshold and starves grid snap in favor
    // of whatever geometry now falls inside the oversized radius.
    ortho.zoom = 2;
    ortho.updateProjectionMatrix();
    const atZoom2 = engine.planePixelWorld();

    expect(atZoom2).toBeCloseTo(atZoom1 / 2, 6);

    engine.dispose();
  });

  it("renders every split preview body and restores every hidden head body", () => {
    const engine = new ViewportEngine();
    const target = new THREE.Group();
    target.userData.bodyId = "target";
    const untouched = new THREE.Group();
    untouched.userData.bodyId = "untouched";
    engine.bodiesRoot.add(target, untouched);
    const first = buildBodyObjects(parseMeshPayload(makeBoxMesh()), "preview-a", 1);
    const second = buildBodyObjects(parseMeshPayload(makeBoxMesh()), "preview-b", 2);

    engine.setPreviewBody(first);
    engine.setPreviewBody(second);
    engine.setPreviewReplacedBodyIds(["target"]);
    expect(engine.previewRoot.children).toHaveLength(2);
    expect(target.visible).toBe(false);
    expect(untouched.visible).toBe(true);

    engine.clearPreviewBody();
    expect(engine.previewRoot.children).toHaveLength(0);
    expect(target.visible).toBe(true);
    expect(untouched.visible).toBe(true);
    first.dispose();
    second.dispose();
    engine.dispose();
  });

  it("hides a fully deleted Cut target even when the candidate has no bodies", () => {
    const engine = new ViewportEngine();
    const target = new THREE.Group();
    target.userData.bodyId = "target";
    engine.bodiesRoot.add(target);

    engine.setPreviewReplacedBodyIds(["target"]);
    expect(engine.previewRoot.children).toHaveLength(0);
    expect(target.visible).toBe(false);
    engine.clearPreviewBody();
    expect(target.visible).toBe(true);
    engine.dispose();
  });

  it("reports whether a preview is holding committed bodies hidden (isolate guard)", () => {
    const engine = new ViewportEngine();
    const target = new THREE.Group();
    target.userData.bodyId = "target";
    engine.bodiesRoot.add(target);

    expect(engine.hasPreviewHiddenBodies()).toBe(false);
    engine.setPreviewReplacedBodyIds(["target"]);
    expect(engine.hasPreviewHiddenBodies()).toBe(true);
    engine.clearPreviewBody();
    expect(engine.hasPreviewHiddenBodies()).toBe(false);
    engine.dispose();
  });
});

/*
 * Sketch-root split (audit item #9 — the recorded §10.5 deferral) and the debug
 * origin pill (item #16). Both hang off the one thing the engine already knows:
 * whether a sketch session is open.
 */
describe("ViewportEngine sketch roots", () => {
  const PLANE = {
    kind: "custom" as const,
    origin: [0, 0, 0] as [number, number, number],
    xAxis: [1, 0, 0] as [number, number, number],
    yAxis: [0, 1, 0] as [number, number, number],
    normal: [0, 0, 1] as [number, number, number],
  };

  it("parents the static layer and the live session under SEPARATE sub-roots", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});

    expect(engine.sketchRoot.children).toEqual([engine.staticSketchRoot, engine.activeSketchRoot]);
    engine.getSketchStaticLayer().setSketch("s1", { plane: PLANE, entities: [], regions: [] });
    expect(engine.staticSketchRoot.children.length).toBe(1);
    expect(engine.activeSketchRoot.children.length).toBe(0);

    engine.enterSketch(PLANE, [], "UnderConstrained");
    expect(engine.activeSketchRoot.children.length).toBe(1);
    engine.exitSketch();
    expect(engine.activeSketchRoot.children.length).toBe(0);
    engine.dispose();
  });

  it("Layers → Sketches hides the STATIC half only, never the sketch being drawn", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    engine.enterSketch(PLANE, [], "UnderConstrained");

    engine.setLayerVisible("sketches", false);
    expect(engine.staticSketchRoot.visible).toBe(false);
    expect(engine.activeSketchRoot.visible).toBe(true);
    expect(engine.sketchRoot.visible).toBe(true);

    engine.setLayerVisible("sketches", true);
    expect(engine.staticSketchRoot.visible).toBe(true);
    engine.dispose();
  });

  it("hides the ?vpdebug origin pill while a session is open, restores it after", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, { debug: true });
    const pill = overlay.querySelector("[data-vp-debug-label]") as HTMLElement;
    expect(pill.textContent).toBe("origin");
    const registered = engine.overlay.size;

    engine.enterSketch(PLANE, [], "UnderConstrained");
    expect(pill.style.display).toBe("none");
    // Unregistered too — the driver rewrites `display` on every frame.
    expect(engine.overlay.size).toBe(registered - 1);

    engine.exitSketch();
    expect(pill.style.display).toBe("");
    expect(engine.overlay.size).toBe(registered);
    engine.dispose();
  });

  it("keeps the original model view through a sketch switch and restores it once", async () => {
    const { canvas, overlay } = newDom();
    Object.defineProperty(canvas, "clientWidth", { value: 1000 });
    Object.defineProperty(canvas, "clientHeight", { value: 700 });
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    const controls = (engine as unknown as {
      controls: { setView(view: { yaw: number; pitch: number; distance: number; target: THREE.Vector3 }, animated: boolean): void; getViewState(): { yaw: number; pitch: number; distance: number; target: THREE.Vector3 }; update(now: number): boolean };
    }).controls;
    const original = { yaw: 0.2, pitch: 0.3, distance: 123, target: new THREE.Vector3(7, 8, 9) };
    controls.setView(original, false);

    engine.enterSketch(PLANE, [], "UnderConstrained");
    engine.exitSketch({ restoreView: false });
    engine.enterSketch({ ...PLANE, origin: [50, 0, 0] }, [], "UnderConstrained");
    engine.exitSketch({ restoreView: true });
    controls.update(performance.now() + 10_000);

    const restored = controls.getViewState();
    expect(restored).toMatchObject({ yaw: original.yaw, pitch: original.pitch, distance: original.distance });
    expect(restored.target.toArray()).toEqual([7, 8, 9]);
    engine.dispose();
  });

  it("frames populated sketch geometry in one destination-orientation request", async () => {
    const { canvas, overlay } = newDom();
    Object.defineProperty(canvas, "clientWidth", { value: 1000 });
    Object.defineProperty(canvas, "clientHeight", { value: 700 });
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    const controls = (engine as unknown as { controls: CadOrbitControls }).controls;
    const frame = vi.spyOn(controls, "frameView");
    const directView = vi.spyOn(controls, "setView");

    engine.enterSketch(PLANE, [{ id: "line", type: "Line", p0: [-4, -2], p1: [5, 3] }], "UnderConstrained");

    expect(frame).toHaveBeenCalledTimes(1);
    expect(frame.mock.calls[0][0]).toMatchObject({ yaw: -Math.PI / 2 });
    expect(directView).not.toHaveBeenCalled();
    engine.dispose();
  });

  it("explicit Fit uses the newest sketch preview and forgets it when cleared", async () => {
    const { canvas, overlay } = newDom();
    Object.defineProperty(canvas, "clientWidth", { value: 1000 });
    Object.defineProperty(canvas, "clientHeight", { value: 700 });
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    engine.enterSketch(PLANE, [], "UnderConstrained");
    const controls = (engine as unknown as { controls: CadOrbitControls }).controls;
    const frame = vi.spyOn(controls, "frameView");
    engine.setSketchPreview([{ type: "Line", p0: { x: -3, y: -2 }, p1: { x: 8, y: 4 } }]);

    expect(engine.fitView()).toBe(true);
    expect(frame).toHaveBeenCalledTimes(1);
    engine.setSketchPreview([]);
    expect(engine.fitView()).toBe(false);
    expect(frame).toHaveBeenCalledTimes(1);
    engine.dispose();
  });
});

/*
 * Zoom-to-selection bounds (W3). The invisible-body filter is the load-bearing
 * part: `Box3.setFromObject` recurses through `children` WITHOUT consulting
 * `visible`, so a hidden body would silently widen the frame.
 */
/*
 * projectPoint (FILLET-CHAMFER-UNIFY W0). homeView(false) — called
 * synchronously near the end of init() — poses the camera immediately, with
 * no frame flush needed: a real camera is cheaply available right after
 * `await engine.init(...)`, so this gets real coverage rather than being
 * deferred to W1's controller-level tests.
 */
describe("ViewportEngine.projectPoint", () => {
  it("is null before init (no canvas yet)", () => {
    const engine = new ViewportEngine();
    expect(engine.projectPoint([0, 0, 0])).toBeNull();
  });

  it("projects the camera's target on-screen and rejects a point behind the eye", async () => {
    const { canvas, overlay } = newDom();
    Object.defineProperty(canvas, "clientWidth", { value: 1000, configurable: true });
    Object.defineProperty(canvas, "clientHeight", { value: 800, configurable: true });
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});

    // CadOrbitControls defaults target to the world origin.
    const target = engine.projectPoint([0, 0, 0]);
    expect(target).not.toBeNull();
    expect(Number.isFinite(target!.x)).toBe(true);
    expect(Number.isFinite(target!.y)).toBe(true);

    // A point on the far side of the eye from the target (behind the camera)
    // must report null (w <= 0), not a flipped/garbage projection.
    // getViewDirection() is target→camera (see CadOrbitControls.ts), so
    // cameraPos = target + dir*distance; going further along `dir` walks
    // PAST the eye, away from the target.
    const dir = engine.getViewDirection();
    const dist = engine.getCameraDistance();
    const behind: [number, number, number] = [
      dir.x * (dist + 100),
      dir.y * (dist + 100),
      dir.z * (dist + 100),
    ];
    expect(engine.projectPoint(behind)).toBeNull();

    engine.dispose();
  });
});

describe("ViewportEngine.getBoundsForBodies", () => {
  /** A body group whose geometry is a unit box centred at `center`. */
  function bodyAt(bodyId: string, center: [number, number, number]): THREE.Group {
    const group = new THREE.Group();
    group.userData.bodyId = bodyId;
    const mesh = new THREE.Mesh(new THREE.BoxGeometry(2, 2, 2));
    mesh.position.set(...center);
    group.add(mesh);
    return group;
  }

  function installFrameHarness(engine: ViewportEngine) {
    const frameView = vi.fn<(request: FrameViewRequest) => boolean>(() => true);
    const dispose = vi.fn();
    const container = document.createElement("div");
    Object.defineProperty(container, "clientWidth", { value: 1000 });
    Object.defineProperty(container, "clientHeight", { value: 700 });
    (engine as unknown as { controls: { frameView: typeof frameView; dispose: typeof dispose } }).controls = {
      frameView,
      dispose,
    };
    (engine as unknown as { container: HTMLElement }).container = container;
    return frameView;
  }

  it("unions only the requested bodies", () => {
    const engine = new ViewportEngine();
    engine.bodiesRoot.add(bodyAt("a", [0, 0, 0]), bodyAt("b", [100, 0, 0]));

    const only = engine.getBoundsForBodies(["a"])!;
    expect(only.min.toArray()).toEqual([-1, -1, -1]);
    expect(only.max.toArray()).toEqual([1, 1, 1]);

    const both = engine.getBoundsForBodies(["a", "b"])!;
    expect(both.max.x).toBe(101);
    engine.dispose();
  });

  it("EXCLUDES a hidden body (setFromObject would have included it)", () => {
    const engine = new ViewportEngine();
    const a = bodyAt("a", [0, 0, 0]);
    const b = bodyAt("b", [100, 0, 0]);
    engine.bodiesRoot.add(a, b);
    b.visible = false; // tree eye or isolation

    // Proof the filter is doing real work: three itself does not respect it.
    expect(new THREE.Box3().setFromObject(b).isEmpty()).toBe(false);

    const bounds = engine.getBoundsForBodies(["a", "b"])!;
    expect(bounds.max.x).toBe(1);
    expect(engine.getBoundsForBodies(["b"])).toBeNull();
    engine.dispose();
  });

  it("returns null for unknown ids and for an empty request", () => {
    const engine = new ViewportEngine();
    engine.bodiesRoot.add(bodyAt("a", [0, 0, 0]));
    expect(engine.getBoundsForBodies(["nope"])).toBeNull();
    expect(engine.getBoundsForBodies([])).toBeNull();
    engine.dispose();
  });

  it("frames visible scene bodies inside the measured safe rect", () => {
    const engine = new ViewportEngine();
    const frameView = installFrameHarness(engine);
    engine.bodiesRoot.add(bodyAt("shown", [0, 0, 0]));
    const hidden = bodyAt("hidden", [100, 0, 0]);
    hidden.visible = false;
    engine.bodiesRoot.add(hidden);
    engine.setOverlaySafeRect({ x: 220, y: 80, width: 610, height: 500 });

    expect(engine.fitView()).toBe(true);
    const request = frameView.mock.calls[0][0];
    expect(request.viewport).toEqual({
      width: 1000,
      height: 700,
      safeRect: { x: 220, y: 80, width: 610, height: 500 },
    });
    expect(request.bounds.min.toArray()).toEqual([-1, -1, -1]);
    expect(request.bounds.max.toArray()).toEqual([1, 1, 1]);
    engine.dispose();
  });

  it("does not turn an absent target into a whole-scene fit", () => {
    const engine = new ViewportEngine();
    const frameView = installFrameHarness(engine);
    engine.bodiesRoot.add(bodyAt("other", [0, 0, 0]));

    expect(engine.fitToBodies(["missing"])).toBe(false);
    expect(frameView).not.toHaveBeenCalled();
    engine.dispose();
  });

  it("refuses a hidden requested target even when other bodies are visible", () => {
    const engine = new ViewportEngine();
    const frameView = installFrameHarness(engine);
    const hidden = bodyAt("hidden", [20, 0, 0]);
    hidden.visible = false;
    engine.bodiesRoot.add(bodyAt("other", [0, 0, 0]), hidden);

    expect(engine.fitToBodies(["hidden"])).toBe(false);
    expect(frameView).not.toHaveBeenCalled();
    engine.dispose();
  });

  it("frames preview geometry only when explicitly requested", () => {
    const engine = new ViewportEngine();
    const frameView = installFrameHarness(engine);
    engine.bodiesRoot.add(bodyAt("committed", [100, 0, 0]));
    engine.previewRoot.add(bodyAt("preview", [5, 0, 0]));

    expect(engine.fitPreview()).toBe(true);
    const framed = frameView.mock.calls[0][0].bounds as THREE.Box3;
    expect(framed.min.x).toBe(4);
    expect(framed.max.x).toBe(6);
    expect(frameView).toHaveBeenCalledTimes(1);
    engine.dispose();
  });

  it("uses L1 preview geometry when no exact L2 preview is published", () => {
    const engine = new ViewportEngine();
    const frameView = installFrameHarness(engine);
    const l1Bounds = new THREE.Box3(new THREE.Vector3(2, 3, 4), new THREE.Vector3(7, 8, 9));
    (engine as unknown as { previewMesh: { getBounds(): THREE.Box3; dispose(): void } }).previewMesh = {
      getBounds: () => l1Bounds,
      dispose: () => {},
    };

    expect(engine.fitPreview()).toBe(true);
    expect(frameView.mock.calls[0][0].bounds).toBe(l1Bounds);
    engine.dispose();
  });

  it("refuses preview fit when neither L2 nor L1 geometry exists", () => {
    const engine = new ViewportEngine();
    const frameView = installFrameHarness(engine);
    expect(engine.fitPreview()).toBe(false);
    expect(frameView).not.toHaveBeenCalled();
    engine.dispose();
  });

  it("passes one requested destination orientation to the controls", () => {
    const engine = new ViewportEngine();
    const frameView = installFrameHarness(engine);
    const target = new THREE.Box3(new THREE.Vector3(-1, -1, -1), new THREE.Vector3(1, 1, 1));

    expect(engine.fitBounds(target, { yaw: 0.2, pitch: 0.7 })).toBe(true);
    expect(frameView).toHaveBeenCalledWith(expect.objectContaining({ yaw: 0.2, pitch: 0.7 }));
    engine.dispose();
  });
});

/*
 * captureThumbnail — the explicit-save preview (persisted-cache lane).
 *
 * jsdom implements neither a 2D context nor toDataURL, so both are spied onto
 * HTMLCanvasElement.prototype. That is enough to pin what actually matters here
 * and cannot be checked in a browser-free suite otherwise: that a frame is
 * FORCED before the read-back (rendering is on-demand, so an idle canvas holds a
 * stale buffer), the downscale arithmetic, and every null path.
 */
describe("ViewportEngine.captureThumbnail", () => {
  const DATA_URL = "data:image/png;base64,AAAA";
  let drawImage: ReturnType<typeof vi.fn>;
  let toDataURL: ReturnType<typeof vi.spyOn>;
  let getContext: ReturnType<typeof vi.spyOn>;

  /** Give the mocked renderer a source canvas of `w`×`h` device pixels. */
  function sourceCanvas(w: number, h: number): HTMLCanvasElement {
    const el = document.createElement("canvas");
    el.width = w;
    el.height = h;
    mocks.renderer.domElement = el;
    return el;
  }

  beforeEach(() => {
    drawImage = vi.fn();
    getContext = vi
      .spyOn(HTMLCanvasElement.prototype, "getContext")
      .mockReturnValue({ drawImage } as unknown as CanvasRenderingContext2D);
    toDataURL = vi
      .spyOn(HTMLCanvasElement.prototype, "toDataURL")
      .mockReturnValue(DATA_URL);
  });

  afterEach(() => {
    getContext.mockRestore();
    toDataURL.mockRestore();
    mocks.renderer.domElement = null as unknown as HTMLCanvasElement;
  });

  it("is null before init — there is no renderer to read back from", () => {
    expect(new ViewportEngine().captureThumbnail()).toBeNull();
  });

  it("forces ONE render, then downscales the long edge to maxPx (aspect kept)", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    flushFrame();
    const framesBefore = engine.frameCount;

    const src = sourceCanvas(1024, 768);
    const url = engine.captureThumbnail(512);

    expect(url).toBe(DATA_URL);
    // The forced frame is the point: on-demand rendering means the drawing buffer
    // may otherwise hold a stale (or post-resize blank) image.
    expect(engine.frameCount).toBe(framesBefore + 1);
    expect(toDataURL).toHaveBeenCalledWith("image/png");
    // 1024×768 at scale 0.5 → 512×384, and the source is drawn to fill it.
    expect(drawImage).toHaveBeenCalledWith(src, 0, 0, 512, 384);
    engine.dispose();
  });

  it("never UPSCALES a canvas smaller than maxPx", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    const src = sourceCanvas(200, 100);

    expect(engine.captureThumbnail(512)).toBe(DATA_URL);
    expect(drawImage).toHaveBeenCalledWith(src, 0, 0, 200, 100);
    engine.dispose();
  });

  it("is null on WebGPU — its render() is async, so the read-back is not the frame", async () => {
    mocks.createRenderer.mockResolvedValueOnce({
      renderer: mocks.renderer,
      isWebGPU: true,
      dispose: mocks.handleDispose,
    });
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    sourceCanvas(1024, 768);

    expect(engine.captureThumbnail()).toBeNull();
    expect(toDataURL).not.toHaveBeenCalled();
    engine.dispose();
  });

  it("is null for a zero-sized canvas (pre-layout / detached)", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    sourceCanvas(0, 0);

    expect(engine.captureThumbnail()).toBeNull();
    engine.dispose();
  });

  it("is null when no 2D context is available", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    sourceCanvas(64, 64);
    getContext.mockReturnValue(null);

    expect(engine.captureThumbnail()).toBeNull();
    engine.dispose();
  });

  it("is null when the encoded PNG exceeds the size cap Rust would reject anyway", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    sourceCanvas(64, 64);
    toDataURL.mockReturnValue(`data:image/png;base64,${"A".repeat(400_000)}`);

    expect(engine.captureThumbnail()).toBeNull();
    engine.dispose();
  });

  it("swallows a throwing toDataURL (a tainted/lost canvas must not fail a save)", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    sourceCanvas(64, 64);
    toDataURL.mockImplementation(() => {
      throw new Error("SecurityError");
    });

    expect(() => engine.captureThumbnail()).not.toThrow();
    expect(engine.captureThumbnail()).toBeNull();
    engine.dispose();
  });

  it("is null after dispose()", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    sourceCanvas(64, 64);
    engine.dispose();

    expect(engine.captureThumbnail()).toBeNull();
  });
});

/*
 * The chip has to know where the arrow is on SCREEN to stay off it. Before U5
 * nothing could ask that at all (`projectPoint` returns one point; the bounds
 * helpers are world-space `Box3`s); U5 added the question for the transform
 * gizmo, and the value arrow is the other overlay a chip is anchored beside.
 */
describe("ViewportEngine interaction-overlay bounds", () => {
  it("reports the value handle's box only while it is shown", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});

    expect(engine.getInteractionOverlayBounds("valueHandle")).toBeNull();

    engine.showValueHandle([0, 0, 0], [0, 0, 1]);
    flushFrame();
    const box = engine.getInteractionOverlayBounds("valueHandle");
    const centre = engine.projectPoint([0, 0, 0]);
    expect(box).not.toBeNull();
    // Centred on the anchor, and the rolled AABB of the 30 × 40 corridor: no
    // side shorter than its width, none longer than the 50 px a 45° roll gives.
    expect(box!.x + box!.width / 2).toBeCloseTo(centre!.x, 6);
    expect(box!.y + box!.height / 2).toBeCloseTo(centre!.y, 6);
    expect(Math.min(box!.width, box!.height)).toBeGreaterThanOrEqual(30 - 1e-9);
    expect(Math.max(box!.width, box!.height)).toBeLessThanOrEqual(50 + 1e-9);

    engine.hideValueHandle();
    expect(engine.getInteractionOverlayBounds("valueHandle")).toBeNull();

    engine.dispose();
  });

  /*
   * The keep-out box must hold EVERY pixel the pick target answers to, or a chip
   * pushed clear of the box still covers part of the grab corridor. Measured in
   * e2e on 9f573ecd: a 3D pick cylinder's radius ran along the view direction,
   * so perspective parallax off the optical axis stretched the corridor to
   * 32 × 48 px while the box stayed 40 × 40 — the chip's HUD ate the top rows.
   */
  it("holds every pixel the handle picks, far off the optical axis of a tilted FOV-76 camera", async () => {
    const W = 1200;
    const H = 800;
    const { canvas: container, overlay } = newDom();
    Object.defineProperty(container, "clientWidth", { value: W });
    Object.defineProperty(container, "clientHeight", { value: H });
    const engine = new ViewportEngine();
    await engine.init(container, overlay, {});
    const internals = engine as unknown as {
      canvas: HTMLCanvasElement;
      controls: CadOrbitControls;
      rig: { getCamera(): THREE.Camera };
    };
    internals.canvas.getBoundingClientRect = () =>
      ({ left: 0, top: 0, right: W, bottom: H, width: W, height: H, x: 0, y: 0 }) as DOMRect;
    internals.controls.setView({ yaw: 0.6, pitch: 0.5, distance: 150, target: new THREE.Vector3() }, false);
    flushFrame();
    const camera = internals.rig.getCamera() as THREE.PerspectiveCamera;
    expect(camera.isPerspectiveCamera).toBe(true);
    expect(camera.fov).toBe(76);

    // A world point on the ray through NDC (0.75, 0.65), at view depth 150:
    // near the top-right corner, far from the optical axis.
    const ray = new THREE.Raycaster();
    ray.setFromCamera(new THREE.Vector2(0.75, 0.65), camera);
    const viewDir = camera.getWorldDirection(new THREE.Vector3());
    const anchor = ray.ray.origin
      .clone()
      .addScaledVector(ray.ray.direction, 150 / ray.ray.direction.dot(viewDir));

    // [1,0,0] sits at conditioning ≈ 0.077 at this anchor — just under
    // MIN_AXIS_CONDITIONING — so it draws the vertical screen proxy; the others
    // are well-conditioned axes drawn along their projected direction.
    for (const axis of [[0, 0, 1], [1, 0, 0], [0, 1, 0]] as [number, number, number][]) {
      engine.showValueHandle([anchor.x, anchor.y, anchor.z], axis);
      flushFrame();
      engine.interactionRoot.updateWorldMatrix(true, true);

      const box = engine.getInteractionOverlayBounds("valueHandle")!;
      const centre = engine.projectPoint([anchor.x, anchor.y, anchor.z])!;
      expect(box).not.toBeNull();
      expect(centre.x).toBeGreaterThan(W * 0.8);
      expect(centre.y).toBeLessThan(H * 0.2);

      const outside: string[] = [];
      let hits = 0;
      for (let dy = -60; dy <= 60; dy++) {
        for (let dx = -60; dx <= 60; dx++) {
          const x = centre.x + dx;
          const y = centre.y + dy;
          if (!engine.hitExtrudeHandle(x, y)) continue;
          hits++;
          const inside =
            x >= box.x - 0.5 && x <= box.x + box.width + 0.5 && y >= box.y - 0.5 && y <= box.y + box.height + 0.5;
          if (!inside) outside.push(`(${dx},${dy})`);
        }
      }
      expect(hits).toBeGreaterThan(0);
      expect(outside, `axis ${axis}: hits outside the keep-out box ${JSON.stringify(box)}`).toEqual([]);

      // …and the corridor itself is the 30 × 40 px rectangle, rolled with the
      // glyph: its length runs along the mapping's screen direction — the
      // axis's projected direction, or straight up for a proxy.
      const mapping = engine.valueHandleMapping()!;
      if (mapping.kind === "disabled") throw new Error("no usable scale for the value handle");
      const u = { x: mapping.direction[0], y: mapping.direction[1] };
      if (axis[0] === 1) {
        expect(mapping.kind).toBe("proxy");
        expect([u.x, u.y]).toEqual([0, -1]);
      } else {
        expect(mapping.kind).toBe("world");
        const tip = engine.projectPoint([anchor.x + axis[0] * 0.05, anchor.y + axis[1] * 0.05, anchor.z + axis[2] * 0.05])!;
        const len = Math.hypot(tip.x - centre.x, tip.y - centre.y);
        expect(u.x).toBeCloseTo((tip.x - centre.x) / len, 3);
        expect(u.y).toBeCloseTo((tip.y - centre.y) / len, 3);
      }
      const v = { x: -u.y, y: u.x };
      const at = (along: number, across: number) =>
        engine.hitExtrudeHandle(
          centre.x + u.x * along + v.x * across,
          centre.y + u.y * along + v.y * across,
        );
      expect([at(18, 0), at(-18, 0), at(0, 13), at(0, -13)]).toEqual([true, true, true, true]);
      expect([at(24, 0), at(-24, 0), at(0, 19), at(0, -19)]).toEqual([false, false, false, false]);
    }

    engine.dispose();
  });
});

/*
 * The value handle's mapping at the engine seam (R03/R06). The controller asks
 * at grab, so the answer must be computed from the camera as it is NOW — a
 * camera move with no frame rendered yet must not serve the previous frame's.
 */
describe("ViewportEngine value-handle mapping", () => {
  type Internals = {
    controls: CadOrbitControls;
    rig: { getCamera(): THREE.Camera };
    dragHandle: DragHandle;
  };
  const sized = async () => {
    const { canvas, overlay } = newDom();
    Object.defineProperty(canvas, "clientWidth", { value: 1000, configurable: true });
    Object.defineProperty(canvas, "clientHeight", { value: 800, configurable: true });
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    return { engine, overlay, internals: engine as unknown as Internals };
  };
  const toVec = (v: THREE.Vector3): [number, number, number] => [v.x, v.y, v.z];

  it("is null while the handle is hidden", async () => {
    const { engine } = await sized();
    expect(engine.valueHandleMapping()).toBeNull();
    engine.showValueHandle([0, 0, 0], [1, 0, 0]);
    expect(engine.valueHandleMapping()).not.toBeNull();
    engine.hideValueHandle();
    expect(engine.valueHandleMapping()).toBeNull();
    engine.dispose();
  });

  it("an end-on axis maps and DRAWS as the vertical screen proxy", async () => {
    const { engine, internals } = await sized();
    // getViewDirection() is target→camera: exactly along the view ray.
    engine.showValueHandle([0, 0, 0], toVec(engine.getViewDirection()));
    expect(engine.valueHandleMapping()!.kind).toBe("proxy");
    flushFrame();
    expect(internals.dragHandle.mapping()).toMatchObject({ kind: "proxy", direction: [0, -1] });
    const box = engine.getInteractionOverlayBounds("valueHandle")!;
    expect(box.width).toBeCloseTo(30, 6);
    expect(box.height).toBeCloseTo(40, 6);
    engine.dispose();
  });

  it("is computed fresh after a camera move that has not rendered yet", async () => {
    const { engine, internals } = await sized();
    const axis = engine.getViewDirection().clone();
    engine.showValueHandle([0, 0, 0], toVec(axis));
    flushFrame();
    expect(internals.dragHandle.mapping()!.kind).toBe("proxy");

    internals.controls.setView(
      { yaw: internals.controls.yaw + Math.PI / 2, pitch: 0.1, distance: 200, target: new THREE.Vector3() },
      false,
    );
    expect(Math.abs(engine.getViewDirection().dot(axis))).toBeLessThan(0.9);
    // No frame flushed: the last DRAWN mapping is stale, the query is not.
    expect(internals.dragHandle.mapping()!.kind).toBe("proxy");
    const fresh = engine.valueHandleMapping()!;
    expect(fresh).toMatchObject({ kind: "world" });
    if (fresh.kind === "world") expect(fresh.g0PxPerMm).toBeGreaterThan(0);
    engine.dispose();
  });

  /*
   * H8 / β = 0: the mapping is the TRUE projected derivative of the point that
   * moves. There is no tangent parameter left to reject with — a rejected
   * direction is one H(q) cannot follow.
   */
  it("maps the outward axis itself, with no tangent rejection left to apply", async () => {
    const { engine } = await sized();
    engine.showValueHandle([0, 0, 0], [1, -1, -1]);
    const mapping = engine.valueHandleMapping()!;
    expect(mapping.kind).toBe("world");
    const probe = engine.projectPoint([0, 0, 0])!;
    const tip = engine.projectPoint([0.05 / Math.sqrt(3), -0.05 / Math.sqrt(3), -0.05 / Math.sqrt(3)])!;
    const len = Math.hypot(tip.x - probe.x, tip.y - probe.y);
    if (mapping.kind === "world") {
      expect(mapping.direction[0]).toBeCloseTo((tip.x - probe.x) / len, 3);
      expect(mapping.direction[1]).toBeCloseTo((tip.y - probe.y) / len, 3);
    }
    engine.dispose();
  });

  it("freezes the whole drawn mapping and repaints", async () => {
    const { engine, internals } = await sized();
    engine.showValueHandle([0, 0, 0], [1, 0, 0]);
    flushFrame();
    expect(internals.dragHandle.mapping()!.kind).toBe("world");
    mocks.renderer.render.mockClear();

    engine.freezeValueHandle({ kind: "proxy", q0Mm: 0, direction: [0, -1], mmPerPx: 0.5, reason: "poorScreenSensitivity" });
    flushFrame();
    expect(mocks.renderer.render).toHaveBeenCalledTimes(1);
    expect(internals.dragHandle.mapping()).toMatchObject({ kind: "proxy", mmPerPx: 0.5 });
    // The fresh query is freeze-agnostic: it reports the geometry.
    expect(engine.valueHandleMapping()!.kind).toBe("world");

    engine.freezeValueHandle(null);
    flushFrame();
    expect(internals.dragHandle.mapping()!.kind).toBe("world");
    engine.dispose();
  });

  /*
   * THE HANDLE MOVES (derivation §5). The arrow is at H(q), so a value change
   * has to move it — an arrow pinned at the arm point is attached to nothing.
   */
  it("seats the arrow on a parameter path and moves it as the value changes", async () => {
    const { engine, internals } = await sized();
    engine.showValueHandlePath({ q0Mm: 0, point0Mm: [0, 0, 0], dPointDValue: [0, 0, 1] }, 0);
    expect(internals.dragHandle.worldAnchor().toArray()).toEqual([0, 0, 0]);
    engine.setValueHandleValue(4);
    expect(internals.dragHandle.worldAnchor().toArray()).toEqual([0, 0, 4]);
    engine.dispose();
  });

  it("draws a witness only with the claim its label makes", async () => {
    const { engine, overlay } = await sized();
    expect(engine.isValueWitnessVisible()).toBe(false);
    engine.showValueWitness({
      meaning: "parameterConstruction",
      label: "Radius parameter",
      fromMm: [0, 0, 0],
      toMm: [0, 0, 2],
    });
    expect(engine.isValueWitnessVisible()).toBe(true);
    const label = overlay.querySelector<HTMLElement>('[data-testid="value-witness-label"]');
    expect(label?.textContent).toBe("Radius parameter");
    engine.hideValueWitness();
    expect(engine.isValueWitnessVisible()).toBe(false);
    engine.dispose();
  });

  /*
   * H10: an OffsetFace `Total` draws TWO segments — the prepared reference
   * thickness and the target — and each carries its own claim. One of them is a
   * `measuredReference` and the other is not, so collapsing them into a single
   * label would attach the kernel's measurement to a value nobody has built.
   */
  it("draws every witness segment a tool hands it, each with its own label", async () => {
    const { engine, overlay } = await sized();
    engine.showValueWitness([
      {
        meaning: "measuredReference",
        label: "Reference thickness t0 — measured",
        fromMm: [4, 0, 20],
        toMm: [0, 0, 20],
      },
      {
        meaning: "targetConstruction",
        label: "Total target T — construction",
        fromMm: [0, 0, 20],
        toMm: [6, 0, 20],
      },
    ]);
    expect(engine.isValueWitnessVisible()).toBe(true);
    const labels = [...overlay.querySelectorAll<HTMLElement>('[data-testid="value-witness-label"]')]
      .filter((el) => el.style.display !== "none")
      .map((el) => el.textContent);
    expect(labels).toEqual([
      "Reference thickness t0 — measured",
      "Total target T — construction",
    ]);
    // Falling back to ONE segment must retire the second, not leave it drawn.
    engine.showValueWitness({
      meaning: "targetConstruction",
      label: "Offset target d — construction",
      fromMm: [0, 0, 0],
      toMm: [0, 0, 2],
    });
    const after = [...overlay.querySelectorAll<HTMLElement>('[data-testid="value-witness-label"]')]
      .filter((el) => el.style.display !== "none")
      .map((el) => el.textContent);
    expect(after).toEqual(["Offset target d — construction"]);
    engine.dispose();
  });

  // N9: `setScale` runs inside `renderFrame`; an unconditional invalidate there
  // re-armed a rAF after every frame. `tick` clears `dirty` after rendering, so
  // it drew nothing — but idle must mean no scheduled callback at all.
  it("an idle shown handle schedules no further animation frame (N9)", async () => {
    const { engine } = await sized();
    engine.showValueHandle([0, 0, 0], [1, 0, 0]);
    flushFrame();
    flushFrame();
    // One ordinary repaint with nothing changed: it must not re-arm a frame.
    mocks.renderer.render.mockClear();
    engine.invalidate();
    flushFrame();
    expect(mocks.renderer.render).toHaveBeenCalledTimes(1);
    expect(rafCbs).toHaveLength(0);
    engine.dispose();
  });
});

/*
 * Section view at the engine seam: what `ViewportRoot` calls, what it gets back
 * for the OTHER material library, and the repaint budget.
 */
describe("ViewportEngine.setSection", () => {
  const SECTION = { enabled: true, plane: "XY" as const, offsetMm: 5, flip: false };
  const SKETCH_PLANE = {
    kind: "custom" as const,
    origin: [0, 0, 0] as [number, number, number],
    xAxis: [1, 0, 0] as [number, number, number],
    yAxis: [0, 1, 0] as [number, number, number],
    normal: [0, 0, 1] as [number, number, number],
  };

  it("publishes the live plane, repaints ONCE, then goes idle again", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    flushFrame();
    expect(rafCbs.length).toBe(0); // idle: on-demand rendering schedules nothing

    const planes = engine.setSection(SECTION);

    expect(planes).toHaveLength(1);
    expect(planes![0].constant).toBe(5);
    expect(engine.sectionClippingPlanes()).toBe(planes);
    expect(rafCbs.length).toBe(1); // exactly one frame, not a loop

    flushFrame();
    expect(rafCbs.length).toBe(0);
    engine.dispose();
  });

  it("drops the planes again when the section is turned off", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});

    engine.setSection(SECTION);
    expect(engine.setSection({ ...SECTION, enabled: false })).toBeNull();
    expect(engine.sectionClippingPlanes()).toBeNull();
    engine.dispose();
  });

  it("refuses on WebGPU — those materials ignore clippingPlanes entirely", async () => {
    mocks.createRenderer.mockResolvedValueOnce({
      renderer: mocks.renderer,
      isWebGPU: true,
      dispose: mocks.handleDispose,
    });
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});

    expect(engine.setSection(SECTION)).toBeNull();
    expect(engine.sectionClippingPlanes()).toBeNull();
    engine.dispose();
  });

  it("reports the cut in debugSnapshot, including the CLIPPED MATERIAL count", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});

    const before = engine.debugSnapshot().section as Record<string, unknown>;
    expect(before).toMatchObject({ enabled: false, clippedMaterials: 0, capVisible: false });

    // A real body in the scene, clipped through the material library the way
    // MeshIngest does it — the count has to read the SCENE, not a flag.
    const entry = buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body1", 1);
    const library = new BodyMaterialLibrary();
    engine.bodiesRoot.add(buildBodyObject(entry, library).group);
    const planes = engine.setSection({ ...SECTION, flip: true });
    library.setClippingPlanes(planes);

    const after = engine.debugSnapshot().section as Record<string, unknown>;
    expect(after).toMatchObject({ enabled: true, plane: "XY", offsetMm: 5, flip: true });
    // face + edge material of the one body (the wireframe edge material is not
    // in the scene until the render mode switches to it).
    expect(after.clippedMaterials).toBe(2);

    flushFrame(); // the frame is what builds the stencil pairs behind the cap
    expect((engine.debugSnapshot().section as { capVisible: boolean }).capVisible).toBe(true);

    library.dispose();
    engine.dispose();
  });

  it("clips the selection overlays and the pattern ghosts, not just the bodies", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    const entry = buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body1", 1);
    swap("body1", entry); // the highlight layer resolves refs through the REGISTRY
    const library = new BodyMaterialLibrary();
    engine.bodiesRoot.add(buildBodyObject(entry, library).group);

    // A selection made BEFORE the cut is the defect this covers: the edge
    // overlays are depthTest:false, so an unclipped one paints its tint in the
    // empty half, floating over nothing.
    engine.setHighlightState({ kind: "face", id: "body1#f:4", bodyId: "body1", topoKey: "f:4" }, [
      { kind: "edge", id: "body1#e:0", bodyId: "body1", topoKey: "e:0" },
    ]);
    expect((engine.debugSnapshot().section as { clippedOverlays: number }).clippedOverlays).toBe(0);

    const planes = engine.setSection(SECTION);

    expect(
      (engine.debugSnapshot().section as { clippedOverlays: number }).clippedOverlays,
    ).toBeGreaterThan(0);

    // A ghost layer built AFTER the section was enabled must inherit the cut —
    // a pattern preview is a copy of a real body and has to obey the same plane.
    engine.showGhostPreview(entry, [{ kind: "translate", offset: [10, 0, 0] }]);
    const ghostMat = (() => {
      let found: THREE.Material | null = null;
      engine.interactionRoot.traverse((o) => {
        if (o.parent?.name === "ghostLayer") found = (o as THREE.Mesh).material as THREE.Material;
      });
      return found as THREE.Material | null;
    })();
    expect(ghostMat).not.toBeNull();
    expect(ghostMat!.clippingPlanes).toBe(planes);

    engine.setSection({ ...SECTION, enabled: false });
    expect((engine.debugSnapshot().section as { clippedOverlays: number }).clippedOverlays).toBe(0);
    expect(ghostMat!.clippingPlanes).toBeNull();

    library.dispose();
    engine.dispose();
    disposeAll();
    __resetRegistryForTests();
  });

  it("hides the cap for the duration of a sketch session, keeping the clip", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    const entry = buildBodyObjects(parseMeshPayload(makeBoxMesh()), "body1", 1);
    const library = new BodyMaterialLibrary();
    engine.bodiesRoot.add(buildBodyObject(entry, library).group);
    engine.setSection(SECTION);
    flushFrame();
    expect((engine.debugSnapshot().section as { capVisible: boolean }).capVisible).toBe(true);

    engine.enterSketch(SKETCH_PLANE, [], "UnderConstrained");
    expect((engine.debugSnapshot().section as { capVisible: boolean }).capVisible).toBe(false);
    expect(engine.sectionClippingPlanes()).toHaveLength(1);

    engine.exitSketch();
    expect((engine.debugSnapshot().section as { capVisible: boolean }).capVisible).toBe(true);

    library.dispose();
    engine.dispose();
  });
});
