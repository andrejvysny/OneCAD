import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditorDebug, bodyOptions, CANVAS } from "./helpers";

/*
 * STEP-IMPORT WP-A W4 — the frontend import lane, end to end on the MOCK lane.
 *
 * Two entry points, one fabrication:
 *   - in-editor : File ▸ Import STEP… → `CadClient.insertStep()` (`insert_step`),
 *     which APPENDS to the open document — no swap, nothing lost.
 *   - start screen : "Import STEP…" → `CadClient.importStep(path)` (`import_step`),
 *     which opens a NEW document from the file.
 *
 * The real lane is unreachable here by construction: there is no Tauri bridge in
 * plain Chromium, so `createClient()` hands back `mockClient` and Rust's file
 * dialog never runs. What this spec CAN prove is everything the frontend owns —
 * the menu wiring, the hint copy, the projection landing in the tree + history,
 * the mesh reaching the viewport, and the deliberate ABSENCE of a re-edit editor
 * on an imported row. The kernel half is pinned Rust-side.
 */

/** Body names in the projection store (the tree renders these verbatim). */
function bodyNames(page: Page): Promise<string[]> {
  return page.evaluate(() =>
    Object.values(
      (
        window as unknown as {
          __stores?: { document: { getState(): { bodies: Record<string, { name: string }> } } };
        }
      ).__stores?.document.getState().bodies ?? {},
    ).map((b) => b.name),
  );
}

/** The feature timeline as `[opType, label]` pairs, straight from the projection. */
function featureRows(page: Page): Promise<[string, string][]> {
  return page.evaluate(() =>
    (
      window as unknown as {
        __stores?: {
          document: { getState(): { features: Array<{ opType?: string; label: string }> } };
        };
      }
    ).__stores?.document.getState().features.map(
      (f) => [f.opType ?? "", f.label] as [string, string],
    ) ?? [],
  );
}

/** bodyIds the ViewportEngine actually holds geometry for (`?vpdebug` surface). */
function renderedBodyIds(page: Page): Promise<string[]> {
  return page.evaluate(() =>
    (
      window as unknown as {
        __vpEngine?: { bodiesRoot: { children: Array<{ userData: { bodyId?: string } }> } };
      }
    ).__vpEngine?.bodiesRoot.children.map((c) => String(c.userData.bodyId ?? "")) ?? [],
  );
}

/** Which re-edit chip the ModelToolController armed ("none" = no editor open). */
function armedChipKind(page: Page): Promise<string | undefined> {
  return page.evaluate(
    () =>
      (window as unknown as { __stores?: { toolChip: { getState(): { kind: string } } } }).__stores
        ?.toolChip.getState().kind,
  );
}

/** Open the title-bar File menu and click "Import STEP…". */
async function importViaFileMenu(page: Page): Promise<void> {
  await page.getByRole("button", { name: "File" }).click();
  await page.getByRole("menuitem", { name: /Import STEP/ }).click();
}

/**
 * Reach the FULL feature timeline. A body selection shows only the first three
 * lineage rows (InspectorPanel `SelectionState`), so — like the suppress spec —
 * one more click onto a history row switches to the feature state, which renders
 * every row including the appended import.
 */
async function openFullTimeline(page: Page): Promise<void> {
  await bodyOptions(page).first().click();
  await page.getByTestId("history-row-f3").click();
  await expect(page.getByTestId("history-suppress-f3")).toBeVisible();
}

/** A timeline row by its label (rows carry no per-opType testid). */
function historyRow(page: Page, label: string) {
  return page.locator('[data-testid^="history-row-"]').filter({ hasText: label });
}

/**
 * Attempt a re-edit on a row: SELECT it first, then double-click.
 *
 * The select is not ceremony. `FeatureState`'s header grows a "Double-click to
 * edit the …" line for the editable kinds only, so selecting a row of a different
 * kind reflows the list under the cursor — the two clicks of a bare `dblclick()`
 * would then land on different rows and the browser would never dispatch
 * `dblclick` at all, quietly turning this spec green for the wrong reason.
 */
async function tryReEdit(page: Page, row: ReturnType<typeof historyRow>): Promise<void> {
  await row.click();
  await expect(row).toHaveClass(/bg-sel-bg/);
  await row.dblclick();
}

test("File ▸ Import STEP appends a body, an Import row, and renders it", async ({ page }) => {
  // `vpdemo` drives the seed body through the real ingest path, so the assertion
  // below is "one MORE body rendered", not "the only body rendered".
  await openEditorDebug(page, { mockBody: true });
  await expect(page.locator(`${CANVAS} canvas`)).toBeVisible();
  await expect.poll(() => renderedBodyIds(page)).toHaveLength(1);

  await importViaFileMenu(page);

  // (1) The success hint names the count the backend reported.
  await expect(page.getByText("Imported 1 body")).toBeVisible();

  // (2) The body arrives through the normal body projection — tree row included.
  await expect.poll(() => bodyNames(page)).toContain("Imported 1");
  await expect(bodyOptions(page).filter({ hasText: "Imported 1" })).toHaveCount(1);

  // (3) The history row is an `ImportStep` labelled "Import".
  await expect.poll(() => featureRows(page)).toContainEqual(["ImportStep", "Import"]);

  // (4) The mesh actually reached the viewport (a second body group in the scene),
  //     which is the half a store-only assertion would miss.
  await expect.poll(() => renderedBodyIds(page)).toHaveLength(2);
});

test("double-clicking an imported history row hints instead of opening an editor", async ({
  page,
}) => {
  await openEditorDebug(page, { mockBody: true });
  await importViaFileMenu(page);
  await expect(page.getByText("Imported 1 body")).toBeVisible();

  await openFullTimeline(page);
  const importRow = historyRow(page, "Import");
  await expect(importRow).toHaveCount(1);

  await tryReEdit(page, importRow);

  // An import has no parametric inputs, so no editor may arm. Routing on the
  // coarse `kind` bucket would have opened the BOOLEAN editor here (an import
  // projects as kind "boolean"); the armed-chip kind proves it did not.
  await expect(page.getByText("Imported feature — re-import to update")).toBeVisible();
  await expect.poll(() => armedChipKind(page)).toBe("none");

  // Contrast: a real parametric row on the same list DOES arm an editor, so the
  // assertion above is not vacuously true.
  await tryReEdit(page, historyRow(page, "Fillet"));
  await expect.poll(() => armedChipKind(page)).toBe("filletRadius");
});

test("the start screen's Import STEP opens the editor with the imported body", async ({ page }) => {
  await page.goto("/");

  await page.getByRole("button", { name: /Import STEP/ }).click();

  // The editor shell mounted (the File menu is its stable marker), carrying the
  // imported body + its history row.
  await expect(page.getByRole("button", { name: "File" })).toBeVisible();
  await expect.poll(() => bodyNames(page)).toContain("Imported 1");
  await expect.poll(() => featureRows(page)).toContainEqual(["ImportStep", "Import"]);
});
