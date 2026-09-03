import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  hideSeedSketches,
  openEditorDebug,
  waitForCameraSettled,
  findFaceOnBody,
  getSketchEntityCount,
  clickAtClient,
} from "./helpers";

/*
 * PROJECT EDGES — the WP-P loop in the mock lane.
 *
 * sketch on a real face → `J` → click a body EDGE → Enter → the projected line
 * is in the session, locked, with provenance → the chrome banner offers Detach →
 * Detach gives the geometry back and the banner goes away.
 *
 * The edge pick is a GENUINE raycast (the engine's own `probePick`), because the
 * flow under test IS "the user clicked an edge of the part" — seeding the store
 * would skip the whole gesture.
 *
 * MOCK FALSE-GREEN RULE (same as `sketch-on-face.spec.ts`): every assertion here
 * is STRUCTURAL — counts, presence, provenance keys, "the banner appeared". The
 * mock's projection is analytic (`mockClient.ts`, `MOCK LIMIT` noted there), not
 * kernel output; numeric truth for a projected edge lives in the cargo gates.
 */

/** A canvas pixel where a body EDGE is what the ray hits (`e:N` on the box). */
async function findEdgeOnBody(page: Page): Promise<{ x: number; y: number; topoKey: string }> {
  let found: { x: number; y: number; topoKey: string } | null = null;
  await expect(async () => {
    found = await page.evaluate(() => {
      const engine = (
        window as unknown as {
          __vpEngine?: {
            probePick(x: number, y: number): { bodyId: string; kind: string; topoKey: string } | null;
          };
        }
      ).__vpEngine;
      const canvas = document.querySelector('[data-testid="viewport-canvas"] canvas') as HTMLCanvasElement | null;
      if (!engine || !canvas) return null;
      const rect = canvas.getBoundingClientRect();
      const step = 5; // finer than the face scan: an edge is a 6px-wide pick band
      for (let y = rect.top + step; y <= rect.bottom - step; y += step) {
        for (let x = rect.left + step; x <= rect.right - step; x += step) {
          const hit = engine.probePick(x, y);
          if (hit && hit.kind === "edge") return { x, y, topoKey: hit.topoKey };
        }
      }
      return null;
    });
    expect(found, "no body edge found under any scanned pixel").not.toBeNull();
  }).toPass({ timeout: 15_000, intervals: [150, 300, 600, 1_000] });
  return found as unknown as { x: number; y: number; topoKey: string };
}

/** Ids of the `referenceLocked` entities in the live session. */
async function lockedEntityIds(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: {
        sketch: { getState(): { session: { entities: Array<{ id: string; referenceLocked?: boolean }> } | null } };
      };
    };
    const session = w.__stores?.sketch.getState().session;
    if (!session) throw new Error("no live sketch session");
    return session.entities.filter((e) => e.referenceLocked).map((e) => e.id);
  });
}

/** Frontend entity ids carrying projected-body provenance (`session.projections`). */
async function projectionKeys(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: {
        sketch: { getState(): { session: { projections?: Record<string, unknown> } | null } };
      };
    };
    return Object.keys(w.__stores?.sketch.getState().session?.projections ?? {});
  });
}

test("project a body edge into a sketch, then detach it", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  // The seeded sketches' static fills sit at the origin, INSIDE the box, and
  // would arbitrate against the face pick that opens the sketch.
  await hideSeedSketches(page);

  // ── (a) sketch on a real face of the box ───────────────────────────────────
  const face = await findFaceOnBody(page);
  await clickAtClient(page, face.x, face.y);
  await page.keyboard.press("s");
  await expect(page.getByText(/^Editing /)).toBeVisible({ timeout: 10_000 });
  await waitForCameraSettled(page);

  const seeded = await lockedEntityIds(page);
  expect(seeded, "the host face's projected boundary opens the session").toHaveLength(4);
  expect(await projectionKeys(page), "the host boundary is NOT a projection").toEqual([]);
  // No projections ⇒ no banner. It is the projections, not the sketch, that
  // earn the second chrome row.
  await expect(page.getByTestId("projection-banner")).toHaveCount(0);

  // ── (b) J arms the tool; a click on a body edge accumulates one pick ───────
  await page.keyboard.press("j");
  await expect(page.getByTestId("status-hint")).toContainText(/Project — click a body edge/);

  const edge = await findEdgeOnBody(page);
  expect(edge.topoKey).toMatch(/^e:\d+$/);
  await clickAtClient(page, edge.x, edge.y);
  await expect(page.getByTestId("status-hint")).toContainText(/Project — 1 picked/);

  // ── (c) Enter projects it — and does NOT finish the sketch ────────────────
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("status-hint")).toContainText(/Projected 1 source/, {
    timeout: 10_000,
  });
  await expect(page.getByText(/^Editing /)).toBeVisible();

  const afterProject = await lockedEntityIds(page);
  expect(afterProject.length, "one projected line joined the locked geometry").toBe(
    seeded.length + 1,
  );
  expect(await getSketchEntityCount(page)).toBe(seeded.length + 1);
  const projected = await projectionKeys(page);
  expect(projected, "the new entity carries projected-body provenance").toHaveLength(1);
  expect(afterProject).toContain(projected[0]);

  // ── (d) the chrome banner appears and offers Detach ────────────────────────
  const banner = page.getByTestId("projection-banner");
  await expect(banner).toBeVisible();
  await expect(banner).toContainText("1 projected entity");
  // Mock bodies never move, so nothing here can go stale — the banner is the
  // NORMAL variant, which is exactly why Detach is not gated behind staleness.
  await expect(banner).toHaveAttribute("data-stale", "false");

  await page.getByTestId("projection-detach").click();

  // ── (e) detach hands the geometry back: provenance gone, banner gone ───────
  await expect.poll(() => projectionKeys(page)).toEqual([]);
  await expect(page.getByTestId("projection-banner")).toHaveCount(0);
  // The LINE stays — detach unlocks a projection, it does not delete it.
  expect(await getSketchEntityCount(page)).toBe(seeded.length + 1);
  expect(await lockedEntityIds(page), "the detached line is no longer locked").toHaveLength(
    seeded.length,
  );
});
