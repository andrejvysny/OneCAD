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
  /** The mocked handle. `createEnvironment` is absent by default (as on WebGPU
   *  and on every other mocked handle); a test that needs to observe the PMREM
   *  rebuild supplies it with `mockImplementationOnce`. */
  type MockHandle = {
    renderer: typeof renderer;
    isWebGPU: boolean;
    dispose: typeof handleDispose;
    createEnvironment?: (source: unknown) => { texture: unknown; dispose: () => void } | null;
  };
  const createRenderer = vi.fn(
    async (): Promise<MockHandle> => ({ renderer, isWebGPU: false, dispose: handleDispose }),
  );
  return { renderer, handleDispose, createRenderer };
});

vi.mock("./renderer", () => ({ createRenderer: mocks.createRenderer }));

/** `logError` is an assertion target for the submission-failure throttle. */
const logs = vi.hoisted(() => ({ error: vi.fn() }));
vi.mock("@/debug/log", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/debug/log")>();
  return { ...actual, logError: logs.error };
});

import { ViewportEngine, type FrameSubmission } from "./ViewportEngine";
import * as THREE from "three";
import { makeBoxMesh } from "@/ipc/mockMeshes";
import {
  buildBodyObjects,
  swap,
  disposeAll,
  setCurrentMeshPublication,
  __resetRegistryForTests,
} from "../mesh/meshRegistry";
import * as reg from "../mesh/meshRegistry";
import {
  contributionId,
  createPlatform,
  type ViewportContext,
  type ViewportContributionId,
} from "@/platform";
import { MODELING_MODULE_ID } from "@/modules/modeling/manifest";
import { buildBodyObject } from "./BodyObject";
import { BodyMaterialLibrary } from "./bodyMaterials";
import { parseMeshPayload } from "../mesh/parseMeshPayload";
import type { ProbeCandidateFilter } from "./Picker";
import type { CadOrbitControls, FrameViewRequest } from "./CadOrbitControls";

/**
 * The manual frame queue. Entries carry their id so `cancelAnimationFrame`
 * ACTUALLY removes them — WP02's scheduler cancels a pending frame on
 * dispose/suspend, and a no-op cancel would leave that observable as a phantom
 * queued callback.
 */
let rafCbs: Array<{ id: number; cb: FrameRequestCallback }> = [];
let rafSeq = 0;
function flushFrame(t = 16): void {
  const due = rafCbs;
  rafCbs = [];
  for (const q of due) q.cb(t);
}

