/*
 * Renderer construction — the SOLE place a WebGL/WebGPU renderer is created.
 *
 * WebGL2 is the SUPPORTED PRODUCTION BACKEND and the only one the app selects
 * (VP-HARDENING VP01, spec §5). The WebGPU path still exists, but it is reachable
 * only behind an explicit `allowUnsupportedBackends` — tests and diagnostics —
 * never from a user preference: a half-working editing viewport (no fat lines,
 * no clipping, no PMREM) is worse than the tested one. A saved
 * `experimentalWebGpu` preference therefore falls back to WebGL with a
 * once-per-session diagnostic and a `backendNote`, not an empty viewport.
 *
 * Actual context capabilities are recorded ONCE at construction (spec §5:
 * "Querying these once is acceptable; querying them in every frame is not") and
 * exposed on the handle. Requested attributes and actual attributes are kept as
 * two separate facts, because they routinely differ.
 */
import * as THREE from "three";
import { logWarn } from "@/debug/log";
import { palette } from "./palette";

/** The subset of renderer API the engine relies on (WebGL and WebGPU both satisfy it). */
export interface CadRenderer {
  domElement: HTMLCanvasElement;
  setSize(width: number, height: number, updateStyle?: boolean): void;
  setPixelRatio(ratio: number): void;
  setClearColor(color: THREE.ColorRepresentation, alpha?: number): void;
  render(scene: THREE.Scene, camera: THREE.Camera): void | Promise<void>;
  dispose(): void;
}

export interface RendererPrefs {
  /**
   * The user's saved experimental preference. It NO LONGER selects a backend —
   * it only produces the once-per-session fallback diagnostic. See
   * {@link RendererPrefs.allowUnsupportedBackends}.
   */
  experimentalWebGpu?: boolean;
  /**
   * @internal Diagnostics and the future WebGPU capability lane (WP15/WP16).
   *
   * Permits a backend the capability suite has not passed. NOTHING in the
   * application sets it — no user preference, no setting, no URL flag — so it
   * cannot be the route by which a half-working backend reaches a real
   * document. It exists so the lane that will eventually qualify WebGPU has a
   * supported way in, instead of re-adding the preference gate that VP01 removed.
   */
  allowUnsupportedBackends?: boolean;
}

/** A prefiltered environment map plus the disposer for the render target behind it. */
export interface EnvironmentHandle {
  texture: THREE.Texture;
  dispose(): void;
}

/**
 * What the context actually gave us, read ONCE at construction (spec §5).
 *
 * `attributesRequested` vs `attributesActual` is the load-bearing distinction:
 * asking for `antialias: true` and `stencil: true` does not mean the driver
 * granted either, and the capped-section algorithm silently degrades without a
 * stencil buffer. Recording both is what lets a failure be diagnosed instead of
 * guessed at.
 */
export interface RendererCapabilities {
  readonly backend: "webgl2" | "webgpu";
  readonly attributesRequested: typeof WEBGL_CONTEXT_ATTRS;
  /** `null` when the context refused to report (or there is no GL context at all). */
  readonly attributesActual: WebGLContextAttributes | null;
  readonly depthBits: number;
  readonly stencilBits: number;
  /** MSAA sample count of the default framebuffer (0 when not multisampled). */
  readonly samples: number;
  readonly antialias: boolean;
  readonly maxTextureSize: number;
  readonly maxRenderbufferSize: number;
  /**
   * `EXT_clip_control` is available, so three's `reversedDepthBuffer` COULD be
   * used. Recorded only — reversed depth is NOT enabled: the spec requires its
   * test lane to pass first (§5), and that lane does not exist yet.
   */
  readonly reversedDepthSupported: boolean;
  readonly depthConvention: "conventional";
  /** Set when a requested backend was refused (`"webgpu-preference-ignored"`). */
  readonly backendNote?: string;
}

