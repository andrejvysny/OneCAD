import { promises as fs } from "node:fs";
import { test, expect } from "./fixtures";
import type { Page, TestInfo } from "@playwright/test";
import { CANVAS, openEditorDebug, waitForCameraSettled } from "./helpers";
import { waitForRenderedFrame } from "./modelToolHelpers";

// `testInfo.attach(name, { body })` is never written to disk under this
// project's "list"/"line" reporter (see e2e/fixtures.ts's own doc comment on
// this) — write the file ourselves and push it onto `testInfo.attachments`
// directly, same as `fixtures.ts`'s `attachFile`.
async function attachJson(testInfo: TestInfo, name: string, value: unknown): Promise<void> {
  const body = JSON.stringify(value, null, 2);
  const path = testInfo.outputPath(name);
  await fs.writeFile(path, body, "utf8");
  testInfo.attachments.push({ name, contentType: "application/json", path });
  // Also on stdout so a plain `tee`'d run log carries the measured numbers.
  console.log(`[${name}]`, body);
}

/*
 * TEST-LINE-01 — VP-HARDENING VP03 line-unit contract, real WebGL (lane G).
 *
 * Measures the RENDERED CSS width of a body feature edge (a `LineSegments2`
 * fat line drawn through its own `onBeforeRender`, which pins
 * `material.resolution` to the renderer's LOGICAL viewport every frame — see
 * src/viewport/engine/screenLineStyle.ts for the installed-source citations)
 * at device-pixel-ratio 1 and 2, by reading back the actual pixels of the
 * WebGL canvas rather than trusting the authored constant.
 *
 * History: at the design baseline (65b4c60) the authored width was multiplied
 * by the device pixel ratio before reaching `linewidth`, so a DPR-2 display
 * drew the edge ~1.8× too wide — measured 2.25 CSS px against 1.25 authored
 * (docs/qa/viewport-hardening/baseline/vph-g-lane-probes-at-65b4c60.log).
 * WP01 made every fat-line width CSS (`LINE_WIDTHS_CSS.bodyFeatureEdge`), so
 * all three cases below are plain passes now; a regression here means a
 * device-pixel term crept back onto the line path.
 *
 * No existing e2e spec imports a `src/` constant (see e2e/helpers.ts and every
 * e2e/*.spec.ts), so the authored CSS width is hard-coded here per that
 * convention instead, citing the source constant.
 *
 * PHASED MEASUREMENT (ACCEPTANCE §3.1 — "repeated subpixel positions, not one
 * lucky aligned line"): a single scan measures the edge's coverage-weighted
 * width at whatever sub-pixel phase the edge happens to land on for that exact
 * camera pose, which biases the integral toward however that ONE phase's
 * antialiasing happened to fall. `measurePhased` takes FIVE measurements after
 * five tiny monotonic zoom-in wheel steps (each of which nudges the ortho
 * scale and therefore the edge's screen x by a non-integer device-pixel
 * amount) and reports `{ perPhase, mean, min, max }`; `cssWidth` is the mean
 * across phases.
 *
 * WHEEL-DELTA PARITY IS LOAD-BEARING AT DPR 2. Root-caused empirically (see
 * the "DPR-2 identical phases" investigation): Playwright's `page.mouse.wheel`
 * dispatches a `WheelEvent` whose `deltaY` is HALVED at `deviceScaleFactor: 2`
 * (confirmed by listening for the real `wheel` event on the canvas — a
 * requested `-9` arrives as `-4.5`). This app's own device auto-detection
 * (`src/viewport/engine/navInput.ts` `weighEvidence`) treats a NON-INTEGER
 * wheel delta as trackpad evidence — correctly, for a real trackpad — which
 * flips `prior` to `"trackpad"` for the rest of the session once a single
 * fractional delta lands (`settlePrior`'s margin check), silently turning
 * every subsequent "zoom" wheel call into a vertical PAN instead (verified via
 * `navReduce` fed the exact captured deltas: device flips `mouse`→`trackpad`
 * on the second event and never flips back). A vertical pan does not move a
 * VERTICAL edge's x position at all, which is exactly why DPR-2 runs measured
 * five bit-identical phases. The fix is a genuine one, not a paper-over: every
 * wheel delta below is a MULTIPLE OF 2, so halving at DPR 2 always yields
 * another integer and the classifier keeps reading "mouse" for the whole
 * phase sweep, at both DPRs.
 */

