import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  CANVAS,
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtAwaitingDofChange,
  getSketchSnapshot,
  planePointToClient,
} from "./helpers";

/*
 * Constraint-glyph standoff is a SCREEN constant (SKETCH_UX_AUDIT.md #5's
 * residual, closed).
 *
 * A glyph badge sits off its entity so it cannot eat a select-tool click meant
 * for the curve underneath. That standoff used to be 10mm of PLANE space baked
 * into the badge's anchor, so it scaled with the camera: measured in this lane
 * it was ~24px off the line at sketch entry and ~94px after one wheel-zoom in —
 * crowding the curve at one end of the range and drifting away at the other.
 * `HtmlOverlayDriver` now applies it per frame as a screen-px offset along the
 * projected perpendicular.
 *
 * This spec measures the real thing: the badge's own DOM box against the line
 * midpoint projected through the live camera (`planePointToClient`), at two
 * zooms. The projected LINE LENGTH is asserted to have grown, so a wheel that
 * failed to zoom cannot make the invariance assertion pass vacuously.
 */

interface Standoff {
  /** Badge centre → projected line midpoint, in page px. */
  centre: number;
  /** Badge's near EDGE → the line, i.e. what a click corridor has to clear. */
  nearEdge: number;
  /** The line's own projected length — the zoom witness. */
  lineLenPx: number;
}

async function measure(page: Page): Promise<Standoff> {
  const snap = await getSketchSnapshot(page);
  const line = snap.lines[0];
  const mid = { x: (line.p0[0] + line.p1[0]) / 2, y: (line.p0[1] + line.p1[1]) / 2 };
  const midClient = await planePointToClient(page, snap.plane, mid);
  const a = await planePointToClient(page, snap.plane, { x: line.p0[0], y: line.p0[1] });
  const b = await planePointToClient(page, snap.plane, { x: line.p1[0], y: line.p1[1] });

  const box = await page.locator("[data-testid^='constraint-badge-']").first().boundingBox();
  if (!box) throw new Error("no constraint badge on canvas");
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;
  return {
    centre: Math.hypot(cx - midClient.x, cy - midClient.y),
    // The line is horizontal on screen, so the standoff is purely vertical and
    // the near edge is half the badge's own height in from its centre.
    nearEdge: Math.abs(cy - midClient.y) - box.height / 2,
    lineLenPx: Math.hypot(b.x - a.x, b.y - a.y),
  };
}

test("a constraint glyph keeps the same screen standoff after zooming in", async ({ page }) => {
  // Defaults, not whatever a previous test persisted (settings carry the chip
  // preference this spec needs on, and the input-device routing the wheel uses).
  await page.addInitScript(() => localStorage.removeItem("onecad.settings"));
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // One horizontal line ⇒ autoconstrain infers exactly one Horizontal, so the
  // canvas carries exactly one glyph badge.
  await selectSketchTool(page, "Line");
  await clickAt(page, -160, -80);
  await clickAtAwaitingDofChange(page, 0, -80);
  await page.keyboard.press("Escape");
  await expect(page.locator("[data-testid^='constraint-badge-']")).toHaveCount(1);

  const before = await measure(page);
  expect(before.nearEdge).toBeGreaterThanOrEqual(15.5); // ≥ the 16px corridor

  const box = await page.locator(CANVAS).boundingBox();
  if (!box) throw new Error("no canvas box");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.wheel(0, -600);
  await waitForCameraSettled(page);

  const after = await measure(page);
  // The camera really moved: the same line is at least 1.5x longer on screen.
  expect(after.lineLenPx / before.lineLenPx).toBeGreaterThan(1.5);
  // …and the badge did not follow it. (Pre-fix this delta measured ~69px.)
  expect(Math.abs(after.centre - before.centre)).toBeLessThanOrEqual(1);
  expect(after.nearEdge).toBeGreaterThanOrEqual(15.5);
});
