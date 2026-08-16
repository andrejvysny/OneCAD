import { test, expect } from "./fixtures";
import { CANVAS } from "./helpers";

/*
 * Crash recovery, start screen — the flow that had NO browser coverage at all.
 *
 * `?mockrecovery=1` seeds one offer whose `originalPath` is the FIRST recent card
 * (the same dev-only URL-flag pattern `?mockimport=step` / `?mocklibrary=1` use — a
 * browser has no crashed session to discover). That overlap is deliberate: it makes
 * the destructive path reachable, which is the one this spec exists for.
 *
 * The recovery banner sits ABOVE a fully clickable recents list, so opening the
 * shadowed file was always one click away — and it destroyed the autosave silently,
 * because the reopened document carries the same id and its first autosave
 * overwrites the crash container. The backend now refuses that open; these specs
 * pin the refusal, both resolutions, and the badge that warns before the click.
 */

const SHADOWED = "Bracket v2";

/**
 * The recents CARD for the shadowed project, addressed by its `title` (the project
 * path — `ProjectCard` sets it). Both the recovery banner and the card's own kebab
 * menu carry the project NAME, so name-based locators are ambiguous by construction
 * in exactly this scenario; the path is not.
 */
const shadowedCard = (page: import("@playwright/test").Page) =>
  page.locator(`[role="button"][title$="${SHADOWED}.onecad"]`);

test("the banner names the document and the TIME its work was captured", async ({ page }) => {
  await page.goto("/?mockrecovery=1");

  await expect(page.getByText("Unsaved changes recovered")).toBeVisible();
  const line = page.getByText(/autosaved/);
  await expect(line).toBeVisible();
  // A date alone renders a crash ninety seconds ago identically to one last
  // Tuesday, which is the only thing the user actually needs to know.
  await expect(line).toHaveText(/\d{1,2}:\d{2}/);
});

test("Restore opens the recovered document", async ({ page }) => {
  await page.goto("/?mockrecovery=1");
  await page.getByRole("button", { name: "Restore" }).click();

  await expect(page.locator(`${CANVAS} canvas`)).toBeVisible();
  await expect(page.getByText("Unsaved changes recovered")).toBeHidden();
});

test("Discard clears the offer and stays on the start screen", async ({ page }) => {
  await page.goto("/?mockrecovery=1");
  await page.getByRole("button", { name: "Discard" }).click();

  await expect(page.getByText("Unsaved changes recovered")).toBeHidden();
  await expect(page.getByRole("button", { name: "New project" })).toBeVisible();
});

test("a recent shadowed by an autosave is badged, and clicking it prompts", async ({ page }) => {
  await page.goto("/?mockrecovery=1");
  // The badge is the warning that arrives BEFORE the click.
  await expect(page.getByText("Unsaved", { exact: true })).toBeVisible();

  await shadowedCard(page).click();

  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("newer unsaved changes");
  // Nothing was opened: the destructive act has not happened yet.
  await expect(page.getByRole("button", { name: "New project" })).toBeVisible();
});

test("Cancel on the prompt leaves both the file and the autosave alone", async ({ page }) => {
  await page.goto("/?mockrecovery=1");
  await shadowedCard(page).click();
  await page.getByRole("dialog").getByRole("button", { name: "Cancel" }).click();

  await expect(page.getByRole("dialog")).toBeHidden();
  await expect(page.getByText("Unsaved changes recovered")).toBeVisible();
  await expect(page.getByRole("button", { name: "New project" })).toBeVisible();
});

test("Restore Changes from the prompt recovers rather than opening the older file", async ({
  page,
}) => {
  await page.goto("/?mockrecovery=1");
  await shadowedCard(page).click();
  await page.getByRole("dialog").getByRole("button", { name: "Restore Changes" }).click();

  await expect(page.locator(`${CANVAS} canvas`)).toBeVisible();
});

test("Open Saved Version discards the autosave — only when chosen", async ({ page }) => {
  await page.goto("/?mockrecovery=1");
  await shadowedCard(page).click();
  await page.getByRole("dialog").getByRole("button", { name: "Open Saved Version" }).click();

  await expect(page.locator(`${CANVAS} canvas`)).toBeVisible();
  // The offer is gone deliberately: the user chose the older file over the newer
  // work. Returning to the start screen must not re-offer it.
  const app = await page.evaluate(
    () =>
      (
        window as unknown as { __stores: { app: { getState: () => { recovery: unknown[] } } } }
      ).__stores.app.getState().recovery,
  );
  expect(app).toEqual([]);
});