beforeEach(() => {
  rafCbs = [];
  rafSeq = 0;
  logs.error.mockClear();
  mocks.renderer.render.mockReset();
  mocks.createRenderer.mockClear();
  mocks.handleDispose.mockClear();
  mocks.renderer.render.mockClear();
  mocks.renderer.dispose.mockClear();
  mocks.renderer.setSize.mockClear();
  mocks.renderer.setPixelRatio.mockClear();
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
    const id = ++rafSeq;
    rafCbs.push({ id, cb });
    return id;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => {
    const i = rafCbs.findIndex((q) => q.id === id);
    if (i >= 0) rafCbs.splice(i, 1);
  });
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

/*
 * VP-HARDENING WP01 — one metrics snapshot, and an idle DPR change that wakes
 * exactly one frame (VP03, spec §7.2). `resize()` and the DPR watcher are the
 * ONLY writers of that snapshot.
 */
describe("ViewportEngine metrics and the idle DPR watcher", () => {
  /** A container that reports a real CSS box (jsdom lays nothing out). */
  function sizedDom(width: number, height: number) {
    const { canvas, overlay } = newDom();
    Object.defineProperty(canvas, "clientWidth", { value: width, configurable: true });
    Object.defineProperty(canvas, "clientHeight", { value: height, configurable: true });
    return { canvas, overlay };
  }

  /** A matchMedia whose `change` listeners this test can actually fire. */
  function captureMatchMedia(): { fire: () => void; armedMedia: () => string } {
    let armed: { media: string; listeners: Array<() => void> } | null = null;
    vi.stubGlobal("matchMedia", (media: string) => {
      const q = {
        media,
        listeners: [] as Array<() => void>,
        addEventListener: (_t: string, fn: () => void) => q.listeners.push(fn),
        removeEventListener: (_t: string, fn: () => void) => {
          const i = q.listeners.indexOf(fn);
          if (i >= 0) q.listeners.splice(i, 1);
        },
      };
      armed = q;
      return q as unknown as MediaQueryList;
    });
    return {
      fire: () => armed?.listeners.slice().forEach((fn) => fn()),
      armedMedia: () => armed?.media ?? "",
    };
  }

  it("resize() produces CSS sizes, a capped DPR, and buffer dims from the cap", async () => {
    vi.stubGlobal("devicePixelRatio", 3); // above MAX_DPR
    const { canvas, overlay } = sizedDom(900, 500);
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});

    const m = engine.getMetrics();
    expect([m.cssWidth, m.cssHeight]).toEqual([900, 500]); // CSS, never multiplied
    expect(m.dpr).toBe(2); // capped
    expect([m.bufferWidth, m.bufferHeight]).toEqual([1800, 1000]);
    expect(mocks.renderer.setPixelRatio).toHaveBeenLastCalledWith(2);
    expect(mocks.renderer.setSize).toHaveBeenLastCalledWith(900, 500, false);

    engine.dispose();
  });

  it("setProjection bumps projectionRevision on the snapshot", async () => {
    const { canvas, overlay } = sizedDom(900, 500);
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    const before = engine.getMetrics();

    engine.setProjection("ortho");
    const after = engine.getMetrics();
    expect(after).not.toBe(before);
    expect(after.projectionRevision).toBe(before.projectionRevision + 1);
    expect(after.revision).toBeGreaterThan(before.revision);

    engine.dispose();
  });

  it("an idle DPR change repaints ONCE and moves no camera", async () => {
    const mm = captureMatchMedia();
    vi.stubGlobal("devicePixelRatio", 1);
    const { canvas, overlay } = sizedDom(900, 500);
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    expect(mm.armedMedia()).toBe("(resolution: 1dppx)");

    flushFrame(); // settle: idle from here
    expect(rafCbs.length).toBe(0);
    const framesBefore = engine.frameCount;
    const before = engine.getMetrics();
    const quat = engine.getCameraQuaternion(new THREE.Quaternion());

    // The window was dragged to a 2x display. No CSS resize happens.
    vi.stubGlobal("devicePixelRatio", 2);
    mm.fire();

    expect(rafCbs.length).toBe(1); // exactly one frame scheduled
    flushFrame();
    expect(engine.frameCount).toBe(framesBefore + 1);
    expect(rafCbs.length).toBe(0); // and it settles again

    const after = engine.getMetrics();
    expect(after.dpr).toBe(2);
    expect([after.cssWidth, after.cssHeight]).toEqual([900, 500]); // CSS box unchanged
    expect([after.bufferWidth, after.bufferHeight]).toEqual([1800, 1000]);
    expect(after.revision).toBe(before.revision + 1);
    expect(mocks.renderer.setPixelRatio).toHaveBeenLastCalledWith(2);
    expect(mocks.renderer.setSize).toHaveBeenLastCalledWith(900, 500, false);
    // A repaint, not a camera move.
    expect(engine.getCameraQuaternion(new THREE.Quaternion()).equals(quat)).toBe(true);

    engine.dispose();
  });

  it("two identical DPR events produce ONE redraw", async () => {
    const mm = captureMatchMedia();
    vi.stubGlobal("devicePixelRatio", 1);
    const { canvas, overlay } = sizedDom(900, 500);
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    flushFrame();
    const framesBefore = engine.frameCount;

    vi.stubGlobal("devicePixelRatio", 2);
    mm.fire();
    mm.fire(); // same ratio again — nothing moved
    expect(rafCbs.length).toBe(1);
    flushFrame();
    expect(engine.frameCount).toBe(framesBefore + 1);
    expect(rafCbs.length).toBe(0);

    engine.dispose();
  });

  it("dispose() unhooks the DPR watcher", async () => {
    const mm = captureMatchMedia();
    vi.stubGlobal("devicePixelRatio", 1);
    const { canvas, overlay } = sizedDom(900, 500);
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    flushFrame();
    engine.dispose();

    vi.stubGlobal("devicePixelRatio", 2);
    mm.fire();
    expect(rafCbs.length).toBe(0);
  });
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

  /*
   * D6 — a preview body is an OWNER of its lease, not a detached borrower the
   * registry sweeps up later (PR-06). Every removal site disposes the handle:
   * replacement, id-clear, full clear, and engine teardown.
   */
  it("releases the preview body lease at replacement, clear, and dispose", () => {
    const before = reg.leakTripwireCount;
    const engine = new ViewportEngine();
    const first = buildBodyObjects(parseMeshPayload(makeBoxMesh()), "preview-a", 1);
    const replacement = buildBodyObjects(parseMeshPayload(makeBoxMesh()), "preview-a", 2);
    const other = buildBodyObjects(parseMeshPayload(makeBoxMesh()), "preview-b", 1);
    const full = buildBodyObjects(parseMeshPayload(makeBoxMesh()), "preview-c", 1);
    const atDispose = buildBodyObjects(parseMeshPayload(makeBoxMesh()), "preview-d", 1);

    engine.setPreviewBody(first);
    expect(reg.openLeases(first)).toEqual(["body"]);

    engine.setPreviewBody(replacement); // same bodyId ⇒ replaces `first`
    expect(reg.openLeases(first)).toEqual([]);
    expect(reg.openLeases(replacement)).toEqual(["body"]);

    engine.setPreviewBody(other);
    engine.clearPreviewBody(["preview-a"]);
    expect(reg.openLeases(replacement)).toEqual([]);
    expect(reg.openLeases(other)).toEqual(["body"]);

    engine.setPreviewBody(full);
    engine.clearPreviewBody();
    expect(reg.openLeases(other)).toEqual([]);
    expect(reg.openLeases(full)).toEqual([]);

    engine.setPreviewBody(atDispose);
    engine.dispose();
    expect(reg.openLeases(atDispose)).toEqual([]);

    // Freeing a resource under an open lease is what trips the tripwire, so
    // this is the negative half of the same assertion.
    for (const e of [first, replacement, other, full, atDispose]) e.dispose();
    expect(reg.leakTripwireCount).toBe(before);
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
    const box = engine.getInteractionOverlayBounds("valueHandle");
    expect(box).not.toBeNull();
    // Square, because the billboarded arrow can reach that far in ANY screen
    // direction, and centred on the anchor.
    expect(box!.width).toBeCloseTo(box!.height, 6);
    expect(box!.width).toBeGreaterThan(0);

    engine.hideValueHandle();
    expect(engine.getInteractionOverlayBounds("valueHandle")).toBeNull();

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

/*
 * VP-HARDENING WP02 — the frame scheduler, the submission record, and the
 * renderer/context lifecycle (VP01–VP02, spec §5–§6; findings R06, R17).
 *
 * TEST-LIFE-01/02/04/05 (U). Everything here is about ORDER and BOUNDS: a
 * redraw asked for inside a frame must not be lost, an acknowledgment must name
 * the frame it belongs to, and a failing renderer must stop rather than spin.
 */
describe("ViewportEngine frame scheduler (TEST-LIFE-01)", () => {
  it("answers an invalidate() raised from an after-render listener, then goes idle", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});

    let reentered = false;
    engine.onAfterRender(() => {
      if (reentered) return;
      reentered = true;
      engine.invalidate();
    });

    flushFrame();
    expect(engine.frameCount).toBe(1);
    expect(rafCbs.length).toBe(1); // the reentrant request survived its frame

    flushFrame();
    expect(engine.frameCount).toBe(2); // …and actually drew

    expect(rafCbs.length).toBe(0);
    flushFrame();
    expect(engine.frameCount).toBe(2); // idle, no loop

    engine.dispose();
  });

  it("answers an invalidate() raised from a contribution's frame hook", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});

    const platform = createPlatform();
    // Armed only for the measured frame — attaching a contribution schedules a
    // frame of its own, and the hook must not spend its one shot on that.
    let armed = false;
    platform.viewport.register(MODELING_MODULE_ID, {
      id: contributionId<ViewportContributionId>(
        MODELING_MODULE_ID,
        "onecad.modeling.viewport.wp02probe",
      ),
      attach(ctx: ViewportContext) {
        ctx.onFrame(() => {
          if (!armed) return;
          armed = false;
          engine.invalidate();
        });
        return { dispose: () => {} };
      },
    });
    engine.setContributionRegistry(platform.viewport);
    for (let i = 0; i < 5 && rafCbs.length > 0; i++) flushFrame();
    expect(rafCbs.length).toBe(0);
    const before = engine.frameCount;

    armed = true;
    engine.invalidate();
    flushFrame(); // the hook invalidates from INSIDE this frame
    expect(engine.frameCount).toBe(before + 1);
    expect(rafCbs.length).toBe(1);
    flushFrame();
    expect(engine.frameCount).toBe(before + 2);
    expect(rafCbs.length).toBe(0);

    engine.dispose();
  });
});

