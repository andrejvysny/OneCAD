import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditorDebug } from "./helpers";

/*
 * BOOTING already-dark — the path a returning user actually takes, and the one
 * `theme.spec.ts` does not cover: it always starts light and toggles, which
 * fires a resolved-theme CHANGE and therefore runs applyTheme(). A cold boot in
 * dark fires no change at all, so the engine must read the right tokens at
 * construction time or the viewport stays light forever.
 */

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

/** What the DOM actually resolves the canvas token to right now. */
const canvasToken = (page: Page) =>
  page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue("--color-canvas").trim(),
  );

test("cold boot with a persisted Dark preference renders a DARK viewport", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" }); // OS light, app pinned dark
  await page.addInitScript(() => {
    localStorage.setItem(
      "onecad.settings",
      JSON.stringify({ state: { theme: "dark" }, version: 5 }),
    );
  });

  await openEditorDebug(page);

  expect(await page.evaluate(() => document.documentElement.dataset.theme)).toBe("dark");

  const token = await canvasToken(page);
  const engine = await engineTheme(page);

  // The renderer's clear color must equal the DARK canvas token. If the engine
  // sampled the palette before the theme was stamped, this is the light one.
  expect({ token, clearColor: engine.clearColor }).toEqual({
    token: "#23262b",
    clearColor: "#23262b",
  });
});

test("cold boot on a dark OS with the System preference renders a DARK viewport", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.addInitScript(() => {
    localStorage.setItem(
      "onecad.settings",
      JSON.stringify({ state: { theme: "system" }, version: 5 }),
    );
  });

  await openEditorDebug(page);

  expect(await page.evaluate(() => document.documentElement.dataset.theme)).toBe("dark");
  expect((await engineTheme(page)).clearColor).toBe("#23262b");
});

test("a pre-v5 blob on a dark OS still boots dark (migrate → system)", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.addInitScript(() => {
    localStorage.setItem(
      "onecad.settings",
      JSON.stringify({ state: { displayMode: "shadedEdges" }, version: 4 }),
    );
  });

  await openEditorDebug(page);

  expect(await page.evaluate(() => document.documentElement.dataset.theme)).toBe("dark");
  expect((await engineTheme(page)).clearColor).toBe("#23262b");
});
