import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  hideSeedSketches,
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  dofPill,
  bodyOptions,
  findExtrudeHandle,
  extrudeDebug,
  getSketchSnapshot,
  planePointToClient,
  clickAtClient,
} from "./helpers";

/*
 * HISTORY-HARDEN H3 — edit a PAST feature's primary dimension straight from its
 * history row: click the value, type a new one, downstream rebuilds.
 *
 * The whole loop is real on the mock lane: real row → real `getOperationParams` →
 * real `UpdateOperationParams` (one merged scalar) → real projection hydration →
 * real mesh re-synthesis. Two things this pins that a unit test cannot:
 *   (a) the value stays VISIBLE on the affordance-bearing timeline view (it used
 *       to vanish the moment a row grew its hover cluster), and the number the
 *       user typed reaches the GEOMETRY, not just the label;
 *   (b) the editor is GATED on the Select tool — an armed model tool owns the
 *       viewport gesture, and a history commit underneath it would race its
 *       own preview.
 */

interface FeatureRow {
  id: string;
  label: string;
  valueText: string;
  primaryValue?: number;
}

async function features(page: Page): Promise<FeatureRow[]> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { document: { getState(): { features: FeatureRow[] } } };
    };
    return (w.__stores?.document.getState().features ?? []) as FeatureRow[];
  });
}

/**
 * World-space bounding-box extents of every VISIBLE body. The depth edit grows the
 * box along the sketch normal, and the plane picker's chosen plane is not fixed, so
 * the assertion reads "some axis now measures the typed depth" rather than pinning
 * one axis (`getBoundsForBodies` skips hidden bodies, so the seeded demo body —
 * hidden in `drawAndExtrude` — never pollutes this).
 */
async function bodyExtents(page: Page): Promise<[number, number, number]> {
  return page.evaluate(() => {
    type V = { x: number; y: number; z: number };
    const w = window as unknown as {
      __vpEngine?: { getBoundsForBodies(ids: readonly string[]): { min: V; max: V } | null };
      __stores?: { document: { getState(): { bodies: Record<string, unknown> } } };
    };
    const ids = Object.keys(w.__stores?.document.getState().bodies ?? {});
    const box = w.__vpEngine?.getBoundsForBodies(ids) ?? null;
    if (!box) return [NaN, NaN, NaN] as [number, number, number];
    return [box.max.x - box.min.x, box.max.y - box.min.y, box.max.z - box.min.z] as [
      number,
      number,
      number,
    ];
  });
}

/** How closely `extents` matches `depth` on its best-fitting axis. */
function depthError(extents: readonly number[], depth: number): number {
  return Math.min(...extents.map((e) => Math.abs(e - depth)));
}

/**
 * Draw a rectangle, select its region, extrude it, and COMMIT — the production
 * profile-first flow (see extrude-commit-gesture.spec.ts). Returns the committed
 * feature's projection id.
 */
async function drawAndExtrude(page: Page): Promise<string> {
  await openEditorDebug(page);
  // Hide the seeded demo body + sketches so the picks below can only hit ours.
  await bodyOptions(page).first().getByRole("switch").click();
  await hideSeedSketches(page);

  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -150, -100);
  await clickAt(page, 150, 100);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  const snap = await getSketchSnapshot(page);
  const p0 = snap.lines[0]?.p0;
  const p2 = snap.lines[2]?.p0;
  if (!p0 || !p2) throw new Error("rectangle snapshot is incomplete");
  const centroid = { x: (p0[0] + p2[0]) / 2, y: (p0[1] + p2[1]) / 2 };

  await page.keyboard.press("Enter");
  await waitForCameraSettled(page);
  const client = await planePointToClient(page, snap.plane, centroid);
  await expect
    .poll(() =>
      page.evaluate(
        ({ x, y }) =>
          Boolean(
            (window as unknown as { __vpEngine?: { sketchStaticHitTest(x: number, y: number): unknown } })
              .__vpEngine?.sketchStaticHitTest(x, y),
          ),
        client,
      ),
    )
    .toBe(true);
  await clickAtClient(page, client.x, client.y);
  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();

  // Drag the depth handle, then confirm explicitly (release keeps the tool armed).
  await expect(async () => {
    const { x, y } = await findExtrudeHandle(page);
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(x, y - 20, { steps: 4 });
    const phase = (await extrudeDebug(page))?.phase;
    if (phase !== "dragging") {
      await page.mouse.up();
      throw new Error(`extrude grab missed — phase was ${String(phase)}`);
    }
  }).toPass({ timeout: 10_000, intervals: [200, 400, 800] });
  await page.mouse.up();
  await page.getByTestId("chip-confirm").click();
  await expect(bodyOptions(page)).toHaveCount(2);

  const rows = await features(page);
  const committed = rows[rows.length - 1];
  expect(committed.label).toBe("Extrude");
  expect(committed.primaryValue).toBeGreaterThan(0);
  return committed.id;
}

/**
 * Open the FEATURE state (full timeline + row affordances) on `featureId`.
 *
 * Two hops on purpose: with a BODY selected the inspector shows the lineage SLICE
 * (`features.slice(0, 3)`), which cannot contain a freshly appended op — selecting
 * any feature there is what switches the panel to the full-timeline view.
 */