describe("ViewportEngine init/dispose races (TEST-LIFE-02)", () => {
  it("dispose() before createRenderer resolves releases once and leaves no listener", async () => {
    const container = document.createElement("div");
    const overlay = document.createElement("div");
    const engine = new ViewportEngine();
    const pending = engine.init(container, overlay, {});
    const created = container.querySelector("canvas");
    expect(created).not.toBeNull();

    engine.dispose();
    await pending;

    expect(mocks.handleDispose).toHaveBeenCalledTimes(1);
    expect(rafCbs.length).toBe(0); // no scheduler frame survives
    expect(container.querySelector("canvas")).toBeNull(); // detached

    // The context-loss listener is gone: a dispatched event is not prevented.
    const ev = new Event("webglcontextlost", { cancelable: true });
    const prevented = vi.spyOn(ev, "preventDefault");
    created?.dispatchEvent(ev);
    expect(prevented).not.toHaveBeenCalled();
    expect(engine.getLifecycle()).toBe("disposed");
  });

  it("StrictMode mount/unmount twice leaves zero rAF and one live engine", async () => {
    const first = new ViewportEngine();
    await first.init(document.createElement("div"), document.createElement("div"), {});
    first.dispose();
    expect(rafCbs.length).toBe(0);

    const second = new ViewportEngine();
    await second.init(document.createElement("div"), document.createElement("div"), {});
    expect(rafCbs.length).toBe(1); // exactly the live engine's first frame
    flushFrame();
    expect(second.frameCount).toBe(1);
    expect(first.frameCount).toBe(0);

    second.dispose();
    expect(mocks.handleDispose).toHaveBeenCalledTimes(2);
    expect(rafCbs.length).toBe(0);
  });
});

