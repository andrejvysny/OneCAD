import { test, expect } from "@playwright/test";
import {
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
 * Select-tool point-handle drag (mock lane) — the REAL gesture lane end to end.
 *
 * Draws one line, switches to Select, then drags its End handle — a draggable
 * point per selectGesture.ts's `isDraggableHandle` (Line Start/End, Circle/Arc
 * Center) — past DRAG_PX (4px) toward the line's OWN midpoint. Targeting the
 * midpoint (rather than an arbitrary offset) sidesteps any need to know the
 * canvas-pixel-to-plane-unit scale: it is a point already proven on-screen
 * (both endpoints it's derived from were themselves click targets), see
 * helpers.ts's targeting note on why a pixel-offset guess for an unclicked
 * point is unreliable.
 *
 * SketchController.scheduleSelectSolve raycasts the raw `screenToPlane` target
 * with NO snapping (unlike the draw tools' `snapAt`), and the mock lane's
 * solveDrag/endGesture identity-echo it (localSolver.ts), so the dragged point
 * should land very close to the intended target — "moved toward the drop
 * target", asserted as a relative distance (not exact-echo equality, since a
 * screen->plane->screen round trip is not guaranteed bit-exact the way a
 * literal snapped-entity-coordinate readback is, see sketch-snap.spec.ts).
 */
test("select tool drags a line's End handle toward a real drop target", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // One line, well clear of the floating panels.
  await clickAt(page, -160, -60);
  await clickAtAwaitingDofChange(page, 120, 90);
  await page.keyboard.press("Escape");

  const before = await getSketchSnapshot(page);
  expect(before.lines).toHaveLength(1);
  const { p0, p1 } = before.lines[0];
  const target: [number, number] = [(p0[0] + p1[0]) / 2, (p0[1] + p1[1]) / 2];
  const lineLen = Math.hypot(p1[0] - p0[0], p1[1] - p0[1]);
  const dist = (a: [number, number], b: [number, number]): number => Math.hypot(a[0] - b[0], a[1] - b[1]);

  await selectSketchTool(page, "Select");

  const startClient = await planePointToClient(page, before.plane, { x: p1[0], y: p1[1] });
  const targetClient = await planePointToClient(page, before.plane, { x: target[0], y: target[1] });

  // Grab the End handle, drag well past DRAG_PX, release at the target.
  await page.mouse.move(startClient.x, startClient.y);
  await page.mouse.down();
  await page.mouse.move(targetClient.x, targetClient.y, { steps: 8 });
  await page.mouse.up();

  // beginGesture/solveDrag/endGesture settle asynchronously (SKETCH_LATENCY_MS
  // + the shared mutation queue) — poll until the End point actually moved.
  await expect
    .poll(async () => dist((await getSketchSnapshot(page)).lines[0].p1, p1))
    .toBeGreaterThan(lineLen * 0.3);

  const after = await getSketchSnapshot(page);
  // Landed close to the intended target (well inside the line's own length) —
  // proof of "moved toward the drop target", not a no-op or stray jitter.
  expect(dist(after.lines[0].p1, target)).toBeLessThan(lineLen * 0.1);
  // Start is untouched — only the dragged handle moved.
  expect(after.lines[0].p0).toEqual(p0);
});