const BODY_EDGE_WIDTH_CSS = 1.25; // src/viewport/engine/screenLineStyle.ts LINE_WIDTHS_CSS.bodyFeatureEdge
const WIDTH_TOLERANCE_CSS = 0.25; // spec §3.1
const DELTA_TOLERANCE_CSS = 0.2;
// Only a gross-instability guard (an edge-finder that grabbed a wider dark run
// shows up as a spread of whole pixels); genuine sub-pixel phase variance of a
// 1.25 px line measured 0.17 CSS px at DPR 2 and is expected.
const PHASE_SPREAD_SANITY_CSS = 1.0;
/** All multiples of 2 — see the wheel-delta-parity note above. */
const ZOOM_STEP_DELTAS = [-6, -8, -10, -12, -14] as const;
const PHASE_COUNT = ZOOM_STEP_DELTAS.length;

// Mirrors playwright.config.ts's own PORT/viewport resolution — needed here
// because the DPR-comparison test opens its OWN browser contexts (a
// `deviceScaleFactor` change mid-session has no page-level API; it is a
// context-creation option) rather than reusing the fixture-provided `page`.
const PORT = Number(process.env.E2E_PORT ?? 4177);
const BASE_URL = `http://localhost:${PORT}`;
const VIEWPORT = { width: 1280, height: 800 };

/** One phase's raw coverage read — no averaging, no CSS conversion baked in. */
interface CoverageSample {
  /** Linear-light coverage integral (spec §3.1's "integrated contrast" — the
   *  physically correct one; used for `cssWidth`). */
  coverageDevicePx: number;
  /** The ORIGINAL gamma-space-luminance coverage integral, kept for
   *  before/after comparison, not used for any assertion. */
  coverageGammaSpaceDevicePx: number;
  /** Sub-pixel x of the detected left edge, averaged over the sampled rows
   *  (DEVICE px, in the snapshot canvas's own coordinate space). */
  centerDevicePx: number;
  dpr: number;
  rendererDpr: number;
  webgl2Attributes: WebGLContextAttributes | null;
  msaaSamples: number | null;
  /** `WEBGL_debug_renderer_info`'s `UNMASKED_RENDERER_WEBGL` when the
   *  extension is available (names the real GPU/software renderer, e.g.
   *  SwiftShader); `null` when the extension is unavailable. */
  rendererUnmasked: string | null;
  /** `gl.getParameter(gl.RENDERER)` — Chromium returns the generic, masked
   *  string "WebKit WebGL" here regardless of the underlying renderer unless
   *  the debug extension above is used; kept for comparison. */
  rendererMasked: string | null;
}

/** Five-phase measurement: the required shape is `{ perPhase, mean, min, max }`
 *  (ACCEPTANCE §3.1), plus the renderer/DPR facts from the LAST phase (they do
 *  not vary phase to phase) and `cssWidth` = `mean`, kept for the existing
 *  tolerance assertions below. */
interface EdgeWidthMeasurement {
  perPhase: number[];
  mean: number;
  min: number;
  max: number;
  cssWidth: number;
  /** The old gamma-space measurement, phase-by-phase and averaged, purely for
   *  comparison against the linear-light `perPhase`/`mean` above. */
  perPhaseGammaSpace: number[];
  meanGammaSpace: number;
  /** Sub-pixel edge centre (DEVICE px) per phase — proves the zoom steps
   *  actually moved the edge rather than measuring one position 5 times. */
  centersDevicePx: number[];
  dpr: number;
  rendererDpr: number;
  webgl2Attributes: WebGLContextAttributes | null;
  msaaSamples: number | null;
  rendererUnmasked: string | null;
  rendererMasked: string | null;
}

/** Boot the mock box, orient it top-down, and switch to orthographic — the
 *  view under which a box's silhouette edges are exactly two vertical lines
 *  with no perspective foreshortening to bias the width measurement. */
async function orientTopOrtho(page: Page): Promise<void> {
  await openEditorDebug(page, { mockBody: true });
  await page.getByRole("button", { name: "TOP view" }).click();
  await page.getByRole("tab", { name: "Ortho" }).click();
  await waitForCameraSettled(page);
  await waitForRenderedFrame(page);
}

/**
 * Read back the WebGL canvas (`preserveDrawingBuffer: true`, renderer.ts) into
 * a 2D canvas and measure the left silhouette edge's coverage-weighted width,
 * in DEVICE px, at three horizontal rows (middle, ±40 device px), for ONE
 * camera pose (one sub-pixel phase). `measurePhased` below is what converts
 * this into the required multi-phase measurement.
 */