describe("ViewportEngine idle contract (TEST-LIFE-04)", () => {
  /** A matchMedia whose `change` listeners this test can actually fire. */
  function captureMatchMedia(): { fire: () => void } {
    let armed: { listeners: Array<() => void> } | null = null;
    vi.stubGlobal("matchMedia", (media: string) => {
      const q = {
        media,
        listeners: [] as Array<() => void>,
        addEventListener: (_t: string, fn: () => void) => q.listeners.push(fn),
        removeEventListener: (_t: string, fn: () => void) => {
          const i = q.listeners.indexOf(fn);
          if (i >= 0) q.listeners.splice(i, 1);
        },
      };
      armed = q;
      return q as unknown as MediaQueryList;
    });
    return { fire: () => armed?.listeners.slice().forEach((fn) => fn()) };
  }

  it("a finished Home tween stops the loop dead; a DPR change wakes exactly one frame", async () => {
    const mm = captureMatchMedia();
    vi.stubGlobal("devicePixelRatio", 1);
    const container = document.createElement("div");
    Object.defineProperty(container, "clientWidth", { value: 900, configurable: true });
    Object.defineProperty(container, "clientHeight", { value: 500, configurable: true });
    const engine = new ViewportEngine();
    await engine.init(container, document.createElement("div"), {});
    flushFrame();

    engine.homeView(); // animated tween
    expect(rafCbs.length).toBe(1);
    // Run the tween out. The timestamps must be real-clock, because the tween
    // starts from performance.now() inside the controls.
    const t0 = performance.now();
    for (let i = 0; i < 40 && rafCbs.length > 0; i++) flushFrame(t0 + i * 40);
    expect(rafCbs.length).toBe(0);

    const settled = engine.frameCount;
    for (let i = 0; i < 20; i++) flushFrame(t0 + 2000 + i * 16);
    expect(rafCbs.length).toBe(0); // zero further rAF requests
    expect(engine.frameCount).toBe(settled); // …and zero further renders

    vi.stubGlobal("devicePixelRatio", 2);
    mm.fire();
    expect(rafCbs.length).toBe(1); // a display change still wakes it
    flushFrame();
    expect(engine.frameCount).toBe(settled + 1);
    expect(rafCbs.length).toBe(0);

    engine.dispose();
  });
});

