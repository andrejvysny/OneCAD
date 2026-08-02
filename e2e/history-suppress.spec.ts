import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditor, bodyOptions } from "./helpers";

/*
 * TRUST wave — history suppression is PROJECTION-driven.
 *
 * The dim state used to come from an optimistic frontend overlay
 * (`historyStore`), which is now deleted: `FeatureDto.suppressed` rides every
 * projection, so a row dims off `FeatureMeta.suppressed` and the toggle negates
 * the feature's OWN flag. The overlay's two defects this spec locks out:
 *   (a) a features array replaced wholesale by a later backend edit must NOT
 *       reset the dim state (the overlay and the projection could disagree), and
 *   (b) un-suppressing must actually send `suppressed:false` — the overlay
 *       computed `!undefined === true` for any suppression it had not itself
 *       applied, so a persisted suppression could never be undone.
 *
 * Mock lane: `mockApplyEditCommand` tracks the flag on its feature list, so the
 * whole loop is real UI → real EditCommand → real projection hydration.
 */

/** The `suppressed` flag of a projection feature (`__stores.document`, dev-only). */
async function featureSuppressed(page: Page, id: string): Promise<boolean | undefined> {
  return page.evaluate((featureId) => {
    const w = window as unknown as {
      __stores?: {
        document: { getState(): { features: Array<{ id: string; suppressed?: boolean }> } };
      };
    };
    return w.__stores?.document.getState().features.find((f) => f.id === featureId)?.suppressed;
  }, id);
}

/** Select a body, then its Fillet history chip — the only path to the row affordances. */
async function openFeatureTimeline(page: Page): Promise<void> {
  await bodyOptions(page).first().click();
  await expect(page.getByTestId("history-row-f3")).toBeVisible();
  await page.getByTestId("history-row-f3").click();
  // The FEATURE state renders the full timeline WITH per-row affordances.
  await expect(page.getByTestId("history-suppress-f3")).toBeVisible();
  await expect(page.getByTestId("history-row-f5")).toBeVisible();
}

test("suppressing a history row dims it, survives a later edit's fresh timeline, and un-suppresses", async ({
  page,
}) => {
  await openEditor(page);
  await openFeatureTimeline(page);

  const row = page.getByTestId("history-row-f3");
  await expect(row).not.toHaveClass(/opacity-60/);

  // (1) Suppress → the row dims from the projection flag the command returned.
  await page.getByTestId("history-suppress-f3").click();
  await expect(row).toHaveClass(/opacity-60/);
  await expect(page.getByText("Feature suppressed")).toBeVisible();
  expect(await featureSuppressed(page, "f3")).toBe(true);

  // (2) An UNRELATED edit replaces the whole features array (RemoveOperation
  // returns the rebuilt timeline). The dim state must ride that new array.
  await page.getByTestId("history-delete-f1").click();
  await page.getByTestId("history-delete-confirm-f1").click();
  await expect(page.getByTestId("history-row-f1")).toHaveCount(0);
  await expect(row).toHaveClass(/opacity-60/);
  expect(await featureSuppressed(page, "f3")).toBe(true);

  // (3) Un-suppress: the toggle negates the feature's own flag, so it sends
  // `suppressed:false` (the overlay used to send `true` here, forever).
  await page.getByTestId("history-suppress-f3").click();
  await expect(row).not.toHaveClass(/opacity-60/);
  await expect(page.getByText("Feature unsuppressed")).toBeVisible();
  expect(await featureSuppressed(page, "f3")).toBe(false);
});