async function measureCoverage(page: Page): Promise<CoverageSample> {
  return page.evaluate(() => {
    const canvas = document.querySelector(
      '[data-testid="viewport-canvas"] canvas',
    ) as HTMLCanvasElement | null;
    if (!canvas) throw new Error("no viewport canvas");

    const w = canvas.width;
    const h = canvas.height;
    const rendererDpr = w / canvas.clientWidth;

    const snap = document.createElement("canvas");
    snap.width = w;
    snap.height = h;
    const ctx2d = snap.getContext("2d");
    if (!ctx2d) throw new Error("no 2d context for snapshot canvas");
    ctx2d.drawImage(canvas, 0, 0);

    const hexToRgb = (hex: string): [number, number, number] => {
      const m = hex.trim().replace("#", "");
      return [parseInt(m.slice(0, 2), 16), parseInt(m.slice(2, 4), 16), parseInt(m.slice(4, 6), 16)];
    };
    // Old, GAMMA-space luminance — kept only for the `coverageGammaSpace`
    // before/after comparison value. Rec.601-style luma weights applied
    // directly to the 0-255 sRGB-encoded channel, i.e. NOT physically a
    // luminance at all (that's the bug this whole change fixes).
    const gammaLuminance = ([r, g, b]: [number, number, number]): number =>
      0.299 * r + 0.587 * g + 0.114 * b;

    // sRGB transfer function inverse (IEC 61966-2-1) — decode an 8-bit sRGB
    // channel to LINEAR light before doing any interpolation/blending math on
    // it. The canvas is sRGB-encoded output and MSAA resolve + the fat line's
    // AA blend both happen in linear light, so an antialiased pixel's coverage
    // fraction only round-trips correctly through this decode: gamma-space
    // luminance interpolation systematically UNDER-reads partial coverage
    // (0.5 linear coverage of a dark-over-light edge displays at ≈73% sRGB
    // brightness, so a gamma-space read of that pixel comes back ≈0.27, not
    // 0.5) — exactly a constant per-pixel shortfall on a thin line.
    const srgbToLinear = (c: number): number => {
      const cs = c / 255;
      return cs <= 0.04045 ? cs / 12.92 : Math.pow((cs + 0.055) / 1.055, 2.4);
    };
    // Rec.709 luminance weights — correct ONLY on linear-light values.
    const linearLuminance = ([r, g, b]: [number, number, number]): number =>
      0.2126 * srgbToLinear(r) + 0.7152 * srgbToLinear(g) + 0.0722 * srgbToLinear(b);

    const rootStyle = getComputedStyle(document.documentElement);
    const faceRgb = hexToRgb(rootStyle.getPropertyValue("--color-body-fill"));
    const edgeRgb = hexToRgb(rootStyle.getPropertyValue("--color-body-edge"));
    const lumFaceGamma = gammaLuminance(faceRgb);
    const lumEdgeGamma = gammaLuminance(edgeRgb);
    const lumFaceLinear = linearLuminance(faceRgb);
    const lumEdgeLinear = linearLuminance(edgeRgb);
    // Midpoint classifier (gamma space is fine here — it only needs to locate
    // the edge's approximate x, not measure its coverage): "closer to the
    // edge color than the face color." The canvas clear color
    // (--color-canvas) is lighter than the face token in both themes, so
    // background pixels land on the FACE side of this threshold and never
    // form a spurious "edge" run — verified against src/styles/tokens.css's
    // light/dark values.
    const threshold = (lumFaceGamma + lumEdgeGamma) / 2;

    const rowSamples = (y: number): { gamma: Float64Array; linear: Float64Array } => {
      const data = ctx2d.getImageData(0, y, w, 1).data;
      const gamma = new Float64Array(w);
      const linear = new Float64Array(w);
      for (let x = 0; x < w; x++) {
        const px: [number, number, number] = [data[x * 4], data[x * 4 + 1], data[x * 4 + 2]];
        gamma[x] = gammaLuminance(px);
        linear[x] = linearLuminance(px);
      }
      return { gamma, linear };
    };

    /** Leftmost contiguous run of edge-ish pixels in the row — the box's LEFT
     *  vertical silhouette edge (there are exactly two on a top-view box). */
    const findLeftEdgeCenter = (gamma: Float64Array): number | null => {
      let runStart = -1;
      for (let x = 0; x < gamma.length; x++) {
        const edgeish = gamma[x] < threshold;
        if (edgeish && runStart === -1) runStart = x;
        if (!edgeish && runStart !== -1) return (runStart + (x - 1)) / 2;
      }
      return null;
    };

    /** Coverage-based sub-pixel width: for each pixel near the edge, how much
     *  of it is "edge" rather than "face", linearly interpolated by luminance
     *  between the two tokens and clamped to [0, 1]. Summed, this is the
     *  effective drawn width in DEVICE px — the same trick coverage-based
     *  antialiasing metrics use in reverse. `lumFace`/`lumEdge` and `lum` must
     *  all be in the SAME space (gamma or linear) for this interpolation to
     *  mean anything physically. */
    const coverageAt = (lum: Float64Array, center: number, lumFace: number, lumEdge: number): number => {
      let sum = 0;
      for (let dx = -12; dx <= 12; dx++) {
        const x = Math.round(center) + dx;
        if (x < 0 || x >= lum.length) continue;
        const frac = (lumFace - lum[x]) / (lumFace - lumEdge);
        sum += Math.min(1, Math.max(0, frac));
      }
      return sum;
    };

    const midY = Math.round(h / 2);
    const linearCoverages: number[] = [];
    const gammaCoverages: number[] = [];
    const centers: number[] = [];
    for (const y of [midY - 40, midY, midY + 40]) {
      if (y < 0 || y >= h) continue;
      const { gamma, linear } = rowSamples(y);
      const center = findLeftEdgeCenter(gamma);
      if (center === null) continue;
      centers.push(center);
      linearCoverages.push(coverageAt(linear, center, lumFaceLinear, lumEdgeLinear));
      gammaCoverages.push(coverageAt(gamma, center, lumFaceGamma, lumEdgeGamma));
    }
    if (linearCoverages.length === 0) throw new Error("no left body edge found in any sampled row");
    const avg = (xs: number[]): number => xs.reduce((a, b) => a + b, 0) / xs.length;
    const coverageDevicePx = avg(linearCoverages);
    const coverageGammaSpaceDevicePx = avg(gammaCoverages);
    const centerDevicePx = avg(centers);

    const gl = canvas.getContext("webgl2") as WebGL2RenderingContext | null;
    const webgl2Attributes = gl?.getContextAttributes() ?? null;
    const msaaSamples = gl ? (gl.getParameter(gl.SAMPLES) as number) : null;
    const debugInfo = gl?.getExtension("WEBGL_debug_renderer_info") ?? null;
    const rendererUnmasked = debugInfo ? String(gl!.getParameter(debugInfo.UNMASKED_RENDERER_WEBGL)) : null;
    const rendererMasked = gl ? String(gl.getParameter(gl.RENDERER)) : null;

    return {
      coverageDevicePx,
      coverageGammaSpaceDevicePx,
      centerDevicePx,
      dpr: window.devicePixelRatio,
      rendererDpr,
      webgl2Attributes,
      msaaSamples,
      rendererUnmasked,
      rendererMasked,
    };
  });
}

