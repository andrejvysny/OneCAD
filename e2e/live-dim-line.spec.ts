import { test, expect } from "./fixtures";
import {
  CANVAS,
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  clickAt,
  clickAtAwaitingDofChange,
  dofPill,
  getLiveDims,
  getSketchSnapshot,
  liveDimField,
} from "./helpers";

/** Move the pointer (no click) to a canvas-center-relative offset — hovers a
 *  point so the live dimension chips populate NON-degenerate values before
 *  anything is typed (`clickAt` moves+downs+ups, which would commit early). */
async function hoverAt(page: import("@playwright/test").Page, dx: number, dy: number): Promise<void> {
  const box = await page.locator(CANVAS).boundingBox();
  if (!box) throw new Error("viewport canvas has no bounding box");
  await page.mouse.move(box.x + box.width / 2 + dx, box.y + box.height / 2 + dy);
}

/*
 * Live dimension chips — line tool (SP-1 W4).
 *
 * The mock solver is an identity-echo: it never MOVES a point (`solveSketch` /
 * localSolver.ts), so a typed length/angle lock lands in the committed geometry
 * verbatim, and DOF is a coarse "free params − removed" count that charges
 * exactly 1 DOF per dimensional constraint (`constraintFreedom`, mockSketch.ts) —
 * no real solve is exercised here.
 *
 * Baseline vs typed is captured RELATIVELY: an untyped oblique segment (~31°,
 * outside the ±5° H/V autoconstrain tolerance — the same angle dimension-conflict
 * .spec.ts already pins to DOF 4) is drawn, undone, then the typed segment is
 * drawn in its place — so the -1 DOF delta is attributable to the ONE Distance
 * constraint the typed length authors, not to incidental autoconstraint noise.
 */
test("typing a length + angle on the line chip drives an exact segment and authors one Distance", async ({
  page,
}) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // ── Baseline: an untyped oblique segment, then undo it ─────────────────────
  await clickAt(page, -180, -140);
  await clickAtAwaitingDofChange(page, -80, -80);
  await page.keyboard.press("Escape");
  const baselineDofText = (await dofPill(page).textContent()) ?? "";
  const baselineDof = Number(baselineDofText.match(/\d+/)?.[0]);
  expect(Number.isFinite(baselineDof)).toBe(true);
  const baselineDistanceRows = await page.getByText("Distance", { exact: true }).count();
  expect(baselineDistanceRows).toBe(0); // an oblique lone segment authors nothing

  await page.keyboard.press("ControlOrMeta+z");
  await expect(dofPill(page)).toHaveText("DOF: 0");

  // ── Typed segment: chips visible with sane values before anything is typed ──
  await clickAt(page, -160, -80); // anchor
  await hoverAt(page, -40, -20);
  const dimsWhileHovering = await getLiveDims(page);
  const lengthField = dimsWhileHovering.find((d) => d.field === "length");
  const angleField = dimsWhileHovering.find((d) => d.field === "angle");
  expect(lengthField).toBeDefined();
  expect(angleField).toBeDefined();
  expect(lengthField?.value).toBeGreaterThan(0);
  expect(lengthField?.locked).toBe(false);
  expect(angleField?.value).toBeGreaterThanOrEqual(0);
  expect(angleField?.value).toBeLessThan(360);
  await expect(liveDimField(page, "length")).toBeVisible();
  await expect(liveDimField(page, "angle")).toBeVisible();

  // Type "50" — opens + focuses the length chip (first digit over the viewport).
  await page.keyboard.press("5");
  await expect(liveDimField(page, "length")).toBeFocused();
  await page.keyboard.press("0");
  await page.keyboard.press("Tab"); // locks length=50mm, moves focus to angle
  // DOM focus moves to the angle field via a React effect keyed on the store's
  // `focus` — poll for it rather than typing straight through, or a fast digit
  // right behind Tab can race the effect and land on the still-focused field.
  await expect(liveDimField(page, "angle")).toBeFocused();
  const midDims = await getLiveDims(page);
  const lockedLength = midDims.find((d) => d.field === "length");
  expect(lockedLength?.locked).toBe(true);
  expect(lockedLength?.value).toBeCloseTo(50, 3);

  // Tab's DOM side effect once broke this: the departed length input's stale
  // onBlur (fired by React focusing the angle input) reset the FSM focus to
  // null, swallowing the digits below. Fixed by the field-aware onBlur guard in
  // SketchController (regression: SketchController.liveDim.test.ts "stale blur").
  await page.keyboard.press("3");
  await page.keyboard.press("0");
  await page.keyboard.press("Enter"); // locks angle=30deg AND commits the segment
  // The commit round-trips through the ASYNC mock sketchUpsert — wait for the
  // DOF pill to move off the post-undo "DOF: 0" before reading geometry, same
  // settle gate `clickAtAwaitingDofChange` uses for a mouse-driven commit.
  await expect(dofPill(page)).not.toHaveText("DOF: 0");

  // ── Geometry: exact 50mm @ 30° ───────────────────────────────────────────────
  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(1);
  const [p0, p1] = [snap.lines[0].p0, snap.lines[0].p1];
  const dx = p1[0] - p0[0];
  const dy = p1[1] - p0[1];
  const length = Math.hypot(dx, dy);
  const angleDeg = ((Math.atan2(dy, dx) * 180) / Math.PI + 360) % 360;
  expect(length).toBeCloseTo(50, 3);
  expect(angleDeg).toBeCloseTo(30, 3);

  // ── Constraint + DOF: exactly one Distance, DOF one lower than baseline ─────
  await expect(page.getByText("Distance", { exact: true })).toHaveCount(1);
  const finalDofText = (await dofPill(page).textContent()) ?? "";
  const finalDof = Number(finalDofText.match(/\d+/)?.[0]);
  expect(finalDof).toBe(baselineDof - 1);

  await page.keyboard.press("Escape"); // end the chain cleanly
});
