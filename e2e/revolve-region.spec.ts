import { test, expect } from "./fixtures";
import {
  hideSeedSketches,
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtClient,
  dofPill,
  bodyOptions,
  getSketchSnapshot,
  planePointToClient,
  extrudeDebug,
} from "./helpers";

/*
 * WP-C3 — REVOLVE REGION PARITY (mock lane).
 *
 * Extrude has consumed a typed `sketchRegion` selection since
 * EXTRUDE-REGION-PARITY; revolve did not. It read a whole SKETCH and re-derived
 * a profile from it, so with a region already picked it ignored the pick and
 * re-asked: on this two-region sketch the old lane opened the multi-region
 * PICKER ("Select regions to revolve") even though the user had just clicked
 * the region they wanted. That is what makes this spec non-vacuous — it fails on
 * the old lane, where the axis-pick phase is not reachable in one click.
 *
 * Layout + targeting rationale (region ordering, camera settle, why the click
 * point is projected rather than guessed) is documented in `multiregion.spec.ts`,
 * whose construction this mirrors: the mock's `detectRegions` always returns
 * `[circleRegion, rectRegion]`, so the RECTANGLE is the non-first region.
 */
test("revolve arms on the SELECTED region — no region picker, then commits", async ({ page }) => {
  await openEditorDebug(page);
  await bodyOptions(page).first().getByRole("switch").click();
  await hideSeedSketches(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // Rectangle — the profile. Its own left edge is a valid revolve axis (every
  // profile point lies on one side of it, so `axisSplitsRegion` is false).
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -260, -150);
  await clickAt(page, -60, 50);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  // Circle — a SECOND region, well clear of the rectangle. Without it the sketch
  // has one region and the old sole-visible-sketch fallback would arm too.
  await selectSketchTool(page, "Circle");
  await clickAt(page, 120, -80);
  await clickAt(page, 200, -80);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  // Snapshot BEFORE finishing: sketchStore.session clears once mode leaves sketch.
  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(4); // the rectangle only — a Circle is not a Line
  const sum = snap.lines.reduce(
    (acc, l) => ({ x: acc.x + l.p0[0] + l.p1[0], y: acc.y + l.p0[1] + l.p1[1] }),
    { x: 0, y: 0 },
  );
  const centroid = { x: sum.x / (snap.lines.length * 2), y: sum.y / (snap.lines.length * 2) };
  const minU = Math.min(...snap.lines.flatMap((l) => [l.p0[0], l.p1[0]]));
  const leftEdge = snap.lines.find(
    (l) => Math.abs(l.p0[0] - minU) < 1e-3 && Math.abs(l.p1[0] - minU) < 1e-3,
  );
  if (!leftEdge) throw new Error("no left edge found in the rectangle snapshot");
  const axisMid = { x: minU, y: (leftEdge.p0[1] + leftEdge.p1[1]) / 2 };

  const bodiesBefore = await bodyOptions(page).count();

  await page.keyboard.press("Enter"); // finish → the camera animates back
  await waitForCameraSettled(page);

  // Click the RECTANGLE's fill → a typed `sketchRegion` selection on regions[1].
  const clientPt = await planePointToClient(page, snap.plane, centroid);
  await expect
    .poll(() =>
      page.evaluate(
        ({ x, y }) =>
          Boolean(
            (window as unknown as { __vpEngine?: { sketchStaticHitTest(x: number, y: number): unknown } })
              .__vpEngine?.sketchStaticHitTest(x, y),
          ),
        clientPt,
      ),
    )
    .toBe(true);
  await clickAtClient(page, clientPt.x, clientPt.y);
  const picked = await page.evaluate(
    () =>
      (
        window as unknown as {
          __stores?: {
            selection: {
              getState(): { selected: Array<{ kind: string; sketchId?: string; regionId?: string }> };
            };
          };
        }
      ).__stores?.selection.getState().selected[0] ?? null,
  );
  expect(picked?.kind).toBe("sketchRegion");

  // Revolve consumes that selection directly: straight to axis pick, no picker.
  await page.getByRole("button", { name: "Revolve", exact: true }).click();
  await expect(page.getByText(/Pick axis line/)).toBeVisible();
  await expect(page.getByText(/Select regions to revolve/)).toBeHidden();
  // The EXACT binding: the clicked (non-first) region, never `regions[0]` — the
  // circle — which the old whole-sketch lane's first-region fallback would take.
  await expect
    .poll(async () => (await extrudeDebug(page))?.revolveRegionIds as string[] | undefined)
    .toEqual([picked?.regionId]);

  // Pick the rectangle's left edge → armed at 360°.
  const axisClient = await planePointToClient(page, snap.plane, axisMid);
  await clickAtClient(page, axisClient.x, axisClient.y);
  await expect(page.getByText(/Drag to set angle/)).toBeVisible();
  await expect(page.getByLabel("Dimension value")).toHaveValue("360");

  await page.keyboard.press("Enter");
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
});