/**
 * Take {@link PHASE_COUNT} coverage measurements at DIFFERENT sub-pixel
 * phases, by nudging the ortho zoom by a small alternating amount between
 * reads — each step changes the world-to-screen scale slightly, so the edge's
 * screen x lands at a different fractional device-pixel offset each time
 * (ACCEPTANCE §3.1: "repeated subpixel positions, not one lucky aligned
 * line"). `cssWidth` is the MEAN across phases; `perPhase`/`min`/`max` are
 * kept in the returned measurement (and the test's JSON attachment) as the raw
 * evidence.
 */
async function measurePhased(page: Page): Promise<EdgeWidthMeasurement> {
  const box = await page.locator(CANVAS).boundingBox();
  if (!box) throw new Error("no canvas box");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);

  const perPhase: number[] = [];
  const perPhaseGammaSpace: number[] = [];
  const centersDevicePx: number[] = [];
  let last: CoverageSample | null = null;
  for (const dy of ZOOM_STEP_DELTAS) {
    await page.mouse.wheel(0, dy);
    await waitForCameraSettled(page);
    await waitForRenderedFrame(page);
    const sample = await measureCoverage(page);
    last = sample;
    perPhase.push(sample.coverageDevicePx / sample.rendererDpr);
    perPhaseGammaSpace.push(sample.coverageGammaSpaceDevicePx / sample.rendererDpr);
    centersDevicePx.push(sample.centerDevicePx);
  }
  if (!last) throw new Error("no phase measured");

  const avg = (xs: number[]): number => xs.reduce((a, b) => a + b, 0) / xs.length;
  const mean = avg(perPhase);
  const min = Math.min(...perPhase);
  const max = Math.max(...perPhase);

  return {
    perPhase,
    mean,
    min,
    max,
    cssWidth: mean,
    perPhaseGammaSpace,
    meanGammaSpace: avg(perPhaseGammaSpace),
    centersDevicePx,
    dpr: last.dpr,
    rendererDpr: last.rendererDpr,
    webgl2Attributes: last.webgl2Attributes,
    msaaSamples: last.msaaSamples,
    rendererUnmasked: last.rendererUnmasked,
    rendererMasked: last.rendererMasked,
  };
}

