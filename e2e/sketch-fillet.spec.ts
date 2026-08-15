import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  hideSeedSketches,
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  bodyOptions,
  clickAt,
  clickAtClient,
  dofPill,
  getSketchSnapshot,
  planePointToClient,
  selectSketchTool,
} from "./helpers";

/*
 * Sketch Fillet (WP-C T2b) — the REAL user path (plane picker) end to end,
 * mock lane.
 *
 * Draws a rectangle, arms the Fillet tool, and clicks ONE of its corners at the
 * corner's real plane position projected through the live camera. The corner is
 * replaced by a tangent arc and the two legs are trimmed back to their tangent
 * points, so the profile goes 4 entities → 5 and STAYS a single closed loop —
 * which is precisely the mock region detector's criterion, re-derived here (see
 * `closedLoopVertices`) rather than imported, matching the sketch-trim /
 * sketch-mirror precedent of keeping e2e assertions independent of src.
 *
 * The last leg is the one that matters most in practice: the filleted profile is
 * still EXTRUDABLE. That is asserted through the same surface multiregion.spec
 * uses — the engine's own static region hit-test after finishing the sketch, and
 * the `sketchRegion` selection a click on it publishes.
 */

interface EntitySnap {
  id: string;
  type: string;
  p0?: [number, number];
  p1?: [number, number];
  center?: [number, number];
  radius?: number;
  start?: [number, number];
  end?: [number, number];
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

/** Both endpoints of a chain-forming entity (Line p0/p1, Arc start/end). */
function endsOf(e: EntitySnap): [[number, number], [number, number]] | null {
  if (e.type === "Line" && e.p0 && e.p1) return [e.p0, e.p1];
  if (e.type === "Arc" && e.start && e.end) return [e.start, e.end];
  return null;
}

const key = (p: [number, number]): string =>
  `${Math.round(p[0] * 1e6)},${Math.round(p[1] * 1e6)}`;

/**
 * The vertex→degree map of a chain, quantised at 1e-6 exactly like the mock
 * region detector (`mockSketch.orderedClosedLoop`). A single closed loop is
 * "every degree is 2 AND #vertices === #edges" — so asserting that here is
 * asserting the profile survived as a profile.
 */
function closedLoopVertices(entities: EntitySnap[]): Map<string, number> {
  const degree = new Map<string, number>();
  for (const e of entities) {
    const ends = endsOf(e);
    if (!ends) continue;
    for (const p of ends) degree.set(key(p), (degree.get(key(p)) ?? 0) + 1);
  }
  return degree;
}

function expectSingleClosedLoop(entities: EntitySnap[]): void {
  const degree = closedLoopVertices(entities);
  expect([...degree.values()].every((d) => d === 2)).toBe(true);
  expect(degree.size).toBe(entities.filter((e) => endsOf(e) !== null).length);
}

/** The rectangle corners: the endpoints two lines share (each seen twice). */
function cornersOf(lines: { p0: [number, number]; p1: [number, number] }[]): [number, number][] {
  const seen = new Map<string, { p: [number, number]; n: number }>();
  for (const l of lines) {
    for (const p of [l.p0, l.p1]) {
      const k = key(p);
      const hit = seen.get(k);
      if (hit) hit.n += 1;
      else seen.set(k, { p, n: 1 });
    }
  }
  return [...seen.values()].filter((v) => v.n === 2).map((v) => v.p);
}

test("fillet rounds a rectangle corner and the profile stays extrudable", async ({ page }) => {
  await openEditorDebug(page);
  // Clear the seeded body + sketches so the post-finish region hit-test can only
  // find THIS sketch's profile (same isolation multiregion.spec sets up).
  await bodyOptions(page).first().getByRole("switch").click();
  await hideSeedSketches(page);

  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // A rectangle: four lines, four corners, one closed loop.
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -180, -110);
  await clickAt(page, 180, 110);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  const before = await readEntities(page);
  expect(before).toHaveLength(4);
  expectSingleClosedLoop(before);

  const snap = await getSketchSnapshot(page);
  const corners = cornersOf(snap.lines);
  expect(corners).toHaveLength(4);
  const corner = corners[0];
  const centroid = {
    x: corners.reduce((s, c) => s + c[0], 0) / 4,
    y: corners.reduce((s, c) => s + c[1], 0) / 4,
  };

  // Arm Fillet. Its radius chip opens with the tool and stays open across
  // applies — it parameterises the next fillet, it never commits anything.
  const filletBtn = page.getByRole("button", { name: "Fillet", exact: true });
  await filletBtn.click();
  await expect(filletBtn).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByText(/^Fillet — click a corner/)).toBeVisible();
  const chip = page.locator('[data-testid="model-tool-chip"] input');
  await expect(chip).toBeVisible();
  await expect(chip).toHaveValue("5"); // the seeded default, in mm
  await chip.fill("5");
  await chip.press("Enter");

  // Click the corner's REAL plane position, projected through the live camera.
  const target = await planePointToClient(page, snap.plane, { x: corner[0], y: corner[1] });
  await clickAtClient(page, target.x, target.y);

  // 4 → 5: both legs replaced by their trimmed selves, plus the tangent arc.
  await expect.poll(async () => (await readEntities(page)).length).toBe(5);
  const after = await readEntities(page);

  const arcs = after.filter((e) => e.type === "Arc");
  expect(arcs).toHaveLength(1);
  expect(arcs[0].radius).toBeCloseTo(5, 6);
  expect(after.filter((e) => e.type === "Line")).toHaveLength(4);

  // The old corner vertex is GONE — both legs were trimmed back off it.
  expect(closedLoopVertices(after).has(key(corner))).toBe(false);
  // The arc's centre sits r·√2 from the old corner of a right angle.
  expect(Math.hypot(arcs[0].center![0] - corner[0], arcs[0].center![1] - corner[1])).toBeCloseTo(
    5 * Math.SQRT2,
    5,
  );
  // …and the profile is still ONE closed loop: 5 entities, 5 degree-2 vertices.
  expectSingleClosedLoop(after);

  // The tool stays armed for the next corner (repeatable), chip still open.
  await expect(filletBtn).toHaveAttribute("aria-pressed", "true");
  await expect(chip).toBeVisible();

  // DOF is still a plain non-negative count — no NaN, no over-constrained badge
  // from a redundant Tangent (the fillet authors endpoint welds only).
  await expect(dofPill(page)).toHaveText(/^DOF: \d+$/);

  // EXTRUDABLE: finishing publishes a region the engine can hit-test, and a
  // click on it selects a real `sketchRegion`.
  await page.keyboard.press("Enter");
  await waitForCameraSettled(page);
  const inside = await planePointToClient(page, snap.plane, centroid);
  await expect
    .poll(() =>
      page.evaluate(
        ({ x, y }) =>
          Boolean(
            (
              window as unknown as {
                __vpEngine?: { sketchStaticHitTest(x: number, y: number): unknown };
              }
            ).__vpEngine?.sketchStaticHitTest(x, y),
          ),
        inside,
      ),
    )
    .toBe(true);
  await clickAtClient(page, inside.x, inside.y);
  const selected = await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: {
        selection: { getState(): { selected: Array<{ kind: string; regionId?: string }> } };
      };
    };
    return w.__stores?.selection.getState().selected[0] ?? null;
  });
  expect(selected?.kind).toBe("sketchRegion");
  expect(selected?.regionId).toBeTruthy();
});
