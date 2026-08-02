import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditorDebug, waitForCameraSettled } from "./helpers";

/*
 * Dark mode, end to end in the mock lane.
 *
 * The load-bearing assertion is that the 3D VIEWPORT re-themes, not just the
 * chrome. CSS re-themes itself for free; the engine caches THREE.Color and
 * builds most materials once, so it is the half that can silently stay light.
 * Following the view-ux doctrine, that is asserted against the live engine via
 * `?vpdebug` — `debugSnapshot()` reports the renderer's actual clear color.
 */

/** The engine's own view of the theme + renderer clear color. */
function engineTheme(page: Page): Promise<{ theme: string; clearColor: string }> {
  return page.evaluate(() => {
    const engine = (
      window as unknown as {
        __vpEngine?: { debugSnapshot(): { theme: string; clearColor: string } };
      }
    ).__vpEngine;
    const snap = engine?.debugSnapshot();
    return { theme: snap?.theme ?? "?", clearColor: snap?.clearColor ?? "?" };
  });
}

const rootTheme = (page: Page) =>
  page.evaluate(() => document.documentElement.dataset.theme ?? "unset");

const bodyBackground = (page: Page) =>
  page.evaluate(() => getComputedStyle(document.body).backgroundColor);

/**
 * Open the display popover and pick an appearance.
 *
 * The camera settle is not optional: the ViewCube re-renders on every camera
 * change and the popover lives in the same subtree, so clicking a tab while the
 * intro framing is still animating detaches the node mid-click.
 */
async function pickTheme(page: Page, name: "Light" | "Dark" | "System"): Promise<void> {
  await waitForCameraSettled(page);
  await page.getByRole("button", { name: /^Display mode/ }).click();
  const tab = page.getByRole("tab", { name });
  await expect(tab).toBeVisible();
  await tab.click();
  await page.keyboard.press("Escape");
}

test("picking Dark re-themes the chrome AND the 3D viewport", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await openEditorDebug(page);

  await expect.poll(() => rootTheme(page)).toBe("light");
  const lightBg = await bodyBackground(page);
  const lightEngine = await engineTheme(page);
  expect(lightEngine.theme).toBe("light");

  await pickTheme(page, "Dark");

  await expect.poll(() => rootTheme(page)).toBe("dark");
  expect(await bodyBackground(page)).not.toBe(lightBg);

  // The renderer's clear color must actually have moved — this is what proves
  // resetPaletteCache() + applyTheme() ran, rather than only the CSS flipping.
  await expect.poll(async () => (await engineTheme(page)).theme).toBe("dark");
  const darkEngine = await engineTheme(page);
  expect(darkEngine.clearColor).not.toBe(lightEngine.clearColor);
});

test("the appearance choice survives a reload", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await openEditorDebug(page);
  await pickTheme(page, "Dark");
  await expect.poll(() => rootTheme(page)).toBe("dark");

  await page.reload();

  // Stamped before first paint by the inline no-FOUC script in index.html,
  // so this holds even before the module bundle has executed.
  await expect.poll(() => rootTheme(page)).toBe("dark");
});

test("System follows the OS appearance, in both directions", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await openEditorDebug(page);
  await pickTheme(page, "System");
  await expect.poll(() => rootTheme(page)).toBe("dark");

  await page.emulateMedia({ colorScheme: "light" });
  await expect.poll(() => rootTheme(page)).toBe("light");
  await expect.poll(async () => (await engineTheme(page)).theme).toBe("light");

  await page.emulateMedia({ colorScheme: "dark" });
  await expect.poll(() => rootTheme(page)).toBe("dark");
});

test("an explicit choice ignores the OS", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await openEditorDebug(page);

  await pickTheme(page, "Light");
  await expect.poll(() => rootTheme(page)).toBe("light");

  // OS flips away and back; the explicit preference must not budge.
  await page.emulateMedia({ colorScheme: "light" });
  await expect.poll(() => rootTheme(page)).toBe("light");
  await page.emulateMedia({ colorScheme: "dark" });
  await expect.poll(() => rootTheme(page)).toBe("light");
});

test("the appearance control reports the current preference when reopened", async ({ page }) => {
  await openEditorDebug(page);

  await pickTheme(page, "Dark");
  await page.getByRole("button", { name: /^Display mode/ }).click();
  await expect(page.getByRole("tab", { name: "Dark" })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("tab", { name: "System" })).toHaveAttribute("aria-selected", "false");
});

test("the title-bar toggle cycles the theme and re-themes the viewport", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await openEditorDebug(page);
  await waitForCameraSettled(page);

  // Fresh install defaults to System, which on a light OS resolves light.
  const toggle = page.getByRole("button", { name: /^Appearance:/ });
  await expect(toggle).toHaveAccessibleName("Appearance: System");
  const lightClear = (await engineTheme(page)).clearColor;

  // System → Light (still light), then Light → Dark.
  await toggle.click();
  await expect(toggle).toHaveAccessibleName("Appearance: Light");
  await expect.poll(() => rootTheme(page)).toBe("light");

  await toggle.click();
  await expect(toggle).toHaveAccessibleName("Appearance: Dark");
  await expect.poll(() => rootTheme(page)).toBe("dark");

  // The button must drive the ENGINE, not just the store.
  await expect.poll(async () => (await engineTheme(page)).theme).toBe("dark");
  expect((await engineTheme(page)).clearColor).not.toBe(lightClear);
});

test("the title-bar toggle and the popover control stay in step", async ({ page }) => {
  await openEditorDebug(page);

  await pickTheme(page, "Dark");
  await expect(page.getByRole("button", { name: /^Appearance:/ })).toHaveAccessibleName(
    "Appearance: Dark",
  );

  // ...and back the other way: the toggle's write shows up in the popover.
  await page.getByRole("button", { name: /^Appearance:/ }).click(); // Dark → System
  await page.getByRole("button", { name: /^Display mode/ }).click();
  await expect(page.getByRole("tab", { name: "System" })).toHaveAttribute("aria-selected", "true");
});

test("theme and display mode are independent", async ({ page }) => {
  await openEditorDebug(page);

  const button = page.getByRole("button", { name: /^Display mode/ });
  await button.click();
  await page.getByRole("menuitemradio", { name: "Wireframe" }).click();
  await expect(button).toHaveAccessibleName("Display mode: Wireframe");

  await pickTheme(page, "Dark");

  // Wireframe is the mode dark mode would break if --color-body-edge had not
  // been inverted: edges draw with no faces behind them.
  await expect(button).toHaveAccessibleName("Display mode: Wireframe");
  await expect.poll(() => rootTheme(page)).toBe("dark");
});