/** Fail loudly, naming the actual centres, if the zoom steps did not move the
 *  edge at all — the exact DPR-2 failure mode root-caused above. A distinct
 *  set means at least one pair of phases differs by a nontrivial sub-pixel
 *  amount (not just float noise from re-measuring the same position). */
function assertCentersMoved(centers: number[], label: string): void {
  const distinct = new Set(centers.map((c) => c.toFixed(3))).size;
  expect(
    distinct,
    `${label}: edge centre never moved across phases (centersDevicePx=${JSON.stringify(centers)}) — ` +
      `the zoom steps had no effect (see the wheel-delta-parity note above)`,
  ).toBeGreaterThan(1);
}

test.describe("TEST-LINE-01 body edge width — DPR 1", () => {
  test.use({ deviceScaleFactor: 1 });

  test("TEST-LINE-01: body edge renders at the authored CSS width at DPR 1", async ({ page }, testInfo) => {
    test.setTimeout(60_000);
    await orientTopOrtho(page);
    const m = await measurePhased(page);
    await attachJson(testInfo, "edge-width-dpr1.json", m);
    assertCentersMoved(m.centersDevicePx, "DPR 1");
    expect(Math.abs(m.cssWidth - BODY_EDGE_WIDTH_CSS)).toBeLessThanOrEqual(WIDTH_TOLERANCE_CSS);
  });
});

test.describe("TEST-LINE-01 body edge width — DPR 2", () => {
  test.use({ deviceScaleFactor: 2 });

  test("TEST-LINE-01: body edge renders at the authored CSS width at DPR 2", async ({ page }, testInfo) => {
    test.setTimeout(60_000);
    await orientTopOrtho(page);
    const m = await measurePhased(page);
    await attachJson(testInfo, "edge-width-dpr2.json", m);
    // Sanity check on the phasing itself: the five wheel-zoom steps must
    // actually have moved the edge to different sub-pixel phases rather than
    // measuring one degenerate, unmoved position five times. The per-phase
    // spread is RECORDED (perPhase/min/max in the attachment) but not bounded
    // above: measured spreads of 0.17 CSS px at DPR 2 are genuine sub-pixel
    // phase variance of a 1.25 px line under 4× MSAA, which is exactly why
    // the width assertion uses the phase MEAN (ACCEPTANCE §3.1).
    assertCentersMoved(m.centersDevicePx, "DPR 2");
    const spread = m.max - m.min;
    expect(spread, "phase spread must be a finite number").toBeGreaterThanOrEqual(0);
    expect(spread).toBeLessThan(PHASE_SPREAD_SANITY_CSS);
    expect(Math.abs(m.cssWidth - BODY_EDGE_WIDTH_CSS)).toBeLessThanOrEqual(WIDTH_TOLERANCE_CSS);
  });
});

test("TEST-LINE-01: body edge CSS width is DPR-independent", async ({ browser }, testInfo) => {
  test.setTimeout(120_000);

  const measureAt = async (deviceScaleFactor: number): Promise<EdgeWidthMeasurement> => {
    const context = await browser.newContext({ baseURL: BASE_URL, viewport: VIEWPORT, deviceScaleFactor });
    try {
      const p = await context.newPage();
      await orientTopOrtho(p);
      return await measurePhased(p);
    } finally {
      await context.close();
    }
  };

  const m1 = await measureAt(1);
  const m2 = await measureAt(2);

  await attachJson(testInfo, "edge-width-dpr-comparison.json", { dpr1: m1, dpr2: m2 });

  expect(Math.abs(m1.cssWidth - m2.cssWidth)).toBeLessThanOrEqual(DELTA_TOLERANCE_CSS);
});