describe("ViewportEngine submission record and failure bounds (TEST-LIFE-05)", () => {
  const PUB_1 = { documentId: "doc-1", runtimeSession: "sess-1", snapshotId: 1, generation: 1 };
  const PUB_2 = { documentId: "doc-1", runtimeSession: "sess-1", snapshotId: 2, generation: 2 };

  /** Install a real registry entry (so it carries provenance) and show it. */
  function showBody(
    engine: ViewportEngine,
    bodyId: string,
    provenance: typeof PUB_1 | undefined,
  ): void {
    const entry = buildBodyObjects(
      parseMeshPayload(makeBoxMesh()),
      bodyId,
      1,
      undefined,
      undefined,
      provenance,
    );
    swap(bodyId, entry);
    engine.bodiesRoot.add(buildBodyObject(entry, new BodyMaterialLibrary()).group);
  }

  beforeEach(() => __resetRegistryForTests());
  afterEach(() => {
    setCurrentMeshPublication(null);
    disposeAll();
    __resetRegistryForTests();
  });

  it("names the DISPLAYED publication, not the adopted one", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});

    showBody(engine, "body-shown", PUB_1);
    const hidden = new THREE.Group();
    hidden.userData.bodyId = "body-hidden";
    hidden.visible = false;
    engine.bodiesRoot.add(hidden);

    // A document change ADOPTS the next publication before any of its meshes
    // land. The frame drawn in that window still shows PUB_1 geometry, and
    // saying otherwise is the R06-adjacent lie WP02 has to avoid.
    setCurrentMeshPublication(PUB_2);

    const seen: FrameSubmission[] = [];
    engine.onAfterRender((s) => seen.push(s));
    flushFrame();

    expect(seen).toHaveLength(1);
    expect(seen[0].submission).toBe(1);
    expect(seen[0].requestedRevision).toBeGreaterThan(0);
    expect(seen[0].publication).toEqual(PUB_1); // NOT PUB_2
    expect(seen[0].displayedBodyIds).toEqual(["body-shown"]);
    expect(seen[0].displayedProvenance).toEqual([
      { bodyId: "body-shown", provenance: PUB_1 },
    ]);
    expect(engine.getLastSubmission()).toBe(seen[0]);

    engine.dispose();
  });

  it("a MIXED frame names no publication but reports both generations", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});

    showBody(engine, "body-old", PUB_1);
    showBody(engine, "body-new", PUB_2); // the second body finished ingesting
    setCurrentMeshPublication(PUB_2);

    const seen: FrameSubmission[] = [];
    engine.onAfterRender((s) => seen.push(s));
    flushFrame();

    expect(seen[0].publication).toBeNull();
    expect(seen[0].displayedProvenance).toEqual([
      { bodyId: "body-old", provenance: PUB_1 },
      { bodyId: "body-new", provenance: PUB_2 },
    ]);

    engine.dispose();
  });

  it("no listener runs against an engine a previous listener disposed", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    const second = vi.fn();
    engine.onAfterRender(() => engine.dispose());
    engine.onAfterRender(second);

    flushFrame();
    expect(second).not.toHaveBeenCalled();
  });

  it("a throwing render acknowledges nothing, and three in a row park the engine", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    const acked = vi.fn();
    engine.onAfterRender(acked);

    mocks.renderer.render.mockImplementation(() => {
      throw new Error("CONTEXT_LOST_WEBGL");
    });

    flushFrame(); // failure 1
    expect(acked).not.toHaveBeenCalled();
    expect(engine.frameCount).toBe(0); // a frame that threw was never submitted
    expect(engine.getLifecycle()).toBe("active");

    engine.invalidate();
    flushFrame(); // failure 2
    engine.invalidate();
    flushFrame(); // failure 3
    expect(engine.getLifecycle()).toBe("error");
    expect(rafCbs.length).toBe(0); // parked — no resubmission loop
    expect(acked).not.toHaveBeenCalled();

    // A retry with a working renderer brings it back.
    mocks.renderer.render.mockImplementation(() => undefined);
    await expect(engine.retryRenderer()).resolves.toBe(true);
    expect(engine.getLifecycle()).toBe("active");
    expect(mocks.createRenderer).toHaveBeenCalledTimes(2);

    engine.invalidate();
    flushFrame();
    expect(engine.frameCount).toBe(1);
    expect(acked).toHaveBeenCalledTimes(1);

    engine.dispose(); // still disposable after a failure
  });

  /*
   * PR-05. An after-render listener is the production entry point for the
   * frame-loop defect: `notifySubmitted` ran listeners with no isolation, so
   * one bad contribution took the whole frame down — and a listener that
   * invalidates before it throws kept the queue permanently non-empty.
   */
  it("isolates after-render listeners: one that throws does not stop the next", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    const second = vi.fn();
    engine.onAfterRender(() => {
      throw new Error("listener blew up");
    });
    engine.onAfterRender(second);

    flushFrame();

    expect(second).toHaveBeenCalledTimes(1);
    expect(engine.frameCount).toBe(1); // the frame itself still submitted
    expect(engine.getLifecycle()).toBe("active");
    expect(logs.error).toHaveBeenCalledTimes(1);
    expect(logs.error.mock.calls[0][1]).toBe("after-render listener threw");
    expect(rafCbs).toHaveLength(0); // …and nothing was rescheduled

    engine.dispose();
  });

  it("a listener that invalidates and throws every frame parks the engine after three", async () => {
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    let calls = 0;
    engine.onAfterRender(() => {
      calls++;
      engine.invalidate(); // asks for another frame …
      throw new Error("listener blew up"); // … and fails again
    });

    // Bounded so a live loop still terminates: the bound under test is the
    // engine's, not this loop's.
    for (let i = 0; i < 12 && rafCbs.length > 0; i++) flushFrame();

    expect(calls).toBe(3);
    expect(engine.frameCount).toBe(3);
    expect(rafCbs).toHaveLength(0); // no permanent frame loop (guide §22)
    expect(logs.error).toHaveBeenCalledTimes(1); // identical message, 5 s throttle

    engine.dispose();
  });

  it("an async submission REJECTION is the same failure — nothing is acknowledged", async () => {
    let reject!: (e: Error) => void;
    mocks.renderer.render.mockImplementationOnce(
      () => new Promise<void>((_res, rej) => { reject = rej; }),
    );
    const { canvas, overlay } = newDom();
    const engine = new ViewportEngine();
    await engine.init(canvas, overlay, {});
    const acked = vi.fn();
    engine.onAfterRender(acked);

    flushFrame();
    reject(new Error("device lost"));
    await Promise.resolve();
    await Promise.resolve();
    expect(acked).not.toHaveBeenCalled();

    engine.dispose();
  });
});

