import { test, expect } from "./fixtures";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  dofPill,
  getSketchCircle,
  liveDimField,
} from "./helpers";

/*
 * Dimension EDIT e2e (mock lane) — SKETCH_UX_AUDIT.md Part C item #8 (audit A2).
 *
 * The bug this proves fixed: editing an ALREADY-COMMITTED dimension chip's
 * value used to update the constraint's stored number while leaving the
 * geometry untouched — the chip would say 30, the circle would still measure
 * r=12.5. `mockEnforce.ts` now DRIVES the provable cases (Radius/Diameter on a
 * weld-free, unlocked circle is exactly one of them) so the label and the
 * drawn geometry can never disagree.
 *
 * Both tests draw via the LIVE-DIM chip first (same flow as
 * `live-dim-circle.spec.ts`) to get a committed dimensional constraint on
 * canvas, then edit that SAME constraint's badge chip (`ConstraintBadgeLayer`
 * — a different component from the live-dim chip used while drawing).
 */

test("editing a committed Diameter chip 25 -> 30 moves the circle to r=15, not a stale r=12.5", async ({
  page,
}) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Circle");

  await clickAt(page, -120, -60); // center — cursor==center, radial() falls back to +U
  await page.keyboard.press("2");
  await expect(liveDimField(page, "diameter")).toBeFocused();
  await page.keyboard.press("5");
  await page.keyboard.press("Enter"); // locks diameter=25mm AND commits the circle
  await expect(dofPill(page)).not.toHaveText("DOF: 0");

  const before = await getSketchCircle(page);
  expect(before.radius).toBeCloseTo(12.5, 3);

  // The COMMITTED constraint badge chip — distinct from the live-dim chip used
  // while drawing (that one unmounts on commit). A lone circle authors exactly
  // one dimensional constraint, so this resolves singularly.
  const chip = page.getByTestId("constraint-badges").getByLabel("Dimension value");
  await expect(chip).toHaveValue("25");

  await chip.fill("30");
  await chip.press("Enter");

  // `chip.fill` sets the DOM text synchronously — it is NOT a signal the async
  // commit (`editConstraintValue` -> `sketchUpsert` -> store write-back) has
  // resolved. Poll the actual geometry instead of racing ahead of it.
  await expect.poll(async () => (await getSketchCircle(page)).radius).toBeCloseTo(15, 3);
});

test("editing a committed dimension chip never lets the label and the geometry disagree", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Circle");

  await clickAt(page, 120, 60);
  // Throwaway digit opens (and transiently locks) diameter; Tab moves to radius
  // (same alias-drop as `live-dim-circle.spec.ts`'s Tab test) so this exercises
  // the Radius rule instead of test 1's Diameter rule.
  await page.keyboard.press("9");
  await expect(liveDimField(page, "diameter")).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(liveDimField(page, "radius")).toBeFocused();
  await page.keyboard.press("1");
  await page.keyboard.press("0");
  await page.keyboard.press("Enter"); // locks radius=10mm AND commits the circle
  await expect(dofPill(page)).not.toHaveText("DOF: 0");

  const chip = page.getByTestId("constraint-badges").getByLabel("Dimension value");
  await expect(chip).toHaveValue("10");

  await chip.fill("18");
  await chip.press("Enter");

  // Poll the geometry (the async signal) rather than the DOM text (set
  // synchronously by `fill`, no proof the commit round-trip landed).
  await expect.poll(async () => (await getSketchCircle(page)).radius).toBeCloseTo(18, 3);
  // The invariant under test: once settled, the chip label and the drawn
  // circle read the SAME number — never a lying label over stale geometry.
  await expect(chip).toHaveValue("18");
});
