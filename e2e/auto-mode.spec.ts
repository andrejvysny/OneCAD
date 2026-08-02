import { test, expect } from "./fixtures";
import {
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
 * AUTO-MODE — the editor mode follows tool + context; there is no mode toggle.
 *
 *  1. A sketch-tool SHORTCUT from model mode (L) starts the new-sketch flow
 *     directly (plane picker → session with Line armed).
 *  2. A model-tool shortcut from sketch mode (E) finishes the sketch through
 *     the SAME handoff as Enter — region pick → E/Extrude arms the tool.
 *  3. Double-clicking a finished sketch in the viewport re-enters its edit
 *     session (mirrors the tree row double-click).
 */
test("L from model mode opens the plane picker with Line armed", async ({ page }) => {
  await openEditorDebug(page);

  await page.keyboard.press("l");
  // New-sketch intent: plane-pick chrome, then the quad click opens the session.
  await expect(page.getByText("Select a sketch plane")).toBeVisible();

  const box = await page.locator('[data-testid="viewport-canvas"]').boundingBox();
  if (!box) throw new Error("no canvas box");
  const cx = box.x + box.width / 2;
  const cy = box.y + box.height / 2;
  const chip = page.locator("[data-plane-pick-label]");
  await expect(async () => {
    await page.mouse.move(cx, cy - 1);
    await page.mouse.move(cx, cy);
    expect(await chip.isVisible()).toBe(true);
  }).toPass({ timeout: 20_000, intervals: [150, 250, 400, 600] });
  await page.mouse.down();
  await page.mouse.up();

  // Session opened with the PRESERVED tool (Line), not a re-defaulted one.
  await expect(page.getByText(/^Editing /)).toBeVisible();
  await expect(page.getByRole("button", { name: "Line", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

test("E from sketch mode finishes; region click + E arms Extrude", async ({ page }) => {
  await openEditorDebug(page);
  await bodyOptions(page).first().getByRole("switch").click();
  const visibleSeedSketches = page
    .getByRole("listbox", { name: "Sketches" })
    .locator('[role="switch"][aria-checked="true"]');
  while ((await visibleSeedSketches.count()) > 0) await visibleSeedSketches.first().click();
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Rectangle");

  await clickAt(page, -150, -100);
  await clickAt(page, 150, 100);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);
  const snap = await getSketchSnapshot(page);
  const xs = snap.lines.flatMap((line) => [line.p0[0], line.p1[0]]);
  const ys = snap.lines.flatMap((line) => [line.p0[1], line.p1[1]]);
  const centroid = {
    x: (Math.min(...xs) + Math.max(...xs)) / 2,
    y: (Math.min(...ys) + Math.max(...ys)) / 2,
  };

  // E — the model-tool shortcut — finishes the sketch (same handoff as Enter):
  // model mode, region-selection hint, select tool armed for the pick.
  await page.keyboard.press("e");
  await expect(page.getByText(/^Editing /)).toHaveCount(0);
  await expect(page.getByRole("button", { name: "New sketch", exact: true })).toBeVisible();
  await expect(page.getByText("Select one closed sketch region, then choose Extrude")).toBeVisible();
  await waitForCameraSettled(page);

  // Pick the filled region, then E again (or the Extrude button) arms the tool.
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
  await page.keyboard.press("e");

  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Extrude", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

test("double-clicking a finished sketch re-enters its edit session", async ({ page }) => {
  await openEditorDebug(page);
  await bodyOptions(page).first().getByRole("switch").click();
  const visibleSeedSketches = page
    .getByRole("listbox", { name: "Sketches" })
    .locator('[role="switch"][aria-checked="true"]');
  while ((await visibleSeedSketches.count()) > 0) await visibleSeedSketches.first().click();
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Rectangle");

  await clickAt(page, -150, -100);
  await clickAt(page, 150, 100);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);
  const snap = await getSketchSnapshot(page);
  const edge = snap.lines[0];
  const midpoint = { x: (edge.p0[0] + edge.p1[0]) / 2, y: (edge.p0[1] + edge.p1[1]) / 2 };

  await page.keyboard.press("Enter");
  await expect(page.getByText(/^Editing /)).toHaveCount(0);
  await waitForCameraSettled(page);

  // Double-click the sketch curve (static layer) → edit session reopens.
  const client = await planePointToClient(page, snap.plane, midpoint);
  await expect
    .poll(() =>
      page.evaluate(
        ({ x, y }) => Boolean((window as unknown as { __vpEngine?: { sketchStaticHitTest(x: number, y: number): unknown } })
          .__vpEngine?.sketchStaticHitTest(x, y)),
        client,
      ),
    )
    .toBe(true);
  await page.mouse.move(client.x, client.y);
  await page.mouse.dblclick(client.x, client.y);

  await expect(page.getByText(/^Editing /)).toBeVisible();
  await expect(page.getByRole("button", { name: "Line", exact: true })).toBeVisible();
});
