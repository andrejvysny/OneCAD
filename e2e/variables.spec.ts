import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
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
 *
 * The EXPRESSION suites below extend both loops to real arithmetic: a value
 * cell that takes `20mm`, a binding that takes `=w*2 + 5mm` and previews what it
 * resolves to before committing, a rename that carries the binding with it, and
 * the two unit guardrails. The mock resolves all of it through the shared TS
 * port of `onecad-core`'s evaluator, so these prove the semantics the real
 * backend applies rather than a lane-local approximation.
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

/*
 * W5 — "saved + loud failure banner", end to end.
 *
 * A variable edit whose downstream regen fails has TWO truths, and until this
 * work package the UI stated only the first. Both are asserted here on the real
 * DOM: the table keeps the value the document actually holds (nothing is ever
 * reverted, and nothing is turned into a rejection), and the failure lands on the
 * SAME status-bar affordance every other commit family reports through.
 *
 * The mock lane can decide both failures with no kernel: the delete case is pure
 * `resolve_expr`, and the value case mirrors `ExtrudeOp.cpp`'s `kMinValue` blind
 * -extrude floor by citation.
 */
test("a variable edit that breaks its bound extrude saves the value AND says the rebuild failed", async ({
  page,
}) => {
  await openEditorDebug(page);
  await page.getByTestId("sidebar-tab-variables").click();
  await addVariable(page, "height", "25");
  await expect(page.getByTestId("variable-row-height")).toBeVisible();

  await page.getByTestId("sidebar-tab-model").click();
  const featureId = await drawAndExtrude(page);
  await openTimelineOn(page, featureId);

  // Bind the extrude's distance to the variable, as the binding spec above does.
  await page.getByTestId(`history-value-${featureId}`).click();
  const input = page.getByTestId(`history-row-${featureId}`).getByLabel("Dimension value");
  await expect(input).toBeFocused();
  await input.fill("=height");
  await input.press("Enter");
  await expect
    .poll(async () => (await features(page)).find((f) => f.id === featureId)?.primaryExpr)
    .toBe("height");

  // Now drive it below the kernel's blind-extrude distance floor.
  await page.getByTestId("sidebar-tab-variables").click();
  await page.getByTestId("variable-value-height").fill("0");
  await page.getByTestId("variable-value-height").press("Enter");

  // Truth 1 — the SAVE is real: the row shows what the document holds, at 0.
  await expect(page.getByTestId("variable-value-height")).toHaveValue("0");
  // Truth 2 — the failure is on screen, naming the kernel's own reason.
  await expect(
    page.getByText(/Variable saved, but the rebuild failed: .*distance too small/),
  ).toBeVisible();
  // …and it is NOT reported as a refusal: nothing was rejected, and collapsing
  // the two would lose exactly the distinction the user needs.
  await expect(page.getByTestId("variables-error")).toHaveCount(0);

  // Deleting a variable a record still binds is the same contract: the removal
  // STANDS, and the resolver's own reason is what the user is told.
  await page.getByTestId("variable-delete-height").click();
  await expect(page.getByTestId("variable-row-height")).toHaveCount(0);
  await expect(page.getByTestId("variables-empty")).toBeVisible();
  await expect(
    page.getByText(/Variable saved, but the rebuild failed: .*undefined variable `height`/),
  ).toBeVisible();
});

/**
 * Pick a display unit through the REAL control.
 *
 * A local copy of `units.spec.ts`'s gesture, including its camera settle: the
 * ViewCube re-renders on every camera change and the popover lives in the same
 * subtree, so clicking a tab while the intro framing animates detaches the node
 * mid-click.
 */
async function pickUnit(page: Page, unit: "mm" | "in"): Promise<void> {
  await waitForCameraSettled(page);
  const button = page.getByRole("button", { name: /^Display mode/ });
  await button.click();
  const tab = page.getByRole("tablist", { name: "Units" }).getByRole("tab", { name: unit });
  await expect(tab).toBeVisible();
  await tab.click();
  // Closed by TOGGLING the anchor, never Escape: Escape is also the global
  // cancel, and here it would drop the feature selection the timeline view (and
  // therefore the row under test) depends on. Same hazard units.spec.ts
  // documents for an armed tool.
  await button.click();
  await expect(page.getByRole("tablist", { name: "Units" })).toHaveCount(0);
}

/** Open the inline value editor on a history row and return its input. */
function valueEditor(page: Page, featureId: string) {
  return page.getByTestId(`history-row-${featureId}`).getByLabel("Dimension value");
}

/*
 * The whole expression loop on one document: author a dimensioned variable as
 * TEXT, bind a past extrude to arithmetic over it, watch the preview resolve
 * BEFORE committing, rename the variable and see the binding follow, then
 * delete it and be told loudly.
 *
 * Every step is the real DOM down to the client. The preview in particular is
 * the honesty gate here: it is a live `evaluateExpression` round trip, so a
 * lane that could not actually evaluate `w*2 + 5mm` would show nothing to
 * assert on.
 */
