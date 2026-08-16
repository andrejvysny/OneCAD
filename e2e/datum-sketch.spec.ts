import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  openEditorDebug,
  waitForCameraSettled,
  findPlaneQuad,
  findDatumQuad,
  datumOptions,
  sketchOptions,
  selectSketchTool,
  clickAt,
  dofPill,
} from "./helpers";

/*
 * DATUM W1 — a datum plane as a SKETCH HOST.
 *
 * Two entry routes are covered: the tree row double-click (selection →
 * `tryEnterOnSelectedDatum`) and a datum quad clicked while the world-plane
 * picker is up (`confirmDatumPick`, which wins over the origin quads).
 *
 * The delete guard is exercised LAST and non-vacuously: the mock lane records
 * the sketch→datum attachment when `enterSketch({newOnDatum})` runs, so the
 * rejection here comes from a real reference, not a hard-coded refusal.
 */

/** Create one datum on the XY base at the seeded 10mm offset. */
async function createDatum(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Datum plane", exact: true }).click();
  await waitForCameraSettled(page);
  const quad = await findPlaneQuad(page, "XY");
  await page.mouse.move(quad.x, quad.y);
  await page.mouse.down();
  await page.mouse.up();
  await page.getByTestId("model-tool-chip").getByTestId("chip-confirm").click();
  await expect(datumOptions(page)).toHaveCount(1);
}

/** The plane a sketch session is currently open on (`__stores.sketch`, dev-only). */
async function sessionPlaneOrigin(page: Page): Promise<number[] | null> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { sketch: { getState(): { session: { plane: { origin: number[] } } | null } } };
    };
    return w.__stores?.sketch.getState().session?.plane.origin ?? null;
  });
}

test.describe("datum plane — sketch host", () => {
  test("double-clicking the tree row sketches ON the datum; the sketch survives finish", async ({
    page,
  }) => {
    await openEditorDebug(page);
    await createDatum(page);
    const sketchesBefore = await sketchOptions(page).count();

    // ── the tree row double-click: select + setMode("sketch") with no target ──
    await datumOptions(page).first().dblclick();

    // Sketch chrome is live and the session is a NEW sketch, NOT the plane picker.
    await expect(page.getByText(/^Editing /)).toBeVisible();
    await expect(page.getByText("Select a sketch plane", { exact: true })).toHaveCount(0);
    // …and it opened on the DATUM's resolved frame (XY offset 10 ⇒ world +Z 10),
    // not on world XY. This is the whole point of the feature.
    expect(await sessionPlaneOrigin(page)).toEqual([0, 0, 10]);

    // ── draw a rectangle on it and finish ──
    await waitForCameraSettled(page);
    await selectSketchTool(page, "Rectangle");
    await clickAt(page, -120, -80);
    await clickAt(page, 120, 80);
    await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

    await page.keyboard.press("Enter");
    await expect(page.getByText(/^Editing /)).toHaveCount(0);

    // The hosted sketch is a real tree row, and the datum is still there.
    await expect(sketchOptions(page)).toHaveCount(sketchesBefore + 1);
    await expect(datumOptions(page)).toHaveCount(1);
  });

  test("a datum quad WINS the world-plane picker: 'New sketch' → click the datum", async ({
    page,
  }) => {
    await openEditorDebug(page);
    await createDatum(page);

    await page.getByRole("button", { name: "New sketch", exact: true }).click();
    await expect(page.getByText("Select a sketch plane", { exact: true })).toBeVisible();
    await waitForCameraSettled(page);

    const datum = await findDatumQuad(page);
    await page.mouse.move(datum.x, datum.y);
    await page.mouse.down();
    await page.mouse.up();

    await expect(page.getByText(/^Editing /)).toBeVisible();
    expect(await sessionPlaneOrigin(page)).toEqual([0, 0, 10]);
  });

  test("deleting a REFERENCED datum is rejected, with the blocking sketch named", async ({
    page,
  }) => {
    await openEditorDebug(page);
    await createDatum(page);

    // Host a sketch on it (this is what registers the reference).
    await datumOptions(page).first().dblclick();
    await expect(page.getByText(/^Editing /)).toBeVisible();
    await waitForCameraSettled(page);
    await selectSketchTool(page, "Rectangle");
    await clickAt(page, -120, -80);
    await clickAt(page, 120, 80);
    await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);
    await page.keyboard.press("Enter");
    await expect(page.getByText(/^Editing /)).toHaveCount(0);

    // ── try to delete the datum out from under it ──
    await datumOptions(page).first().click({ button: "right" });
    await page.getByTestId("tree-action-onecad.modeling.command.deleteDatum.action").click();
    await page.getByTestId("tree-action-onecad.modeling.command.deleteDatum.action-confirm").click();

    // Refused, and the message says WHICH sketch is in the way.
    await expect(page.getByText(/Delete datum failed:.*referenced by 1 sketch/)).toBeVisible();
    await expect(datumOptions(page)).toHaveCount(1);
  });

  test("an UNREFERENCED datum deletes cleanly (two-click confirm)", async ({ page }) => {
    await openEditorDebug(page);
    await createDatum(page);

    await datumOptions(page).first().click({ button: "right" });
    await page.getByTestId("tree-action-onecad.modeling.command.deleteDatum.action").click();
    await expect(datumOptions(page)).toHaveCount(1); // first click only arms
    await page.getByTestId("tree-action-onecad.modeling.command.deleteDatum.action-confirm").click();

    await expect(datumOptions(page)).toHaveCount(0);
    await expect(page.getByText("Datum deleted")).toBeVisible();
  });
});
