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
 * Center-rectangle tool (W2-B) — draw + finish → closed-region handoff.
 *
 * Centre click, then ONE corner: the opposite corner is that corner reflected
 * through the centre, so the committed 4 lines still form one closed loop (same
 * region contract the corner-to-corner rectangle satisfies). Mirrors rect.spec.ts:
 * finish the sketch, pick the persistent filled cell, and prove Extrude arms from
 * it — which is what "closed region" actually has to mean.
 */
test("center rectangle draws a closed region and arms extrude from the finished sketch", async ({
  page,
}) => {
  await openEditorDebug(page);
  await bodyOptions(page).first().getByRole("switch").click();
  await hideSeedSketches(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Center rectangle");

  // Centre → one corner (distinct in both axes → non-degenerate half-extents).
  await clickAt(page, 0, 0);
  await clickAt(page, 150, 100);

  // 4 lines minus autoconstraints (H/V + 4 coincidences) → under-constrained.
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);
  await expect(page.getByText(/^Under-constrained · DOF [1-9]/).first()).toBeVisible();

  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(4);
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
        ({ x, y }) =>
          Boolean(
            (window as unknown as { __vpEngine?: { sketchStaticHitTest(x: number, y: number): unknown } })
              .__vpEngine?.sketchStaticHitTest(x, y),
          ),
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
