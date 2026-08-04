import { test, expect } from "./fixtures";
import {
  CANVAS,
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  dofPill,
  getLiveDims,
  getSketchSnapshot,
  liveDimField,
} from "./helpers";

/** Move the pointer (no click) to a canvas-center-relative offset — hovers a
 *  point so the live dimension chips populate NON-degenerate values. */
async function hoverAt(page: import("@playwright/test").Page, dx: number, dy: number): Promise<void> {
  const box = await page.locator(CANVAS).boundingBox();
  if (!box) throw new Error("viewport canvas has no bounding box");
  await page.mouse.move(box.x + box.width / 2 + dx, box.y + box.height / 2 + dy);
}

/*
 * Live dimension chips — rectangle tool (SP-1 W4).
 *
 * The mock solver is an identity-echo (no real solve); DOF is a coarse
 * "free params − removed" count charging exactly 1 DOF per dimensional
 * constraint (`constraintFreedom`, mockSketch.ts).
 *
 * Baseline vs typed captured RELATIVELY: an untyped rectangle is drawn, its DOF
 * + Distance-row count read, then undone — so the typed rectangle's -2 DOF /
 * +2 Distance-row delta is attributable ONLY to the two typed extents, not to
 * the (identical either way) H/V + corner-coincidence autoconstraints every
 * axis-aligned rect gets regardless of typed dims.
 */
test("typing width + height on the rect chip drives exact extents and authors two Distance", async ({
  page,
}) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Rectangle");

  // ── Baseline: an untyped rect, then undo it ─────────────────────────────────
  await clickAt(page, -150, -100);
  await clickAt(page, 150, 100);
  await expect(dofPill(page)).not.toHaveText("DOF: 0");
  const baselineDofText = (await dofPill(page).textContent()) ?? "";
  const baselineDof = Number(baselineDofText.match(/\d+/)?.[0]);
  const baselineDistanceRows = await page.getByText("Distance", { exact: true }).count();
  expect(baselineDistanceRows).toBe(0); // width/height untyped ⇒ nothing dimensional

  await page.keyboard.press("ControlOrMeta+z");
  await expect(dofPill(page)).toHaveText("DOF: 0");

  // ── Typed rect: chips visible with sane values before anything is typed ─────
  await clickAt(page, -140, -90); // corner
  await hoverAt(page, 20, 40);
  const dims = await getLiveDims(page);
  const width = dims.find((d) => d.field === "width");
  const height = dims.find((d) => d.field === "height");
  expect(width?.value).toBeGreaterThan(0);
  expect(height?.value).toBeGreaterThan(0);
  expect(width?.locked).toBe(false);
  expect(height?.locked).toBe(false);
  await expect(liveDimField(page, "width")).toBeVisible();
  await expect(liveDimField(page, "height")).toBeVisible();

  // Type "80" — opens + focuses the width chip.
  await page.keyboard.press("8");
  await expect(liveDimField(page, "width")).toBeFocused();
  await page.keyboard.press("0");
  await page.keyboard.press("Tab"); // locks width=80mm, moves focus to height
  // NOTE: same confirmed src bug as live-dim-line.spec.ts's Tab step (Tab
  // between two live-dim fields drops the FSM's focus back to null one tick
  // after DOM focus lands on the second field — see that spec for the full
  // root-cause writeup). This assertion is written to the intended contract
  // and is expected to fail until the bug is fixed.
  await expect(liveDimField(page, "height")).toBeFocused();
  const midDims = await getLiveDims(page);
  const lockedWidth = midDims.find((d) => d.field === "width");
  expect(lockedWidth?.locked).toBe(true);
  expect(lockedWidth?.value).toBeCloseTo(80, 3);

  await page.keyboard.press("4");
  await page.keyboard.press("0");
  await page.keyboard.press("Enter"); // locks height=40mm AND commits the rect
  // The commit round-trips through the ASYNC mock sketchUpsert — wait for the
  // DOF pill to move off the post-undo "DOF: 0" before reading geometry.
  await expect(dofPill(page)).not.toHaveText("DOF: 0");

  // ── Geometry: 4 lines, extents 80×40 ────────────────────────────────────────
  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(4);
  const widthLine = snap.lines[0]; // rectLines(a,b): index0 = horizontal edge
  const heightLine = snap.lines[1]; // index1 = vertical edge
  const lineLength = (l: { p0: [number, number]; p1: [number, number] }) =>
    Math.hypot(l.p1[0] - l.p0[0], l.p1[1] - l.p0[1]);
  expect(lineLength(widthLine)).toBeCloseTo(80, 3);
  expect(lineLength(heightLine)).toBeCloseTo(40, 3);

  // ── Constraint + DOF: two Distance rows, DOF two lower than baseline ────────
  await expect(page.getByText("Distance", { exact: true })).toHaveCount(2);
  const finalDofText = (await dofPill(page).textContent()) ?? "";
  const finalDof = Number(finalDofText.match(/\d+/)?.[0]);
  expect(finalDof).toBe(baselineDof - 2);
});
