import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
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
  commitExtrudeAtHandle,
} from "./helpers";

/*
 * Unsaved-changes guard (F-shell). Every in-app close path — TitleBar × and ⌘W —
 * shares ONE entry point, `appStore.requestClose` (fileActions.closeProject); a
 * DIRTY document must prompt via the UnsavedChangesDialog instead of discarding
 * work silently, while a clean one keeps closing immediately (unchanged
 * behavior, covered by the existing navigation/acceptance specs).
 *
 * The mock lane (this file) only exercises the "close" intent — TitleBar × / ⌘W.
 * The native window-close button and ⌘Q both route through the SAME dialog via a
 * second intent ("quit", see appStore.ts + EditorScreen's onCloseRequested
 * subscription), but that path needs a real Tauri window/process and is
 * therefore manual-gate territory (see src-tauri/src/lib.rs's on_window_event /
 * RunEvent::ExitRequested wiring + the confirm_exit/cancel_exit commands).
 */

/** Read the live `documentStore.dirty` flag (always exposed under dev — see
 *  helpers.ts's getSketchSnapshot for the same `window.__stores` pattern). */
async function isDirty(page: Page): Promise<boolean> {
  return page.evaluate(() => {
    const w = window as unknown as { __stores?: { document: { getState(): { dirty: boolean } } } };
    return w.__stores?.document.getState().dirty ?? false;
  });
}

/**
 * Draw a rectangle, finish the sketch, select its region, and commit a REAL
 * extrude gesture — mirrors extrude-commit-gesture.spec.ts's `armExtrude` +
 * `commitExtrudeAtHandle` (that flow is private to that file; helpers.ts holds
 * the shared primitives this composes). The shortest real path to a dirty
 * document: `ModelToolController.applyResult` flips `documentStore.dirty` on
 * every committed op.
 */
async function commitRectExtrude(page: Page): Promise<void> {
  await openEditorDebug(page);
  await bodyOptions(page).first().getByRole("switch").click();
  await hideSeedSketches(page);
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
  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();
  await commitExtrudeAtHandle(page);

  await expect.poll(() => isDirty(page)).toBe(true);
}

const closeButton = (page: Page) => page.getByRole("button", { name: "Close project" });
const dialog = (page: Page) => page.getByRole("dialog");
const startScreen = (page: Page) => page.getByRole("button", { name: "New project" });

test("TitleBar × on a dirty document opens the unsaved-changes prompt (no silent discard)", async ({ page }) => {
  await commitRectExtrude(page);

  await closeButton(page).click();

  await expect(dialog(page)).toBeVisible();
  await expect(startScreen(page)).toHaveCount(0); // still in the editor, nothing closed yet
  await expect.poll(() => isDirty(page)).toBe(true);
});

test("Cancel keeps the editor open and discards nothing", async ({ page }) => {
  await commitRectExtrude(page);
  await closeButton(page).click();
  await expect(dialog(page)).toBeVisible();

  await dialog(page).getByRole("button", { name: "Cancel" }).click();

  await expect(dialog(page)).toHaveCount(0);
  await expect(closeButton(page)).toBeVisible(); // still the editor shell
  await expect.poll(() => isDirty(page)).toBe(true); // nothing was discarded
});

test("Don't Save discards and returns to the start screen", async ({ page }) => {
  await commitRectExtrude(page);
  await closeButton(page).click();
  await expect(dialog(page)).toBeVisible();

  await dialog(page)
    .getByRole("button", { name: /Don.t Save/ })
    .click();

  await expect(dialog(page)).toHaveCount(0);
  await expect(startScreen(page)).toBeVisible();
});

test("Save (mock resolves) saves then returns to the start screen", async ({ page }) => {
  await commitRectExtrude(page);
  await closeButton(page).click();
  await expect(dialog(page)).toBeVisible();

  await dialog(page)
    .getByRole("button", { name: "Save", exact: true })
    .click();

  await expect(dialog(page)).toHaveCount(0);
  await expect(startScreen(page)).toBeVisible();
});

test("Escape cancels the prompt, same as clicking Cancel", async ({ page }) => {
  await commitRectExtrude(page);
  await closeButton(page).click();
  await expect(dialog(page)).toBeVisible();

  await page.keyboard.press("Escape");

  await expect(dialog(page)).toHaveCount(0);
  await expect(closeButton(page)).toBeVisible();
});
