import { test, expect } from "@playwright/test";
import {
  openEditor,
  enterSketchViaPlanePicker,
  selectSketchTool,
  clickAt,
  dofPill,
  sketchOptions,
} from "./helpers";

/*
 * Rectangle tool — draw + finish → closed-region handoff.
 *
 * Two corner clicks commit 4 axis-aligned lines forming one closed loop. Enter
 * finishes the sketch back to model mode. Arming Extrude on the finished sketch
 * then drives the mock lane's finishSketch → detectRegions: it resolves the loop
 * into an extrudable region (proof = the depth prompt appears; a sketch with no
 * closed region would instead report "No closed region to extrude").
 */
test("rectangle draws a closed region and arms extrude from the finished sketch", async ({ page }) => {
  await openEditor(page);
  await enterSketchViaPlanePicker(page);
  await selectSketchTool(page, "Rectangle");

  // Corner → opposite corner (distinct in both axes → non-degenerate rect).
  await clickAt(page, -150, -100);
  await clickAt(page, 150, 100);

  // 4 lines minus autoconstraints (H/V + 4 coincidences) → under-constrained.
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);
  await expect(page.getByText(/^Under-constrained · DOF [1-9]/).first()).toBeVisible();

  // Finish the sketch (Enter) → model mode: chrome gone, model toolbar back.
  await page.keyboard.press("Enter");
  await expect(page.getByText(/^Editing /)).toHaveCount(0);
  await expect(page.getByRole("button", { name: "New sketch", exact: true })).toBeVisible();

  // Arm Extrude on the just-finished sketch (last tree row) → closed region
  // resolves and the extrude depth prompt appears.
  await sketchOptions(page).last().click();
  await page.getByRole("button", { name: "Extrude", exact: true }).click();

  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Extrude", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});
