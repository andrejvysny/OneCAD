import { test, expect } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtClient,
  clickAtAwaitingDofChange,
  dofPill,
  bodyOptions,
  sketchOptions,
  getSketchSnapshot,
  getSketchCircle,
  getFeatureLabels,
  planePointToClient,
  commitExtrudeAtHandle,
} from "./helpers";

/*
 * MODEL-HARDEN Wave 2 — multi-region extrude → N separate ops (mock lane).
 *
 * A finished sketch with a rectangle AND a circle detects as TWO regions. Wave 2
 * turns that into a MULTI-select: each region is toggled on, the region-select chip
 * counts them, Enter confirms, ONE drag sets the shared depth, and a final Enter
 * commits N separate extrude ops (N history rows, N bodies). The consumed sketch
 * auto-hides after all commits succeed.
 *
 * Region + camera targeting follows multiregion.spec.ts: read the real geometry
 * BEFORE finishing (session clears on exit) and project each region's centroid
 * through the SETTLED camera — a pixel-offset guess for a point never directly
 * clicked is unreliable against the perspective raycast.
 */
test("multi-region: toggle both regions → one drag → Enter commits 2 bodies + auto-hides the sketch", async ({
  page,
}) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // Rectangle — its own closed loop, well clear of the circle below.
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -260, -150);
  await clickAt(page, -60, 50);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  // Circle — separated in plane space from the rectangle. The radius click commits
  // the circle; await the DOF change so the circle is in the session before reading it
  // (a plain positive-DOF check already held from the rectangle — it wouldn't wait).
  await selectSketchTool(page, "Circle");
  await clickAt(page, 120, -80); // center
  await clickAtAwaitingDofChange(page, 200, -80); // radius → commits the circle

  // Snapshot the plane + rectangle lines + circle BEFORE finishing (session clears).
  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(4);
  const circle = await getSketchCircle(page);
  const sum = snap.lines.reduce(
    (acc, l) => ({ x: acc.x + l.p0[0] + l.p1[0], y: acc.y + l.p0[1] + l.p1[1] }),
    { x: 0, y: 0 },
  );
  const rectCentroid = { x: sum.x / (snap.lines.length * 2), y: sum.y / (snap.lines.length * 2) };
  const circleCentroid = { x: circle.center[0], y: circle.center[1] };

  const bodiesBefore = await bodyOptions(page).count();
  const featuresBefore = (await getFeatureLabels(page)).length;

  // Finish → 2 regions → multi-select.
  await page.keyboard.press("Enter");
  await expect(page.getByText(/^Select regions to extrude/)).toBeVisible();
  await waitForCameraSettled(page);

  // Toggle BOTH regions; the chip counts the growing selection.
  const rectPt = await planePointToClient(page, snap.plane, rectCentroid);
  await clickAtClient(page, rectPt.x, rectPt.y);
  await expect(page.getByTestId("chip-region-count")).toHaveText("1 region");
  const circlePt = await planePointToClient(page, snap.plane, circleCentroid);
  await clickAtClient(page, circlePt.x, circlePt.y);
  await expect(page.getByTestId("chip-region-count")).toHaveText("2 regions");

  // Enter confirms the selection → arms the 2-region extrude (one shared handle).
  await page.keyboard.press("Enter");
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();

  // One drag + Enter commits both regions.
  await commitExtrudeAtHandle(page);

  // Two new bodies + two new timeline rows; the consumed sketch is hidden in the tree.
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 2);
  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(featuresBefore + 2);
  await expect(sketchOptions(page).last().getByRole("switch")).toHaveAttribute("aria-checked", "false");
});
