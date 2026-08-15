import { test, expect } from "./fixtures";
import {
  hideSeedSketches,
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtClient,
  dofPill,
  bodyOptions,
  getSketchSnapshot,
  planePointToClient,
} from "./helpers";

/*
 * Rectangle tool — draw + finish → closed-region handoff.
 *
 * Two corner clicks commit 4 axis-aligned lines forming one closed loop. Enter
 * finishes the sketch back to model mode. Selecting its persistent filled cell,
 * then invoking Extrude, proves the exact closed region is available.
 */
test("rectangle draws a closed region and arms extrude from the finished sketch", async ({ page }) => {
  await openEditorDebug(page);
  await bodyOptions(page).first().getByRole("switch").click();
  await hideSeedSketches(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Rectangle");

  // Corner → opposite corner (distinct in both axes → non-degenerate rect).
  await clickAt(page, -150, -100);
  await clickAt(page, 150, 100);

  // 4 lines minus autoconstraints (H/V + 4 coincidences) → under-constrained.
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);
  await expect(page.getByText(/^Under-constrained · DOF [1-9]/).first()).toBeVisible();
  const snap = await getSketchSnapshot(page);
  const xs = snap.lines.flatMap((line) => [line.p0[0], line.p1[0]]);
  const ys = snap.lines.flatMap((line) => [line.p0[1], line.p1[1]]);
  const centroid = {
    x: (Math.min(...xs) + Math.max(...xs)) / 2,
    y: (Math.min(...ys) + Math.max(...ys)) / 2,
  };

  // Finish the sketch (Enter) → model mode: chrome gone, model toolbar back.
  await page.keyboard.press("Enter");
  await expect(page.getByText(/^Editing /)).toHaveCount(0);
  await expect(page.getByRole("button", { name: "New sketch", exact: true })).toBeVisible();
  await waitForCameraSettled(page);

  // Select the exact filled profile, then invoke Extrude.
  const client = await planePointToClient(page, snap.plane, centroid);
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
  await page.getByRole("button", { name: "Extrude", exact: true }).click();

  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Extrude", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});
