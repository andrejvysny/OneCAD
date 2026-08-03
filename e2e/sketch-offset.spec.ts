import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  clickAt,
  clickAtClient,
  dofPill,
  getSketchSnapshot,
  planePointToClient,
  selectSketchTool,
} from "./helpers";

/*
 * Sketch Offset (WP-C T2b) — the REAL user path (plane picker) end to end,
 * mock lane.
 *
 * Draws a rectangle, arms Offset, types 3 mm into its chip, and clicks just
 * OUTSIDE one wall. The whole connected chain is offset in one commit: four new
 * lines whose corners MEET (each line-line join extended/trimmed to the offset
 * corner) and whose bounding box has grown by exactly 3 on every side.
 *
 * The outward click target is computed in SCREEN space — the wall midpoint
 * nudged 4 px directly away from the rectangle's centroid. That keeps it inside
 * the 8 px pick reach whatever the camera's plane↔canvas scale is, while being
 * unambiguously on the outer side, which is what selects the offset direction.
 */

interface EntitySnap {
  id: string;
  type: string;
  p0?: [number, number];
  p1?: [number, number];
}

function readEntities(page: Page): Promise<EntitySnap[]> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { sketch: { getState(): { session: { entities: EntitySnap[] } | null } } };
    };
    const s = w.__stores?.sketch.getState().session;
    return s ? s.entities.map((e) => ({ ...e })) : [];
  });
}

const key = (p: [number, number]): string => `${Math.round(p[0] * 1e6)},${Math.round(p[1] * 1e6)}`;

/** Every endpoint of `lines` is shared by exactly two of them (a closed ring). */
function expectCornersMeet(lines: EntitySnap[]): void {
  const degree = new Map<string, number>();
  for (const l of lines) {
    for (const p of [l.p0!, l.p1!]) degree.set(key(p), (degree.get(key(p)) ?? 0) + 1);
  }
  expect([...degree.values()].every((d) => d === 2)).toBe(true);
  expect(degree.size).toBe(lines.length);
}

function boundsOf(lines: { p0: [number, number]; p1: [number, number] }[]): {
  minU: number;
  maxU: number;
  minV: number;
  maxV: number;
} {
  const us = lines.flatMap((l) => [l.p0[0], l.p1[0]]);
  const vs = lines.flatMap((l) => [l.p0[1], l.p1[1]]);
  return {
    minU: Math.min(...us),
    maxU: Math.max(...us),
    minV: Math.min(...vs),
    maxV: Math.max(...vs),
  };
}

test("offset authors a parallel copy of a rectangle chain, outward by 3", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -180, -110);
  await clickAt(page, 180, 110);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  const before = await readEntities(page);
  expect(before).toHaveLength(4);
  const snap = await getSketchSnapshot(page);
  const srcBounds = boundsOf(snap.lines);

  // Arm Offset and type a NON-default distance, so the chip's commit path is
  // exercised rather than just its seeded value.
  const offsetBtn = page.getByRole("button", { name: "Offset", exact: true });
  await offsetBtn.click();
  await expect(offsetBtn).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByText(/^Offset — hover a chain/)).toBeVisible();
  const chip = page.locator('[data-testid="model-tool-chip"] input');
  await expect(chip).toBeVisible();
  await expect(chip).toHaveValue("5");
  await chip.fill("3");
  await chip.press("Enter");

  // One wall's midpoint, nudged 4 px away from the centroid in SCREEN space.
  const wall = snap.lines[0];
  const wallMid = { x: (wall.p0[0] + wall.p1[0]) / 2, y: (wall.p0[1] + wall.p1[1]) / 2 };
  const centroid = {
    x: snap.lines.reduce((s, l) => s + l.p0[0] + l.p1[0], 0) / (snap.lines.length * 2),
    y: snap.lines.reduce((s, l) => s + l.p0[1] + l.p1[1], 0) / (snap.lines.length * 2),
  };
  const midPx = await planePointToClient(page, snap.plane, wallMid);
  const centroidPx = await planePointToClient(page, snap.plane, centroid);
  const dx = midPx.x - centroidPx.x;
  const dy = midPx.y - centroidPx.y;
  const len = Math.hypot(dx, dy);
  const outward = { x: midPx.x + (dx / len) * 4, y: midPx.y + (dy / len) * 4 };

  await clickAtClient(page, outward.x, outward.y);

  // 4 → 8: the source chain is untouched and the copy is appended.
  await expect.poll(async () => (await readEntities(page)).length).toBe(8);
  const after = await readEntities(page);
  for (const src of before) expect(after.some((e) => e.id === src.id)).toBe(true);

  const copies = after.slice(4);
  expect(copies).toHaveLength(4);
  expect(copies.every((e) => e.type === "Line")).toBe(true);
  // Joins meet: the four offset walls close into a ring with no gaps.
  expectCornersMeet(copies);

  // Outward by exactly 3 on every side.
  const copyBounds = boundsOf(copies.map((e) => ({ p0: e.p0!, p1: e.p1! })));
  expect(copyBounds.minU).toBeCloseTo(srcBounds.minU - 3, 5);
  expect(copyBounds.maxU).toBeCloseTo(srcBounds.maxU + 3, 5);
  expect(copyBounds.minV).toBeCloseTo(srcBounds.minV - 3, 5);
  expect(copyBounds.maxV).toBeCloseTo(srcBounds.maxV + 3, 5);

  // Repeatable: the tool stays armed with its chip open.
  await expect(offsetBtn).toHaveAttribute("aria-pressed", "true");
  await expect(chip).toBeVisible();
  await expect(dofPill(page)).toHaveText(/^DOF: \d+$/);

  // The commit survives finishing the sketch: back in model mode, and the
  // sketch's own timeline row is there (the exit path mints/refreshes it).
  const sketchRows = page.getByRole("listbox", { name: "Sketches" }).getByRole("option");
  const rowsBefore = await sketchRows.count();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("button", { name: "Select", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(sketchRows).toHaveCount(rowsBefore);
  await expect(page.getByText(/^Editing /)).toHaveCount(0);
});
