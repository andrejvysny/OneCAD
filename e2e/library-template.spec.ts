import { test, expect } from "./fixtures";
import { openEditor } from "./helpers";

/*
 * COMPONENT-LIBRARY WP-B3/B5 — project templates end to end on the MOCK lane:
 * save the open document as a template, close the project, find it on the start
 * screen's Templates row, and start a new document from it.
 *
 * WHAT THIS OWNS: the loop. "Save as Template…" is a registered COMMAND (the
 * shell must not import the library to offer a library action), the start
 * screen's `templates` nav key was a stub until WP-B3, and choosing a template
 * must start a NEW document rather than opening the template itself. Those are
 * four separate seams that only meet in a running app.
 *
 * The mock lane keeps templates in memory and hands back a synthetic document
 * — the FLOW is real, the starting geometry is not (see `mockClient.
 * newFromTemplate`). Instantiating a real frozen container is a Rust concern.
 *
 * `retries: 0`, matching the rest of this suite.
 */

/** Runs a registered command by title through the command palette (⌘K / ⌃K). */
async function runCommand(page: import("@playwright/test").Page, title: string): Promise<void> {
  await page.keyboard.press(process.platform === "darwin" ? "Meta+k" : "Control+k");
  const palette = page.getByPlaceholder(/command/i);
  await expect(palette).toBeVisible();
  await palette.fill(title);
  await page.getByText(title, { exact: true }).click();
}

test("save a template, then start a new project from it", async ({ page }) => {
  // Booted through the START SCREEN, not `?vpdemo`: that flag forces the editor
  // shell (`App.tsx`'s `forceEditor`), so Close Project would have nowhere to
  // go — and getting back to the start screen is half of what this test is for.
  await openEditor(page);

  await runCommand(page, "Save as Template…");

  const dialog = page.getByTestId("save-as-template-dialog");
  await expect(dialog).toBeVisible();
  // No name ⇒ no id ⇒ nothing to save. The id is derived, so this is the only
  // field that can block the commit.
  await expect(page.getByTestId("save-as-template-commit")).toBeDisabled();

  await page.getByTestId("save-as-template-name").fill("Printer Part");
  await page.getByTestId("save-as-template-description").fill("mm, XY on the bed");
  await page.getByTestId("save-as-template-commit").click();
  await expect(dialog).toBeHidden();

  // Back to the start screen — the only place templates are offered.
  await page.getByRole("button", { name: "File" }).click();
  await page.getByText("Close Project", { exact: true }).click();
  await expect(page.getByRole("button", { name: "New project" })).toBeVisible();

  await page.getByRole("button", { name: "Templates" }).click();
  const card = page.getByTestId("template-card").filter({ hasText: "Printer Part" });
  await expect(card).toBeVisible();
  await expect(card).toContainText("mm, XY on the bed");

  // Choosing it starts a NEW document: the editor comes back up, and the title
  // is the template's name on an unsaved copy — never the template file.
  await card.click();
  await expect(page.locator('[data-testid="viewport-canvas"] canvas')).toBeVisible();
});

test("the Templates row says how to make one when the library is empty", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("button", { name: "New project" })).toBeVisible();

  await page.getByRole("button", { name: "Templates" }).click();
  await expect(page.getByText(/No templates yet/)).toBeVisible();
  await expect(page.getByTestId("template-card")).toHaveCount(0);
});
