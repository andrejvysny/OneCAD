import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditorDebug, bodyOptions, CANVAS } from "./helpers";

/*
 * PROJECT-IMPORT — the File ▸ "Import Project…" lane, end to end on the MOCK
 * lane.
 *
 * Like STEP-IMPORT WP-A, the real lane is unreachable in plain Chromium (no
 * Tauri bridge → `createClient()` hands back `mockClient`), so this spec pins
 * the FRONTEND half: the menu wiring, the hinted success, and the appended body
 * + history row reaching the projection and the viewport. The "#1049 core" half
 * (document_runtime::import_project) is pinned Rust-side.
 *
 * `mockClient.importProject()` shares the STEP fabrication — one body, an
 * `ImportStep` row — so the assertions below mirror step-import's.
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

test("File ▸ Import Project appends a body, an Import row, and renders it", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await expect(page.locator(`${CANVAS} canvas`)).toBeVisible();
  await expect.poll(() => renderedBodyIds(page)).toHaveLength(1);

  await page.getByRole("button", { name: "File" }).click();
  await page.getByRole("menuitem", { name: /Import Project/ }).click();

  await expect(page.getByText("Project imported")).toBeVisible();
  await expect.poll(() => bodyNames(page)).toContain("Imported 1");
  await expect(bodyOptions(page).filter({ hasText: "Imported 1" })).toHaveCount(1);
  await expect.poll(() => featureRows(page)).toContainEqual(["ImportStep", "Import"]);
  await expect.poll(() => renderedBodyIds(page)).toHaveLength(2);
});

test("the start screen's Import Project opens the editor with the imported body", async ({
  page,
}) => {
  await page.goto("/");

  // The sidebar's bottom-of-rail import group: second button, below Import STEP.
  await page.getByRole("button", { name: /Import Project/ }).click();

  await expect(page.getByRole("button", { name: "File" })).toBeVisible();
  await expect.poll(() => bodyNames(page)).toContain("Imported 1");
  await expect.poll(() => featureRows(page)).toContainEqual(["ImportStep", "Import"]);
});