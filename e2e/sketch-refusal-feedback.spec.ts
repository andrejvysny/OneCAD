import { test, expect } from "./fixtures";
import {
  CANVAS,
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtAwaitingDofChange,
  clickAtClient,
  constraintToolbar,
  dofPill,
  getSketchConstraints,
  getSketchSnapshot,
  liveDimField,
  planePointToClient,
  type SketchLineSnapshot,
} from "./helpers";

/*
 * Sketch feedback residuals of SKETCH_UX_AUDIT.md item #10 (mock lane).
 *
 * 1. NEAR-ACTION refusal pulse. A refused edit already reaches the status bar
 *    in the bottom-left corner, which is nowhere near where the user is
 *    looking. `SketchErrorPulse` mirrors the same error-severity hint next to
 *    the action, once, and then gets out of the way.
 *    The refusal used here is a genuinely CONFLICTING dimension edit: a rect
 *    edge is corner-welded, so `mockEnforce` cannot drive its Distance to a new
 *    value and reports the edit as `Conflicting` — `editConstraintValueNow`
 *    reverts it and raises an error hint (audit item #8's reject branch).
 *
 * 2. A12 — the Esc ladder's idle exit used to leave sketch mode in total
 *    silence. It now reports the same confirmation the Finish button does.
 */

const mid = (l: SketchLineSnapshot): { x: number; y: number } => ({
  x: (l.p0[0] + l.p1[0]) / 2,
  y: (l.p0[1] + l.p1[1]) / 2,
});

/** Move the pointer (no click) to a canvas-center-relative offset, so the live
 *  dimension chips populate non-degenerate values. */
async function hoverAt(page: import("@playwright/test").Page, dx: number, dy: number): Promise<void> {
  const box = await page.locator(CANVAS).boundingBox();
  if (!box) throw new Error("viewport canvas has no bounding box");
  await page.mouse.move(box.x + box.width / 2 + dx, box.y + box.height / 2 + dy);
}

test("a refused dimension edit pulses next to the action, then clears itself", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Rectangle");

  // A rect whose WIDTH is typed: that authors one Distance on the horizontal
  // edge (`liveDimConstraints.boxSpecs`), and the rect's own corner
  // coincidences weld it — accepted on creation, refusable on edit.
  await clickAt(page, -140, -90);
  await hoverAt(page, 60, 40);
  await page.keyboard.press("8");
  await expect(liveDimField(page, "width")).toBeFocused();
  await page.keyboard.press("0");
  await page.keyboard.press("Enter"); // locks width=80mm AND commits the rect
  await expect(dofPill(page)).not.toHaveText("DOF: 0");

  const chip = page.getByTestId("constraint-badges").getByLabel("Dimension value");
  await expect(chip).toHaveValue("80");

  // A real click first: it is what puts the pointer where the pulse anchors
  // (`fill()` alone never moves the mouse).
  await chip.click();
  await chip.fill("120");
  await chip.press("Enter");

  const bubble = page.getByTestId("sketch-error-pulse");
  await expect(bubble).toBeVisible();
  await expect(bubble).toContainText(/reverted/i);
  // The status bar keeps its own copy — the pulse ADDS a surface, it does not
  // move the message.
  await expect(page.getByTestId("status-hint")).toContainText(/reverted/i);
  // It floats over the canvas the next click has to reach.
  expect(await bubble.evaluate((el) => getComputedStyle(el).pointerEvents)).toBe("none");
  // Near the action, not parked in a corner of the window.
  const box = await bubble.boundingBox();
  expect(box).not.toBeNull();
  const chipBox = await chip.boundingBox();
  expect(chipBox).not.toBeNull();
  expect(Math.abs(box!.y - chipBox!.y)).toBeLessThan(240);

  // Transient by design (~2.5s), unlike the sticky corner copy.
  await expect(bubble).toBeHidden({ timeout: 8000 });
});

test("a conflicting constraint refusal reaches BOTH surfaces: the corner pulse and the near-action one", async ({
  page,
}) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // One horizontal line and one vertical line, drawn apart so they share no
  // point: `autoConstrain` gives the first a Horizontal and the second a
  // Vertical. Parallel between them is then a provable contradiction
  // (`mockConflicts` R5: H(a) + V(b) + Parallel(a,b)).
  await selectSketchTool(page, "Line");
  await clickAt(page, -180, -140);
  await clickAtAwaitingDofChange(page, -60, -140);
  await page.keyboard.press("Escape");

  await selectSketchTool(page, "Line");
  await clickAt(page, 60, -40);
  await clickAtAwaitingDofChange(page, 60, 80);
  await page.keyboard.press("Escape");

  // Precondition, asserted rather than assumed: without the H/V pair there is
  // no contradiction and this spec would be testing nothing.
  const kinds = (await getSketchConstraints(page)).map((c) => c.type);
  expect(kinds).toContain("Horizontal");
  expect(kinds).toContain("Vertical");

  await selectSketchTool(page, "Select");
  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(2);
  const a = await planePointToClient(page, snap.plane, mid(snap.lines[0]));
  const b = await planePointToClient(page, snap.plane, mid(snap.lines[1]));

  await clickAtClient(page, a.x, a.y);
  await page.keyboard.down("Shift");
  await clickAtClient(page, b.x, b.y);
  await page.keyboard.up("Shift");

  const toolbar = await constraintToolbar(page);
  const parallelBtn = toolbar.getByRole("button", { name: "Parallel", exact: true });
  await expect(parallelBtn).toBeEnabled();
  await parallelBtn.click();

  // Near the action…
  const bubble = page.getByTestId("sketch-error-pulse");
  await expect(bubble).toBeVisible();
  await expect(bubble).toContainText(/Constraint removed/);
  // …and in the corner, where the error treatment (item #10's one-shot pulse)
  // is what a refusal now gets instead of ordinary grey prompt text.
  const corner = page.getByTestId("status-hint");
  await expect(corner).toContainText(/Constraint removed/);
  await expect(corner).toHaveClass(/hint-error-pulse/);

  // Refused, not applied: the Parallel never entered the constraint set.
  expect((await getSketchConstraints(page)).map((c) => c.type)).not.toContain("Parallel");
});

test("A12: the idle Esc ladder leaves the sketch with a confirmation, not in silence", async ({
  page,
}) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Line");

  await clickAt(page, -120, -60);
  await clickAtAwaitingDofChange(page, 60, 40);

  // The ladder, one rung per press. The third rung is the model selection:
  // opening a session SELECTS its own sketch (`SketchController.openSession`),
  // so a freshly entered sketch always has one to clear before the exit rung.
  await page.keyboard.press("Escape"); // in-gesture tier: end the chain
  await page.keyboard.press("Escape"); // rung 1: drop the Line tool
  await page.keyboard.press("Escape"); // rung 3: clear the selection
  await page.keyboard.press("Escape"); // rung 4: leave sketch mode

  await expect(page.getByTestId("status-hint")).toContainText(/^Finished .+ — 1 entity$/);
  // Really out of sketch mode: the sketch chrome row is gone.
  await expect(page.getByRole("button", { name: /Finish sketch/ })).toBeHidden();
});