describe("ViewportEngine context lifecycle (spec §6)", () => {
  it("lost suspends submissions; restored rebuilds the environment and wakes one frame", async () => {
    const envDispose = vi.fn();
    const createEnvironment = vi.fn(() => ({ texture: {}, dispose: envDispose }));
    mocks.createRenderer.mockImplementationOnce(async () => ({
      renderer: mocks.renderer,
      isWebGPU: false,
      dispose: mocks.handleDispose,
      createEnvironment,
    }));

    const container = document.createElement("div");
    const engine = new ViewportEngine();
    const states: string[] = [];
    engine.onLifecycleChanged((s) => states.push(s));
    await engine.init(container, document.createElement("div"), {});
    flushFrame();
    expect(engine.getLifecycle()).toBe("active");
    expect(createEnvironment).toHaveBeenCalledTimes(1);

    const gl = container.querySelector("canvas");
    expect(gl).not.toBeNull();

    const lost = new Event("webglcontextlost", { cancelable: true });
    const prevented = vi.spyOn(lost, "preventDefault");
    gl?.dispatchEvent(lost);
    expect(prevented).toHaveBeenCalledTimes(1);
    expect(engine.getLifecycle()).toBe("lost");

    const framesAtLoss = engine.frameCount;
    engine.invalidate();
    expect(rafCbs.length).toBe(0); // nothing may be submitted into a dead context
    flushFrame();
    expect(engine.frameCount).toBe(framesAtLoss);

    gl?.dispatchEvent(new Event("webglcontextrestored"));
    expect(engine.getLifecycle()).toBe("restoring");
    await Promise.resolve(); // the deferred rebuild microtask
    expect(engine.getLifecycle()).toBe("active");
    expect(createEnvironment).toHaveBeenCalledTimes(2); // rebuilt exactly once more
    expect(rafCbs.length).toBe(1); // exactly one recovery frame
    flushFrame();
    expect(engine.frameCount).toBe(framesAtLoss + 1);
    expect(rafCbs.length).toBe(0);

    expect(states).toEqual(["active", "lost", "restoring", "active"]);
    engine.dispose();
    expect(envDispose).toHaveBeenCalled();
  });
});

