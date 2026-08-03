import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
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
 * WP-C3 — the extrude DRAFT ANGLE is authorable (mock lane).
 *
 * `ExtrudeParams.draftAngleDeg` has crossed FE→core→worker since W-WP6 — the
 * worker applies it with `BRepOffsetAPI_DraftAngle` (`ExtrudeOp.cpp apply_draft`)
 * — but no UI ever dispatched one, so the whole parameter was unreachable.
 *
 * SCOPE NOTE (why this asserts a round-trip and not a volume): the mock lane's
 * extrude synthesis is a straight prism (`makeExtrudeBodyMesh`) — it models no
 * draft at all, so a drafted and an undrafted mock body are geometrically
 * identical and a volume assertion here would be vacuous. The GEOMETRY is pinned
 * against the real kernel in `worker/tests/test_wp6_extrude.cpp` (draftAngleDeg
 * 10); what this spec owns is the UI chain the kernel never sees: chip → FSM →
 * committed record → re-edit seed.
 */

/** The last projection feature row (id + label). */
async function lastFeature(page: Page): Promise<{ id: string; label: string }> {
  const f = await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { document: { getState(): { features: Array<{ id: string; label: string }> } } };
    };
    const all = w.__stores?.document.getState().features ?? [];
    return all[all.length - 1];
  });
  if (!f) throw new Error("the projection has no features");
  return f;
}

/** The draft segment's own degrees input (the cluster also has a mm depth one). */
function draftInput(page: Page) {
  return page.getByTestId("chip-draft-input").getByLabel("Dimension value");
}

/**
 * Draw a rectangle on a clean document, finish it, click its fill, and arm
 * Extrude on that region. The seeded body + sketches are hidden first so the
 * region click cannot land on a body face or another sketch's curve.
 */
async function armExtrudeOnFreshRectangle(page: Page): Promise<void> {
  await openEditorDebug(page);
  await bodyOptions(page).first().getByRole("switch").click();
  const visibleSeedSketches = page
    .getByRole("listbox", { name: "Sketches" })
    .locator('[role="switch"][aria-checked="true"]');
  while ((await visibleSeedSketches.count()) > 0) await visibleSeedSketches.first().click();
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -180, -110);
  await clickAt(page, 60, 90);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  // Snapshot BEFORE finishing (sketchStore.session clears on leaving sketch mode);
  // project the centroid through the camera pose AFTER the restore tween settles.
  const snap = await getSketchSnapshot(page);
  const sum = snap.lines.reduce(
    (acc, l) => ({ x: acc.x + l.p0[0] + l.p1[0], y: acc.y + l.p0[1] + l.p1[1] }),
    { x: 0, y: 0 },
  );
  const centroid = { x: sum.x / (snap.lines.length * 2), y: sum.y / (snap.lines.length * 2) };

  await page.keyboard.press("Enter");
  await waitForCameraSettled(page);
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
  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();
}

test("the [Draft] segment authors an angle that survives the commit and the re-edit", async ({
  page,
}) => {
  await armExtrudeOnFreshRectangle(page);
  const bodiesBefore = await bodyOptions(page).count();

  // (1) A fresh arm opens COLLAPSED at zero draft — the common extrude is unchanged.
  await expect(page.getByTestId("chip-draft")).toHaveText("Draft");
  await expect(draftInput(page)).toHaveCount(0);
  expect((await extrudeDebug(page))?.draftAngleDeg).toBe(0);

  // (2) Expand + author 10°. Tab blurs (commit-on-blur) rather than Enter, which
  // would confirm the whole extrude before the armed state can be read.
  await page.getByTestId("chip-draft").click();
  await draftInput(page).fill("10");
  await page.keyboard.press("Tab");
  await expect.poll(async () => (await extrudeDebug(page))?.draftAngleDeg).toBe(10);
  await expect(page.getByTestId("chip-draft")).toHaveText("Draft 10°");

  // (3) Commit → one new body, one Extrude row.
  await page.getByTestId("chip-confirm").click();
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
  const { id, label } = await lastFeature(page);
  expect(label).toBe("Extrude");

  // (4) The RECORD carries it: re-editing the row re-arms seeded at 10°, which is
  // only possible if the committed params kept the draft.
  // Reach the FULL timeline: the body state shows only that body's rows, the
  // feature state shows every one (InspectorPanel BodyState vs FeatureState).
  await bodyOptions(page).first().click();
  await page.getByTestId("history-row-f3").click();
  const row = page.getByTestId(`history-row-${id}`);
  await expect(row).toBeVisible();
  await row.dblclick();
  await expect.poll(async () => (await extrudeDebug(page))?.draftAngleDeg).toBe(10);
  await expect(page.getByTestId("chip-draft")).toHaveText("Draft 10°");
  await expect(draftInput(page)).toHaveValue("10"); // a stored draft opens expanded
});

test("the draft input clamps to the legacy ±89° range", async ({ page }) => {
  await armExtrudeOnFreshRectangle(page);

  // ±90° would make the side face parallel to the extrude direction (a degenerate
  // prism), so the legacy dialog's own range is the ceiling — and the chip reads
  // the CLAMPED state back, never the typed text.
  await page.getByTestId("chip-draft").click();
  await draftInput(page).fill("200");
  await page.keyboard.press("Tab");
  await expect.poll(async () => (await extrudeDebug(page))?.draftAngleDeg).toBe(89);
  await expect(draftInput(page)).toHaveValue("89");
});
