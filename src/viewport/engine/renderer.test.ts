/*
 * Renderer construction — the two GL settings the section view depends on, and
 * the one it must not have broken.
 *
 * jsdom has no WebGL, so `THREE.WebGLRenderer` is stubbed: constructing the real
 * one throws before any of this could be read. What is pinned here is exactly
 * what `createWebGl` decides — the context attributes it asks for and the flags
 * it sets afterwards — because all three fail SILENTLY:
 *   - `localClippingEnabled` off ⇒ every `material.clippingPlanes` is skipped
 *     and the scene renders uncut, with no warning,
 *   - no stencil buffer ⇒ the capped cut degrades to a hollow shell,
 *   - `preserveDrawingBuffer` off ⇒ `captureThumbnail` reads back a blank frame
 *     (on-demand rendering means the buffer is not otherwise retained).
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

const mocks = vi.hoisted(() => {
  const constructed: Array<Record<string, unknown>> = [];
  class FakeWebGLRenderer {
    localClippingEnabled = false;
    toneMapping = 0;
    toneMappingExposure = 0;
    domElement: unknown;
    constructor(params: Record<string, unknown>) {
      constructed.push(params);
      this.domElement = params.canvas;
    }
    setClearColor() {}
    forceContextLoss() {}
    dispose() {}
  }
  return { constructed, FakeWebGLRenderer };
});

vi.mock("three", async (importOriginal) => {
  const actual = await importOriginal<typeof import("three")>();
  return { ...actual, WebGLRenderer: mocks.FakeWebGLRenderer };
});

import { createRenderer, WEBGL_CONTEXT_ATTRS } from "./renderer";

beforeEach(() => {
  mocks.constructed.length = 0;
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
});