async function openTimelineOn(page: Page, featureId: string): Promise<void> {
  await selectRow(page, "f1");
  await expect(page.getByTestId(`history-row-${featureId}`)).toBeVisible();
  await selectRow(page, featureId);
  await expect(page.getByTestId(`history-suppress-${featureId}`)).toBeVisible();
}

/**
 * Click a history row to SELECT it — deliberately near the left edge (the icon /
 * label), because the row centre lands on the value chip, which is now its own
 * click target and would open the editor instead.
 */
async function selectRow(page: Page, id: string): Promise<void> {
  await page.getByTestId(`history-row-${id}`).click({ position: { x: 8, y: 16 } });
}

test("clicking a past extrude's history value retypes its depth and rebuilds the body", async ({
  page,
}) => {
  const featureId = await drawAndExtrude(page);
  await openTimelineOn(page, featureId);

  const before = (await features(page)).find((f) => f.id === featureId)!;
  const extentsBefore = await bodyExtents(page);
  // The committed body already measures the dragged depth on one axis.
  expect(depthError(extentsBefore, before.primaryValue!)).toBeLessThan(0.5);

  // (1) The value is VISIBLE even though the row carries its affordance cluster.
  const value = page.getByTestId(`history-value-${featureId}`);
  await expect(value).toBeVisible();
  await expect(page.getByTestId(`history-suppress-${featureId}`)).toBeVisible();

  // (2) One click swaps in the editor — and does NOT re-select anything else.
  await value.click();
  const input = page.getByLabel("Dimension value");
  await expect(input).toBeFocused();

  // (3) Type a depth clearly different from whatever the drag produced.
  const target = Math.round(before.primaryValue! + 40);
  await input.fill(String(target));
  await input.press("Enter");

  // (4) The PROJECTION carries the new number in both forms: the unit-aware
  //     `primaryValue` the row renders, and the millimetre-fixed `valueText` the
  //     re-edit parsers read (M18 — the two must not drift).
  await expect
    .poll(async () => (await features(page)).find((f) => f.id === featureId)?.primaryValue)
    .toBe(target);
  const after = (await features(page)).find((f) => f.id === featureId)!;
  expect(after.valueText).toBe(`${target.toFixed(1)} mm`);
  expect(after.valueText).not.toBe(before.valueText);
  await expect(value).toHaveText(`${target} mm`);

  // (5) …and the GEOMETRY followed. A label-only update would leave the box alone.
  await expect.poll(async () => depthError(await bodyExtents(page), target)).toBeLessThan(0.5);
  expect(await bodyExtents(page)).not.toEqual(extentsBefore);
});

test("the inline value editor is disabled while a model tool is armed", async ({ page }) => {
  const featureId = await drawAndExtrude(page);
  await openTimelineOn(page, featureId);

  // SCOPED to the row on purpose: the model-tool chip's depth field carries the
  // same "Dimension value" label, so a page-wide locator conflates "the history
  // row's inline editor is open" with "a tool chip exists" — which is exactly
  // what this test must tell apart once arming Extrude shows a chip.
  const rowEditor = page.getByTestId(`history-row-${featureId}`).getByLabel("Dimension value");

  // Select tool ⇒ editable.
  await page.getByTestId(`history-value-${featureId}`).click();
  await expect(rowEditor).toBeVisible();
  await page.keyboard.press("Escape");

  // Arm Extrude: the row's value must go read-only immediately, without a
  // re-selection — and stay read-only under a click.
  //
  // The toolbar is applicability-gated (`toolApplicability.ts`, landed with
  // multi-region extrude): Extrude enables on a sketch-region / sketch selection,
  // or on the document's SOLE visible sketch. Committing the extrude HIDES its
  // sketch, so at this point none of the three holds and the button is
  // `aria-disabled` — the click would wait out the whole test timeout. Re-show the
  // sketch we drew (the eye toggle a user would flip) rather than changing the
  // SELECTION, which has to stay on the history row for the rest of this test.
  const sketchId = await page.evaluate(() => {
    const ids = Object.keys(
      (
        window as unknown as {
          __stores?: { document: { getState(): { sketches: Record<string, unknown> } } };
        }
      ).__stores?.document.getState().sketches ?? {},
    );
    return ids[ids.length - 1] ?? "";
  });
  expect(sketchId).not.toBe("");
  await page.evaluate((id) => {
    (
      window as unknown as {
        __stores?: { document: { getState(): { setVisibility(id: string, v: boolean): void } } };
      }
    ).__stores?.document.getState().setVisibility(id, true);
  }, sketchId);
  const extrudeBtn = page.getByRole("button", { name: "Extrude", exact: true });
  await expect(extrudeBtn).toBeEnabled();
  await extrudeBtn.click();
  await expect(rowEditor).toHaveCount(0);
  await page.getByTestId(`history-value-${featureId}`).click();
  await expect(rowEditor).toHaveCount(0);
  // The value itself is still READABLE — only the editing affordance is gated.
  await expect(page.getByTestId(`history-value-${featureId}`)).toBeVisible();
});
