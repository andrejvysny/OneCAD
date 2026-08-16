import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  sketchOptions,
} from "./helpers";

/*
 * MULTI-OBJECT SKETCH SESSION (mock lane) — the user-reported defect.
 *
 * A sketch must hold MANY disconnected sub-sketches: a rectangle, then a
 * separate standalone line, then a third object drawn AFTER re-entering the
 * sketch. Every one of them must survive Finish and be there again on re-entry
 * (tree double-click → `setMode("sketch", id)`).
 *
 * The re-entry half is the one that broke: the FIRST edit of a re-opened sketch
 * is where the client's id-map must be REBASED onto the authoritative
 * `enter_sketch` wire — a binding left over from the previous session marshals
 * as a `removeEntity` and deletes everything drawn before it.
 *
 * Counts come from the LIVE session (`sketchStore`), which only exists while in
 * sketch mode — hence the re-enter before each post-finish assertion. The reader
 * returns -1 rather than throwing so `expect.poll` can wait for the async
 * `enterSketch` round-trip instead of failing on the first miss.
 */
function entityCount(page: Page): Promise<number> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { sketch: { getState(): { session: { entities: unknown[] } | null } } };
    };
    return w.__stores?.sketch.getState().session?.entities.length ?? -1;
  });
}

/** Re-open the sketch the way a user does: double-click its model-tree row. */
async function reenterSketch(page: Page): Promise<void> {
  await sketchOptions(page).last().dblclick();
  await expect(page.getByText(/^Editing /)).toBeVisible();
}

test("a rectangle, a separate line, and a post-re-entry line all survive Finish", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  // Settle the SKETCH view before drawing: `enterSketch` tweens the camera to
  // look along the plane normal, and a click issued mid-tween resolves against a
  // moving pose (see `waitForCameraSettled`'s own root-cause note, and
  // `sketch-degenerate.spec.ts`, which documents the same hazard).
  await waitForCameraSettled(page);

  // Gesture 1 — the rectangle tool: corner → opposite corner ⇒ 4 lines.
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -170, -90);
  await clickAt(page, -40, 10);
  await expect.poll(() => entityCount(page)).toBe(4);

  // Gesture 2 — a SEPARATE, disconnected line well clear of the rectangle.
  await selectSketchTool(page, "Line");
  await clickAt(page, 60, 60);
  await clickAt(page, 170, 60);
  await page.keyboard.press("Enter"); // ends the CHAIN (a gesture is in progress)
  await expect.poll(() => entityCount(page)).toBe(5);

  // Finish the sketch (Enter with no open chain → the global finishSketch action).
  await page.keyboard.press("Enter");
  await expect(page.getByText(/^Editing /)).toHaveCount(0);

  // Both objects must be back on re-entry.
  await reenterSketch(page);
  await expect.poll(() => entityCount(page)).toBe(5);

  // Gesture 3 — a THIRD object drawn AFTER the re-entry (the regression seam).
  await selectSketchTool(page, "Line");
  await clickAt(page, 60, -80);
  await clickAt(page, 170, -80);
  await page.keyboard.press("Enter"); // end the chain
  await expect.poll(() => entityCount(page)).toBe(6);

  await page.keyboard.press("Enter"); // finish
  await expect(page.getByText(/^Editing /)).toHaveCount(0);
  await reenterSketch(page);
  await expect.poll(() => entityCount(page)).toBe(6);
});
