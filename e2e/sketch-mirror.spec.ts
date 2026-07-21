import { test, expect } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  clickAt,
  clickAtAwaitingDofChange,
  clickAtClient,
  getSketchSnapshot,
  planePointToClient,
  type SketchLineSnapshot,
} from "./helpers";

/*
 * Mirror tool (mock lane) — the REAL user path (plane picker) end to end.
 *
 * Draws two independent lines: A (left of an axis) and B (the mirror axis,
 * roughly vertical, crossing near the canvas center). Mirror tool Phase A
 * click (empty selection) selects A; Phase B click (axis pick — a LINE body
 * not already in the set) reflects A across B and appends ONE new Line + two
 * Symmetric constraints in one sketchUpsert (SketchController.performMirrorNow
 * / mirrorMath.ts). Both clicks target a line's MIDPOINT — `hitTestSketch`
 * prioritizes vertex handles within tolerance, so a midpoint always resolves
 * to the body pick the Mirror tool's selection step needs (same reasoning
 * acceptance.spec.ts's `mid()` helper documents).
 *
 * The expected reflected coordinates are computed HERE with the exact same
 * formula mirrorMath.ts's `reflectPoint` uses (duplicated as pure arithmetic,
 * not imported — this suite keeps e2e assertions independent of src internals,
 * matching sketch-trim.spec.ts's crossesInterior/crossParam precedent), from
 * the REAL A/B endpoints read back from the live session — so the assertion
 * holds regardless of the camera's exact canvas<->plane mapping or axis tilt.
 */

const mid = (l: SketchLineSnapshot): { x: number; y: number } => ({
  x: (l.p0[0] + l.p1[0]) / 2,
  y: (l.p0[1] + l.p1[1]) / 2,
});

/** Reflect point p across the infinite line through a,b (2*proj - p) — mirrors
 *  mirrorMath.ts's `reflectPoint` exactly. */
function reflect(p: [number, number], a: [number, number], b: [number, number]): [number, number] {
  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const lenSq = dx * dx + dy * dy;
  const t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / lenSq;
  const projX = a[0] + t * dx;
  const projY = a[1] + t * dy;
  return [2 * projX - p[0], 2 * projY - p[1]];
}

/** Signed perpendicular side of p relative to the line through a,b (cross
 *  product sign) — orientation-correct regardless of the axis's exact tilt. */
function sideOf(p: [number, number], a: [number, number], b: [number, number]): number {
  return Math.sign((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]));
}

test("mirror tool reflects a line across an axis line", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page); // default tool: Line
  await waitForCameraSettled(page);

  // Line A: off to one side, independent of the axis (no shared endpoint).
  await clickAt(page, -220, -80);
  await clickAtAwaitingDofChange(page, -100, 40);
  await page.keyboard.press("Escape");

  // Line B: the mirror axis, roughly vertical, crossing the canvas center.
  await clickAt(page, 0, -150);
  await clickAtAwaitingDofChange(page, 0, 150);
  await page.keyboard.press("Escape");

  const before = await getSketchSnapshot(page);
  expect(before.lines).toHaveLength(2);
  const [lineA, axis] = before.lines;

  const clickPlanePoint = async (p: { x: number; y: number }): Promise<void> => {
    const c = await planePointToClient(page, before.plane, p);
    await clickAtClient(page, c.x, c.y);
  };

  // Arm the Mirror tool.
  const mirrorBtn = page.getByRole("button", { name: "Mirror", exact: true });
  await mirrorBtn.click();
  await expect(mirrorBtn).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByText("Select entities to mirror first")).toBeVisible();

  // Phase A: click line A's midpoint (a body pick — see the header note).
  await clickPlanePoint(mid(lineA));
  await expect(page.getByText("Click the mirror axis line · Esc to exit")).toBeVisible();

  // Phase B: click the axis line's midpoint → mirrors A across it.
  await clickPlanePoint(mid(axis));

  await expect.poll(async () => (await getSketchSnapshot(page)).lines.length).toBe(3);
  const after = await getSketchSnapshot(page);
  const mirrored = after.lines[2]; // appended last: [...session.entities, ...mirrored]

  const expectedP0 = reflect(lineA.p0, axis.p0, axis.p1);
  const expectedP1 = reflect(lineA.p1, axis.p0, axis.p1);
  expect(mirrored.p0[0]).toBeCloseTo(expectedP0[0], 6);
  expect(mirrored.p0[1]).toBeCloseTo(expectedP0[1], 6);
  expect(mirrored.p1[0]).toBeCloseTo(expectedP1[0], 6);
  expect(mirrored.p1[1]).toBeCloseTo(expectedP1[1], 6);

  // Coarse sanity check alongside the exact reflection above: the mirrored
  // copy sits on the OPPOSITE side of the axis from the source.
  expect(sideOf(mirrored.p0, axis.p0, axis.p1)).toBe(-sideOf(lineA.p0, axis.p0, axis.p1));

  // Selection follows the mirrored copy; the tool stays armed (repeatable).
  await expect(mirrorBtn).toHaveAttribute("aria-pressed", "true");
});
