import { test, expect, type Page } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtAwaitingDofChange,
  dofPill,
  getSketchEntityCount,
  getSketchSnapshot,
  planePointToClient,
  type SketchLineSnapshot,
  type SketchPlaneSnapshot,
} from "./helpers";

/*
 * Marquee / box select (mock lane) — the REAL empty-space LMB drag, end to end.
 *
 * Rightward = WINDOW (full containment), leftward = CROSSING (touch). Both drags
 * start on genuinely EMPTY plane space (the gesture is only armed when
 * `hitTestSketch` misses), so the box also proves the deliberate claim of the
 * empty-space LMB drag away from orbit.
 *
 * The viewport has no per-entity DOM, so "what got selected" is asserted through
 * real UI consequences: the floating ConstraintContextChips row (only rendered when
 * the applicable set is non-empty — "Equal" needs exactly two lines) and the DOF
 * pill / live entity count after applying a constraint or pressing Delete.
 *
 * Targeting note (see helpers.ts): plane→client projection is a real camera
 * projection, so the drag endpoints are derived from the LIVE entity coordinates
 * and projected, never guessed from pixel-offset arithmetic. Screen x can be
 * mirrored relative to plane u, so each drag orders its two projected endpoints by
 * CLIENT x to guarantee the intended direction (window vs crossing).
 */

const chips = (page: Page) => page.getByTestId("constraint-context-chips");

/** Plane-space AABB of a set of lines, grown by `pad` on every side. */
function boundsOf(lines: SketchLineSnapshot[], pad: number): { min: [number, number]; max: [number, number] } {
  const xs = lines.flatMap((l) => [l.p0[0], l.p1[0]]);
  const ys = lines.flatMap((l) => [l.p0[1], l.p1[1]]);
  return {
    min: [Math.min(...xs) - pad, Math.min(...ys) - pad],
    max: [Math.max(...xs) + pad, Math.max(...ys) + pad],
  };
}

/**
 * Drag a box between two PLANE points, forcing the screen-x direction so the mode
 * is deterministic: "window" starts at the left-most projected corner, "crossing"
 * at the right-most one.
 */
async function dragBox(
  page: Page,
  plane: SketchPlaneSnapshot,
  a: [number, number],
  b: [number, number],
  mode: "window" | "crossing",
  midDrag?: () => Promise<void>,
): Promise<void> {
  const pa = await planePointToClient(page, plane, { x: a[0], y: a[1] });
  const pb = await planePointToClient(page, plane, { x: b[0], y: b[1] });
  const leftFirst = pa.x <= pb.x;
  const [from, to] = (mode === "window") === leftFirst ? [pa, pb] : [pb, pa];
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(to.x, to.y, { steps: 8 });
  if (midDrag) await midDrag();
  await page.mouse.up();
}

/** The controller-owned rubber-band div: assert it is up and styled for the mode. */
async function expectRubberBand(page: Page, mode: "window" | "crossing"): Promise<void> {
  const band = page.locator("[data-sketch-marquee]");
  await expect(band).toHaveAttribute("data-marquee-mode", mode);
  const style = await band.evaluate((el) => {
    const cs = getComputedStyle(el);
    return { border: cs.borderTopStyle, color: cs.borderTopColor, fill: cs.backgroundColor };
  });
  expect(style.border).toBe(mode === "crossing" ? "dashed" : "solid");
  // Token-driven, so a missing/renamed CSS var would show up as a transparent band.
  expect(style.color).not.toBe("rgba(0, 0, 0, 0)");
  expect(style.fill).not.toBe("rgba(0, 0, 0, 0)");
}

/** Two non-axis-aligned, well-separated lines (no autoconstraint between them). */
async function drawTwoLines(page: Page): Promise<void> {
  await selectSketchTool(page, "Line");
  await clickAt(page, -180, -140);
  await clickAtAwaitingDofChange(page, -80, -80);
  await page.keyboard.press("Escape");

  await selectSketchTool(page, "Line");
  await clickAt(page, 60, 40);
  await clickAtAwaitingDofChange(page, 160, 180);
  await page.keyboard.press("Escape");

  // Two free lines, 4 DOF each, nothing inferred between them.
  await expect(dofPill(page)).toHaveText("DOF: 8");
}

test("rightward WINDOW marquee selects both lines; a constraint applies to the pair", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await drawTwoLines(page);

  await selectSketchTool(page, "Select");
  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(2);

  // A box comfortably enclosing BOTH lines, starting from empty plane space.
  const { min, max } = boundsOf(snap.lines, 20);
  await dragBox(page, snap.plane, min, max, "window", () => expectRubberBand(page, "window"));

  // The band is a drag-scoped overlay: gone on release, never left behind.
  await expect(page.locator("[data-sketch-marquee]")).toHaveCount(0);

  // Two line BODIES selected ⇒ the context chips offer Equal (and the pair-only
  // kinds); a one-entity or empty selection would never render this row.
  const equal = chips(page).getByRole("button", { name: "Equal", exact: true });
  await expect(equal).toBeVisible();
  await equal.click();

  // The constraint really landed: Equal removes 1 DOF in the mock solver.
  await expect(dofPill(page)).toHaveText("DOF: 7");
});

test("leftward CROSSING marquee grazes one line; Delete removes only that one", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await drawTwoLines(page);
  expect(await getSketchEntityCount(page)).toBe(2);

  await selectSketchTool(page, "Select");
  const snap = await getSketchSnapshot(page);
  const second = snap.lines[1];

  // A small square straddling the MIDDLE of the second line: it cuts that line's
  // curve but contains neither endpoint, so only CROSSING can select it — and the
  // first line is nowhere near.
  //
  // The drag must BEGIN on empty space (the marquee only arms on a hit-test miss),
  // and for a diagonal line one of the square's two diagonals has its corners
  // sitting essentially ON the line — inside the 8px pick tolerance. Both diagonals
  // span the same AABB, so pick the one whose corners are farthest from the line:
  // perpendicular offset is |ox·uy − oy·ux|, i.e. h·|uy∓ux| for the two diagonals,
  // and the larger of those is always ≥ h.
  const dx = second.p1[0] - second.p0[0];
  const dy = second.p1[1] - second.p0[1];
  const len = Math.hypot(dx, dy);
  const [ux, uy] = [dx / len, dy / len];
  const mid: [number, number] = [(second.p0[0] + second.p1[0]) / 2, (second.p0[1] + second.p1[1]) / 2];
  const h = len * 0.15;
  const useMainDiagonal = Math.abs(uy - ux) >= Math.abs(uy + ux);
  const a: [number, number] = [mid[0] - h, useMainDiagonal ? mid[1] - h : mid[1] + h];
  const b: [number, number] = [mid[0] + h, useMainDiagonal ? mid[1] + h : mid[1] - h];
  await dragBox(page, snap.plane, a, b, "crossing", () => expectRubberBand(page, "crossing"));
  await expect(page.locator("[data-sketch-marquee]")).toHaveCount(0);

  const chipRow = chips(page);
  await expect(chipRow).toBeVisible(); // something is selected
  // Exactly one line ⇒ the two-line-only kinds are gone from the applicable set.
  await expect(chipRow.getByRole("button", { name: "Equal", exact: true })).toHaveCount(0);

  await page.keyboard.press("Delete");
  // Only the grazed line went; DOF drops by its 4 free parameters.
  await expect(dofPill(page)).toHaveText("DOF: 4");
  expect(await getSketchEntityCount(page)).toBe(1);
});