describe("ViewportEngine renderer retry (TEST-BACKEND-03)", () => {
  it("rebuilds on a FRESH canvas after a failed init, with listeners bound once", async () => {
    mocks.createRenderer.mockRejectedValueOnce(new Error("no webgl2"));
    const container = document.createElement("div");
    const engine = new ViewportEngine();
    await expect(
      engine.init(container, document.createElement("div"), {}),
    ).rejects.toThrow("no webgl2");

    const original = container.querySelector("canvas");
    expect(original).not.toBeNull();
    expect(engine.getRendererCapabilities()).toBeNull(); // nothing claimed yet

    // A retry after a FAILED init re-runs the WHOLE init path: a `true` for a
    // shell with no grid, triad, section, picker or observers would be a
    // success claim for a viewport that cannot do anything (N5).
    await expect(engine.retryRenderer()).resolves.toBe(true);
    expect(mocks.createRenderer).toHaveBeenCalledTimes(2);
    expect(engine.getLifecycle()).toBe("active");
    // The features init builds AFTER the renderer are actually there now.
    const built = engine as unknown as {
      grid: unknown;
      triad: unknown;
      section: unknown;
      picker: unknown;
      controls: unknown;
      resizeObserver: unknown;
      disposeDprWatcher: unknown;
    };
    for (const key of [
      "grid",
      "triad",
      "section",
      "picker",
      "controls",
      "resizeObserver",
      "disposeDprWatcher",
    ] as const) {
      expect(built[key], key).not.toBeNull();
    }
    // …and exactly one light rig, not two (buildScene is idempotent).
    expect(
      (engine as unknown as { scene: THREE.Scene }).scene.children.filter(
        (c) => c instanceof THREE.DirectionalLight,
      ),
    ).toHaveLength(2);

    // A force-lost context can never be revived on the same element.
    const replacement = container.querySelector("canvas");
    expect(replacement).not.toBe(original);
    expect(original?.isConnected).toBe(false);
    expect(container.querySelectorAll("canvas")).toHaveLength(1);

    // The context-loss listener is bound to the NEW canvas exactly once.
    const lost = new Event("webglcontextlost", { cancelable: true });
    const prevented = vi.spyOn(lost, "preventDefault");
    replacement?.dispatchEvent(lost);
    expect(prevented).toHaveBeenCalledTimes(1);
    expect(engine.getLifecycle()).toBe("lost");

    engine.dispose();
  });

  it("re-measures the CSS box before sizing the rebuilt renderer", async () => {
    const container = document.createElement("div");
    Object.defineProperty(container, "clientWidth", { value: 900, configurable: true });
    Object.defineProperty(container, "clientHeight", { value: 500, configurable: true });
    const engine = new ViewportEngine();
    await engine.init(container, document.createElement("div"), {});
    flushFrame();

    // The layout moved while the renderer was down. `resize()` early-returns
    // with no handle, so only the retry can pick it up (N6).
    Object.defineProperty(container, "clientWidth", { value: 400, configurable: true });
    Object.defineProperty(container, "clientHeight", { value: 300, configurable: true });
    await expect(engine.retryRenderer()).resolves.toBe(true);

    expect(engine.getMetrics().cssWidth).toBe(400);
    expect(engine.getMetrics().cssHeight).toBe(300);
    expect(mocks.renderer.setSize).toHaveBeenLastCalledWith(400, 300, false);

    engine.dispose();
  });

  it("stays in error and claims nothing when the retry itself fails", async () => {
    const container = document.createElement("div");
    const engine = new ViewportEngine();
    await engine.init(container, document.createElement("div"), {});
    flushFrame();

    mocks.createRenderer.mockRejectedValueOnce(new Error("adapter gone"));
    await expect(engine.retryRenderer()).resolves.toBe(false);
    expect(engine.getLifecycle()).toBe("error");
    expect(engine.getRendererCapabilities()).toBeNull();
    expect(rafCbs.length).toBe(0); // no retry loop

    engine.dispose();
  });
});

/*
 * WP02 review findings B2 and N1 — a transition must cost exactly one frame per
 * update tick, and must NEVER be able to keep a failing renderer resubmitting.
 * Both are about the same mechanism: `controls.update()` commits the camera and
 * re-invalidates on every tick, including the last one.
 */
