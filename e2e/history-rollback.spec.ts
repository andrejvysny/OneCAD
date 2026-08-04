import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditor, bodyOptions } from "./helpers";

/*
 * HISTORY-HARDEN H7b — the rollback cursor is REACHABLE and VISIBLE.
 *
 * Before this wave a rollback was invisible on the mock lane (the mock's
 * `setRollback` was a revision bump) and unreachable without hovering a row's
 * icon cluster. The loop pinned here is real end to end: right-click → menu →
 * `SetRollback` → the mock's cursor + body masking (`setMockCursor`, mirroring
 * core `Timeline::set_cursor`) → the cursor riding the edit result → grayed rows,
 * the marker, and the banner that offers the way back.
 *
 * Cursor semantics under test (timeline.rs): the cursor is the APPLIED OP COUNT,
 * so "roll to here" on row k is cursor = k + 1 and rows [cursor, len) are drafts.
 */

/** The projection's rollback cursor + timeline length (`__stores`, dev-only). */
async function cursor(page: Page): Promise<{ applied: number; total: number }> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: {
        document: { getState(): { appliedOps: number; features: unknown[] } };
      };
    };
    const s = w.__stores?.document.getState();
    return { applied: s?.appliedOps ?? -1, total: s?.features.length ?? -1 };
  });
}

/**
 * Reach the FULL timeline: a BODY selection renders only a three-row lineage
 * slice (InspectorPanel `SelectionState`), and a slice deliberately gets NO
 * cursor chrome — its indices are not global. One more click onto a history row
 * switches to the feature state, which renders every row.
 */
async function openFullTimeline(page: Page): Promise<void> {
  await bodyOptions(page).first().click();
  await expect(page.getByTestId("history-row-f3")).toBeVisible();
  await page.getByTestId("history-row-f3").click();
  await expect(page.getByTestId("history-suppress-f3")).toBeVisible();
  await expect(page.getByTestId("history-row-f5")).toBeVisible();
}

test("roll to a middle row via the context menu, then roll back to the end", async ({ page }) => {
  await openEditor(page);
  await openFullTimeline(page);

  // Fully applied to start: no marker, no banner, every row live.
  expect(await cursor(page)).toEqual({ applied: 5, total: 5 });
  await expect(page.getByTestId("history-rollback-banner")).toHaveCount(0);
  await expect(page.getByTestId("history-rollback-marker")).toHaveCount(0);

  // (1) Right-click row f2 (index 1) → "Roll to here" ⇒ cursor 2.
  await page.getByTestId("history-row-f2").click({ button: "right" });
  await expect(page.getByTestId("history-menu-roll-here")).toBeVisible();
  await page.getByTestId("history-menu-roll-here").click();
  await expect(page.getByText("Rolled timeline")).toBeVisible();
  await expect.poll(() => cursor(page)).toEqual({ applied: 2, total: 5 });

  // (2) The bar is drawn, the three drafts read as drafts, the prefix does not.
  await expect(page.getByTestId("history-rollback-marker")).toBeVisible();
  await expect(page.getByTestId("history-rollback-marker")).toHaveText(/rolled back/);
  for (const id of ["f1", "f2"]) {
    await expect(page.getByTestId(`history-row-${id}`)).toHaveAttribute("data-applied", "true");
  }
  for (const id of ["f3", "f4", "f5"]) {
    const row = page.getByTestId(`history-row-${id}`);
    await expect(row).toHaveAttribute("data-applied", "false");
    await expect(row).toHaveClass(/italic/);
    await expect(row).toHaveAttribute("title", "Not applied — beyond the rollback bar");
  }

  // (3) The persistent banner counts them and offers the way back.
  await expect(page.getByTestId("history-rollback-banner")).toHaveText(/3 operations rolled back/);
  await page.getByTestId("history-roll-to-end").click();
  await expect.poll(() => cursor(page)).toEqual({ applied: 5, total: 5 });
  await expect(page.getByTestId("history-rollback-banner")).toHaveCount(0);
  await expect(page.getByTestId("history-rollback-marker")).toHaveCount(0);
  await expect(page.getByTestId("history-row-f5")).toHaveAttribute("data-applied", "true");
});

test("the menu offers Roll to end only while the timeline is rolled back", async ({ page }) => {
  await openEditor(page);
  await openFullTimeline(page);

  // Fully applied ⇒ a "Roll to end" item would be a dead affordance.
  await page.getByTestId("history-row-f3").click({ button: "right" });
  await expect(page.getByTestId("history-menu-roll-here")).toBeVisible();
  await expect(page.getByTestId("history-menu-roll-end")).toHaveCount(0);

  // Right-clicking ANOTHER row closes the open menu (Popover's outside-press) and
  // re-anchors onto that row. Escape would too — but it is also the editor's
  // "clear selection" chord, which would take the whole timeline off screen.
  await page.getByTestId("history-row-f1").click({ button: "right" });
  await page.getByTestId("history-menu-roll-here").click();
  await expect.poll(() => cursor(page)).toEqual({ applied: 1, total: 5 });

  await page.getByTestId("history-row-f1").click({ button: "right" });
  await page.getByTestId("history-menu-roll-end").click();
  await expect.poll(() => cursor(page)).toEqual({ applied: 5, total: 5 });
});
