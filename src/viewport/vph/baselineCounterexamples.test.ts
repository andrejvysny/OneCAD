/*
 * VP-HARDENING WP00 — baseline counterexamples (lane U).
 *
 * Each `it.fails` below is a RED reproduction of a finding in
 * docs/viewport-hardening/reference/original-rendering-review.md, written against
 * the PRODUCTION code path at commit 65b4c60 and asserting the behaviour the
 * specification requires. `it.fails` passes while the defect exists and turns
 * red the moment the defect is fixed — that is the ratchet: the work package
 * that fixes a finding must flip its case to a plain `it` (and usually move it
 * next to the module it now proves). The first red run is preserved at
 * docs/qa/viewport-hardening/baseline/vph-counterexamples-red.log.
 *
 * Plain `it` cases are installed-dependency contracts the design relies on;
 * they are green today and must stay green.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import * as THREE from "three";
import { LineSegments2 } from "three/examples/jsm/lines/LineSegments2.js";

import { choosePreferredHit, linePickThreshold } from "../engine/Picker";
import { CameraRig } from "../engine/CameraRig";
import { CadOrbitControls } from "../engine/CadOrbitControls";
import { entityPolyline } from "../engine/SketchObject";
import { segmentsForEntity, needsRetessellation, MAX_SAGITTA_PX, MAX_SEGMENTS } from "../engine/curveTessellation";
import { bakeFaceColors } from "../mesh/faceColors";
import { makeBodyMeshViewFixture } from "@/test/fixtures/bodyMeshView";
import { LINE_WIDTHS_CSS } from "../engine/screenLineStyle";
import { BODY_EDGE_WIDTH_CSS } from "../engine/bodyMaterials";
import { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import type { Rgba } from "@/ipc/types";

// ── R01 — installed Three.js contract for screen-space lines ─────────────────
describe("R01 installed contract (TEST-LINE-01 prerequisite)", () => {
  it("LineSegments2.onBeforeRender overwrites material.resolution from renderer.getViewport()", () => {
    const src = LineSegments2.prototype.onBeforeRender.toString();
    expect(src).toContain("getViewport");
    expect(src).toContain("resolution");
  });

  it("a CSS width reaches LineMaterial.linewidth unscaled (WP01 fixed R01)", () => {
    // Flipped by WP01. The draw-time resolution is LOGICAL pixels (see the case
    // above plus WebGLRenderer.js:781-785), so the authored CSS width IS the
    // value handed to `linewidth` — there is no DPR term anywhere on the path.
    vi.stubGlobal("devicePixelRatio", 2);
    const mat = new LineMaterial({ linewidth: LINE_WIDTHS_CSS.bodyFeatureEdge });
    expect(mat.linewidth).toBe(1.25);
    expect(BODY_EDGE_WIDTH_CSS).toBe(1.25);
    mat.dispose();
    vi.unstubAllGlobals();
  });
});

// ── R06 — reentrant invalidation lost by tick() ordering ─────────────────────
const mocks = vi.hoisted(() => {
  const renderer = {
    domElement: null as unknown as HTMLCanvasElement,
    render: vi.fn(),
    setSize: vi.fn(),
    setPixelRatio: vi.fn(),
    setClearColor: vi.fn(),
    dispose: vi.fn(),
  };
  const createRenderer = vi.fn(async () => ({ renderer, isWebGPU: false, dispose: vi.fn() }));
  return { renderer, createRenderer };
});
vi.mock("../engine/renderer", () => ({ createRenderer: mocks.createRenderer }));
import { ViewportEngine } from "../engine/ViewportEngine";

let rafCbs: FrameRequestCallback[] = [];
function flushFrame(t = 16): void {
  const cbs = rafCbs;
  rafCbs = [];
  for (const cb of cbs) cb(t);
}

beforeEach(() => {
  rafCbs = [];
  mocks.renderer.render.mockClear();
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
    rafCbs.push(cb);
    return rafCbs.length;
  });
  vi.stubGlobal("cancelAnimationFrame", () => {});
});
afterEach(() => vi.unstubAllGlobals());

describe("R06 scheduler (TEST-LIFE-01)", () => {
  // GREEN since WP02 (FrameScheduler consumes the dirty mask before the frame work).
  it("an invalidate() raised from an after-render listener produces the next frame", async () => {
    const engine = new ViewportEngine();
    await engine.init(document.createElement("div"), document.createElement("div"), {});
    let reentered = false;
    engine.onAfterRender(() => {
      if (reentered) return;
      reentered = true;
      engine.invalidate();
    });
    flushFrame(); // frame 1: listener fires and invalidates during the frame
    expect(engine.frameCount).toBe(1);
    expect(rafCbs.length).toBe(1); // a second frame IS scheduled …
    flushFrame(); // … and it now actually draws (the mask was consumed BEFORE the work)
    expect(engine.frameCount).toBe(2);
    engine.dispose();
  });
});

// ── R07 — camera resize / clamped zoom ───────────────────────────────────────
describe("R07 camera (TEST-CAM-01, TEST-CAM-02)", () => {
  it.fails("a stationary orthographic view picks up a new aspect on setAspect()", () => {
    const rig = new CameraRig(76);
    rig.setProjection("ortho");
    rig.setAspect(1);
    rig.apply(new THREE.Vector3(), new THREE.Vector3(0, 0, 100), 100);
    expect(rig.ortho.right / rig.ortho.top).toBeCloseTo(1, 9);
    rig.setAspect(2); // resize without any camera motion
    expect(rig.ortho.right / rig.ortho.top).toBeCloseTo(2, 9);
  });

  it.fails("zooming at the minimum distance does not move the target", () => {
    const el = document.createElement("div");
    Object.defineProperty(el, "clientHeight", { value: 800, configurable: true });
    Object.defineProperty(el, "clientWidth", { value: 1000, configurable: true });
    el.getBoundingClientRect = () =>
      ({ left: 0, top: 0, width: 1000, height: 800, right: 1000, bottom: 800 }) as DOMRect;
    document.body.appendChild(el);
    const rig = new CameraRig(76);
    rig.setAspect(1000 / 800);
    const controls = new CadOrbitControls({
      rig,
      element: el,
      onChange: () => {},
      getBounds: () => null,
      getDevicePref: () => "mouse",
      isDragActive: () => false,
    });
    controls.distance = 0.5; // CAMERA_MIN_DISTANCE — zoom-in cannot progress
    controls.applyToRig();
    const before = controls.target.clone();
    el.dispatchEvent(new WheelEvent("wheel", { deltaY: -120, clientX: 900, clientY: 200, bubbles: true, cancelable: true }));
    expect(controls.distance).toBe(0.5);
    expect(controls.target.distanceTo(before)).toBeLessThan(1e-9);
    controls.dispose();
    el.remove();
  });
});

// ── R09 — sketch curve quality ───────────────────────────────────────────────
describe("R09 sketch curve quality (TEST-SK-05, TEST-SK-06)", () => {
  it.fails("the fixed-count polyline path meets the 0.35 px settled target at r = 1000 px", () => {
    const pts = entityPolyline({ type: "Circle", center: [0, 0], radius: 1000 });
    const n = pts.length / 3 - 1; // closed strip repeats the first sample
    const sagitta = 1000 * (1 - Math.cos(Math.PI / n));
    expect(n).toBe(64); // today's ARC_SEGMENTS
    expect(sagitta).toBeCloseTo(1.2045, 3);
    expect(sagitta).toBeLessThanOrEqual(MAX_SAGITTA_PX);
  });

  it.fails("a capped large curve must not freeze a small curve's refinement (session-max trigger)", () => {
    const big = { type: "Circle", center: [0, 0], radius: 5e5 } as Parameters<typeof segmentsForEntity>[0];
    const small = { type: "Circle", center: [0, 0], radius: 5 } as Parameters<typeof segmentsForEntity>[0];
    const px = 1;
    const beforeMax = Math.max(segmentsForEntity(big, px), segmentsForEntity(small, px));
    const afterMax = Math.max(segmentsForEntity(big, px * 4), segmentsForEntity(small, px * 4));
    expect(segmentsForEntity(big, px)).toBe(MAX_SEGMENTS);
    expect(segmentsForEntity(small, px * 4)).toBeGreaterThan(segmentsForEntity(small, px));
    // The production trigger compares session maxima; both are the cap, so it says "no rebuild".
    expect(needsRetessellation(beforeMax, afterMax)).toBe(true);
  });
});

// ── R10 — pick radius reused as a depth allowance ────────────────────────────
describe("R10 picking (TEST-PICK-01)", () => {
  const cam = new THREE.PerspectiveCamera(76, 1, 0.1, 1000);
  const bias = linePickThreshold(cam, 1000, 260);

  it("6 CSS px at 260 mm / 76° / 1000 px is 2.4376 mm of world allowance", () => {
    expect(bias).toBeCloseTo(2.4376, 3);
  });

  it.fails("an edge 1 mm behind a face does not win ordinary selection", () => {
    const face = { distance: 260 } as THREE.Intersection;
    const edge = { distance: 261 } as THREE.Intersection;
    expect(choosePreferredHit(face, edge, bias)?.kind).toBe("face");
  });
});

// ── R12 — float32 world coordinates ─────────────────────────────────────────
describe("R12 precision envelope (TEST-PREC-01 prerequisite)", () => {
  it("float32 spacing is 0.0625 mm near 1e6 mm and 1 mm near 1e7 mm", () => {
    // ULP of a normal float32 = 2^(exponent − 23); checked against fround so the
    // formula is not the only oracle.
    const spacing = (x: number) => 2 ** (Math.floor(Math.log2(x)) - 23);
    for (const x of [1e6, 1e7]) {
      const f = Math.fround(x);
      const s = spacing(x);
      expect(Math.fround(f + s)).not.toBe(f);
      expect(Math.fround(f + s / 4)).toBe(f);
    }
    expect(spacing(1e6)).toBe(0.0625);
    expect(spacing(1e7)).toBe(1);
  });
});

// ── R18 — face-color lookup uses a triangle index as a face ordinal ──────────
describe("R18 face colors (TEST-COLOR-01)", () => {
  const RED: Rgba = [255, 0, 0, 255];
  const GREEN: Rgba = [0, 255, 0, 255];
  const BLUE: Rgba = [0, 0, 255, 255];
  const tri: readonly [number, number, number] = [0, 1, 2];
  const view = makeBodyMeshViewFixture({
    faces: [
      { id: "f:0", triangles: [tri, tri] },
      { id: "f:1", triangles: [tri, tri, tri, tri, tri, tri, tri] },
      { id: "f:2", triangles: [tri] },
    ],
  });
  const authored = new Map<string, Rgba>([["f:0", RED], ["f:1", GREEN], ["f:2", BLUE]]);
  const rgbOfTriangle = (baked: Float32Array, t: number) => [baked[t * 9], baked[t * 9 + 1], baked[t * 9 + 2]];

  it.fails("with triangle counts 2/7/1 each face receives only its own authored color", () => {
    const baked = bakeFaceColors(view, undefined, authored);
    const lin = (c: Rgba) => {
      const col = new THREE.Color().setRGB(c[0] / 255, c[1] / 255, c[2] / 255, THREE.SRGBColorSpace);
      return [Math.fround(col.r), Math.fround(col.g), Math.fround(col.b)];
    };
    expect(rgbOfTriangle(baked, 0)).toEqual(lin(RED));
    expect(rgbOfTriangle(baked, 2)).toEqual(lin(GREEN)); // first triangle of f:1 — today red (idAt(1) → tri 1 → f:0)
    expect(rgbOfTriangle(baked, 9)).toEqual(lin(BLUE)); // f:2 — today green (idAt(2) → tri 2 → f:1)
  });
});
