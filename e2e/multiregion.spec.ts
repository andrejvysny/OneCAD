import { test, expect } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtClient,
  dofPill,
  bodyOptions,
  getSketchSnapshot,
  planePointToClient,
  commitExtrudeAtHandle,
} from "./helpers";

/*
 * Non-first-region extrude (mock lane): a sketch with a separated rectangle AND
 * a circle detects as TWO regions, so finishing the sketch hands pointer
 * ownership to a region pick instead of auto-arming ("Select a region to
 * extrude" — ModelToolController.enterRegionPick). Clicking a region resolves
 * that SPECIFIC regionId into the extrude draft (see
 * ModelToolController.regionPick.test.ts "clicking region 2 arms extrude with
 * regions[1].regionId in the draft") — this spec proves the picked, non-first
 * region flows all the way through to a committed body.
 *
 * ADAPTATION vs the original design note ("click inside the circle"): verified
 * against `detectRegions` (src/ipc/mockSketch.ts) — it scans ALL entities for
 * Circles first and pushes a region per circle, THEN appends the (at most one)
 * closed line/arc loop's region — so for one circle + one rectangle the mock
 * ALWAYS returns `regions = [circleRegion, rectRegion]` regardless of draw
 * order. The circle is therefore always `regions[0]` (the FIRST region), and
 * the rectangle is always `regions[1]` (the actual non-first one). Clicking the
 * circle would exercise the already-well-covered first-region path, so this
 * spec clicks the RECTANGLE instead to genuinely prove the non-first pick.
 *
 * Targeting note: the click point is the rectangle's centroid computed from its
 * REAL line entities (`getSketchSnapshot`, read BEFORE finishing — the session
 * clears on leaving sketch mode) and projected through the live camera
 * (`planePointToClient`), not a guessed canvas-relative offset — `screenToPlane`
 * is a real perspective raycast, not affine, so a pixel-offset guess for a point
 * never directly clicked (like a shape's centroid) is unreliable (verified
 * empirically against an earlier, offset-based version of this spec).
 *
 * Camera-settle note (two DIFFERENT tweens, both confirmed the hard way):
 * entering the plane picker kicks off an animated re-home, so drawing must not
 * race it (`waitForCameraSettled` right after `enterSketchViaPlanePicker`).
 * SEPARATELY, `ViewportEngine.exitSketch()` — run when Enter finishes the sketch
 * — animates the camera BACK to whatever view was active before the sketch was
 * entered. The plane's own basis (origin/xAxis/yAxis) is fixed and safe to read
 * before finishing, but the click point must be projected through the camera
 * pose AFTER that restore tween completes — projecting with the sketch-mode
 * pose and clicking after the restore targets the wrong screen point. Hence
 * `waitForCameraSettled` again after Enter, before `planePointToClient`.
 */
test("multi-region sketch: picking the non-first region (the rectangle) arms and commits an extrude", async ({
  page,
}) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // Rectangle — its own closed loop, well clear of the circle drawn below.
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -260, -150);
  await clickAt(page, -60, 50);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  // Circle — separated in screen (and therefore plane) space from the rectangle.
  await selectSketchTool(page, "Circle");
  await clickAt(page, 120, -80); // center
  await clickAt(page, 200, -80); // radius point (radius 80)
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  // Snapshot BEFORE finishing: sketchStore.session clears once mode leaves "sketch".
  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(4); // the rectangle only — circle isn't a Line entity
  // The rectangle's centroid: each of its 4 unique corners is shared by exactly 2
  // lines, so averaging ALL 8 raw endpoints (each corner counted twice, uniformly)
  // equals the average of the 4 unique corners — no dedup needed.
  const sum = snap.lines.reduce(
    (acc, l) => ({ x: acc.x + l.p0[0] + l.p1[0], y: acc.y + l.p0[1] + l.p1[1] }),
    { x: 0, y: 0 },
  );
  const centroid = { x: sum.x / (snap.lines.length * 2), y: sum.y / (snap.lines.length * 2) };

  const bodiesBefore = await bodyOptions(page).count();

  // Finish → 2 regions (circle + rectangle) → MULTI-select region pick (Wave 2),
  // NOT an instant arm.
  await page.keyboard.press("Enter");
  await expect(page.getByText(/^Select regions to extrude/)).toBeVisible();
  // exitSketch() animates the camera BACK to its pre-sketch pose — settle before
  // projecting the click point through it (see the camera-settle note above).
  await waitForCameraSettled(page);

  // Click inside the rectangle's fill → TOGGLES regions[1] (the non-first region);
  // the region-select chip reflects the single selection.
  const clientPt = await planePointToClient(page, snap.plane, centroid);
  await clickAtClient(page, clientPt.x, clientPt.y);
  await expect(page.getByTestId("chip-region-count")).toHaveText("1 region");
  // Enter confirms the selection → arms the extrude on that one region.
  await page.keyboard.press("Enter");
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();

  // Wave 1 gesture: drag the handle, release (stays armed), Enter confirms.
  await commitExtrudeAtHandle(page);
  // "Extruded" is a transient statusHint (ModelToolController.finishExtrude) that
  // can already be gone by the time we poll — the durable proof of a successful
  // commit is the new Body row, which finishExtrude also selects.
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
  await expect(bodyOptions(page).last()).toHaveAttribute("aria-selected", "true");
});
