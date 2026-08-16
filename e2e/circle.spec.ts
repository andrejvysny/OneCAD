import { test, expect } from "./fixtures";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  selectSketchTool,
  clickAt,
  dofPill,
  sketchOptions,
} from "./helpers";

/*
 * Circle tool — center → radius commits one Circle.
 *
 * A lone circle contributes exactly 3 free DOF (center 2 + radius 1) with no
 * autoconstraints, so the DOF pill lands on an exact, deterministic value. The
 * inspector shows the under-constrained state with no constraints yet. Finishing
 * returns to model mode with the sketch persisted in the tree.
 */
test("circle tool draws a circle and reports its degrees of freedom", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  // The mock seeds three sketches; entering just added ours.
  const sketchCount = await sketchOptions(page).count();
  await selectSketchTool(page, "Circle");

  await expect(dofPill(page)).toHaveText("DOF: 0");

  // Center, then a radius point.
  await clickAt(page, 0, 0);
  await clickAt(page, 120, 0);

  // Circle present: 3 DOF exactly (mock solveDof: center 2 + radius 1).
  // U7: the first click lands ON the sketch origin (screen centre), where the
  // origin snap is now offered — accepting it persists a `Fixed` that pins the
  // centre, so two translation DOF are gone before any dimension is applied.
  await expect(dofPill(page)).toHaveText("DOF: 1");
  await expect(page.getByText("Under-constrained · DOF 1").first()).toBeVisible();
  // No RELATIONSHIP was inferred — a solitary circle has nothing to relate to.
  // The one constraint present is the origin anchor the centre click accepted.
  await expect(page.getByText("No constraints yet.")).toHaveCount(0);

  // Finish cleanly back to model mode; the circle persists as a sketch.
  await page.keyboard.press("Enter");
  await expect(page.getByText(/^Editing /)).toHaveCount(0);
  await expect(sketchOptions(page)).toHaveCount(sketchCount);
});
