import { test, expect } from "./fixtures";
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
 * 2 cap arcs, plus the tool-authored constraint set (W0b: Coincident ×4 welding
 * each wall endpoint to the cap endpoint it touches, + Equal ×1).
 *
 * DOF NOTE — the pill value asserted below comes from `mockSketch.solveDof`, a
 * coarse "2·points (+radius/angle) − removed" count rather than a Jacobian rank
 * (2 lines = 8, 2 arcs = 10, each Coincident removes 2 and Equal 1 ⇒ 18 − 9 = 9).
 * Here it AGREES with the real PlaneGCS lane for the same 4 entities: the worker
 * mints each arc's endpoints and couples them with DOF-neutral internal arc
 * rules, so a worker-authored welded slot also diagnoses 9
 * (`worker/tests/test_sketch_arc_endpoints.cpp`). The same slot pushed through
 * the RUST typed document reads 14 — the wire emits each arc centre both as a
 * standalone Point entity and as inline coords, so the worker mints a second,
 * unconstrained centre point per arc (pre-existing, tracked separately).
 */
test("slot tool draws a slot with welded caps and reports its DOF", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Slot");

  await expect(dofPill(page)).toHaveText("DOF: 0");

  await clickAt(page, -150, 0); // p0
  await clickAt(page, 150, 0); // p1 — the centerline
  await clickAt(page, 0, 80); // width point — perpendicular offset ⇒ slot radius

  // 18 free − 4 Coincident (2 each) − 1 Equal (see the header).
  await expect(dofPill(page)).toHaveText("DOF: 9");
  await expect(page.getByText(/^Under-constrained · DOF 9/).first()).toBeVisible();

  // The authored set is visible in the inspector's Constraints list.
  await expect(page.getByText("Coincident", { exact: true }).first()).toBeVisible();
  await expect(page.getByText("Equal", { exact: true }).first()).toBeVisible();

  // 2 walls + 2 cap arcs — nothing else.
  expect(await getSketchEntityCount(page)).toBe(4);
});
