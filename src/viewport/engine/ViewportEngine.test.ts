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
import { buildBodyObjects } from "../mesh/meshRegistry";
import { parseMeshPayload } from "../mesh/parseMeshPayload";

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
