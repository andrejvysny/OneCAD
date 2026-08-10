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
 * The ANGLE chip is opt-in: it exists only against a REFERENCE, which for the
 * line tool is the previous chain segment (`liveDimFrames.ts segmentFrame`). A
 * FIRST segment has none — `angleLadder` returns [] without a `prev`, so an
 * oblique value there authors nothing and a chip that looked drivable would be
 * a lie. This test therefore types on the SECOND leg of a chain.
 *
 * What the FIELD types is the visual CORNER angle, not the raw turn:
 * `LiveDimField.tsx`'s `cornerAngleOf` is `180 - |turn|`, so a typed 30 means a
 * 30° corner between the two segments (a 150° turn off the previous leg). The
 * store's `chip.value` stays the turn — the conversion lives only at the
 * display/input boundary — which is why the hover assertions below check a
 * signed turn while the geometry assertion checks a 30° corner.
 *
 * The geometry is measured as the UNDIRECTED angle at the shared vertex, so it
 * depends on neither the camera (the plane's +U need not follow screen +x, and
 * does not here) nor the solver's endpoint ordering (leg 2 is stored
 * end-first). Both authored constraints are asserted by name — one `Distance`
 * from the typed length, one `Angle(leg1, leg2)` from the typed corner.
 */
test("typing a length + corner angle on the second chain leg drives an exact segment and authors Distance + Angle", async ({
  page,
}) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // ── Leg 1: axis-aligned, and the reference the angle chip measures against ─
  await clickAt(page, -160, -80);
  await clickAtAwaitingDofChange(page, -40, -80);

  // ── Leg 2: NOW there is an angle chip, because there is a reference ────────
  await hoverAt(page, 20, -20);
  // The chips repopulate on the frame AFTER the pointer moves, so a single read
  // can catch the degenerate pre-move state (cursor still on the anchor, length
  // 0) on a slow machine. Poll for a real length before reading the set.
  await expect
    .poll(async () => (await getLiveDims(page)).find((d) => d.field === "length")?.value ?? 0)
    .toBeGreaterThan(0);
  const dims = await getLiveDims(page);
  const lengthField = dims.find((d) => d.field === "length");
  const angleField = dims.find((d) => d.field === "angle");
  expect(lengthField).toBeDefined();
  expect(angleField).toBeDefined();
  expect(lengthField?.value).toBeGreaterThan(0);
  expect(lengthField?.locked).toBe(false);
  // A SIGNED turn folded to the shorter arc — a reflex value never appears.
  expect(angleField?.value).toBeGreaterThan(-180);
  expect(angleField?.value).toBeLessThanOrEqual(180);
  await expect(liveDimField(page, "length")).toBeVisible();
  await expect(liveDimField(page, "angle")).toBeVisible();

  // Type "50" — opens + focuses the length chip (first digit over the viewport).
  await page.keyboard.press("5");
  await expect(liveDimField(page, "length")).toBeFocused();
  await page.keyboard.press("0");
  await page.keyboard.press("Tab"); // locks length=50mm, moves focus to angle
  // DOM focus moves via a React effect keyed on the store's `focus` — poll for
  // it rather than typing straight through, or a fast digit right behind Tab
  // can race the effect and land on the still-focused field.
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
  await page.keyboard.press("Enter"); // locks the corner AND commits the leg
  // The commit round-trips through the ASYNC mock sketchUpsert, so the leg must
  // be OBSERVED before ending the chain — an Escape sent into that window
  // cancels the in-flight leg and only leg 1 survives.
  await expect.poll(async () => (await getSketchSnapshot(page)).lines.length).toBe(2);
  await page.keyboard.press("Escape"); // end the chain cleanly

  // ── Geometry: leg 2 is exactly 50mm, meeting leg 1 at exactly a 30° corner ─
  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(2);
  const [leg1, leg2] = snap.lines;
  expect(Math.hypot(leg2.p1[0] - leg2.p0[0], leg2.p1[1] - leg2.p0[1])).toBeCloseTo(50, 3);

  // Measured from the shared vertex outwards, because neither the plane's +U nor
  // a line's stored p0/p1 order is guaranteed to follow the drawing direction —
  // leg 2 here is stored end-first, so a naive p0→p1 heading reads 180° off.
  const same = (a: number[], b: number[]): boolean =>
    Math.hypot(a[0] - b[0], a[1] - b[1]) < 1e-6;
  const shared = [leg2.p0, leg2.p1].find((p) => same(p, leg1.p0) || same(p, leg1.p1));
  expect(shared).toBeDefined();
  const away = (l: typeof leg1): number[] => (same(l.p0, shared as number[]) ? l.p1 : l.p0);
  const degFrom = (from: number[], to: number[]): number =>
    ((Math.atan2(to[1] - from[1], to[0] - from[0]) * 180) / Math.PI + 360) % 360;
  // Both rays point AWAY from the junction, so this is the undirected corner
  // angle — free of any sign or winding convention, and exactly the number the
  // field types (`cornerAngleOf`).
  const spread = Math.abs(
    degFrom(shared as number[], away(leg1)) - degFrom(shared as number[], away(leg2)),
  );
  expect(Math.min(spread, 360 - spread)).toBeCloseTo(30, 3);

  // ── Constraints: the typed length and the typed corner, each authored once ─
  await expect(page.getByText("Distance", { exact: true })).toHaveCount(1);
  await expect(page.getByText("Angle", { exact: true })).toHaveCount(1);
});
