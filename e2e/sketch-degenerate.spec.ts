import { test, expect } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  getSketchEntityCount,
} from "./helpers";

/*
 * Degenerate-input rejection (mock lane) — a same-point / same-axis click
 * commits NOTHING, across the three tools that can be fed exactly this input
 * (toolMachine.ts, `minSize` = a screen-scaled ~4px world-unit floor):
 *   - Line:   a click within `minSize` of the LAST anchor ends the chain with
 *             NO commit at all (covers the 1-anchor case: first === last).
 *   - Circle: a same-point second click gives radius < minSize — the click is
 *             IGNORED (the center anchor stays armed, waiting for a real
 *             radius click) rather than ending the gesture, but nothing
 *             commits either way.
 *   - Rect:   a same-x second click is zero-extent on the x axis — likewise
 *             ignored (the check is per-axis; either alone is enough to
 *             reject), corner anchor kept, nothing committed.
 *
 * Camera settle (`waitForCameraSettled`) matters here specifically: two of the
 * three cases fire the SAME client pixel twice back-to-back with no async
 * wait in between (nothing commits, so there is no DOF change to poll on) —
 * an in-flight camera tween moving between those two raycasts would desync
 * "same screen pixel" from "same plane point" (see helpers.ts's documented
 * root cause for `waitForCameraSettled`). Switching tools between cases resets
 * the tool machine (SketchController.selectMachine always calls `m.init()`),
 * so each case starts clean regardless of the previous one leaving its own
 * tool armed and waiting for a real second click.
 */
test("a same-point / same-axis click commits nothing for line, circle, and rectangle", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page); // default tool: Line
  await waitForCameraSettled(page);

  await expect(getSketchEntityCount(page)).resolves.toBe(0);

  // Line: click A, click A again (identical client pixel) -> chain ends, no commit.
  await clickAt(page, -80, -40);
  await clickAt(page, -80, -40);
  await expect(getSketchEntityCount(page)).resolves.toBe(0);

  // Circle: center click, same-point click again -> radius 0 < minSize, ignored.
  await selectSketchTool(page, "Circle");
  await clickAt(page, 40, 40);
  await clickAt(page, 40, 40);
  await expect(getSketchEntityCount(page)).resolves.toBe(0);

  // Rectangle: corner click, same-x second click (different y) -> zero
  // x-extent, ignored.
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -60, 100);
  await clickAt(page, -60, -150);
  await expect(getSketchEntityCount(page)).resolves.toBe(0);
});