test("author, bind, preview, rename and break an expression-driven dimension", async ({
  page,
}) => {
  await openEditorDebug(page);
  await page.getByTestId("sidebar-tab-variables").click();

  // (1) A variable authored as TEXT with a unit — not a number field.
  await addVariable(page, "w", "20mm");
  await expect(page.getByTestId("variable-row-w")).toBeVisible();
  await expect(page.getByTestId("variable-value-w")).toHaveValue("=20mm");
  // The row says what it resolves to, with the unit its own expression implies.
  await expect(page.getByTestId("variable-resolved-w")).toHaveText("= 20 mm");

  await page.getByTestId("sidebar-tab-model").click();
  const featureId = await drawAndExtrude(page);
  await openTimelineOn(page, featureId);

  // (2) Bind the extrude to ARITHMETIC over it. The preview resolves live…
  await page.getByTestId(`history-value-${featureId}`).click();
  const input = valueEditor(page, featureId);
  await expect(input).toBeFocused();
  await input.fill("=w*2 + 5mm");
  await expect(page.getByTestId("dimension-expr-preview")).toHaveText("= 45 mm");

  // …and the committed number is THAT one, recorded with its expression.
  await input.press("Enter");
  await expect
    .poll(async () => (await features(page)).find((f) => f.id === featureId)?.primaryExpr)
    .toBe("w*2 + 5mm");
  const chip = page.getByTestId(`history-value-${featureId}`);
  await expect(chip).toHaveText("=w*2 + 5mm");
  await expect(chip).toHaveAttribute("title", "w*2 + 5mm = 45 mm");

  // (3) RENAME. Not remove+add: the binding must follow, in one transaction.
  await page.getByTestId("sidebar-tab-variables").click();
  await page.getByTestId("variable-name-w").fill("width");
  await page.getByTestId("variable-name-w").press("Enter");
  await expect(page.getByTestId("variable-row-width")).toBeVisible();
  await expect(page.getByTestId("variable-row-w")).toHaveCount(0);
  await expect
    .poll(async () => (await features(page)).find((f) => f.id === featureId)?.primaryExpr)
    .toBe("width*2 + 5mm");

  // (4) DELETE the variable the extrude still binds. The removal STANDS and the
  // failure is loud, naming the field and the evaluator's own reason. (On this
  // lane the surface is the sticky status hint — the mock runs no regen, so
  // there is no per-step `EXPR_UNRESOLVED` tint for it to stamp.)
  await page.getByTestId("variable-delete-width").click();
  await expect(page.getByTestId("variable-row-width")).toHaveCount(0);
  await expect(
    page.getByText(
      /Variable saved, but the rebuild failed: Extrude\.distance: undefined variable `width`/,
    ),
  ).toBeVisible();
});

/*
 * The two rules a value field now holds at once, and the guardrails that keep
 * them from contradicting each other silently:
 *
 *   - a bare number INSIDE an expression is millimetres (the site's canonical
 *     unit, matching `substitute_variables`);
 *   - a bare number typed on its OWN is read in the display unit.
 *
 * Displaying inches is the only place they disagree, so that is where this
 * test lives.
 */
test("expression unit guardrails and the site refusal", async ({ page }) => {
  await openEditorDebug(page);
  await page.getByTestId("sidebar-tab-variables").click();
  await addVariable(page, "w", "20mm");
  await expect(page.getByTestId("variable-row-w")).toBeVisible();

  await page.getByTestId("sidebar-tab-model").click();
  const featureId = await drawAndExtrude(page);
  await openTimelineOn(page, featureId);
  await pickUnit(page, "in");

  // GUARDRAIL (b): a PURE literal after `=` is the plain path — 2 INCHES, the
  // same as typing `2`, and no binding is recorded. The two spellings of a lone
  // number must never mean different lengths.
  await page.getByTestId(`history-value-${featureId}`).click();
  await valueEditor(page, featureId).fill("=2");
  await valueEditor(page, featureId).press("Enter");
  await expect
    .poll(async () => (await features(page)).find((f) => f.id === featureId)?.primaryValue)
    .toBeCloseTo(50.8, 3);
  expect((await features(page)).find((f) => f.id === featureId)?.primaryExpr).toBeUndefined();

  // GUARDRAIL (a)+(c): a real expression previews the millimetres it resolved
  // to AND the display-unit equivalent, and says where the bare numbers land.
  await page.getByTestId(`history-value-${featureId}`).click();
  await valueEditor(page, featureId).fill("=w/2");
  await expect(page.getByTestId("dimension-expr-preview")).toHaveText("= 10 mm (0.3937 in)");
  await expect(page.getByTestId("dimension-expr-hint")).toContainText(
    "bare numbers inside = are millimetres",
  );

  // An ANGLE in a length field is a loud refusal, inline — never a silent
  // 45 mm. Typed into the SAME open editor, so nothing is dismissed in between.
  await valueEditor(page, featureId).fill("=45deg");
  await expect(page.getByTestId("dimension-expr-preview")).toContainText("expected length");
  await valueEditor(page, featureId).press("Enter");
  await expect(valueEditor(page, featureId)).toHaveAttribute("aria-invalid", "true");
  // Nothing was recorded: the row never took the binding.
  expect((await features(page)).find((f) => f.id === featureId)?.primaryExpr).toBeUndefined();
});
