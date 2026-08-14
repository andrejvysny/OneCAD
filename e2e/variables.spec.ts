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
  commitExtrudeAtHandle,
} from "./helpers";

/*
 * WP-VE.2 — document variables, end to end on the mock lane.
 *
 * Two loops, both real from the DOM down to the client:
 *
 *  (1) the Variables inspector section — add / re-value / delete, against the
 *      mock's real in-memory table (which mirrors the backend's validation);
 *  (2) BINDING — typing `=name` into a past extrude's history value writes a
 *      `Scalar {value, expr}` through `UpdateOperationParams`, and the row then
 *      reads `=name` because the PROJECTION says so.
 *
 * (2) is the honesty gate this WP turns on: the row's `=name` is rendered from
 * `FeatureMeta.primaryExpr`, which the backend mints from the very scalar the
 * number came from. If the commit had not recorded the binding, the row would
 * come back showing a plain number and this test would fail — which is exactly
 * what a unit test over the input component cannot prove.
 */

interface FeatureRow {
  id: string;
  label: string;
  valueText: string;
  primaryValue?: number;
  primaryExpr?: string;
}

function features(page: Page): Promise<FeatureRow[]> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { document: { getState(): { features: FeatureRow[] } } };
    };
    return (w.__stores?.document.getState().features ?? []) as FeatureRow[];
  });
}

/** Add a variable through the section's draft row, as a user would. */
async function addVariable(page: Page, name: string, value: string): Promise<void> {
  await page.getByTestId("variable-new-name").fill(name);
  await page.getByTestId("variable-new-value").fill(value);
  await page.getByTestId("variable-new-value").press("Enter");
}

/**
 * Draw a rectangle and extrude it, returning the committed feature's id.
 *
 * The SEEDED timeline cannot stand in for this: its rows carry no stored params,
 * so `getOperationParams` — the read half of the merge-patch every inline edit
 * makes — has nothing to serve.
 */
async function drawAndExtrude(page: Page): Promise<string> {
  // Hide the seeded demo body + sketches first: the region pick below must be
  // unambiguous, and the Extrude button is applicability-gated on "a region is
  // selected, or there is exactly ONE visible sketch".
  await bodyOptions(page).first().getByRole("switch").click();
  const visibleSeeds = page
    .getByRole("listbox", { name: "Sketches" })
    .locator('[role="switch"][aria-checked="true"]');
  while ((await visibleSeeds.count()) > 0) await visibleSeeds.first().click();

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
            (
              window as unknown as {
                __vpEngine?: { sketchStaticHitTest(x: number, y: number): unknown };
              }
            ).__vpEngine?.sketchStaticHitTest(x, y),
          ),
        client,
      ),
    )
    .toBe(true);
  await clickAtClient(page, client.x, client.y);
  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await commitExtrudeAtHandle(page);
  await expect(bodyOptions(page)).toHaveCount(2);

  const rows = await features(page);
  const committed = rows[rows.length - 1];
  expect(committed.label).toBe("Extrude");
  return committed.id;
}

/**
 * Open the FEATURE state (full timeline + row affordances) on `featureId`.
 *
 * Two hops: with a BODY selected the inspector shows the lineage SLICE, which
 * cannot contain a freshly appended op — selecting a feature there is what
 * switches the panel to the full-timeline view.
 */
async function openTimelineOn(page: Page, featureId: string): Promise<void> {
  await page.getByTestId("history-row-f1").click({ position: { x: 8, y: 16 } });
  await expect(page.getByTestId(`history-row-${featureId}`)).toBeVisible();
  await page.getByTestId(`history-row-${featureId}`).click({ position: { x: 8, y: 16 } });
  await expect(page.getByTestId(`history-suppress-${featureId}`)).toBeVisible();
}

