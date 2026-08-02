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

/*
 * Revolve kernel-preview LIFECYCLE (mock lane).
 *
 * Picking the axis is what makes a Revolve well-formed, so that is the moment the
 * shared preview lane opens: the controller claims it (`previewOwner: "revolve"`),
 * opens one session per armed region, and paces exact candidates through the
 * throttle as the angle changes.
 *
 * GEOMETRY IS DELIBERATELY NOT ASSERTED HERE. The mock client has no kernel, so
 * `localSolver` publishes an EMPTY, non-committed result for a Revolve session
 * (settling the epoch without faking a mesh it cannot honestly synthesize) — the
 * visible preview in this lane stays the L1 Rodrigues lathe. That preview mesh
 * ==  commit equivalence is proven against the REAL worker in
 * `src-tauri/tests/preview_revolve.rs`; what this spec proves is the lane
 * lifecycle the real client shares with the mock: open → epoch-advance → commit.
 *
 * Axis choice + camera projection follow `revolve-commit.spec.ts` verbatim (the
 * rectangle's own left edge is a valid axis; a pixel-offset guess for a point
 * never directly clicked is unreliable — screenToPlane is a real perspective ray).
 */
test("revolve preview: axis pick opens the lane → drag advances epochs → commit lands a body", async ({
  page,
}) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -150, -110);
  await clickAt(page, 150, 110);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

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

  await page.keyboard.press("Enter"); // finish → auto-arms extrude; camera animates back
  await waitForCameraSettled(page);

  await page.getByRole("listbox", { name: "Sketches" }).getByRole("option").last().click();
  await page.getByRole("button", { name: "Revolve", exact: true }).click();
  await expect(page.getByText(/Pick axis line/)).toBeVisible();

  // Axis-pick phase: the draft carries no axis, so the lane MUST still be free.
  const beforeAxis = await extrudeDebug(page);
  expect(beforeAxis?.previewOwner).toBeNull();
  expect(beforeAxis?.previewSessionCount).toBe(0);

  // Pick the axis → armed at 360° AND the preview lane opens on this region.
  const axisClient = await planePointToClient(page, snap.plane, axisMid);
  await clickAtClient(page, axisClient.x, axisClient.y);
  await expect(page.getByText(/Drag to set angle/)).toBeVisible();

  await expect
    .poll(async () => (await extrudeDebug(page))?.previewOwner, { timeout: 5_000 })
    .toBe("revolve");
  const armed = await extrudeDebug(page);
  expect(armed?.revolvePhase).toBe("armed");
  expect(armed?.previewSessionCount).toBe(1);
  expect(armed?.sessionId).not.toBeNull();
  const armEpoch = armed?.throttleEpoch as number;
  expect(armEpoch).toBeGreaterThan(0); // the initial exact candidate went out

  // Drag the angle: every frame re-sends the candidate through the shared throttle,
  // so the epoch advances past the arm's first send.
  const box = await page.locator(CANVAS).boundingBox();
  const sx = (box?.x ?? 0) + (box?.width ?? 0) / 2 - 40;
  const sy = (box?.y ?? 0) + (box?.height ?? 0) / 2 + 130;
  await page.mouse.move(sx, sy);
  await page.mouse.down();
  await page.mouse.move(sx - 90, sy, { steps: 5 });
  await page.mouse.up();

  const dragged = await extrudeDebug(page);
  expect(dragged?.angle).not.toBe(360); // the drag really moved the angle
  expect(dragged?.revolvePhase).toBe("armed"); // release never commits
  await expect
    .poll(async () => (await extrudeDebug(page))?.throttleEpoch as number, { timeout: 5_000 })
    .toBeGreaterThan(armEpoch);
  // Still exactly one session — a drag re-sends, it never re-opens the lane.
  expect((await extrudeDebug(page))?.previewSessionCount).toBe(1);
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore); // nothing committed yet

  // Explicit confirm → the previewed candidate materializes and the lane is released.
  await page.keyboard.press("Enter");
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
  await expect(bodyOptions(page).last()).toHaveAttribute("aria-selected", "true");
  await expect
    .poll(async () => (await extrudeDebug(page))?.previewSessionCount, { timeout: 5_000 })
    .toBe(0);
  expect((await extrudeDebug(page))?.previewOwner).toBeNull();
});
