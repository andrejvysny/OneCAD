import { test, expect } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  dofPill,
  getSketchEntityCount,
} from "./helpers";

/*
 * Slot tool (W2-B) — 3 clicks: p0, p1 (centerline), then a width point whose
 * PERPENDICULAR distance to the centerline is the slot radius. Commits 2 walls +
 * 2 cap arcs, plus the tool-authored constraint set (Tangent ×4 + Equal ×1).
 *
 * DOF NOTE — the pill value asserted below is a MOCK value: `mockSketch.solveDof`
 * is a coarse "2·points (+radius/angle) − removed" count, not a Jacobian rank
 * (2 lines = 8, 2 arcs = 10, each Tangent/Equal removes 1 ⇒ 18 − 5 = 13). The
 * real PlaneGCS lane lands at ≈13 too, because the slot deliberately authors NO
 * wall↔arc endpoint Coincidents (an Arc Start/End has no wire point uuid, so the
 * marshaller would silently drop them — see toolMachine.ts's slot header).
 */
test("slot tool draws a slot with tangent caps and reports its (mock) DOF", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Slot");

  await expect(dofPill(page)).toHaveText("DOF: 0");

  await clickAt(page, -150, 0); // p0
  await clickAt(page, 150, 0); // p1 — the centerline
  await clickAt(page, 0, 80); // width point — perpendicular offset ⇒ slot radius

  // MOCK-ONLY value (see the header): 18 free − 4 Tangent − 1 Equal.
  await expect(dofPill(page)).toHaveText("DOF: 13");
  await expect(page.getByText(/^Under-constrained · DOF 13/).first()).toBeVisible();

  // The authored set is visible in the inspector's Constraints list.
  await expect(page.getByText("Tangent", { exact: true }).first()).toBeVisible();
  await expect(page.getByText("Equal", { exact: true }).first()).toBeVisible();

  // 2 walls + 2 cap arcs — nothing else.
  expect(await getSketchEntityCount(page)).toBe(4);
});
