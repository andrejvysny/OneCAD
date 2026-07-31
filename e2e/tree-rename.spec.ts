import { test, expect, type Page } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtClient,
  dofPill,
  bodyOptions,
  sketchOptions,
  getSketchSnapshot,
  planePointToClient,
  commitExtrudeAtHandle,
} from "./helpers";

/*
 * TRUST wave — inline tree rename, backed by `RenameBody` / `RenameSketch`.
 *
 * Same shape as tree-visibility: the interesting assertion is not that the label
 * changes, it is that the new name SURVIVES the next authoritative projection. The
 * mock lane re-asserts its own metadata registry on every committed op (see
 * mockClient), so a rename that never reached the backend is reverted here exactly
 * as it would be against the real one.
 */

/** The mock seed body's live name, straight from the projection store. */
function seedBodyName(page: Page): Promise<string | undefined> {
  return page.evaluate(
    () =>
      (
        window as unknown as {
          __stores?: { document: { getState(): { bodies: Record<string, { name: string }> } } };
        }
      ).__stores?.document.getState().bodies.body1?.name,
  );
}

/** Right-click a tree row → Rename → type a new name → Enter. */
async function renameRow(page: Page, row: ReturnType<Page["locator"]>, from: string, to: string) {
  await row.click({ button: "right" });
  await page.getByTestId("tree-menu-rename").click();
  const field = page.getByLabel(`Rename ${from}`);
  await expect(field).toBeFocused();
  await field.fill(to);
  await field.press("Enter");
}

/** Commit one unrelated extrude (the event that used to wipe local tree state). */
async function commitUnrelatedExtrude(page: Page): Promise<void> {
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -150, -100);
  await clickAt(page, 150, 100);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);
  const snap = await getSketchSnapshot(page);
  const p0 = snap.lines[0]?.p0;
  const p2 = snap.lines[2]?.p0;
  if (!p0 || !p2) throw new Error("rectangle snapshot is incomplete");
  const centroid = { x: (p0[0] + p2[0]) / 2, y: (p0[1] + p2[1]) / 2 };

  await page.keyboard.press("Enter");
  await waitForCameraSettled(page);
  const client = await planePointToClient(page, snap.plane, centroid);
  await expect
    .poll(() =>
      page.evaluate(
        ({ x, y }) =>
          Boolean(
            (
              window as unknown as {
                __vpEngine?: { sketchStaticHitTest(x: number, y: number): unknown };
              }
            ).__vpEngine?.sketchStaticHitTest(x, y),
          ),
        client,
      ),
    )
    .toBe(true);
  await clickAtClient(page, client.x, client.y);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as unknown as {
              __stores?: { selection: { getState(): { selected: Array<{ kind: string }> } } };
            }
          ).__stores?.selection.getState().selected[0]?.kind,
      ),
    )
    .toBe("sketchRegion");
  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();
  await commitExtrudeAtHandle(page);
}

async function hideSeedSketches(page: Page): Promise<void> {
  const visible = page
    .getByRole("listbox", { name: "Sketches" })
    .locator('[role="switch"][aria-checked="true"]');
  while ((await visible.count()) > 0) await visible.first().click();
}

test("renaming a body via the context menu SURVIVES a later unrelated commit", async ({ page }) => {
  await openEditorDebug(page);
  await hideSeedSketches(page);
  await bodyOptions(page).first().getByRole("switch").click(); // hide it → NewBody extrude

  await renameRow(page, bodyOptions(page).first(), "Body 1", "Housing");
  await expect(bodyOptions(page).first()).toContainText("Housing");
  expect(await seedBodyName(page)).toBe("Housing");

  await commitUnrelatedExtrude(page);

  expect(await seedBodyName(page)).toBe("Housing");
  await expect(page.getByRole("listbox", { name: "Bodies" })).toContainText("Housing");
});

test("Escape abandons an inline rename; F2 reopens it on the selected row", async ({ page }) => {
  await openEditorDebug(page);
  const row = sketchOptions(page).filter({ hasText: "Sketch 4" });

  await row.click({ button: "right" });
  await page.getByTestId("tree-menu-rename").click();
  const field = page.getByLabel("Rename Sketch 4");
  await field.fill("Profile");
  await field.press("Escape");
  await expect(page.getByLabel("Rename Sketch 4")).toHaveCount(0);
  await expect(row).toContainText("Sketch 4");

  // F2 on the (now selected) row reopens the field, and Enter commits for real.
  await row.click();
  await page.keyboard.press("F2");
  const reopened = page.getByLabel("Rename Sketch 4");
  await expect(reopened).toBeFocused();
  await reopened.fill("Profile");
  await reopened.press("Enter");
  await expect(sketchOptions(page).filter({ hasText: "Profile" })).toHaveCount(1);
});

test("an empty rename is refused: the name is kept and the failure is surfaced", async ({ page }) => {
  await openEditorDebug(page);
  await renameRow(page, bodyOptions(page).first(), "Body 1", "   ");
  await expect(bodyOptions(page).first()).toContainText("Body 1");
  expect(await seedBodyName(page)).toBe("Body 1");
  await expect(page.getByText(/name cannot be empty/)).toBeVisible();
});
