import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  clickAt,
  clickAtAwaitingDofChange,
  clickAtClient,
  getSketchSnapshot,
  planePointToClient,
} from "./helpers";

/*
 * Trim tool — the REAL user path (plane picker) end to end, mock lane.
 *
 * Draws three independent lines: A and C are near-vertical (left / right), B is a
 * long near-horizontal line crossing BOTH. Trimming B's middle span (the piece
 * between its two crossings) replaces B with its two surviving end pieces:
 * entity count 3 → 4 and B's id disappears. Every geometric fact is read back
 * from the live sketch session (window.__stores) — the trim click target is the
 * REAL plane midpoint of B projected to a client pixel through the live camera.
 */

interface EntitySnap {
  id: string;
  type: string;
  p0?: [number, number];
  p1?: [number, number];
}

/** Read the live sketch entities (id + coords) from the dev-exposed store. */
function readEntities(page: Page): Promise<EntitySnap[]> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { sketch: { getState(): { session: { entities: EntitySnap[] } | null } } };
    };
    const session = w.__stores?.sketch.getState().session;
    return session ? session.entities.map((e) => ({ id: e.id, type: e.type, p0: e.p0, p1: e.p1 })) : [];
  });
}

/** True if segments a and b cross with the crossing STRICTLY inside a (interior cut). */
function crossesInterior(a: EntitySnap, b: EntitySnap): boolean {
  if (!a.p0 || !a.p1 || !b.p0 || !b.p1) return false;
  const [p1, p2, p3, p4] = [a.p0, a.p1, b.p0, b.p1];
  const d1x = p2[0] - p1[0];
  const d1y = p2[1] - p1[1];
  const d2x = p4[0] - p3[0];
  const d2y = p4[1] - p3[1];
  const cross = d1x * d2y - d1y * d2x;
  if (Math.abs(cross) < 1e-9) return false; // parallel
  const dx = p3[0] - p1[0];
  const dy = p3[1] - p1[1];
  const t = (dx * d2y - dy * d2x) / cross; // param along a
  const u = (dx * d1y - dy * d1x) / cross; // param along b
  return t > 1e-4 && t < 1 - 1e-4 && u >= -1e-4 && u <= 1 + 1e-4;
}

/** The param along `a` where `a` crosses `b` (a must cross b — see crossesInterior). */
function crossParam(a: EntitySnap, b: EntitySnap): number {
  const [p1, p2, p3, p4] = [a.p0!, a.p1!, b.p0!, b.p1!];
  const d1x = p2[0] - p1[0];
  const d1y = p2[1] - p1[1];
  const d2x = p4[0] - p3[0];
  const d2y = p4[1] - p3[1];
  const cross = d1x * d2y - d1y * d2x;
  return ((p3[0] - p1[0]) * d2y - (p3[1] - p1[1]) * d2x) / cross;
}

test("trim removes a line's middle segment between its two crossings", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // Draw A (left vertical), B (long horizontal), C (right vertical). Each line is
  // anchor + one committing click, then Esc to end the chain (tool stays Line).
  await clickAt(page, -60, -70);
  await clickAtAwaitingDofChange(page, -60, 70); // A commits
  await page.keyboard.press("Escape");

  await clickAt(page, -120, 0);
  await clickAtAwaitingDofChange(page, 120, 0); // B commits (crosses A and C's future span)
  await page.keyboard.press("Escape");

  await clickAt(page, 60, -70);
  await clickAtAwaitingDofChange(page, 60, 70); // C commits
  await page.keyboard.press("Escape");

  // Three independent lines exist.
  const before = await readEntities(page);
  expect(before).toHaveLength(3);

  // The trim target B is the ONE line crossed by BOTH others in its interior
  // (the two others are parallel, so only the crossing line has 2 crossings).
  // Identifying it geometrically avoids any assumption about the screen↔plane
  // axis mapping (a screen-vertical stroke need not be a plane-vertical line).
  const B = before.find((e) => before.filter((o) => o.id !== e.id && crossesInterior(e, o)).length === 2);
  expect(B, "a line crossed by both others (2 interior crossings) exists").toBeTruthy();
  const bId = B!.id;
  // Click the midpoint BETWEEN B's two crossings → the removed middle span.
  const crossers = before.filter((o) => o.id !== bId && crossesInterior(B!, o));
  const params = crossers.map((o) => crossParam(B!, o)).sort((a, b) => a - b);
  const tMid = (params[0] + params[1]) / 2;
  const mid = {
    x: B!.p0![0] + tMid * (B!.p1![0] - B!.p0![0]),
    y: B!.p0![1] + tMid * (B!.p1![1] - B!.p0![1]),
  };

  // Arm the Trim tool.
  const trimBtn = page.getByRole("button", { name: "Trim", exact: true });
  await trimBtn.click();
  await expect(trimBtn).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByText("Click a segment to trim")).toBeVisible();

  // Click B's true plane midpoint (between its two crossings) → trims the middle.
  const { plane } = await getSketchSnapshot(page);
  const target = await planePointToClient(page, plane, mid);
  await clickAtClient(page, target.x, target.y);

  // B is replaced by its two end pieces: 3 → 4 and B's id is gone.
  await expect.poll(async () => (await readEntities(page)).length).toBe(4);
  const after = await readEntities(page);
  expect(after.some((e) => e.id === bId)).toBe(false);
  // The survivors are all lines (2 originals A/C + 2 fresh pieces).
  expect(after.every((e) => e.type === "Line")).toBe(true);
});
