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
  extrudeDebug,
  CANVAS,
} from "./helpers";

/*
 * MODEL-HARDEN Wave 1 — the revolve commit gesture (mock lane).
 *
 * A revolve arms on a sketch region, picks a sketch LINE as the axis, then arms
 * at the 360° default. The old plain-click "quickCommit" is GONE: the tool now
 * stays armed and commits only on Enter / chip-✓ / click-away — an angle DRAG
 * followed by a release also keeps it armed. This proves: axis pick → armed 360°
 * → drag angle → release stays armed → Enter commits a body.
 *
 * Axis choice: the rectangle's own left edge is a valid revolve axis (all profile
 * points lie on one side of it — axisSplitsRegion returns false), so no separate
 * axis line is needed. The edge geometry is read back from the live sketch
 * (getSketchSnapshot, BEFORE finishing) and projected through the settled camera
 * (planePointToClient) — a pixel-offset guess for a point never directly clicked
 * is unreliable (screenToPlane is a real perspective raycast). See multiregion.spec.ts.
 */
test("revolve: axis pick → armed 360° → drag angle → release stays armed → Enter commits", async ({
  page,
}) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // A rectangle — its own closed region is the profile; its left edge is the axis.
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -150, -110);
  await clickAt(page, 150, 110);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  // Snapshot the plane + edges BEFORE finishing (sketchStore.session clears on exit).
  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(4);
  const xs = snap.lines.flatMap((l) => [l.p0[0], l.p1[0]]);
  const minU = Math.min(...xs);
  const leftEdge = snap.lines.find(
    (l) => Math.abs(l.p0[0] - minU) < 1e-3 && Math.abs(l.p1[0] - minU) < 1e-3,
  );
  if (!leftEdge) throw new Error("no left edge found in the rectangle snapshot");
  const axisMid = { x: minU, y: (leftEdge.p0[1] + leftEdge.p1[1]) / 2 };

  const bodiesBefore = await bodyOptions(page).count();

  // Finish → auto-arms extrude; exitSketch() animates the camera back — settle.
  await page.keyboard.press("Enter");
  await waitForCameraSettled(page);

  // Switch to Revolve: select the finished sketch, then arm the Revolve tool on it.
  await page.getByRole("listbox", { name: "Sketches" }).getByRole("option").last().click();
  await page.getByRole("button", { name: "Revolve", exact: true }).click();
  await expect(page.getByText(/Pick axis line/)).toBeVisible();

  // Pick the rectangle's left edge as the axis → arms at 360°.
  const axisClient = await planePointToClient(page, snap.plane, axisMid);
  await clickAtClient(page, axisClient.x, axisClient.y);
  await expect(page.getByText(/Drag to set angle/)).toBeVisible();
  expect((await extrudeDebug(page))?.revolvePhase).toBe("armed");
  await expect(page.getByLabel("Dimension value")).toHaveValue("360");
  // Wave 2: an existing body (the mock's seeded Body 1) offers the boolean segments
  // on the armed revolve cluster.
  await expect(page.getByTestId("chip-bool-cut")).toBeVisible();

  // Drag the angle, then release — the tool must STAY armed (no implicit commit).
  const box = await page.locator(CANVAS).boundingBox();
  const sx = (box?.x ?? 0) + (box?.width ?? 0) / 2 - 40;
  const sy = (box?.y ?? 0) + (box?.height ?? 0) / 2 + 130;
  await page.mouse.move(sx, sy);
  await page.mouse.down();
  await page.mouse.move(sx + 90, sy, { steps: 5 }); // horizontal → angle change
  await page.mouse.up();
  expect((await extrudeDebug(page))?.revolvePhase).toBe("armed");
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore); // nothing committed yet

  // Explicit confirm → a revolve body.
  await page.keyboard.press("Enter");
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
  await expect(bodyOptions(page).last()).toHaveAttribute("aria-selected", "true");
});
