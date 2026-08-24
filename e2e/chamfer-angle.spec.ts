import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditorDebug, getFeatureLabels, bodyOptions } from "./helpers";
import {
  seedSelection,
  toolPhases,
  openFilletOverflow,
} from "./modelToolHelpers";

/*
 * The DISTANCE-ANGLE chamfer (SCHEMA §7.3) in the browser lane: author an angle,
 * commit it, re-open the row seeded from the stored params, and watch the type
 * flip get refused by name.
 *
 * WHAT THIS SPEC DOES NOT ASSERT, and why. The mock kernel builds no chamfer
 * GEOMETRY at all — `mockClient.mutateOp` re-emits the target body and adds a
 * history row (its own stated MOCK LIMIT), so there is no mesh here in which a
 * 30° bevel could differ from an equal-leg one. Fabricating one would be a
 * second, worse kernel living in the mock. What this lane owns is the UI chain:
 * the field, the mutual exclusion between the two chamfer modes, the params
 * round-trip through commit and re-open, and the refusal the real backend issues
 * VERBATIM. The geometry is proven against the real kernel by the worker and
 * Rust lanes.
 */

const EDGE_REF = {
  kind: "edge",
  id: "body1#e:5",
  bodyId: "body1",
  topoKey: "e:5",
  elementId: "el-edge-5",
  anchor: { worldPoint: [40, 0, 15] as [number, number, number] },
};

async function armChamfer(page: Page): Promise<void> {
  await openEditorDebug(page);
  await seedSelection(page, [EDGE_REF]);
  await page.getByRole("button", { name: "Fillet / Chamfer", exact: true }).click();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await openFilletOverflow(page);
  await page.getByTestId("chip-edgeop-chamfer").click();
  await expect.poll(async () => (await toolPhases(page))?.edgeOpKind).toBe("Chamfer");
}

/** The newest projection row (the mock appends). */
async function lastFeature(
  page: Page,
): Promise<{ id: string; label: string; valueText: string }> {
  const f = await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: {
        document: {
          getState(): { features: Array<{ id: string; label: string; valueText: string }> };
        };
      };
    };
    const all = w.__stores?.document.getState().features ?? [];
    return all[all.length - 1];
  });
  if (!f) throw new Error("the projection has no features");
  return f;
}

/** The armed chamfer's angle field (empty when the distance-angle mode is off). */
function angleField(page: Page) {
  return page.getByLabel("Chamfer angle");
}

/** The armed chamfer's second-distance field (`=` when equal-leg). */
function d2Field(page: Page) {
  return page.getByLabel("Second distance");
}

/** Open the timeline and double-click row `id` — the parametric re-edit entry. */
async function reopenRow(page: Page, id: string): Promise<void> {
  await bodyOptions(page).first().click();
  await page.getByTestId("history-row-f3").click();
  const row = page.getByTestId(`history-row-${id}`);
  await expect(row).toBeVisible();
  await row.dblclick();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await openFilletOverflow(page);
}

test("the angle field is CHAMFER-only and starts empty", async ({ page }) => {
  await openEditorDebug(page);
  await seedSelection(page, [EDGE_REF]);
  await page.getByRole("button", { name: "Fillet / Chamfer", exact: true }).click();
  await expect.poll(async () => (await toolPhases(page))?.filletPhase).toBe("armed");
  await openFilletOverflow(page);
  // SCHEMA §7.3 forbids a Fillet from carrying `angleDeg`, so the field that
  // authors it must not be reachable there at all.
  await expect(angleField(page)).toHaveCount(0);

  await page.getByTestId("chip-edgeop-chamfer").click();
  await expect(angleField(page)).toHaveValue("");
  expect((await toolPhases(page))?.edgeOpAngleDeg).toBeNull();

  await page.getByTestId("chip-edgeop-fillet").click();
  await expect(angleField(page)).toHaveCount(0);
});

