import { test, expect } from "./fixtures";
import { findFaceOnBody, openEditorDebug } from "./helpers";

/**
 * Overlap chooser regression: a real face hit must open the chooser only for a
 * stationary, unmodified secondary click. The mock demo is the normal startup
 * path; this spec deliberately does not inject a second publication.
 */
test("chooses overlapping body geometry without opening on drag-away", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  const hit = await findFaceOnBody(page);

  await page.mouse.click(hit.x, hit.y, { button: "right" });
  const dialog = page.getByRole("dialog", { name: "Select overlapping geometry" });
  await expect(dialog).toBeVisible();

  const filters = dialog.getByRole("button");
  for (const name of ["Body", "Face", "Edge"]) {
    const filter = dialog.getByRole("button", { name, exact: true });
    await expect(filter).toHaveAttribute("aria-pressed", "true");
    await filter.click();
    await expect(filter).toHaveAttribute("aria-pressed", "false");
    await filter.click();
  }
  expect(await filters.count()).toBeGreaterThanOrEqual(3);

  const candidates = dialog.getByRole("list", { name: "Overlapping geometry candidates" }).getByRole("button");
  await expect(candidates.first()).toBeVisible();
  await candidates.first().hover();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect(dialog).toBeHidden();
  await expect.poll(() => page.evaluate(() => {
    const selection = (window as unknown as {
      __stores?: { selection: { getState(): { selected: unknown[] } } };
    }).__stores?.selection.getState().selected ?? [];
    return selection.length;
  })).toBe(1);

  await page.mouse.click(hit.x, hit.y, { button: "right" });
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();

  await page.mouse.move(hit.x, hit.y);
  await page.mouse.down({ button: "right" });
  await page.mouse.move(hit.x + 40, hit.y + 40);
  await page.mouse.move(hit.x, hit.y);
  await page.mouse.up({ button: "right" });
  await expect(dialog).toBeHidden();
});
