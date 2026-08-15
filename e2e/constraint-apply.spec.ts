import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtAwaitingDofChange,
  clickAtClient,
  dofPill,
  constraintToolbar,
  getSketchSnapshot,
  getSketchCircle,
  planePointToClient,
  type SketchLineSnapshot,
} from "./helpers";

/*
 * Constraint-apply e2e (mock lane) — the three newly user-applicable geometric
 * kinds this WP unblocks (everything below the UI already shipped: wire
 * encoding, Rust enum, C++ PlaneGCS translation): Tangent, Equal, Midpoint.
 * Each scenario draws two entities that offer the target kind per
 * `constraintApplicability.ts`'s matrix, selects both through the REAL
 * select-tool hit-test (not store injection), clicks the toolbar button, and
 * asserts the DOF pill drops by exactly the mock's `constraintFreedom` for
 * that kind (`mockSketch.ts`: Midpoint removes 2, Tangent/Equal remove 1 —
 * already correct pre-existing mock bookkeeping, not touched by this WP).
 *
 * Angles/offsets below are deliberately chosen away from axis-aligned and
 * away from each other's angle so `autoConstrain.ts` never fires an
 * unrelated Horizontal/Vertical/Parallel/Perpendicular on these lines — the
 * DOF arithmetic has to be exact for the "before − after == N" assertions.
 */

const mid = (l: SketchLineSnapshot): { x: number; y: number } => ({
  x: (l.p0[0] + l.p1[0]) / 2,
  y: (l.p0[1] + l.p1[1]) / 2,
});

const rows = (page: Page) => page.locator('[data-testid^="constraint-row-"]');

/**
 * Wait for the DOF pill to settle on an EXACT expected value, then return it.
 * A one-shot `textContent()` read right after a drawing click races the async
 * mock `sketchUpsert` round-trip (the Circle tool's radius click, in
 * particular, commits the entity on an async round-trip same as the Line
 * tool's segment commits) — an auto-retrying `toHaveText` on the PRECISE
 * expected string is what `circle.spec.ts`/`line.spec.ts` already rely on to
 * avoid that race, so this mirrors it instead of a bare read.
 */
async function settledDof(page: Page, expected: number): Promise<number> {
  await expect(dofPill(page)).toHaveText(`DOF: ${expected}`);
  return expected;
}

test("Tangent (line + circle): shift-select both, apply, DOF drops by 1", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // A single non-axis-aligned line (angle ~31°, safe from the 5° H/V tolerance).
  await selectSketchTool(page, "Line");
  await clickAt(page, -180, -140);
  await clickAtAwaitingDofChange(page, -80, -80);
  await page.keyboard.press("Escape");

  // A circle well clear of the line (no shared points ⇒ no autoConstrain).
  await selectSketchTool(page, "Circle");
  await clickAt(page, 90, 60);
  await clickAt(page, 210, 60);

  // Line (4 free DOF, no autoconstraint) + Circle (3 free DOF) = 7.
  const dofBefore = await settledDof(page, 7);

  await selectSketchTool(page, "Select");
  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(1);
  const circle = await getSketchCircle(page);
  const plane = snap.plane;

  const linePoint = await planePointToClient(page, plane, mid(snap.lines[0]));
  const circlePoint = await planePointToClient(page, plane, {
    x: circle.center[0] + circle.radius,
    y: circle.center[1],
  });

  await clickAtClient(page, linePoint.x, linePoint.y); // plain: selects the line body
  await page.keyboard.down("Shift");
  await clickAtClient(page, circlePoint.x, circlePoint.y); // + circle body
  await page.keyboard.up("Shift");

  const toolbar = await constraintToolbar(page);
  const tangentBtn = toolbar.getByRole("button", { name: "Tangent", exact: true });
  await expect(tangentBtn).toBeEnabled();
  const rowsBefore = await rows(page).count();
  await tangentBtn.click();

  await expect(dofPill(page)).toHaveText(`DOF: ${dofBefore - 1}`);
  await expect(rows(page)).toHaveCount(rowsBefore + 1);
  await expect(rows(page).last()).toContainText("Tangent");
});

