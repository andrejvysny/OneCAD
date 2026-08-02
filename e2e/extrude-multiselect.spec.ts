import { test, expect } from "./fixtures";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtClient,
  clickAtAwaitingDofChange,
  dofPill,
  bodyOptions,
  getSketchSnapshot,
  getSketchCircle,
  planePointToClient,
} from "./helpers";

/*
 * V1 Extrude is intentionally single-profile. Persistent region selection may
 * contain several cells, but invoking Extrude must reject that ambiguity rather
 * than choosing the first cell or authoring hidden N-operation behavior.
 */
test("multiple selected regions are rejected without preview or commit", async ({
  page,
}) => {
  await openEditorDebug(page);
  await bodyOptions(page).first().getByRole("switch").click();
  const visibleSeedSketches = page
    .getByRole("listbox", { name: "Sketches" })
    .locator('[role="switch"][aria-checked="true"]');
  while ((await visibleSeedSketches.count()) > 0) await visibleSeedSketches.first().click();
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // Rectangle — its own closed loop, well clear of the circle below.
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -260, -150);
  await clickAt(page, -60, 50);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  // Circle — separated in plane space from the rectangle. The radius click commits
  // the circle; await the DOF change so the circle is in the session before reading it
  // (a plain positive-DOF check already held from the rectangle — it wouldn't wait).
  await selectSketchTool(page, "Circle");
  await clickAt(page, 120, -80); // center
  await clickAtAwaitingDofChange(page, 200, -80); // radius → commits the circle

  // Snapshot the plane + rectangle lines + circle BEFORE finishing (session clears).
  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(4);
  const circle = await getSketchCircle(page);
  const sum = snap.lines.reduce(
    (acc, l) => ({ x: acc.x + l.p0[0] + l.p1[0], y: acc.y + l.p0[1] + l.p1[1] }),
    { x: 0, y: 0 },
  );
  const rectCentroid = { x: sum.x / (snap.lines.length * 2), y: sum.y / (snap.lines.length * 2) };
  const circleCentroid = { x: circle.center[0], y: circle.center[1] };

  const bodiesBefore = await bodyOptions(page).count();

  await page.keyboard.press("Enter");
  await waitForCameraSettled(page);

  const rectPt = await planePointToClient(page, snap.plane, rectCentroid);
  await expect
    .poll(() =>
      page.evaluate(
        ({ x, y }) => Boolean((window as unknown as { __vpEngine?: { sketchStaticHitTest(x: number, y: number): unknown } })
          .__vpEngine?.sketchStaticHitTest(x, y)),
        rectPt,
      ),
    )
    .toBe(true);
  await clickAtClient(page, rectPt.x, rectPt.y);

  const circlePt = await planePointToClient(page, snap.plane, circleCentroid);
  await page.keyboard.down("Shift");
  await clickAtClient(page, circlePt.x, circlePt.y);
  await page.keyboard.up("Shift");

  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await expect(page.getByText("Extrude takes exactly one region — deselect down to one")).toBeVisible();
  await expect(page.getByText(/^Drag the arrow to set depth/)).toHaveCount(0);
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore);
});
