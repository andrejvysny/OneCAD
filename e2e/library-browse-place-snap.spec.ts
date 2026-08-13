import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { findFaceOnBody } from "./helpers";

/*
 * COMPONENT-LIBRARY WP-1.7 — browse → arm → hover → snap ghost → commit →
 * cancel, on the MOCK lane. This is WP-1.5's own manual `playwright-cli`
 * verification (armed card → hover → correctly-oriented ghost → click
 * commits → tree updates → Escape cancels), converted into a repeatable gate.
 *
 * SCOPE NOTE, found while writing this (not assumed from the plan doc):
 * "save → close → reopen → still see the screw with the library folder
 * deleted" (spec §12's flagship claim) is NOT provable in the Playwright
 * MOCK lane, because `mockClient.newDocument()`/`openDocument()` fabricate a
 * fresh synthetic document on every call — there is no in-memory or on-disk
 * persistence for the mock lane to round-trip through, so a `page.reload()`
 * would not carry the placed component forward at all (it would prove
 * nothing, or worse, silently prove something that never happened). That
 * invariant is a REAL-worker/Rust concern and IS covered there:
 * `src-tauri/tests/component_ops.rs::place_component_survives_save_and_a_fresh_worker_reopen`
 * saves, shuts the worker down, spawns a FRESH worker process, reopens, and
 * asserts identical geometry — for the generator-source case P1 ships. What
 * this spec owns instead is the FRONTEND WIRING chain the kernel test never
 * sees: card → armed state → hover → classify → ghost → commit → tree →
 * Escape. `retries: 0`, matching the rest of this suite.
 *
 * KNOWN RESIDUAL (recorded, not fixed here — out of this WP's scope): the
 * embedded-source variant of "reopen without the library folder" — the
 * actual differentiator spec §12 describes (a cached BLOB, not a
 * regenerated-from-scratch generator) — has no automated test yet either.
 * WP-1.3's gate entry in TODO.md shipped generator-source only "matches the
 * worker's WP-1.2 scope"; the embedded-blob authoring-time copy-in path
 * described in the original plan's WP-1.3 section was not built. Flagged for
 * whoever picks up P2/P3.
 */

/** Body names in the projection store (mirrors `project-import.spec.ts`'s helper). */
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

/** `ViewportEngine.previewBodies` size (`?vpdebug`) — >0 while a placement ghost is live. */
function previewBodyCount(page: Page): Promise<number> {
  return page.evaluate(
    () =>
      (window as unknown as { __vpEngine?: { previewBodies?: Map<string, unknown> } }).__vpEngine
        ?.previewBodies?.size ?? 0,
  );
}

test.beforeEach(async ({ page }) => {
  // `?mocklibrary=1` opts the seed catalog in (WP-1.4/1.5's
  // honest-empty-by-default rule); `vpdemo` gives the mock BOX body real
  // geometry to hover — the SHCS fixture's `headSeat` attachment accepts
  // `["plane"]`, so any planar face on the box matches.
  await page.goto("/?vpdebug&vpdemo&mocklibrary=1");
  await expect(page.locator('[data-testid="viewport-canvas"] canvas')).toBeVisible();
});

test("browse, arm, hover-snap, commit — one new named body", async ({ page }) => {
  await page.getByTestId("sidebar-tab-library").click();

  // Scoped by name: the opt-in catalog seeds the whole shipped set (WP-F3), so
  // an unscoped `library-card` locator resolves four elements, not one.
  const card = page.getByTestId("library-card").filter({ hasText: "Socket Head Cap Screw" });
  await expect(card).toBeVisible();

  const before = await bodyNames(page);
  expect(before).toHaveLength(1); // the vpdemo box, nothing placed yet

  await card.click();
  await expect(card).toHaveAttribute("data-armed", "true");

  const face = await findFaceOnBody(page);
  await page.mouse.move(face.x, face.y);
  // Classify → attachment match → PreviewOp round trip is async (mock
  // latency); the ghost appearing IS the signal all three steps succeeded.
  await expect.poll(() => previewBodyCount(page), { timeout: 5_000 }).toBeGreaterThan(0);

  await page.mouse.down();
  await page.mouse.up();

  // Commit tears the gesture down (ghost cleared) and un-arms the card.
  await expect.poll(() => previewBodyCount(page)).toBe(0);
  await expect(card).not.toHaveAttribute("data-armed", "true");

  await expect.poll(() => bodyNames(page)).toHaveLength(2);
  const after = await bodyNames(page);
  expect(after).toContain("Socket Head Cap Screw M6×20");
});

test("Escape cancels an armed-but-uncommitted placement cleanly", async ({ page }) => {
  await page.getByTestId("sidebar-tab-library").click();

  const card = page.getByTestId("library-card").filter({ hasText: "Socket Head Cap Screw" });
  await expect(card).toBeVisible();

  const before = await bodyNames(page);

  await card.click();
  await expect(card).toHaveAttribute("data-armed", "true");

  const face = await findFaceOnBody(page);
  await page.mouse.move(face.x, face.y);
  await expect.poll(() => previewBodyCount(page), { timeout: 5_000 }).toBeGreaterThan(0);

  await page.keyboard.press("Escape");

  await expect(card).not.toHaveAttribute("data-armed", "true");
  await expect.poll(() => previewBodyCount(page)).toBe(0);
  expect(await bodyNames(page)).toEqual(before);
});