test("Equal (line + line): shift-select both, apply, DOF drops by 1", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // Two lines at angles far enough apart (~31° vs ~54°) that neither H/V,
  // Parallel, nor Perpendicular auto-infers between them.
  await selectSketchTool(page, "Line");
  await clickAt(page, -180, -140);
  await clickAtAwaitingDofChange(page, -80, -80);
  await page.keyboard.press("Escape");

  await selectSketchTool(page, "Line");
  await clickAt(page, 60, 40);
  await clickAtAwaitingDofChange(page, 160, 180);
  await page.keyboard.press("Escape");

  // Two lines, 4 free DOF each, no autoconstraint between them = 8.
  const dofBefore = await settledDof(page, 8);

  await selectSketchTool(page, "Select");
  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(2);
  const plane = snap.plane;

  const p1 = await planePointToClient(page, plane, mid(snap.lines[0]));
  const p2 = await planePointToClient(page, plane, mid(snap.lines[1]));

  await clickAtClient(page, p1.x, p1.y); // plain: selects line 1
  await page.keyboard.down("Shift");
  await clickAtClient(page, p2.x, p2.y); // + line 2
  await page.keyboard.up("Shift");

  const toolbar = await constraintToolbar(page);
  const equalBtn = toolbar.getByRole("button", { name: "Equal", exact: true });
  await expect(equalBtn).toBeEnabled();
  const rowsBefore = await rows(page).count();
  await equalBtn.click();

  await expect(dofPill(page)).toHaveText(`DOF: ${dofBefore - 1}`);
  await expect(rows(page)).toHaveCount(rowsBefore + 1);
  await expect(rows(page).last()).toContainText("Equal");
});

test("Midpoint (circle center + line): shift-select both, apply, DOF drops by 2", async ({ page }) => {
  // Feasibility note (per the task brief): there is no dedicated point tool, so
  // this exercises the OTHER legal Midpoint pair from the applicability matrix —
  // a Circle's CENTER selected as a named point target (`marshalsAsPoint`
  // includes Circle/Arc Center) + a Line. `hitTestSketch`'s named-point tier
  // (`entityPoints` in autoConstrain.ts) registers a Circle's Center, so a plain
  // click there resolves to `{entityId, point: "Center"}` — a real point target,
  // not store injection.
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // A non-axis-aligned line, well clear of where the circle will be drawn.
  await selectSketchTool(page, "Line");
  await clickAt(page, -180, -140);
  await clickAtAwaitingDofChange(page, -80, -80);
  await page.keyboard.press("Escape");

  await selectSketchTool(page, "Circle");
  await clickAt(page, 100, 60);
  await clickAt(page, 220, 60);

  // Line (4 free DOF, no autoconstraint) + Circle (3 free DOF) = 7.
  const dofBefore = await settledDof(page, 7);

  await selectSketchTool(page, "Select");
  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(1);
  const circle = await getSketchCircle(page);
  const plane = snap.plane;

  const centerPoint = await planePointToClient(page, plane, { x: circle.center[0], y: circle.center[1] });
  const linePoint = await planePointToClient(page, plane, mid(snap.lines[0]));

  await clickAtClient(page, centerPoint.x, centerPoint.y); // plain: selects the circle's Center point
  await page.keyboard.down("Shift");
  await clickAtClient(page, linePoint.x, linePoint.y); // + the line body
  await page.keyboard.up("Shift");

  const toolbar = await constraintToolbar(page);
  const midpointBtn = toolbar.getByRole("button", { name: "Midpoint", exact: true });
  await expect(midpointBtn).toBeEnabled();
  const rowsBefore = await rows(page).count();
  await midpointBtn.click();

  await expect(dofPill(page)).toHaveText(`DOF: ${dofBefore - 2}`);
  await expect(rows(page)).toHaveCount(rowsBefore + 1);
  await expect(rows(page).last()).toContainText("Midpoint");
});
