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
  getSketchSnapshot,
  planePointToClient,
  commitExtrudeAtHandle,
} from "./helpers";

/*
 * TRUST wave — the model tree's eye is BACKEND-BACKED, not a local zustand flip.
 *
 * The defect this pins: hiding a body wrote only `documentStore.setVisibility`, so
 * the next authoritative projection (`projection-updated` after ANY commit, a
 * reopen, a from-0 regen) silently restored `visible: true`. The honest assertion
 * is therefore NOT "the eye flips" — it is "the hide SURVIVES an unrelated commit".
 *
 * The mock lane models that authority faithfully: `mockClient` keeps its own
 * body/sketch metadata registry and re-asserts it onto the projection store on
 * every committed op, exactly as the real backend does through projection-updated.
 * A local-only flip is therefore reverted here too — so this spec fails against the
 * pre-TRUST tree instead of passing vacuously.
 */

/** The mock seed body's live visibility, straight from the projection store. */
function seedBodyVisible(page: Page): Promise<boolean | undefined> {
  return page.evaluate(
    () =>
      (
        window as unknown as {
          __stores?: {
            document: { getState(): { bodies: Record<string, { visible: boolean }> } };
          };
        }
      ).__stores?.document.getState().bodies.body1?.visible,
  );
}

/**
 * Commit one UNRELATED extrude through the production flow (draw a rectangle →
 * finish → select its region → Extrude → drag + Enter). This is the event that
 * used to wipe the tree's local visibility state.
 */
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

  await page.keyboard.press("Enter"); // finish the sketch → back to model mode
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

/** Hide every seed sketch so the static layer never shadows the region pick. */
async function hideSeedSketches(page: Page): Promise<void> {
  const visible = page
    .getByRole("listbox", { name: "Sketches" })
    .locator('[role="switch"][aria-checked="true"]');
  while ((await visible.count()) > 0) await visible.first().click();
}

test("hiding a body through the tree eye SURVIVES a later unrelated commit", async ({ page }) => {
  await openEditorDebug(page);
  await hideSeedSketches(page);

  const seedBody = bodyOptions(page).first();
  const eye = seedBody.getByRole("switch");
  await expect(eye).toHaveAttribute("aria-checked", "true");

  await eye.click();
  await expect(eye).toHaveAttribute("aria-checked", "false");
  expect(await seedBodyVisible(page)).toBe(false);

  await commitUnrelatedExtrude(page);

  // The whole point: the hide is the BACKEND's fact now, so the commit's
  // authoritative projection re-asserts it rather than resetting it.
  expect(await seedBodyVisible(page)).toBe(false);
  await expect(bodyOptions(page).first().getByRole("switch")).toHaveAttribute(
    "aria-checked",
    "false",
  );
});

test("the row context menu drives the same visibility command", async ({ page }) => {
  await openEditorDebug(page);
  const seedBody = bodyOptions(page).first();

  await seedBody.click({ button: "right" });
  await expect(page.getByTestId("tree-menu-visibility")).toHaveText("Hide");
  await page.getByTestId("tree-menu-visibility").click();

  await expect(seedBody.getByRole("switch")).toHaveAttribute("aria-checked", "false");
  expect(await seedBodyVisible(page)).toBe(false);

  // Re-opening the menu on a hidden row offers Show, and it flips back.
  await seedBody.click({ button: "right" });
  await expect(page.getByTestId("tree-menu-visibility")).toHaveText("Show");
  await page.getByTestId("tree-menu-visibility").click();
  await expect(seedBody.getByRole("switch")).toHaveAttribute("aria-checked", "true");
  expect(await seedBodyVisible(page)).toBe(true);
});
