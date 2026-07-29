import { test, expect } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtClient,
  dofPill,
  bodyOptions,
  extrudeDebug,
  getFeatureLabels,
  getSketchSnapshot,
  planePointToClient,
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
  const visibleSeedSketches = page
    .getByRole("listbox", { name: "Sketches" })
    .locator('[role="switch"][aria-checked="true"]');
  while ((await visibleSeedSketches.count()) > 0) await visibleSeedSketches.first().click();
  const targetVisibility = bodyOptions(page).first().getByRole("switch");
  await targetVisibility.click();
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // A single rectangle → select its filled region → arm Extrude.
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -150, -100);
  await clickAt(page, 150, 100);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);
  const snap = await getSketchSnapshot(page);
  const xs = snap.lines.flatMap((line) => [line.p0[0], line.p1[0]]);
  const ys = snap.lines.flatMap((line) => [line.p0[1], line.p1[1]]);

  const bodiesBefore = await bodyOptions(page).count();
  expect(bodiesBefore).toBeGreaterThan(0); // the seeded Body 1 is the boolean target

  await page.keyboard.press("Enter");
  await waitForCameraSettled(page);
  const client = await planePointToClient(page, snap.plane, {
    x: Math.min(...xs) + (Math.max(...xs) - Math.min(...xs)) * 0.1,
    y: Math.min(...ys) + (Math.max(...ys) - Math.min(...ys)) * 0.1,
  });
  await expect
    .poll(() =>
      page.evaluate(
        ({ x, y }) => Boolean((window as unknown as { __vpEngine?: { sketchStaticHitTest(x: number, y: number): unknown } })
          .__vpEngine?.sketchStaticHitTest(x, y)),
        client,
      ),
    )
    .toBe(true);
  await clickAtClient(page, client.x, client.y);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as {
            __stores?: { selection: { getState(): { selected: Array<{ kind: string }> } } };
          }).__stores?.selection.getState().selected[0]?.kind,
      ),
    )
    .toBe("sketchRegion");
  await targetVisibility.click();
  await expect(targetVisibility).toHaveAttribute("aria-checked", "true");
  await page.getByRole("button", { name: "Extrude", exact: true }).click();
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
