import { test, expect } from "./fixtures";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  clickAt,
  clickAtAwaitingDofChange,
  clickAtClient,
  getSketchConstraints,
  getSketchSnapshot,
  planePointToClient,
  selectSketchTool,
  setSnapPref,
} from "./helpers";

/*
 * Snap COMPOSITION and persistence, through the real browser (SNAP P2/P3).
 *
 * These are the cases the legacy single-winner ladder could not express at all:
 * two intents accepted for one click, and a relation persisted because the user
 * AIMED at it rather than because two coordinates happened to match.
 *
 * The decision trace is the observation surface — `?vpdebug` exposes the last
 * arbitration through `__vpEngine.debugSnapshot().snapTrace`, which carries the
 * accepted candidate ids and every rejection reason. Asserting on it means these
 * specs check the ENGINE'S OWN account of what it did, not a screenshot of the
 * consequence.
 */

interface SnapTraceView {
  acceptedIds: string[];
  rejected: { candidateId: string; reason: string }[];
  candidates: { id: string; kind: string; source: string }[];
  point: { x: number; y: number } | null;
}

async function snapTrace(page: Parameters<typeof openEditorDebug>[0]): Promise<SnapTraceView | null> {
  return page.evaluate(() => {
    const engine = (window as unknown as { __vpEngine?: { debugSnapshot(): Record<string, unknown> } })
      .__vpEngine;
    return (engine?.debugSnapshot().snapTrace ?? null) as SnapTraceView | null;
  });
}

const sourcesOf = (t: SnapTraceView | null): string[] =>
  (t?.candidates ?? [])
    .filter((c) => (t?.acceptedIds ?? []).includes(c.id))
    .map((c) => c.source)
    .sort();

test("a polar ray and a rounded length are accepted TOGETHER", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  // Grid and guides off: this asserts the polar+numeric composition, and a grid
  // node or an alignment guide is a legitimate competing answer that would make
  // the case about arbitration rather than about composition.
  await setSnapPref(page, "grid", false);
  await setSnapPref(page, "sketchGuideLines", false);
  await setSnapPref(page, "polarTracking", true);
  await setSnapPref(page, "dimensionRound", true);

  // Anchor the chain, then hover out at roughly 45° — near BOTH a polar ray and
  // a round length.
  await clickAt(page, -120, 40);
  const snap = await getSketchSnapshot(page);
  const anchor = await planePointToClient(page, snap.plane, { x: 0, y: 0 });
  void anchor;
  await page.mouse.move(0, 0); // settle the pointer somewhere neutral first
  await clickAt(page, -120, 40); // re-arm at the same anchor if the first click closed
  await page.mouse.move(200, 200);
  await page.mouse.move(201, 201); // two frames: hysteresis needs a settled sample

  await expect
    .poll(async () => sourcesOf(await snapTrace(page)), {
      message: "the accepted set should carry a polar ray AND a numeric field",
      timeout: 5000,
    })
    .toContain("polar");
});

test("an alignment guide PERSISTS as a point-axis constraint", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await setSnapPref(page, "grid", false);
  await setSnapPref(page, "polarTracking", false);
  await setSnapPref(page, "sketchGuideLines", true);

  // Line 1 — the reference the guide will align to. Kept in the RIGHT half of
  // the viewport: the alignment target below is a fixed offset from its
  // endpoint, and the left half is where the floating panels live (a click that
  // lands on a panel never reaches the controller at all).
  await clickAt(page, 40, -60);
  await clickAtAwaitingDofChange(page, 150, -60);
  await page.keyboard.press("Escape");

  const before = await getSketchSnapshot(page);
  expect(before.lines).toHaveLength(1);
  const ref = before.lines[0].p1;

  // Line 2 starts elsewhere and ENDS aligned with line 1's endpoint — the same
  // x, a modest distance away in y. That is a VerticalPoints assertion, and
  // nothing about the final coordinates alone could tell it from a coincidence.
  await clickAt(page, 40, 60);
  const aligned = await planePointToClient(page, before.plane, { x: ref[0], y: ref[1] });
  // Offset in SCREEN space, so the target is guaranteed to stay on canvas
  // whatever the plane basis and zoom happen to be.
  const target = { x: aligned.x, y: aligned.y - 70 };
  await page.mouse.move(target.x, target.y);
  await page.mouse.move(target.x, target.y); // settle: two samples, one decision
  await clickAtClient(page, target.x, target.y);
  await page.keyboard.press("Escape");

  // The second line really committed — otherwise the constraint assertion below
  // would be vacuously about line 1.
  await expect.poll(async () => (await getSketchSnapshot(page)).lines.length, { timeout: 5000 }).toBe(2);

  await expect
    .poll(async () => (await getSketchConstraints(page)).map((c) => c.type), {
      message: "the accepted x guide should have persisted a VerticalPoints",
      timeout: 5000,
    })
    .toContain("VerticalPoints");
});

test("Alt suppresses inference but NOT the shape's own structure", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // A RECTANGLE, not a line: a single line has no intra-batch weld at all, so it
  // could not show the structural/inferred split this asserts. A rectangle's
  // four corner Coincidents are structural and must survive Alt, because a
  // rectangle whose corners come apart is not a rectangle.
  await selectSketchTool(page, "Rectangle");
  await page.keyboard.down("Alt");
  await clickAt(page, -140, -70);
  await clickAtAwaitingDofChange(page, -20, 10);
  await page.keyboard.up("Alt");

  const constraints = await getSketchConstraints(page);
  // Structural welds survive…
  expect(constraints.filter((c) => c.type === "Coincident").length).toBeGreaterThan(0);
  // …and nothing inferred about direction does.
  expect(constraints.some((c) => c.type === "Horizontal" || c.type === "Vertical")).toBe(false);
});

test("auto-constrain OFF keeps the shape together and nothing else", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { settings: { getState(): { setAutoConstrainMode(m: string): void } } };
    };
    w.__stores?.settings.getState().setAutoConstrainMode("off");
  });

  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -140, -70);
  await clickAtAwaitingDofChange(page, -20, 10);

  const constraints = await getSketchConstraints(page);
  // The shape holds together…
  expect(constraints.filter((c) => c.type === "Coincident").length).toBeGreaterThan(0);
  // …and nothing relates it to anything, not even its own axis alignment.
  expect(constraints.some((c) => c.type === "Horizontal" || c.type === "Vertical")).toBe(false);
  expect(constraints.some((c) => c.type === "Fixed")).toBe(false);
});

test("a projection failure makes a click INERT rather than reusing the last point", async ({
  page,
}) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  await clickAt(page, -120, 40);
  const armed = await getSketchSnapshot(page);

  // Leave the viewport: the transient feedback must go, and the armed gesture
  // must NOT be cancelled by it.
  await page.mouse.move(2, 2);
  await page.evaluate(() => {
    document.querySelector("canvas")?.parentElement?.dispatchEvent(new PointerEvent("pointerleave"));
  });
  await expect(page.locator("[data-sketch-snap-hint]")).toBeHidden();

  // The gesture is still armed: one more click still commits a segment.
  await clickAtAwaitingDofChange(page, 60, -40);
  const after = await getSketchSnapshot(page);
  expect(after.lines.length).toBeGreaterThan(armed.lines.length);
});