test("add, re-value and delete a document variable from the inspector", async ({ page }) => {
  await openEditorDebug(page);
  // Variables lives in its own left-sidebar tab now, not the right inspector.
  await page.getByTestId("sidebar-tab-variables").click();

  await expect(page.getByTestId("variables-empty")).toBeVisible();

  // The section is reachable with NOTHING selected too — a document-level table
  // must not be hidden behind picking some unrelated body. (The inspector's EMPTY
  // state used to host no sections at all; WP-VE.2 changed that deliberately.)
  await page.evaluate(() => {
    (
      window as unknown as { __stores?: { selection: { getState(): { clear(): void } } } }
    ).__stores?.selection.getState().clear();
  });
  await expect(page.getByText("Nothing selected")).toBeVisible();
  await expect(page.getByTestId("variables-empty")).toBeVisible();

  // (1) Add.
  await addVariable(page, "height", "25");
  await expect(page.getByTestId("variable-row-height")).toBeVisible();
  await expect(page.getByTestId("variable-value-height")).toHaveValue("25");
  await expect(page.getByTestId("variables-empty")).toHaveCount(0);
  // The draft row clears, so a second add cannot silently resubmit the first.
  await expect(page.getByTestId("variable-new-name")).toHaveValue("");

  // (2) A malformed name is refused INLINE, and adds nothing.
  await addVariable(page, "2wide", "5");
  await expect(page.getByTestId("variables-error")).toContainText("Invalid variable name");
  await expect(page.getByTestId("variable-row-2wide")).toHaveCount(0);

  // (3) Re-value in place.
  await page.getByTestId("variable-value-height").fill("40");
  await page.getByTestId("variable-value-height").press("Enter");
  await expect(page.getByTestId("variable-value-height")).toHaveValue("40");

  // (4) A duplicate NAME is a re-value, never a second row.
  await addVariable(page, "height", "12");
  await expect(page.getByTestId("variable-row-height")).toHaveCount(1);
  await expect(page.getByTestId("variable-value-height")).toHaveValue("12");

  // (5) Delete.
  await page.getByTestId("variable-delete-height").click();
  await expect(page.getByTestId("variable-row-height")).toHaveCount(0);
  await expect(page.getByTestId("variables-empty")).toBeVisible();
});

test("typing =name into a past extrude's value binds it to a variable", async ({ page }) => {
  await openEditorDebug(page);
  // Variables lives in its own left-sidebar tab now, not the right inspector.
  await page.getByTestId("sidebar-tab-variables").click();
  await addVariable(page, "height", "25");
  await expect(page.getByTestId("variable-row-height")).toBeVisible();

  // `drawAndExtrude` reads the Bodies listbox, which only renders on the
  // Model tab (the two sidebar tabs share one `Slots.ShellLeft` footprint).
  await page.getByTestId("sidebar-tab-model").click();
  const featureId = await drawAndExtrude(page);
  await openTimelineOn(page, featureId);

  const before = (await features(page)).find((f) => f.id === featureId)!;
  expect(before.primaryExpr).toBeUndefined();

  // Open the inline editor and type a BINDING rather than a number.
  await page.getByTestId(`history-value-${featureId}`).click();
  const input = page.getByTestId(`history-row-${featureId}`).getByLabel("Dimension value");
  await expect(input).toBeFocused();
  await input.fill("=height");
  await input.press("Enter");

  // The PROJECTION carries the binding — i.e. the backend recorded it. The row
  // then reads `=height` instead of the resolved number, with the number in the
  // tooltip.
  await expect
    .poll(async () => (await features(page)).find((f) => f.id === featureId)?.primaryExpr)
    .toBe("height");
  const chip = page.getByTestId(`history-value-${featureId}`);
  await expect(chip).toHaveText("=height");
  await expect(chip).toHaveAttribute("title", /^height = /);

  // Re-opening the editor seeds it from the binding, not from the number.
  await chip.click();
  await expect(input).toHaveValue("=height");

  // …and typing a plain number UNBINDS it: the row must never keep showing a
  // binding the document no longer holds.
  await input.fill("60");
  await input.press("Enter");
  await expect
    .poll(async () => (await features(page)).find((f) => f.id === featureId)?.primaryExpr)
    .toBeUndefined();
  await expect(chip).toHaveText("60 mm");
});
