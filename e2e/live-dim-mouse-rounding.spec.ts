import { test, expect } from "./fixtures";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtAwaitingDofChange,
  getSketchSnapshot,
  setSnapPref,
} from "./helpers";

/*
 * Zoom-adaptive cursor rounding (SP-1 §3/§8) — no typing at all. A mouse-drawn
 * length always ROUNDS to a quantum derived from the current zoom
 * (`dimQuantum(chooseGridStep(camDist).minor, unit)`, `liveDimension.ts`) while
 * `snapTo.dimensionRound` is on (the default); with it off, the raw perspective
 * -projected length passes straight through.
 *
 * The two pure helpers below MIRROR (not import — Playwright specs don't share
 * the Vite alias graph with `src/`) `chooseGridStep`/`snapToDecade`
 * (`src/viewport/engine/GridPlane.ts`) and `decadeFloor`/`dimQuantum`
 * (`src/tools/sketch/liveDimension.ts`), read against the REAL camera distance
 * (`window.__vpEngine.getCameraDistance()`) so the expected quantum tracks
 * whatever the default view actually is rather than a hardcoded guess.
 */

/** Mirrors `GridPlane.ts` `snapToDecade` — 1/5/10 ladder, rounds UP. */
function snapToDecade(v: number): number {
  const value = Math.max(v, 1e-6);
  const pow = Math.pow(10, Math.floor(Math.log10(value)));
  const n = value / pow;
  const m = n < 2 ? 1 : n < 5 ? 5 : 10;
  return m * pow;
}

/** Mirrors `liveDimension.ts` `decadeFloor` — 1/2/5 ladder, rounds DOWN. */
function decadeFloor(v: number): number {
  if (!Number.isFinite(v) || v <= 0) return 0;
  const pow = Math.pow(10, Math.floor(Math.log10(v)));
  const n = v / pow;
  const eps = 1e-9;
  const m = n >= 5 - eps ? 5 : n >= 2 - eps ? 2 : 1;
  return m * pow;
}

/** Mirrors `liveDimension.ts` `dimQuantum` for the "mm" display unit (decimals=3,
 *  finest = 1e-3 — `src/units/lengthUnits.ts`). */
function lengthQuantumMm(camDist: number): number {
  const minor = snapToDecade(Math.max(camDist, 1) / 50);
  return Math.max(decadeFloor(minor / 10), 0.001);
}

async function getCameraDistance(page: import("@playwright/test").Page): Promise<number> {
  return page.evaluate(() => {
    const engine = (window as unknown as { __vpEngine?: { getCameraDistance(): number } }).__vpEngine;
    if (!engine) throw new Error("getCameraDistance: window.__vpEngine missing (use openEditorDebug)");
    return engine.getCameraDistance();
  });
}

test("a mouse-drawn line rounds to the zoom-adaptive quantum; off it does not", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Line");

  const camDist = await getCameraDistance(page);
  const q = lengthQuantumMm(camDist);
  expect(q).toBeGreaterThan(0);
  const remainder = (len: number): number => {
    const r = len - q * Math.round(len / q);
    return Math.abs(r);
  };

  // ── dimensionRound ON (default): a scruffy pixel offset still lands exactly
  //    on a multiple of the quantum ────────────────────────────────────────────
  await clickAt(page, -137, -58); // anchor at a deliberately un-round offset
  await clickAtAwaitingDofChange(page, 91, 47); // equally un-round — commits
  await page.keyboard.press("Escape"); // end the chain

  const snap1 = await getSketchSnapshot(page);
  expect(snap1.lines).toHaveLength(1);
  const len1 = Math.hypot(
    snap1.lines[0].p1[0] - snap1.lines[0].p0[0],
    snap1.lines[0].p1[1] - snap1.lines[0].p0[1],
  );
  expect(remainder(len1)).toBeLessThan(1e-6);

  // ── dimensionRound OFF: the raw (unrounded) length passes straight through ──
  // Grid is turned off too — with `dimensionRound` alone off, the machine-armed
  // grid tier (`snapAt`'s `enableGrid: settings.snapTo.grid && !dimensionRoundingActive()`)
  // would still quantize x/y independently at a COARSER step, which is not what
  // this half of the spec is isolating.
  await setSnapPref(page, "dimensionRound", false);
  await setSnapPref(page, "grid", false);

  await clickAt(page, -113, 66); // a different scruffy offset
  await clickAtAwaitingDofChange(page, 68, -29);
  await page.keyboard.press("Escape");

  const snap2 = await getSketchSnapshot(page);
  expect(snap2.lines).toHaveLength(2);
  const line2 = snap2.lines[1];
  const len2 = Math.hypot(line2.p1[0] - line2.p0[0], line2.p1[1] - line2.p0[1]);
  // Meaningfully off the quantum grid, not merely "not bit-identical" — guards
  // against a coincidental near-multiple from the raw projection.
  expect(remainder(len2)).toBeGreaterThan(q * 0.05);
});
