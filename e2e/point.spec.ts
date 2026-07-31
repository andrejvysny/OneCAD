import { test, expect } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  clickAt,
  dofPill,
  getSketchEntityCount,
} from "./helpers";

/*
 * Point tool (W2-B) — the keyboard path (`P`) plus the free-constraint payoff.
 *
 * A Point contributes exactly 2 free DOF and no autoconstraints, so the pill
 * lands on an exact value after each click. The third click re-uses the FIRST
 * click's screen position: the camera is settled, so `screenToPlane` returns the
 * identical plane point (and endpoint snapping would land it there anyway), which
 * is precisely the case `inferConstraints` turns into a Coincident — so the DOF
 * gained by the new point is immediately removed again.
 */
test("point tool drops points and auto-constrains a repeat click", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // `P` is the sketch-mode binding (it deliberately shadows the cross-mode
  // fallback to the model Linear-pattern tool — see keymap.ts).
  await page.keyboard.press("p");
  await expect(page.getByRole("button", { name: "Point", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );

  await expect(dofPill(page)).toHaveText("DOF: 0");

  await clickAt(page, -120, -60);
  await expect(dofPill(page)).toHaveText("DOF: 2"); // one point = 2 free DOF

  await clickAt(page, 120, 60);
  await expect(dofPill(page)).toHaveText("DOF: 4");

  // Snap-to-endpoint: +2 free from the new point, −2 from the inferred
  // Coincident ⇒ the pill does not move, but the constraint row appears.
  await clickAt(page, -120, -60);
  await expect(page.getByText("Coincident", { exact: true }).first()).toBeVisible();
  await expect(dofPill(page)).toHaveText("DOF: 4");
  expect(await getSketchEntityCount(page)).toBe(3);
});
