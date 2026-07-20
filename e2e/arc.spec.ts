import { test, expect } from "@playwright/test";
import { openEditor, enterSketchViaPlanePicker, selectSketchTool, clickAt, dofPill } from "./helpers";

/*
 * Arc tool — center → start → end commits one Arc; plus a mid-gesture Esc that
 * cancels the tool cleanly.
 *
 * A lone arc contributes exactly 5 free DOF (center 2 + radius 1 + 2 sweep
 * angles), so the DOF pill is deterministic. The Esc case verifies the
 * SketchController's capture-phase intercept: an in-progress gesture is dropped
 * WITHOUT the global Esc ladder switching tools or leaving sketch mode.
 */
test("arc tool draws a center-start-end arc and Esc cancels mid-gesture cleanly", async ({ page }) => {
  await openEditor(page);
  await enterSketchViaPlanePicker(page);
  await selectSketchTool(page, "Arc");

  await expect(dofPill(page)).toHaveText("DOF: 0");

  // Center, start (fixes the radius), end (fixes the sweep).
  await clickAt(page, 0, 0);
  await clickAt(page, 120, 0);
  await clickAt(page, 0, 120);

  // Arc present: 5 DOF exactly (mock solveDof: center 2 + radius 1 + 2 angles).
  await expect(dofPill(page)).toHaveText("DOF: 5");
  await expect(page.getByText("Under-constrained · DOF 5").first()).toBeVisible();

  // ── mid-gesture Esc ──
  const arcBtn = page.getByRole("button", { name: "Arc", exact: true });
  await expect(arcBtn).toHaveAttribute("aria-pressed", "true");

  // Begin a new arc (place only the center), then abort with Esc.
  await clickAt(page, -120, 40);
  await page.keyboard.press("Escape");

  // Clean cancel: still in sketch mode, Arc still the active tool, and no phantom
  // entity was committed (DOF unchanged).
  await expect(page.getByText(/^Editing /)).toBeVisible();
  await expect(arcBtn).toHaveAttribute("aria-pressed", "true");
  await expect(dofPill(page)).toHaveText("DOF: 5");
});