export interface RendererHandle {
  renderer: CadRenderer;
  isWebGPU: boolean;
  /** The one capability record for this context. Absent on mocked handles. */
  capabilities?: RendererCapabilities;
  dispose(): void;
  /**
   * Prefilter `source` into an environment map (PMREM).
   *
   * WebGL ONLY — `THREE.PMREMGenerator` takes a `WebGLRenderer`. The method is
   * simply ABSENT on the WebGPU handle (and on mocked handles in unit tests),
   * which is the guard: callers do `handle.createEnvironment?.(scene)` and skip
   * IBL entirely when it is undefined. No `isWebGPU` branch belongs in the engine.
   *
   * The caller OWNS the returned handle and must `dispose()` it while the GL
   * context is still alive — `PMREMGenerator.dispose()` does not free the render
   * target that `fromScene` returns.
   */
  createEnvironment?(source: THREE.Scene): EnvironmentHandle | null;
}

async function webGpuAvailable(): Promise<boolean> {
  try {
    // @webgpu/types is not installed; probe structurally to avoid the dep.
    const gpu = (navigator as unknown as {
      gpu?: { requestAdapter(): Promise<unknown> };
    }).gpu;
    if (!gpu) return false;
    const adapter = await gpu.requestAdapter();
    return adapter != null;
  } catch {
    return false;
  }
}

/**
 * The WebGL context attributes, exported so the unit suite can pin them without
 * a GL context (jsdom has none). Two of the four are load-bearing:
 *
 * - `preserveDrawingBuffer` — on-demand rendering means the last frame must keep
 *   displaying while idle, and `ViewportEngine.captureThumbnail` reads it back.
 *   Without it a demand-driven canvas composites blank between renders.
 * - `stencil` — three asks for a stencil buffer by DEFAULT, but stating it here
 *   makes the dependency explicit: `SectionLayer`'s capped cut is a stencil
 *   algorithm and silently degrades to an uncapped hole without one.
 */
export const WEBGL_CONTEXT_ATTRS = {
  antialias: true,
  powerPreference: "high-performance",
  preserveDrawingBuffer: true,
  stencil: true,
} as const;

/** A `getParameter` that never throws and never returns a non-number. */
function glNumber(gl: WebGL2RenderingContext, pname: number): number {
  try {
    const value = gl.getParameter(pname) as unknown;
    return typeof value === "number" && Number.isFinite(value) ? value : 0;
  } catch {
    return 0;
  }
}

/**
 * Read the capability record. Called EXACTLY ONCE per renderer, at construction
 * — every value here is fixed for the life of the context, and `getParameter`
 * is a pipeline stall.
 *
 * Defensive throughout because it also runs against the stubbed renderer in the
 * unit lane, where there is no GL context at all: a missing context yields a
 * record of zeros rather than a thrown init.
 */
function readWebGlCapabilities(
  renderer: THREE.WebGLRenderer,
  backendNote?: string,
): RendererCapabilities {
  let gl: WebGL2RenderingContext | null = null;
  try {
    gl = renderer.getContext() as WebGL2RenderingContext;
  } catch {
    gl = null;
  }
  const attributesActual = (() => {
    try {
      return gl?.getContextAttributes() ?? null;
    } catch {
      return null;
    }
  })();
  const extensions = (() => {
    try {
      return gl?.getSupportedExtensions() ?? [];
    } catch {
      return [];
    }
  })();
  // `renderer.capabilities.reversedDepthBuffer` is three's own answer (0.185.1,
  // WebGLRenderer.js:3635-3658); the extension list is the fallback when the
  // renderer is stubbed. Either way this is RECORDED, never acted on.
  const rendererSaysReversed =
    (renderer.capabilities as unknown as { reversedDepthBuffer?: boolean } | undefined)
      ?.reversedDepthBuffer === true;

  return {
    backend: "webgl2",
    attributesRequested: WEBGL_CONTEXT_ATTRS,
    attributesActual,
    depthBits: gl ? glNumber(gl, gl.DEPTH_BITS) : 0,
    stencilBits: gl ? glNumber(gl, gl.STENCIL_BITS) : 0,
    samples: gl ? glNumber(gl, gl.SAMPLES) : 0,
    antialias: attributesActual?.antialias ?? false,
    maxTextureSize: gl ? glNumber(gl, gl.MAX_TEXTURE_SIZE) : 0,
    maxRenderbufferSize: gl ? glNumber(gl, gl.MAX_RENDERBUFFER_SIZE) : 0,
    reversedDepthSupported: rendererSaysReversed || extensions.includes("EXT_clip_control"),
    // Reversed depth stays OFF until its acceptance lane exists (spec §5).
    depthConvention: "conventional",
    ...(backendNote ? { backendNote } : {}),
  };
}

