import { test, expect } from "./fixtures";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  clickAt,
  clickAtAwaitingDofChange,
  clickAtClient,
  dofPill,
  getSketchSnapshot,
  planePointToClient,
  setSnapPref,
  waitForCameraSettled,
} from "./helpers";

/*
 * Line tool — the REAL user path (plane picker) end to end.
 *
 * Draws a 3-segment polyline: the first click drops the anchor, each subsequent
 * click commits a segment. Every committed segment round-trips through
 * sketchUpsert → the mock solver, and autoConstrain infers a Coincident at every
 * shared endpoint (the chain reuses the exact snapped point). Between committing
 * clicks we settle on the DOF pill so the async commits can't clobber each other.
 * Assertions go through store-driven chrome: DOF pill + inspector Constraints.
 */
test("line tool draws a 3-segment chain with autoconstraint feedback", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);

  // Fresh empty sketch: fully constrained, DOF 0.
  await expect(dofPill(page)).toHaveText("DOF: 0");

  // Anchor + 3 chained segments (canvas-center-relative offsets, well spaced and
  // clear of the floating panels). Esc ends the chain.
  await clickAt(page, -160, -80);
  await clickAtAwaitingDofChange(page, -40, -80);
  await clickAtAwaitingDofChange(page, 60, 10);
  await clickAtAwaitingDofChange(page, 160, 90);
  await page.keyboard.press("Escape");

  // Entities exist → DOF is now non-zero (3 lines, minus autoconstraints).
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  // Chrome + inspector reflect the entities: under-constrained + Coincident rows
  // from autoconstrain (chained endpoints snapped exactly onto each other).
  // (chrome bar + inspector card both render the status; either is a valid signal)
  await expect(page.getByText(/^Under-constrained · DOF [1-9]/).first()).toBeVisible();
  // Per-row ConstraintList: a 3-segment chain infers one Coincident per shared
  // endpoint, so assert at least one row rather than exact-single text. The row
  // names its geometry now (S10), so match the prefix, not the bare kind.
  await expect(
    page.locator('[data-testid^="constraint-row-"]').filter({ hasText: "Coincident" }).first(),
  ).toBeVisible();
});

/*
 * A-16 — the mock lane's upsert write-back, which is the behaviour the REAL lane
 * gains from the worker's solved positions (WP-2 wire).
 *
 * A line drawn 1° off horizontal auto-infers `Horizontal` (`autoConstrain`'s ±5°
 * window). `mockEnforce` then PROJECTS both endpoints onto their shared mid-y,
 * exactly as PlaneGCS does, and `sketchUpsert` returns the moved coordinates —
 * so the document's geometry agrees with the constraint instead of keeping the
 * tilt the user drew. Without the write-back the extruded solid is a skewed
 * prism (review finding B2).
 */
test("a 1°-off line inferred Horizontal comes back with both endpoints on one y", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  // Raw click coordinates: the rounding/guide tiers would place the endpoints
  // themselves, and this test is about what the SOLVER did to them.
  await setSnapPref(page, "grid", false);
  await setSnapPref(page, "dimensionRound", false);
  await setSnapPref(page, "sketchGuideLines", false);
  await setSnapPref(page, "polarTracking", false);

  await expect(dofPill(page)).toHaveText("DOF: 0");
  // ~1.15° off horizontal: inside the ±5° H/V inference window.
  await clickAt(page, -250, -10);
  await clickAtAwaitingDofChange(page, 250, 0);
  await page.keyboard.press("Escape");

  await expect(
    page.locator('[data-testid^="constraint-row-"]').filter({ hasText: "Horizontal" }).first(),
  ).toBeVisible();

  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(1);
  const [line] = snap.lines;
  expect(line.p0[1]).toBeCloseTo(line.p1[1], 6);
});

/*
 * UX review S3 — "closing a loop happens silently". A click 2 mm from the open
 * start DOES close the profile, but nothing said so beforehand: no badge, no
 * highlight on the target. The hint now names the act itself, so the user can
 * see the loop close before committing to it — and an open profile costs a
 * whole extrude (B5), which is why silence there is the wrong default.
 */
test("hovering the chain's open start reads 'Close loop'", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // Two legs — a chain with a start that is no longer its current end.
  await clickAt(page, -150, -60);
  await clickAtAwaitingDofChange(page, 40, -60);
  await clickAtAwaitingDofChange(page, 40, 70);

  const snap = await getSketchSnapshot(page);
  const start = snap.lines[0].p0;
  const target = await planePointToClient(page, snap.plane, { x: start[0], y: start[1] });

  const hint = page.locator("[data-sketch-snap-hint]");
  await page.mouse.move(target.x, target.y);
  await expect(hint).toBeVisible();
  await expect(hint).toHaveText("Close loop");

  // Clicking there really does close it: the third leg lands back on the start.
  await clickAtClient(page, target.x, target.y);
  await expect.poll(async () => (await getSketchSnapshot(page)).lines.length).toBe(3);
  const closed = await getSketchSnapshot(page);
  const last = closed.lines[2];
  expect(
    Math.hypot(last.p0[0] - start[0], last.p0[1] - start[1]) < 1e-6 ||
      Math.hypot(last.p1[0] - start[0], last.p1[1] - start[1]) < 1e-6,
  ).toBe(true);
});