describe("ViewportEngine transitions and the failure park (B2, N1)", () => {
  /** Init with a measured CSS box so the camera work is meaningful. */
  async function sizedEngine() {
    const container = document.createElement("div");
    Object.defineProperty(container, "clientWidth", { value: 900, configurable: true });
    Object.defineProperty(container, "clientHeight", { value: 500, configurable: true });
    const engine = new ViewportEngine();
    await engine.init(container, document.createElement("div"), {});
    flushFrame();
    return { engine, container };
  }

  /** Run the frame queue out, returning how many ticks it took. */
  function runToIdle(t0: number, stepMs = 50, cap = 60): number {
    let ticks = 0;
    for (let i = 0; i < cap && rafCbs.length > 0; i++) {
      ticks++;
      flushFrame(t0 + i * stepMs);
    }
    return ticks;
  }

  it("a tween draws once per advancing tick and adds NO extra frame at the end", async () => {
    const { engine } = await sizedEngine();
    mocks.renderer.render.mockClear();

    engine.homeView(); // animated tween
    const ticks = runToIdle(performance.now());

    // The final tick advances an already-finished tween and finds an empty mask,
    // so it draws nothing. Before N1 the tween's own last `commit()` dirtied the
    // mask and that tick rendered too — one wasted frame per tween.
    expect(ticks).toBeGreaterThan(2);
    expect(mocks.renderer.render).toHaveBeenCalledTimes(ticks - 1);
    expect(rafCbs.length).toBe(0);

    engine.dispose();
  });

  it("a tween against a throwing renderer stops dead after 3 attempts", async () => {
    const { engine } = await sizedEngine();
    mocks.renderer.render.mockImplementation(() => {
      throw new Error("CONTEXT_LOST_WEBGL");
    });
    mocks.renderer.render.mockClear();
    logs.error.mockClear();

    engine.homeView();
    runToIdle(performance.now(), 10);

    expect(mocks.renderer.render).toHaveBeenCalledTimes(3); // not once per tween tick
    expect(logs.error).toHaveBeenCalledTimes(1); // identical message, 5 s throttle
    expect(engine.getLifecycle()).toBe("error");
    expect(rafCbs.length).toBe(0); // parked, with the tween abandoned

    engine.dispose();
  });

  it("each EXTERNAL invalidate buys exactly one attempt, then re-parks", async () => {
    const { engine } = await sizedEngine();
    mocks.renderer.render.mockImplementation(() => {
      throw new Error("CONTEXT_LOST_WEBGL");
    });
    mocks.renderer.render.mockClear();
    logs.error.mockClear();
    for (let i = 0; i < 3; i++) {
      engine.invalidate();
      flushFrame();
    }
    expect(engine.getLifecycle()).toBe("error");

    for (let i = 0; i < 20; i++) {
      engine.invalidate(); // e.g. a held orbit drag's pointermove repaints
      flushFrame();
      expect(rafCbs.length).toBe(0); // never leaves a frame behind
    }
    // One attempt per request — 23 in total, never a self-sustaining loop.
    expect(mocks.renderer.render).toHaveBeenCalledTimes(23);
    // …and the identical message is reported exactly once in the 5 s window.
    expect(logs.error).toHaveBeenCalledTimes(1);

    engine.dispose();
  });
});

describe("ViewportEngine lifecycle guards (N7, N8, N9)", () => {
  it("ignores webglcontextrestored when the context was never lost", async () => {
    const createEnvironment = vi.fn(() => ({ texture: {}, dispose: vi.fn() }));
    mocks.createRenderer.mockImplementationOnce(async () => ({
      renderer: mocks.renderer,
      isWebGPU: false,
      dispose: mocks.handleDispose,
      createEnvironment,
    }));
    const container = document.createElement("div");
    const engine = new ViewportEngine();
    const states: string[] = [];
    engine.onLifecycleChanged((s) => states.push(s));
    await engine.init(container, document.createElement("div"), {});
    flushFrame();
    expect(createEnvironment).toHaveBeenCalledTimes(1);

    const gl = container.querySelector("canvas");
    gl?.dispatchEvent(new Event("webglcontextrestored"));
    gl?.dispatchEvent(new Event("webglcontextrestored"));
    await Promise.resolve();

    expect(createEnvironment).toHaveBeenCalledTimes(1); // no spurious rebuild
    expect(states).toEqual(["active"]); // and no `restoring` flash
    expect(engine.getLifecycle()).toBe("active");

    engine.dispose();
  });

  it("a context lost DURING async init is not overwritten with active", async () => {
    let release!: () => void;
    const gate = new Promise<void>((res) => { release = res; });
    mocks.createRenderer.mockImplementationOnce(async () => {
      await gate;
      return { renderer: mocks.renderer, isWebGPU: false, dispose: mocks.handleDispose };
    });

    const container = document.createElement("div");
    const engine = new ViewportEngine();
    const pending = engine.init(container, document.createElement("div"), {});
    // The canvas listeners are bound before the renderer is awaited, which is
    // exactly why this window exists.
    container.querySelector("canvas")?.dispatchEvent(
      new Event("webglcontextlost", { cancelable: true }),
    );
    expect(engine.getLifecycle()).toBe("lost");

    release();
    await pending;
    expect(engine.getLifecycle()).toBe("lost"); // NOT active
    expect(rafCbs.length).toBe(0); // and still suspended

    engine.dispose();
  });

  it("captureThumbnail refuses to force a frame outside the active state", async () => {
    const container = document.createElement("div");
    Object.defineProperty(container, "clientWidth", { value: 40, configurable: true });
    Object.defineProperty(container, "clientHeight", { value: 30, configurable: true });
    const engine = new ViewportEngine();
    await engine.init(container, document.createElement("div"), {});
    flushFrame();
    const framesBefore = engine.frameCount;

    container.querySelector("canvas")?.dispatchEvent(
      new Event("webglcontextlost", { cancelable: true }),
    );
    expect(engine.getLifecycle()).toBe("lost");
    expect(engine.captureThumbnail()).toBeNull();
    expect(engine.frameCount).toBe(framesBefore); // no forced submission

    engine.dispose();
  });
});
