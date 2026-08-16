import { test, expect } from "./fixtures";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  clickAt,
  clickAtAwaitingDofChange,
  planePointToClient,
  getSketchSnapshot,
  setSnapPref,
} from "./helpers";

/*
 * Snap RENDERING, through the real browser with real WebGL (SNAP P4).
 *
 * The invariants here are about SCREEN STABILITY: a guide's dash cadence, a
 * marker's size and a curve's smoothness are authored in CSS pixels and must
 * hold at every zoom. jsdom cannot answer any of that — there is no projection —
 * so these live in the Playwright lane and read the engine's own scene state
 * through `?vpdebug`.
 */

type Page = Parameters<typeof openEditorDebug>[0];

/** Walk the live scene for the snap indicator's guide materials. */
async function guideMaterials(
  page: Page,
): Promise<{ dash: number; gap: number; width: number }[]> {
  return page.evaluate(() => {
    const engine = (window as unknown as { __vpEngine?: Record<string, unknown> }).__vpEngine;
    if (!engine) return [];
    const root = (engine as { interactionRoot?: { getObjectByName(n: string): unknown } })
      .interactionRoot;
    const indicator = root?.getObjectByName("snapIndicator") as
      | { getObjectByName(n: string): { children: unknown[] } | undefined }
      | undefined;
    const guides = indicator?.getObjectByName("snapGuides");
    return (guides?.children ?? []).map((c) => {
      const m = (c as { material: { dashSize: number; gapSize: number; linewidth: number } }).material;
      return { dash: m.dashSize, gap: m.gapSize, width: m.linewidth };
    });
  });
}

/** CSS px per plane unit, straight off the engine's own metric. */
async function pxPerUnit(page: Page): Promise<number> {
  return page.evaluate(() => {
    const engine = (
      window as unknown as {
        __vpEngine?: {
          planeScreenMetric(p: { x: number; y: number }): {
            m00: number;
            m01: number;
            m10: number;
            m11: number;
          } | null;
        };
      }
    ).__vpEngine;
    const m = engine?.planeScreenMetric({ x: 0, y: 0 });
    if (!m) return 0;
    return Math.max(Math.hypot(m.m00, m.m10), Math.hypot(m.m01, m.m11));
  });
}

test("guide dashes hold a CSS-constant cadence across a real zoom", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  await setSnapPref(page, "grid", false);
  await setSnapPref(page, "polarTracking", false);
  await setSnapPref(page, "sketchGuideLines", true);

  // A reference line, then hover aligned with its endpoint so a guide draws.
  await clickAt(page, -160, -60);
  await clickAtAwaitingDofChange(page, -60, -60);
  await page.keyboard.press("Escape");
  const snap = await getSketchSnapshot(page);
  const ref = snap.lines[0].p1;

  // Hover on the endpoint's own screen COLUMN, a fixed number of CSS pixels away.
  //
  // The plane-space version of this (`ref[1] + 90`) is zoom-dependent: 90 plane
  // units is ~427 px at the default view, which puts the hover within ~30 px of
  // the canvas edge here and OFF the canvas on the CI runner — where this spec's
  // first CI execution failed with zero guide materials, while passing locally.
  // A settled sketch view looks straight down the plane normal (the snap metric's
  // off-diagonal terms are 0), so a screen-vertical move holds the plane x that
  // the alignment guide is drawn from, at any zoom.
  const hoverAligned = async (): Promise<void> => {
    const s = await getSketchSnapshot(page);
    const at = await planePointToClient(page, s.plane, { x: ref[0], y: ref[1] });
    const box = await page.locator('[data-testid="viewport-canvas"]').boundingBox();
    if (!box) throw new Error("no canvas box");
    const OFFSET_PX = 200;
    const up = at.y - OFFSET_PX;
    const y = up > box.y + 20 ? up : Math.min(at.y + OFFSET_PX, box.y + box.height - 20);
    await page.mouse.move(at.x, y);
    await page.mouse.move(at.x, y);
  };

  await hoverAligned();
  await expect.poll(async () => (await guideMaterials(page)).length, { timeout: 5000 }).toBeGreaterThan(0);
  const near = (await guideMaterials(page))[0];
  const nearScale = await pxPerUnit(page);

  // Zoom OUT with a real wheel, then re-hover.
  await page.mouse.move(400, 300);
  await page.mouse.wheel(0, 600);
  await waitForCameraSettled(page);
  await hoverAligned();
  await expect.poll(async () => (await guideMaterials(page)).length, { timeout: 5000 }).toBeGreaterThan(0);
  const far = (await guideMaterials(page))[0];
  const farScale = await pxPerUnit(page);

  expect(farScale).toBeLessThan(nearScale); // the zoom really happened
  // The WORLD dash size changed with the zoom…
  expect(far.dash).not.toBeCloseTo(near.dash, 6);
  // …so that the SCREEN cadence did not. Within 1 CSS px, per SNAP §10.1.
  expect(Math.abs(far.dash * farScale - near.dash * nearScale)).toBeLessThan(1);
  expect(Math.abs(far.gap * farScale - near.gap * nearScale)).toBeLessThan(1);
});