function createWebGl(canvas: HTMLCanvasElement, backendNote?: string): RendererHandle {
  const renderer = new THREE.WebGLRenderer({ canvas, ...WEBGL_CONTEXT_ATTRS });
  // Material-local clipping planes (`material.clippingPlanes`, which is how
  // section view clips ONLY the committed bodies) are skipped ENTIRELY when this
  // is false — no warning, no error, just an unclipped scene. Off by default.
  renderer.localClippingEnabled = true;
  renderer.setClearColor(palette.clear(), 1);
  // Studio look: Neutral (Khronos PBR neutral) compresses highlights without the
  // filmic color shift ACES imposes, so a body's albedo token still reads as
  // itself. Exposure 1.0 — the light rig, not the exposure, sets the level.
  renderer.toneMapping = THREE.NeutralToneMapping;
  renderer.toneMappingExposure = 1.0;
  // Stated EXPLICITLY (spec §5) even though 0.185.1 already defaults to it
  // (WebGLRenderer.js:304): the whole color pipeline — token colors authored in
  // sRGB, `MeshStandardMaterial` working in linear — is wrong end-to-end if a
  // future default flips, and a silently linear output looks merely "washed out"
  // rather than broken.
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  const capabilities = readWebGlCapabilities(renderer, backendNote);
  return {
    renderer,
    isWebGPU: false,
    capabilities,
    dispose() {
      // forceContextLoss frees the GL context promptly (StrictMode re-inits).
      renderer.forceContextLoss();
      renderer.dispose();
    },
    createEnvironment(source: THREE.Scene): EnvironmentHandle | null {
      const pmrem = new THREE.PMREMGenerator(renderer);
      const rt = pmrem.fromScene(source, 0.04);
      pmrem.dispose(); // frees the generator's scratch targets, NOT `rt`
      return { texture: rt.texture, dispose: () => rt.dispose() };
    },
  };
}

async function createWebGpu(canvas: HTMLCanvasElement): Promise<RendererHandle> {
  // Dynamic import keeps the WebGPU build out of the default chunk.
  const { WebGPURenderer } = await import("three/webgpu");
  const renderer = new WebGPURenderer({ canvas, antialias: true });
  await renderer.init();
  renderer.setClearColor(palette.clear(), 1);
  return {
    renderer: renderer as unknown as CadRenderer,
    isWebGPU: true,
    dispose() {
      renderer.dispose();
    },
  };
}

/**
 * Once-per-session latch for the ignored-preference diagnostic. Module level on
 * purpose: a StrictMode remount constructs a second renderer, and the user does
 * not need to be told twice.
 */
let webGpuPreferenceWarned = false;

/**
 * Construct a renderer for `canvas`. Returns a handle whose `dispose()` fully
 * releases GPU resources.
 *
 * WebGL2 is ALWAYS the production selection (spec §5). `experimentalWebGpu`
 * alone can no longer choose WebGPU — it only produces the once-per-session
 * fallback diagnostic and the `webgpu-preference-ignored` note. WebGPU requires
 * `allowUnsupportedBackends`, which nothing in the app sets.
 */
export async function createRenderer(
  canvas: HTMLCanvasElement,
  prefs: RendererPrefs = {},
): Promise<RendererHandle> {
  if (prefs.allowUnsupportedBackends === true && prefs.experimentalWebGpu) {
    if (await webGpuAvailable()) {
      try {
        return await createWebGpu(canvas);
      } catch (err) {
        // Fall through to WebGL — WebGL is the tested path.
        logWarn("vp", "WebGPU init failed, falling back to WebGL", { error: err });
      }
    }
    return createWebGl(canvas);
  }

  let backendNote: string | undefined;
  if (prefs.experimentalWebGpu) {
    backendNote = "webgpu-preference-ignored";
    if (!webGpuPreferenceWarned) {
      webGpuPreferenceWarned = true;
      logWarn(
        "vp",
        "experimental WebGPU preference ignored: capability suite not passed",
        { backend: "webgl2" },
      );
    }
  }
  // The capability record is logged ONCE by the engine, in its `engine init`
  // line — one place, with the backend identity beside it.
  return createWebGl(canvas, backendNote);
}