test("the two chamfer modes clear each other in the UI — last authored wins", async ({ page }) => {
  await armChamfer(page);

  await d2Field(page).fill("2.5");
  await d2Field(page).blur();
  await expect.poll(async () => (await toolPhases(page))?.edgeOpDistance2).toBe(2.5);

  // Authoring an angle turns the two-distance mode off, and the second-leg field
  // visibly returns to `=` — core refuses a record carrying both by name.
  await angleField(page).fill("30");
  await angleField(page).blur();
  await expect(d2Field(page)).toHaveValue("=");
  await expect.poll(async () => (await toolPhases(page))?.edgeOpAngleDeg).toBe(30);
  expect((await toolPhases(page))?.edgeOpDistance2).toBeNull();

  // …and back the other way.
  await d2Field(page).fill("4");
  await d2Field(page).blur();
  await expect(angleField(page)).toHaveValue("");
  await expect.poll(async () => (await toolPhases(page))?.edgeOpDistance2).toBe(4);
  expect((await toolPhases(page))?.edgeOpAngleDeg).toBeNull();
});

test("author an angle → commit → re-open: the row and the chip both carry it", async ({ page }) => {
  await armChamfer(page);
  const before = await getFeatureLabels(page);

  await angleField(page).fill("30");
  // Enter applies the value THEN confirms the op — the same single-fire contract
  // the first distance's input has.
  await angleField(page).press("Enter");

  await expect.poll(async () => (await getFeatureLabels(page)).length).toBe(before.length + 1);
  const row = await lastFeature(page);
  expect(row.label).toBe("Chamfer");
  // Mirrors Rust `dto.rs feature_value_text` (pinned there by
  // `chamfer_value_text_shows_the_second_distance_only_when_set`): d1 LEADS, so a
  // re-edit still seeds the radius from the leading number.
  expect(row.valueText).toBe("1.0 mm ∠30.0°");

  // The angle comes back from the STORED params, never from that display string.
  await reopenRow(page, row.id);
  await expect(page.getByTestId("chip-edgeop-chamfer")).toHaveAttribute("aria-pressed", "true");
  await expect(angleField(page)).toHaveValue("30");
  expect((await toolPhases(page))?.edgeOpAngleDeg).toBe(30);
});

test("a distance-angle chamfer BLOCKS the flip to Fillet, and allows it once cleared", async ({
  page,
}) => {
  await armChamfer(page);
  await angleField(page).fill("30");
  await angleField(page).press("Enter");
  await expect.poll(async () => (await lastFeature(page)).label).toBe("Chamfer");
  const { id } = await lastFeature(page);
  const rowCount = (await getFeatureLabels(page)).length;

  // Flipping to Fillet would DROP the user's angle, so the backend refuses the
  // edit — and the hint names the field to clear (mirrored VERBATIM from core
  // `session::CHAMFER_ANGLE_FLIP_REASON`).
  await reopenRow(page, id);
  await page.getByTestId("chip-edgeop-fillet").click();
  await page.getByLabel("Dimension value").click();
  await page.keyboard.press("Enter");
  await expect(page.getByText(/clear angleDeg first/)).toBeVisible();
  // The rejected flip wrote nothing.
  expect((await lastFeature(page)).label).toBe("Chamfer");
  expect((await lastFeature(page)).valueText).toBe("1.0 mm ∠30.0°");
  expect((await getFeatureLabels(page)).length).toBe(rowCount);

  // Clearing the angle is an ordinary params edit…
  await reopenRow(page, id);
  await angleField(page).fill("");
  await angleField(page).press("Enter");
  await expect.poll(async () => (await lastFeature(page)).valueText).toBe("1.0 mm");
  expect((await lastFeature(page)).label).toBe("Chamfer");

  // …and NOW the sanctioned Fillet⇄Chamfer swap goes through on the SAME row.
  await reopenRow(page, id);
  await expect(angleField(page)).toHaveValue("");
  await page.getByTestId("chip-edgeop-fillet").click();
  await page.getByLabel("Dimension value").click();
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await lastFeature(page)).label).toBe("Fillet");
  expect((await getFeatureLabels(page)).length).toBe(rowCount);
});
