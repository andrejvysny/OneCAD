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
 * Polygon tool (W2-C) — `G` arms it, digit keys 3-9 set the side count (claimed
 * by the SketchController's capture-phase keydown, before the global keymap
 * ladder), then centre → vertex commits n ring lines + ONE construction
 * circumcircle.
 *
 * The tool authors its own regularity set (Coincident ring ×n, OnCurve ×n,
 * Equal ×(n−1)), so the sketch settles at DOF (4n + 3) − 2n − n − (n − 1) = 4 —
 * centre 2 + radius 1 + rotation 1 — INDEPENDENT of n. The side count is
 * therefore asserted through the status hint + the entity count, and the DOF pill
 * asserts the regularity set actually landed.
 */
test("polygon tool changes its side count with a digit key and settles at DOF 4", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  await page.keyboard.press("g");
  await expect(page.getByRole("button", { name: "Polygon", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByText("Polygon — 6 sides · 3–9 to change")).toBeVisible();

  // Digit key → the armed machine, not the global ladder.
  await page.keyboard.press("5");
  await expect(page.getByText("Polygon — 5 sides · 3–9 to change")).toBeVisible();

  await expect(dofPill(page)).toHaveText("DOF: 0");
  await clickAt(page, 0, 0); // centre
  await clickAt(page, 140, 0); // one vertex — the rest of the ring follows

  await expect(dofPill(page)).toHaveText("DOF: 4");
  // 5 ring lines + the construction circumcircle.
  expect(await getSketchEntityCount(page)).toBe(6);
  // The side count is sticky, so the tool re-arms at 5.
  await expect(page.getByText("Polygon — 5 sides · 3–9 to change")).toBeVisible();
});
