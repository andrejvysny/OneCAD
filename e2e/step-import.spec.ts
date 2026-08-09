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

/**
 * Live face-material facts per body, straight off the scene graph (`?vpdebug`):
 * does its face Mesh draw with a vertex-colored material, and does its geometry
 * actually carry the baked `color` attribute? Asserting the STORE here would
 * prove nothing — FACE_COLORS never reaches a store; the whole path is
 * parse → de-index → attribute → material kind, and only the scene shows it.
 */
function faceColorFacts(page: Page): Promise<Record<string, { vertexColors: boolean; colorAttr: boolean; indexed: boolean }>> {
  return page.evaluate(() => {
    const engine = (
      window as unknown as {
        __vpEngine?: {
          bodiesRoot: {
            children: Array<{
              userData: { bodyId?: string };
              children: Array<{
                userData: { kind?: string };
                material?: { vertexColors?: boolean };
                geometry?: { attributes: Record<string, unknown>; index: unknown };
              }>;
            }>;
          };
        };
      }
    ).__vpEngine;
    const out: Record<string, { vertexColors: boolean; colorAttr: boolean; indexed: boolean }> = {};
    for (const group of engine?.bodiesRoot.children ?? []) {
      const face = group.children.find((c) => c.userData.kind === "face");
      if (!face) continue;
      out[String(group.userData.bodyId)] = {
        vertexColors: face.material?.vertexColors === true,
        colorAttr: face.geometry?.attributes.color != null,
        indexed: face.geometry?.index != null,
      };
    }
    return out;
  });
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

test("the imported body renders with its authored MESH1 face colors", async ({ page }) => {
  // The seed body is the CONTRAST: same scene, same mode, no FACE_COLORS — so a
  // blanket "everything got vertexColors" regression cannot pass this.
  await openEditorDebug(page, { mockBody: true });
  await expect.poll(() => renderedBodyIds(page)).toHaveLength(1);
  const [seedId] = Object.keys(await faceColorFacts(page));

  await importViaFileMenu(page);
  await expect(page.getByText("Imported 1 body")).toBeVisible();
  await expect.poll(() => renderedBodyIds(page)).toHaveLength(2);

  const facts = await faceColorFacts(page);
  const importedId = Object.keys(facts).find((id) => id !== seedId)!;

  // The imported body: baked color attribute, vertex-colored material, and
  // de-indexed (the index is dropped so a shared vertex cannot bleed between two
  // differently-colored faces).
  expect(facts[importedId]).toEqual({ vertexColors: true, colorAttr: true, indexed: false });
  // The seed body keeps the plain shaded material on zero-copy indexed geometry.
  expect(facts[seedId]).toEqual({ vertexColors: false, colorAttr: false, indexed: true });
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

test("the start screen's Import… opens the editor with the imported STEP body", async ({ page }) => {
  // ONE import button now; `appStore.importFromDialog` routes on the extension the
  // picker returns. `?mockimport=step` makes the mock picker return a `.step` path,
  // which is the only way to reach the STEP half of that router from a browser lane.
  await page.goto("/?mockimport=step");

  await page.getByRole("button", { name: /^Import…$/ }).click();

  // The editor shell mounted (the File menu is its stable marker), carrying the
  // imported body + its history row.
  await expect(page.getByRole("button", { name: "File" })).toBeVisible();
  await expect.poll(() => bodyNames(page)).toContain("Imported 1");
  await expect.poll(() => featureRows(page)).toContainEqual(["ImportStep", "Import"]);
});
