import { test, expect } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  clickAt,
  clickAtAwaitingDofChange,
  clickAtClient,
  getSketchSnapshot,
  planePointToClient,
} from "./helpers";

/*
 * Snap indicator (mock lane) — the REAL user path (plane picker) end to end.
 *
 * Draws one line, then — STILL in the Line tool — hovers line 1's own
 * endpoint. The hint chip is SnapIndicator's HTML overlay element
 * (`[data-sketch-snap-hint]`, ViewportEngine.setSketchSnap ->
 * SnapIndicator.show), gated only by `settingsStore.show.snappingHints`
 * (default true — no toggle needed). It shows "Endpoint" (snapEngine.ts's
 * KIND_LABEL), because the endpoint tier (0) always outranks every other
 * candidate (grid/guide-line/onCurve/...) within the 8px SNAP_PX reach,
 * regardless of distance ties.
 *
 * Clicking there starts a new chain anchored at the snapped point:
 * SketchController.onPointerUp recomputes the snap FRESH at click time
 * (`snapAt`), and the winning candidate's point is the LITERAL entity
 * coordinate read out of `entitySnapPoints` (not the raw click position) — so
 * the new segment's Start ends up bit-identical to line 1's endpoint. The mock
 * lane is an identity solve (`solvedPositions: {}` — see localSolver.ts), so
 * nothing downstream perturbs it either: this is the "float-exact coincidence
 * = snap worked" proof, not just an on-screen visual match.
 */
test("hovering an existing endpoint shows the Endpoint hint and clicking snaps exactly onto it", async ({
  page,
}) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page); // default tool: Line
  await waitForCameraSettled(page);

  // Line 1, well clear of the floating panels.
  await clickAt(page, -150, -60);
  await clickAtAwaitingDofChange(page, -20, 60);
  await page.keyboard.press("Escape");

  const before = await getSketchSnapshot(page);
  expect(before.lines).toHaveLength(1);
  const endpoint = before.lines[0].p1;
  const target = await planePointToClient(page, before.plane, { x: endpoint[0], y: endpoint[1] });

  // Hover the endpoint — no click yet.
  const hint = page.locator("[data-sketch-snap-hint]");
  await page.mouse.move(target.x, target.y);
  await expect(hint).toBeVisible();
  await expect(hint).toHaveText("Endpoint");

  // Click there: anchors a NEW chain at the snapped point (float-exact — see
  // header). A second click elsewhere commits the segment.
  await clickAtClient(page, target.x, target.y);
  await clickAtAwaitingDofChange(page, 140, -30);
  await page.keyboard.press("Escape");

  const after = await getSketchSnapshot(page);
  expect(after.lines).toHaveLength(2);
  // The new segment's Start is EXACTLY line 1's endpoint (float-exact coincidence).
  expect(after.lines[1].p0).toEqual(endpoint);
});
