import { test, expect } from "./fixtures";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtAwaitingDofChange,
  getSketchSnapshot,
  planePointToClient,
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

/** Mirrors `GridPlane.ts` `snapToDecade` — 1/2/5/10 ladder, rounds UP. */
function snapToDecade(v: number): number {
  const value = Math.max(v, 1e-6);
  const pow = Math.pow(10, Math.floor(Math.log10(value)));
  const n = value / pow;
  const m = n <= 1 ? 1 : n <= 2 ? 2 : n <= 5 ? 5 : 10;
  return m * pow;
}

/** Mirrors `GridPlane.ts` `chooseGridStep().minor` (CELLS_ACROSS = 25). */
function gridStepMm(camDist: number): number {
  return snapToDecade(Math.max(camDist, 1) / 25);
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
  return Math.max(decadeFloor(gridStepMm(camDist) / 10), 0.001);
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

  // ── dimensionRound ON, GRID OFF: a scruffy pixel offset still lands exactly
  //    on a multiple of the quantum ────────────────────────────────────────────
  //
  // The grid has to be off for this half to isolate what it claims to isolate,
  // and that is a POLICY CHANGE this spec predates rather than a workaround.
  // SNAP P2 replaced the single-winner ladder with composable candidates, in
  // which the grid produces a FULL POINT and `composePoint` states that "a full
  // point claims everything; nothing else may be in the set". So with the grid
  // on, a click resolves to a grid point and the numeric fields are dropped —
  // measured, from the engine's own trace at the commit click:
  //
  //   accepted  grid:19:-10:1
  //   rejected  numeric:length:0.1  reason "claim-conflict"
  //             numeric:angle:1     reason "claim-conflict"
  //
  // …which lands p0 (-29, 12) → p1 (19, -10): both endpoints exactly on the grid
  // (1 mm at the zoom this trace was captured, coarser since `CELLS_ACROSS`
  // widened the cells), and a length of hypot(48, 22) = 52.8015… that is a multiple of no
  // quantum at all. That is correct behaviour — grid snap wins a click, as it
  // does in every CAD — and the third assertion at the bottom now pins it, so it
  // is covered rather than silently traded away. What this spec's own comment
  // still cited, `dimensionRoundingActive()`, was DELETED in P2 along with the
  // rule that rounding "replaces the grid tier".
  await setSnapPref(page, "grid", false);

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
  // The grid is already off from the first half; with `dimensionRound` off too,
  // nothing claims the point and the raw projection survives.
  await setSnapPref(page, "dimensionRound", false);

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

  // ── grid ON: an accepted grid point OUT-CLAIMS the numeric fields (SNAP P2) ─
  // The other side of the policy quoted above, asserted rather than assumed.
  //
  // The click has to land ON a grid intersection for this to be deterministic:
  // the grid candidate only exists within `gridReachPx` (min of the user's point
  // reach and 0.35 × the projected cell), so a coordinate that is not a multiple
  // of the CURRENT step is usually out of its reach and the numeric fields win
  // instead. That is not a second policy, it is the same one seen from the other
  // side, and it is why this case aims at exact grid nodes — derived from the
  // live camera distance, since the step is zoom-adaptive — rather than at a
  // screen offset or a hardcoded plane coordinate.
  await setSnapPref(page, "dimensionRound", true);
  await setSnapPref(page, "grid", true);

  const step = gridStepMm(camDist);
  const node = (mm: number): number => step * Math.round(mm / step);
  const plane = (await getSketchSnapshot(page)).plane;
  // Well clear of both earlier lines: a click inside the endpoint snap radius of
  // an existing point welds to it (Coincident) and the DOF would not move.
  const a = await planePointToClient(page, plane, { x: node(-46), y: node(27) });
  const b = await planePointToClient(page, plane, { x: node(-18), y: node(33) });
  // move → down → up, as `clickAt` does: the tools steer off pointermove, and a
  // bare `mouse.click()` pair also reads as a double-click at these distances.
  await page.mouse.move(a.x, a.y);
  await page.mouse.down();
  await page.mouse.up();
  await page.mouse.move(b.x, b.y);
  await expect
    .poll(async () => (await getSketchSnapshot(page)).lines.length, { timeout: 8000 })
    .toBe(2);
  await page.mouse.down();
  await page.mouse.up();
  await expect
    .poll(async () => (await getSketchSnapshot(page)).lines.length, { timeout: 8000 })
    .toBe(3);
  await page.keyboard.press("Escape");

  const trace = await page.evaluate(() => {
    const engine = (
      window as unknown as {
        __vpEngine?: {
          debugSnapshot(): {
            snapTrace?: {
              gridStep?: number;
              acceptedIds?: string[];
              rejected?: Array<{ candidateId: string; reason: string }>;
            };
          };
        };
      }
    ).__vpEngine;
    return engine?.debugSnapshot().snapTrace ?? null;
  });
  expect(trace?.acceptedIds?.some((id) => id.startsWith("grid:"))).toBe(true);
  // …and with the point claimed, the length/angle candidates are dropped BY NAME.
  const numericRejections = (trace?.rejected ?? []).filter((r) => r.candidateId.startsWith("numeric:"));
  expect(numericRejections.length).toBeGreaterThan(0);
  for (const r of numericRejections) expect(r.reason).toBe("claim-conflict");

  const snap3 = await getSketchSnapshot(page);
  expect(snap3.lines).toHaveLength(3);
  const line3 = snap3.lines[2];
  const gridStep = trace?.gridStep ?? 0;
  expect(gridStep).toBeGreaterThan(0);
  const onGrid = (v: number): boolean => Math.abs(v - gridStep * Math.round(v / gridStep)) < 1e-6;
  expect(onGrid(line3.p1[0])).toBe(true);
  expect(onGrid(line3.p1[1])).toBe(true);
});
