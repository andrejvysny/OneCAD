import { test, expect } from "./fixtures";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  dofPill,
  getLiveDims,
  getSketchCircle,
  liveDimField,
} from "./helpers";

/*
 * Live dimension chips — circle tool (SP-1 W4).
 *
 * The mock solver is an identity-echo (no real solve); DOF is a coarse
 * "free params − removed" count charging exactly 1 DOF per dimensional
 * constraint (`constraintFreedom`, mockSketch.ts). A lone circle is
 * DETERMINISTIC at DOF 3 with no autoconstraints (circle.spec.ts already pins
 * this), so the -1 delta from a typed Diameter/Radius needs no separate
 * baseline draw.
 */
test("typing on the diameter chip authors a Diameter constraint at half the value as radius", async ({
  page,
}) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Circle");

  await clickAt(page, -120, -60); // center — cursor==center, radial() falls back to +U
  const dims = await getLiveDims(page);
  const diameterField = dims.find((d) => d.field === "diameter");
  const radiusField = dims.find((d) => d.field === "radius");
  expect(diameterField).toBeDefined();
  expect(radiusField).toBeDefined();
  await expect(liveDimField(page, "diameter")).toBeVisible();
  await expect(liveDimField(page, "radius")).toBeVisible();

  // Type "25" — opens + focuses the diameter chip (Tab order: diameter first).
  await page.keyboard.press("2");
  await expect(liveDimField(page, "diameter")).toBeFocused();
  await page.keyboard.press("5");
  await page.keyboard.press("Enter"); // locks diameter=25mm AND commits the circle
  // The commit round-trips through the ASYNC mock sketchUpsert
  // (`enqueueSketchMutation`/`commitNow`, fire-and-forget from the chip's Enter)
  // — wait for the DOF pill to leave its fresh-sketch "DOF: 0" before reading
  // the committed geometry, same settle gate `clickAtAwaitingDofChange` uses.
  await expect(dofPill(page)).not.toHaveText("DOF: 0");

  const circle = await getSketchCircle(page);
  expect(circle.radius).toBeCloseTo(12.5, 3);

  await expect(page.getByText("Diameter", { exact: true })).toHaveCount(1);
  await expect(page.getByText("Radius", { exact: true })).toHaveCount(0);
  await expect(dofPill(page)).toHaveText("DOF: 2"); // 3 (lone circle) − 1 (Diameter)
});

/*
 * Reaching "radius" from a cold "diameter" open requires Tab — a digit only
 * ever opens `fields[0]`, and clicking the chip directly bubbles through the
 * viewport listener as a genuine (degenerate) draw click. The Tab path once
 * dropped FSM focus via the departed field's stale onBlur (fixed in
 * SketchController's onBlur field guard; regression pinned in
 * SketchController.liveDim.test.ts "stale blur").
 */
test("Tab-ing from diameter to radius authors a Radius constraint instead", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Circle");

  await clickAt(page, 120, 60); // a second, independent circle

  // Throwaway digit opens (and transiently locks) diameter; Tab moves to radius
  // and the alias rule (`DIM_ALIASES`) drops the diameter lock once radius locks.
  await page.keyboard.press("9");
  await expect(liveDimField(page, "diameter")).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(liveDimField(page, "radius")).toBeFocused();

  await page.keyboard.press("1");
  await page.keyboard.press("2");
  await page.keyboard.press(".");
  await page.keyboard.press("5");
  await page.keyboard.press("Enter"); // locks radius=12.5mm AND commits the circle
  await expect(dofPill(page)).not.toHaveText("DOF: 0");

  const circle = await getSketchCircle(page);
  expect(circle.radius).toBeCloseTo(12.5, 3);
  await expect(page.getByText("Radius", { exact: true })).toHaveCount(1);
});
