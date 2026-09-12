import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  clickAt,
  clickAtAwaitingDofChange,
  getSketchSnapshot,
  selectSketchTool,
  sketchOptions,
  waitForCameraSettled,
} from "./helpers";

/*
 * Sketch-scoped undo/redo (mock lane) — the REAL user path (plane picker).
 *
 * Draws a 2-segment line chain (anchor + 2 committing clicks; Enter ends the
 * chain — SketchController's capture-phase Enter intercept fires because a
 * gesture is in progress, i.e. `machineState.anchors.length > 0`, so it is NOT
 * the global finishSketch shortcut). Each committed segment pushes ONE undo
 * snapshot in `SketchController.commitNow` (sketchService.ts's "Sketch-scoped
 * undo / redo" section), captured from the state BEFORE that segment landed —
 * so undoing pops the chain one segment at a time, LIFO. ⌘Z/Ctrl+Z and
 * ⇧⌘Z/⇧Ctrl+Z resolve to `undoSketch`/`redoSketch` (useShortcuts.ts) only while
 * `toolStore.mode === "sketch"`.
 *
 * Entity counts are read via `getSketchSnapshot`, which requires the LIVE
 * session and therefore must run before finishing/leaving sketch mode
 * (sketchStore.session clears once mode leaves "sketch") — every read here
 * happens while still mid-sketch, by construction.
 */
test("sketch-scoped undo/redo walks a 2-segment line chain back and forward", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page); // default tool: Line

  const lineCount = async (): Promise<number> => (await getSketchSnapshot(page)).lines.length;

  // Anchor + 2 committing clicks → 2 Line entities, well clear of the panels.
  await clickAt(page, -160, -80);
  await clickAtAwaitingDofChange(page, -40, -80);
  await clickAtAwaitingDofChange(page, 60, 10);
  await page.keyboard.press("Enter"); // ends the chain, does NOT finish the sketch

  await expect.poll(lineCount).toBe(2);

  // Undo x2: pops one committed segment per undo entry, most-recent-first.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(lineCount).toBe(1);

  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(lineCount).toBe(0);

  // A third undo is a no-op (undoStack empty) — count stays put.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(lineCount).toBe(0);

  // Redo x2: replays both undone commits, restoring the original chain.
  await page.keyboard.press("ControlOrMeta+Shift+Z");
  await expect.poll(lineCount).toBe(1);

  await page.keyboard.press("ControlOrMeta+Shift+Z");
  await expect.poll(lineCount).toBe(2);

  // Still mid-sketch throughout (undo/redo never leaves sketch mode).
  await expect(page.getByText(/^Editing /)).toBeVisible();
});

/** Live sketch entity count, or -1 when no session is open (see the re-entry
 *  note in the second test — a poll must be able to wait, not throw). */
function liveEntityCount(page: Page): Promise<number> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { sketch: { getState(): { session: { entities: unknown[] } | null } } };
    };
    return w.__stores?.sketch.getState().session?.entities.length ?? -1;
  });
}

/*
 * WP-U7 D-1 — the chrome bar's **Cancel** DISCARDS everything done since entering
 * the sketch, where Esc and Finish keep it. Two shapes, because the backend can
 * only do half of the first one: a sketch MINTED during this visit has its
 * `AddSketch` below the session watermark, so the revert cannot remove it and the
 * frontend's `deleteSketch` compensation must.
 */
test("Cancel deletes a sketch created in this visit", async ({ page }) => {
  await openEditorDebug(page);
  const rowsBefore = await sketchOptions(page).count();

  await enterSketchViaPlanePicker(page); // default tool: Line
  await waitForCameraSettled(page);
  await expect(sketchOptions(page)).toHaveCount(rowsBefore + 1);

  await clickAt(page, -160, -80);
  await clickAtAwaitingDofChange(page, -40, -80);
  await clickAtAwaitingDofChange(page, 60, 10);
  await page.keyboard.press("Enter"); // ends the chain, does NOT finish the sketch
  await expect.poll(() => liveEntityCount(page)).toBe(2);

  await page.getByRole("button", { name: "Cancel", exact: true }).click();

  await expect(page.getByText(/^Editing /)).toHaveCount(0);
  await expect(sketchOptions(page)).toHaveCount(rowsBefore);
});

test("Cancel restores an existing sketch to its state at entry", async ({ page }) => {
  await openEditorDebug(page);

  // Re-open a persisted sketch the way a user does (double-click its tree row).
  // The count reader returns -1 instead of throwing so `expect.poll` can wait out
  // the async `enterSketch` round-trip — the chrome shows "Editing …" as soon as
  // the tree sets the active sketch, which is BEFORE the session lands (the same
  // reason `sketch-multi-object.spec.ts` reads it this way).
  await sketchOptions(page).last().dblclick();
  await expect(page.getByText(/^Editing /)).toBeVisible();
  await waitForCameraSettled(page);
  const entitiesAtEntry = await liveEntityCount(page);
  expect(entitiesAtEntry).toBeGreaterThan(0);

  await selectSketchTool(page, "Line");
  await clickAt(page, -160, -80);
  await clickAtAwaitingDofChange(page, -40, -80);
  await page.keyboard.press("Enter");
  await expect.poll(() => liveEntityCount(page)).toBe(entitiesAtEntry + 1);

  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.getByText(/^Editing /)).toHaveCount(0);

  // Re-enter: the drawn line is gone and the sketch itself survives.
  await sketchOptions(page).last().dblclick();
  await expect(page.getByText(/^Editing /)).toBeVisible();
  await expect.poll(() => liveEntityCount(page)).toBe(entitiesAtEntry);
});
