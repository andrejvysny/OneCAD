import { test, expect } from "./fixtures";

/*
 * COMPONENT-LIBRARY WP-C2 — "Import components…" through the real UI chain on
 * the MOCK lane: Library modal → header button → the dialog → the hidden
 * multi-file input (this lane's stand-in for a real Tauri multi-select
 * dialog, which does not exist yet — see `IngestComponentsDialog`'s header
 * comment) → the batch call → one row per requested file.
 *
 * The mock lane has no STEP reader and no worker, so both files refuse by
 * name (`mockClient.ingestComponents`'s own MOCK LIMIT) — this spec owns the
 * WIRING (button → dialog → picker → call → rows), not geometry.
 *
 * The trigger is a standalone header BUTTON, not the "Library options"
 * overflow beside it — that Popover renders at a lower z-index than this
 * modal's own content and its clicks land on the detail rail instead (a
 * pre-existing defect, confirmed against "Rebuild index" too and reported,
 * not fixed, since `Popover` is a shared primitive outside this WP).
 */

test.beforeEach(async ({ page }) => {
  await page.goto("/?vpdebug&vpdemo&mocklibrary=1");
  await expect(page.locator('[data-testid="viewport-canvas"] canvas')).toBeVisible();
});

test("imports two files, both refuse with the MOCK LIMIT reason, and the Library modal stays usable", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Library" }).click();
  const libraryDialog = page.getByRole("dialog", { name: "Library" });
  await expect(libraryDialog).toBeVisible();

  await page.getByTestId("library-import-components").click();

  const importDialog = page.getByTestId("library-ingest-dialog");
  await expect(importDialog).toBeVisible();

  const submit = page.getByTestId("library-ingest-submit");
  await expect(submit).toBeDisabled();

  await page.getByTestId("library-ingest-file-input").setInputFiles([
    { name: "bracket.step", mimeType: "application/octet-stream", buffer: Buffer.from("dummy") },
    { name: "rail.stp", mimeType: "application/octet-stream", buffer: Buffer.from("dummy") },
  ]);
  await expect(submit).toBeEnabled();
  await submit.click();

  const rows = page.getByTestId("library-ingest-result-row");
  await expect(rows).toHaveCount(2);
  for (const row of await rows.all()) {
    await expect(row).toHaveAttribute("data-status", "refused");
    await expect(row).toContainText("MOCK LIMIT");
  }

  // Closing the import dialog leaves the Library modal open and searchable —
  // the batch call never touched its own chrome.
  await page.getByTestId("library-ingest-cancel").click();
  await expect(importDialog).toBeHidden();
  await expect(libraryDialog).toBeVisible();
  await page.getByPlaceholder("Search components…").fill("shcs");
  await expect(page.getByPlaceholder("Search components…")).toHaveValue("shcs");

  await page.getByLabel("Close library").click();
  await expect(libraryDialog).toBeHidden();
});
