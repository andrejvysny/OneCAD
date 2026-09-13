/*
 * Renderer construction — the GL settings the section view depends on, the
 * capability record, and the backend policy (VP-HARDENING VP01, spec §5).
 *
 * jsdom has no WebGL, so `THREE.WebGLRenderer` is stubbed: constructing the real
 * one throws before any of this could be read. What is pinned here is exactly
 * what `createWebGl` decides — the context attributes it asks for and the flags
 * it sets afterwards — because they fail SILENTLY:
 *   - `localClippingEnabled` off ⇒ every `material.clippingPlanes` is skipped
 *     and the scene renders uncut, with no warning,
 *   - no stencil buffer ⇒ the capped cut degrades to a hollow shell,
 *   - `preserveDrawingBuffer` off ⇒ `captureThumbnail` reads back a blank frame
 *     (on-demand rendering means the buffer is not otherwise retained),
 *   - a non-sRGB output color space just looks "washed out".
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

const mocks = vi.hoisted(() => {
  const constructed: Array<Record<string, unknown>> = [];
  const getParameter = vi.fn((pname: number) => pname);
  const gl = {
    DEPTH_BITS: 24,
    STENCIL_BITS: 8,
    SAMPLES: 4,
    MAX_TEXTURE_SIZE: 16384,
    MAX_RENDERBUFFER_SIZE: 16384,
    getParameter,
    getContextAttributes: vi.fn(() => ({
      antialias: true,
      stencil: true,
      depth: true,
      preserveDrawingBuffer: true,
    })),
    getSupportedExtensions: vi.fn(() => ["EXT_clip_control", "OES_texture_float"]),
  };
  class FakeWebGLRenderer {
    localClippingEnabled = false;
    toneMapping = 0;
    toneMappingExposure = 0;
    outputColorSpace = "";
    capabilities = { reversedDepthBuffer: false };
    domElement: unknown;
    constructor(params: Record<string, unknown>) {
      constructed.push(params);
      this.domElement = params.canvas;
    }
    getContext() {
      return gl;
    }
    setClearColor() {}
    setPixelRatio() {}
    setSize() {}
    render() {}
    forceContextLoss() {}
    dispose() {}
  }
  return { constructed, gl, getParameter, FakeWebGLRenderer };
});

vi.mock("three", async (importOriginal) => {
  const actual = await importOriginal<typeof import("three")>();
  return { ...actual, WebGLRenderer: mocks.FakeWebGLRenderer };
});

const logs = vi.hoisted(() => ({ warn: vi.fn() }));
vi.mock("@/debug/log", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/debug/log")>();
  return { ...actual, logWarn: logs.warn };
});

import * as THREE from "three";
import { createRenderer, WEBGL_CONTEXT_ATTRS } from "./renderer";

beforeEach(() => {
  mocks.constructed.length = 0;
  mocks.getParameter.mockClear();
  logs.warn.mockClear();
});

describe("createRenderer (WebGL)", () => {
  it("asks for a stencil buffer AND keeps the preserved drawing buffer", () => {
    expect(WEBGL_CONTEXT_ATTRS.stencil).toBe(true);
    expect(WEBGL_CONTEXT_ATTRS.preserveDrawingBuffer).toBe(true);
  });

  it("passes those attributes verbatim to the renderer, with the canvas", async () => {
    const canvas = document.createElement("canvas");
    await createRenderer(canvas);

    expect(mocks.constructed).toHaveLength(1);
    expect(mocks.constructed[0]).toEqual({ canvas, ...WEBGL_CONTEXT_ATTRS });
  });

  it("turns local clipping ON — without it the section view is a silent no-op", async () => {
    const handle = await createRenderer(document.createElement("canvas"));
    expect((handle.renderer as unknown as { localClippingEnabled: boolean })
      .localClippingEnabled).toBe(true);
    expect(handle.isWebGPU).toBe(false);
  });

  it("states the output color space EXPLICITLY rather than relying on the default", async () => {
    const handle = await createRenderer(document.createElement("canvas"));
    expect((handle.renderer as unknown as { outputColorSpace: string }).outputColorSpace).toBe(
      THREE.SRGBColorSpace,
    );
    expect((handle.renderer as unknown as { toneMappingExposure: number }).toneMappingExposure)
      .toBe(1.0);
  });
});

describe("renderer capability baseline (TEST-BACKEND-01, spec §5)", () => {
  it("records requested AND actual attributes, limits, and clip-control support", async () => {
    const handle = await createRenderer(document.createElement("canvas"));
    const caps = handle.capabilities;
    expect(caps).toBeDefined();
    expect(caps?.backend).toBe("webgl2");
    expect(caps?.attributesRequested).toEqual(WEBGL_CONTEXT_ATTRS);
    expect(caps?.attributesActual).toEqual({
      antialias: true,
      stencil: true,
      depth: true,
      preserveDrawingBuffer: true,
    });
    expect(caps?.depthBits).toBe(24);
    expect(caps?.stencilBits).toBe(8);
    expect(caps?.samples).toBe(4);
    expect(caps?.antialias).toBe(true);
    expect(caps?.maxTextureSize).toBe(16384);
    expect(caps?.maxRenderbufferSize).toBe(16384);
    // Recorded only — reversed depth is NOT switched on (its lane does not exist).
    expect(caps?.reversedDepthSupported).toBe(true);
    expect(caps?.depthConvention).toBe("conventional");
  });

  it("queries the context ONCE at construction and never again per frame", async () => {
    const handle = await createRenderer(document.createElement("canvas"));
    const afterConstruction = mocks.getParameter.mock.calls.length;
    expect(afterConstruction).toBeGreaterThan(0);

    // Three real submissions through the handle — `getParameter` is a pipeline
    // stall, and spec §5 allows the query at construction only.
    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera();
    for (let i = 0; i < 3; i++) {
      handle.renderer.setPixelRatio(2);
      handle.renderer.setSize(900, 500, false);
      handle.renderer.render(scene, camera);
    }
    expect(mocks.getParameter.mock.calls.length - afterConstruction).toBe(0);

    // …and the record is per-CONTEXT: a second renderer queries its own.
    await createRenderer(document.createElement("canvas"));
    expect(mocks.getParameter.mock.calls.length).toBe(afterConstruction * 2);
  });

  it("a saved experimentalWebGpu preference selects WebGL and warns ONCE per session", async () => {
    const first = await createRenderer(document.createElement("canvas"), {
      experimentalWebGpu: true,
    });
    expect(first.isWebGPU).toBe(false);
    expect(first.capabilities?.backend).toBe("webgl2");
    expect(first.capabilities?.backendNote).toBe("webgpu-preference-ignored");
    expect(logs.warn).toHaveBeenCalledTimes(1);
    expect(logs.warn.mock.calls[0][1]).toBe(
      "experimental WebGPU preference ignored: capability suite not passed",
    );

    logs.warn.mockClear();
    const second = await createRenderer(document.createElement("canvas"), {
      experimentalWebGpu: true,
    });
    expect(second.isWebGPU).toBe(false);
    expect(second.capabilities?.backendNote).toBe("webgpu-preference-ignored");
    expect(logs.warn).not.toHaveBeenCalled(); // once-per-session latch
  });

  it("carries no backendNote when the preference is off", async () => {
    const handle = await createRenderer(document.createElement("canvas"));
    expect(handle.capabilities?.backendNote).toBeUndefined();
    expect(logs.warn).not.toHaveBeenCalled();
  });

  it("only an explicit allowUnsupportedBackends can even attempt WebGPU", async () => {
    // No navigator.gpu in jsdom, so the attempt falls back to WebGL — the point
    // is that the DIAGNOSTIC path is not taken and nothing is marked ignored.
    const handle = await createRenderer(document.createElement("canvas"), {
      experimentalWebGpu: true,
      allowUnsupportedBackends: true,
    });
    expect(handle.isWebGPU).toBe(false);
    expect(handle.capabilities?.backendNote).toBeUndefined();
  });
});
