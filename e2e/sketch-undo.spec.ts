import { test, expect } from "@playwright/test";
import {
  openEditor,
  enterSketchViaPlanePicker,
  clickAt,
  clickAtAwaitingDofChange,
  getSketchSnapshot,
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
  await openEditor(page);
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
