import { test, expect } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  dofPill,
  bodyOptions,
  extrudeDebug,
  getFeatureLabels,
  commitExtrudeAtHandle,
} from "./helpers";

/*
 * MODEL-HARDEN Wave 2 — extrude boolean modes (mock lane).
 *
 * With an existing body in the document (the mock seeds Body 1), a fresh extrude
 * arm offers the New Body / Add / Cut segment group. Exactly one visible body means
 * Cut AUTO-TARGETS it (no target pick). Committing a Cut adds a history row but
 * creates NO new body — the mock has no CSG (documented limit), so the assertion
 * rides on the row + body-diff, never the resulting geometry.
 */
test("extrude boolean: Cut segment visible + auto-targets the sole body; commit keeps the body count", async ({
  page,
}) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // A single rectangle → one region → auto-arms extrude with the boolean segments.
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -150, -100);
  await clickAt(page, 150, 100);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  const bodiesBefore = await bodyOptions(page).count();
  expect(bodiesBefore).toBeGreaterThan(0); // the seeded Body 1 is the boolean target

  await page.keyboard.press("Enter"); // finish → auto-arms extrude
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();

  // The boolean segment group is offered (an existing body enables Add / Cut).
  await expect(page.getByTestId("chip-bool-cut")).toBeVisible();
  await expect(page.getByTestId("chip-bool-cut")).toBeEnabled();

  // Pick Cut → auto-targets the sole visible body; the debug surface reflects it.
  await page.getByTestId("chip-bool-cut").click();
  await expect.poll(async () => (await extrudeDebug(page))?.booleanMode).toBe("Cut");
  expect((await extrudeDebug(page))?.booleanTargetId).toBeTruthy();

  // Commit: one drag + Enter.
  await commitExtrudeAtHandle(page);

  // A Cut commits a timeline row but NO new body (mock CSG limit) — count unchanged.
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore);
  await expect.poll(async () => await getFeatureLabels(page)).toContain("Extrude (Cut)");
});