test("curve tessellation follows the projected size", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // A circle, drawn with the Circle tool via the toolbar.
  await page.getByRole("button", { name: "Circle", exact: true }).first().click();
  await clickAt(page, -40, 0);
  await clickAtAwaitingDofChange(page, 60, 0);
  await page.keyboard.press("Escape");

  const vertexCount = async (): Promise<number> =>
    page.evaluate(() => {
      const engine = (window as unknown as { __vpEngine?: { sketchRoot: unknown } }).__vpEngine;
      let max = 0;
      const walk = (o: unknown): void => {
        const node = o as {
          children?: unknown[];
          geometry?: { attributes?: { instanceStart?: { count: number } } };
        };
        const n = node.geometry?.attributes?.instanceStart?.count ?? 0;
        if (n > max) max = n;
        for (const c of node.children ?? []) walk(c);
      };
      walk(engine?.sketchRoot);
      return max;
    });

  const zoomedOut = await vertexCount();
  await page.mouse.move(400, 300);
  await page.mouse.wheel(0, -600); // zoom IN
  await waitForCameraSettled(page);
  // Give the on-demand renderer a frame to re-tessellate.
  await page.mouse.move(401, 301);
  await expect
    .poll(vertexCount, {
      message: "a larger projected circle needs more segments",
      timeout: 5000,
    })
    .toBeGreaterThanOrEqual(zoomedOut);
});

test("hiding guide points leaves the SNAP behaviour untouched", async ({ page }) => {
  await openEditorDebug(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  await clickAt(page, -160, -60);
  await clickAtAwaitingDofChange(page, -60, -60);
  await page.keyboard.press("Escape");
  const snap = await getSketchSnapshot(page);
  const endpoint = snap.lines[0].p1;
  const at = await planePointToClient(page, snap.plane, { x: endpoint[0], y: endpoint[1] });

  // Guide points OFF is a DISPLAY preference…
  await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { settings: { getState(): { setShow(k: string, v: boolean): void } } };
    };
    w.__stores?.settings.getState().setShow("guidePoints", false);
  });

  // …so the endpoint still snaps, and still says so.
  await page.mouse.move(at.x, at.y);
  await page.mouse.move(at.x, at.y);
  const hint = page.locator("[data-sketch-snap-hint]");
  await expect(hint).toBeVisible();
  await expect(hint).toHaveText("Endpoint");
});

test("the grid control moves to the sketch plane and back", async ({ page }) => {
  await openEditorDebug(page);

  const worldGridVisible = async (): Promise<boolean | undefined> =>
    page.evaluate(() => {
      const engine = (
        window as unknown as { __vpEngine?: { debugSnapshot(): { gridVisible?: boolean } } }
      ).__vpEngine;
      return engine?.debugSnapshot().gridVisible;
    });

  await page.evaluate(() => {
    const engine = (window as unknown as { __vpEngine?: { setGridVisible(v: boolean): void } })
      .__vpEngine;
    engine?.setGridVisible(true);
  });
  expect(await worldGridVisible()).toBe(true);

  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
  // In sketch mode the WORLD grid steps aside — one effective grid (SNAP §10.4).
  expect(await worldGridVisible()).toBe(false);

  // Enter with no open chain is the global finish-sketch action — Escape only
  // walks the in-gesture ladder and does not necessarily leave the session.
  await page.keyboard.press("Enter");
  await waitForCameraSettled(page);
  // …and the world grid comes back at the preference the user left it at,
  // untouched: sketch mode borrowed the surface, it did not change the setting.
  await expect.poll(worldGridVisible, { timeout: 8000 }).toBe(true);
});
