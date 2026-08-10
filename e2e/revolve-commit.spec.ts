import { test, expect } from "./fixtures";
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
import { openRevolveOverflow, closeRevolveOverflow } from "./modelToolHelpers";

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
  // on the armed revolve cluster — behind its own `⋯` since UNIFY-UX Phase 2.
  await openRevolveOverflow(page);
  await expect(page.getByTestId("chip-bool-cut")).toBeVisible();
  await closeRevolveOverflow(page);

  // Drag the angle, then release — the tool must STAY armed (no implicit commit).
  const box = await page.locator(CANVAS).boundingBox();
  const sx = (box?.x ?? 0) + (box?.width ?? 0) / 2 - 40;
  const sy = (box?.y ?? 0) + (box?.height ?? 0) / 2 + 130;
  await page.mouse.move(sx, sy);
  await page.mouse.down();
  // Drag LEFT: a rightward drag from the 360° default clamps and stays 360 (an inert
  // drag that would make the "stays armed" check pass vacuously). Leftward decreases it.
  await page.mouse.move(sx - 90, sy, { steps: 5 });
  await page.mouse.up();
  // The debug `angle` updates on release — assert the drag actually moved it off 360
  // before the armed / commit assertions (finding 16).
  const draggedAngle = (await extrudeDebug(page))?.angle as number;
  expect(draggedAngle).not.toBe(360);
  expect((await extrudeDebug(page))?.revolvePhase).toBe("armed");
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore); // nothing committed yet

  // Explicit confirm → a revolve body.
  await page.keyboard.press("Enter");
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
  await expect(bodyOptions(page).last()).toHaveAttribute("aria-selected", "true");
});

/*
 * UNIFY-UX Phase 2 — Revolve's in-viewport guidance. Before this, the ONLY
 * feedback for "nothing selected" or "axis not picked yet" was a small
 * StatusBar string; both states now also get a real viewport-space element:
 * a screen-fixed banner with nothing selected, a leader-anchored chip once a
 * profile is bound but no axis chosen.
 */
test("Revolve guidance: an empty-state banner before any pick, an axis-hint chip during axisPick", async ({
  page,
}) => {
  await openEditorDebug(page);

  // Nothing selected, no sketch in the seeded document yet — the empty-state
  // banner is the only in-viewport feedback (StatusBar carries the rest).
  await page.getByRole("button", { name: "Revolve", exact: true }).click();
  await expect(page.getByTestId("revolve-empty-hint")).toBeVisible();
  await expect(page.getByTestId("chip-revolve-axis-hint")).toHaveCount(0);

  await page.getByRole("button", { name: "Select", exact: true }).click();
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -150, -110);
  await clickAt(page, 150, 110);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);
  await page.keyboard.press("Enter");
  await waitForCameraSettled(page);

  await page.getByRole("listbox", { name: "Sketches" }).getByRole("option").last().click();
  await page.getByRole("button", { name: "Revolve", exact: true }).click();

  // A profile is bound now, but no axis yet — the empty-state banner is gone
  // (the chip store's `kind` moved off "none") and the axis-hint chip is up.
  await expect(page.getByTestId("revolve-empty-hint")).toHaveCount(0);
  await expect(page.getByTestId("chip-revolve-axis-hint")).toHaveText("Pick an axis line");

  // ✕ backs all the way out to Select — there is no value yet to keep armed.
  await page.getByTestId("chip-cancel").click();
  await expect(page.getByRole("button", { name: "Select", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByTestId("chip-revolve-axis-hint")).toHaveCount(0);
});
