import { test, expect } from "./fixtures";

/*
 * COMPONENT-LIBRARY WP-B6 — component previews in the browser UI.
 *
 * WHAT THIS ASSERTS, and why it is shaped this way: a card must ALWAYS show
 * something. The mock lane has no kernel, and the Chromium used here runs on
 * SwiftShader, so whether a given machine actually rasterizes a thumbnail is not
 * a property worth pinning — but "the card renders its picture OR its icon,
 * never a hole" is, because that is the difference between a degraded preview
 * and a broken grid. The detail preview is asserted the same way: it mounts and
 * reports a state, `ready` or `unavailable`, rather than sitting blank.
 *
 * The GEOMETRY half is a real-worker concern and lives there:
 * `src-tauri/tests/component_ops.rs::every_seeded_component_meshes_for_the_library_ui`
 * drives the same command over the real kernel and asserts a MESH1 with
 * vertices comes back for every seeded component.
 *
 * `retries: 0`, matching the rest of this suite.
 */

test("every library card shows a picture or its fallback icon — never a hole", async ({ page }) => {
  await page.goto("/?vpdebug&vpdemo&mocklibrary=1");
  await expect(page.locator('[data-testid="viewport-canvas"] canvas')).toBeVisible();

  await page.getByRole("button", { name: "Library" }).click();
  const card = page.getByTestId("library-modal-card").first();
  await expect(card).toBeVisible();

  const picture = card.getByTestId("component-thumbnail");
  const fallback = card.getByTestId("component-thumbnail-fallback");
  await expect
    .poll(async () => (await picture.count()) + (await fallback.count()))
    .toBeGreaterThan(0);
});

test("the start-screen browser opens a component's detail preview", async ({ page }) => {
  // `?mocklibrary=1` opts the fixture catalog in; the start screen's library
  // browser is reachable with no project open, which is the whole point of
  // previewing there.
  await page.goto("/?mocklibrary=1");
  await expect(page.getByRole("button", { name: "New project" })).toBeVisible();

  await page.getByRole("button", { name: "Library" }).click();
  const card = page.getByTestId("start-library-card").first();
  await expect(card).toBeVisible();

  await card.click();
  const preview = page.getByTestId("component-preview-3d");
  await expect(preview).toBeVisible();
  // It resolves one way or the other; a preview stuck on "loading" would mean
  // the request never settled.
  await expect
    .poll(async () => await preview.getAttribute("data-state"), { timeout: 10_000 })
    .not.toBe("loading");
});
