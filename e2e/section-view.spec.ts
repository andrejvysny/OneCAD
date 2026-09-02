import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  findFaceOnBody,
  hideSeedSketches,
  openEditorDebug,
  waitForCameraSettled,
} from "./helpers";

/*
 * Section view (capped clipping), end to end in the mock lane.
 *
 * There is no per-body DOM and a cut is a GPU-side fact, so everything is
 * asserted through `?vpdebug`'s `window.__vpEngine.debugSnapshot().section`:
 *
 *   clippedMaterials — counted off the SCENE, so it proves the clip reached the
 *                      COMMITTED bodies' material library (the one the engine
 *                      cannot see, driven by ViewportRoot), not just a flag,
 *   capVisible       — the stencil pairs exist and the cut cap is being drawn.
 *
 * Picking is checked through the engine's own `probePick` plus the selection
 * store, because "you can click the face that was cut away" is invisible to any
 * screenshot and is exactly what the whole feature would get wrong.
 */

interface SectionSnapshot {
  enabled: boolean;
  plane: string | null;
  offsetMm: number;
  flip: boolean;
  clippedMaterials: number;
  /** Clipped materials in `interactionRoot` — the selection/hover overlays. */
  clippedOverlays: number;
  capVisible: boolean;
}

/** The `?vpdemo=cyl` bushing, published at x = 80 (mockClient DEMO_BORE_ORIGIN). */
const BUSHING = "body_demo_bore";

interface ProbeHit {
  bodyId: string;
  kind: string;
  topoKey: string;
  worldPos: { x: number; y: number; z: number };
}

function section(page: Page): Promise<SectionSnapshot> {
  return page.evaluate(
    () =>
      (
        window as unknown as {
          __vpEngine?: { debugSnapshot(): { section: SectionSnapshot } };
        }
      ).__vpEngine!.debugSnapshot().section,
  );
}

/** The engine's OWN pick at a client pixel — the same raycast a click runs. */
function probeAt(page: Page, x: number, y: number): Promise<ProbeHit | null> {
  return page.evaluate(
    ([px, py]) =>
      (
        window as unknown as { __vpEngine?: { probePick(x: number, y: number): ProbeHit | null } }
      ).__vpEngine!.probePick(px as number, py as number),
    [x, y],
  );
}

/** The engine's scene bounds (`?vpdebug`) — what the offset seeding measures. */
function sceneBounds(page: Page): Promise<{ min: number[]; max: number[] }> {
  return page.evaluate(() => {
    const snap = (
      window as unknown as {
        __vpEngine?: { debugSnapshot(): { bounds: { min: number[]; max: number[] } | null } };
      }
    ).__vpEngine!.debugSnapshot();
    if (!snap.bounds) throw new Error("no scene bounds — no bodies loaded");
    return snap.bounds;
  });
}

/** Does `bodyId` still have ANY pickable face on screen? */
async function bodyIsPickable(page: Page, bodyId: string): Promise<boolean> {
  return page.evaluate((want) => {
    const engine = (
      window as unknown as { __vpEngine?: { probePick(x: number, y: number): ProbeHit | null } }
    ).__vpEngine;
    const canvas = document.querySelector(
      '[data-testid="viewport-canvas"] canvas',
    ) as HTMLCanvasElement | null;
    if (!engine || !canvas) return false;
    const rect = canvas.getBoundingClientRect();
    const step = 8;
    for (let y = rect.top + step; y <= rect.bottom - step; y += step) {
      for (let x = rect.left + step; x <= rect.right - step; x += step) {
        const hit = engine.probePick(x, y);
        if (hit && hit.kind === "face" && hit.bodyId === want) return true;
      }
    }
    return false;
  }, bodyId);
}

/** Drive the section offset through the store (the slider's own action). */
function setOffset(page: Page, mm: number): Promise<void> {
  return page.evaluate((v) => {
    (
      window as unknown as {
        __stores?: { viewport: { getState(): { setSectionOffset(mm: number): void } } };
      }
    ).__stores?.viewport.getState().setSectionOffset(v);
  }, mm);
}

/** Rendered frame counter (`?vpdebug`), for the idle-zero-frames contract. */
function frames(page: Page): Promise<number> {
  return page.evaluate(() => (window as unknown as { __vpFrames?: number }).__vpFrames ?? 0);
}

/** The live selection's first entry (`__stores.selection`, dev-only). */
function selectedTopoKey(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { selection: { getState(): { selected: Array<{ topoKey?: string }> } } };
    };
    return w.__stores?.selection.getState().selected[0]?.topoKey ?? null;
  });
}

/**
 * A canvas pixel where the LIVE cut has exposed interior geometry — the pick
 * lands comfortably below the world XY plane, i.e. on the half the unflipped
 * section at 0 keeps.
 *
 * Scanned with the engine's own raycast rather than computed, because the exact
 * pixel depends on the auto-fit camera.
 */
async function pixelOnKeptHalf(page: Page): Promise<{ x: number; y: number; topoKey: string }> {
  let found: { x: number; y: number; topoKey: string } | null = null;
  await expect(async () => {
    found = await page.evaluate(() => {
      const engine = (
        window as unknown as {
          __vpEngine?: { probePick(x: number, y: number): ProbeHit | null };
        }
      ).__vpEngine;
      const canvas = document.querySelector(
        '[data-testid="viewport-canvas"] canvas',
      ) as HTMLCanvasElement | null;
      if (!engine || !canvas) return null;
      const rect = canvas.getBoundingClientRect();
      const step = 8;
      for (let y = rect.top + step; y <= rect.bottom - step; y += step) {
        for (let x = rect.left + step; x <= rect.right - step; x += step) {
          const hit = engine.probePick(x, y);
          // A margin off the cut plane, so float wobble at the boundary cannot
          // decide the test.
          if (hit && hit.kind === "face" && hit.worldPos.z < -1) {
            return { x, y, topoKey: hit.topoKey };
          }
        }
      }
      return null;
    });
    expect(found, "no face found below the cut plane").not.toBeNull();
  }).toPass({ timeout: 15_000, intervals: [150, 300, 600, 1_000] });
  return found as unknown as { x: number; y: number; topoKey: string };
}

test("⇧X cuts the committed bodies and caps the cut; Esc puts them back", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await waitForCameraSettled(page);

  expect(await section(page)).toMatchObject({
    enabled: false,
    clippedMaterials: 0,
    capVisible: false,
  });

  await page.keyboard.press("Shift+X");

  await expect.poll(() => section(page)).toMatchObject({
    enabled: true,
    plane: "XY",
    offsetMm: 0,
    flip: false,
    capVisible: true,
  });
  // The fan-out reached the COMMITTED bodies (face + edge materials), which is
  // the half of the wiring the engine cannot do for itself.
  expect((await section(page)).clippedMaterials).toBeGreaterThan(0);
  await expect(page.getByText("Section view on — Esc or ⇧X to exit")).toBeVisible();

  // The stencil buffer the cap needs did not cost us the preserved drawing
  // buffer the save thumbnail reads back (renderer.ts context attributes).
  const thumbnail = await page.evaluate(
    () =>
      (window as unknown as { __vpEngine?: { captureThumbnail(px: number): string | null } })
        .__vpEngine!.captureThumbnail(256),
  );
  expect(thumbnail?.slice(0, 15)).toBe("data:image/png;");

  await page.keyboard.press("Escape");
  await expect.poll(() => section(page)).toMatchObject({
    enabled: false,
    clippedMaterials: 0,
    capVisible: false,
  });
  await expect(page.getByText("Section view on — Esc or ⇧X to exit")).toHaveCount(0);
});

test("the NavPill button and the Layers controls drive the one cut", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await waitForCameraSettled(page);

  const button = page.getByTestId("nav-section");
  await expect(button).toHaveAttribute("aria-pressed", "false");
  await button.click();
  await expect(button).toHaveAttribute("aria-pressed", "true");
  await expect.poll(() => section(page)).toMatchObject({ enabled: true, capVisible: true });

  // Plane, flip and offset live in the Layers menu — the same state, a third
  // entry point (⇧X and the button being the other two).
  await page.getByRole("button", { name: "Viewport layers" }).click();
  await expect(page.getByTestId("layers-menu")).toBeVisible();
  await expect(page.getByTestId("section-toggle")).toHaveAttribute("aria-checked", "true");

  await page.getByTestId("section-plane-yz").click();
  await expect.poll(() => section(page)).toMatchObject({ plane: "YZ", offsetMm: 0 });

  await page.getByTestId("section-flip").click();
  await expect.poll(() => section(page)).toMatchObject({ plane: "YZ", flip: true });

  // The slider spans the scene along the plane's axis (the demo box is 80 wide
  // in X, so ±40 for YZ) — a mid-range offset must reach the engine.
  await page.getByTestId("section-offset").fill("10");
  await expect.poll(() => section(page)).toMatchObject({ plane: "YZ", offsetMm: 10 });
  // Still cutting: moving the plane never silently disables it.
  expect((await section(page)).capVisible).toBe(true);

  await page.getByTestId("section-toggle").click();
  await expect.poll(() => section(page)).toMatchObject({ enabled: false, clippedMaterials: 0 });
  await expect(button).toHaveAttribute("aria-pressed", "false");
});

test("clicking through the cut selects the interior it exposed, not the removed face", async ({
  page,
}) => {
  await openEditorDebug(page, { mockBody: true });
  await waitForCameraSettled(page);
  // A seed sketch lying on the body would win the pick over a face behind it
  // (`refFromModelHits`), which has nothing to do with clipping.
  await hideSeedSketches(page);

  await page.keyboard.press("Shift+X");
  await expect.poll(() => section(page)).toMatchObject({ enabled: true, capVisible: true });
  const exposed = await pixelOnKeptHalf(page);

  // The SAME pixel with the cut off resolves to a different face, above the
  // plane — that is the one the cut removes, and it must never win once the
  // section is on. (Without the Picker's clipped-hit filter it always would:
  // it is the nearest hit along the ray.)
  await page.keyboard.press("Escape");
  await expect.poll(() => section(page)).toMatchObject({ enabled: false });
  const removed = await probeAt(page, exposed.x, exposed.y);
  expect(removed?.kind).toBe("face");
  expect(removed!.topoKey).not.toBe(exposed.topoKey);
  expect(removed!.worldPos.z).toBeGreaterThan(0);

  // Cut back on, and a REAL click: the selection store gets the exposed face.
  await page.keyboard.press("Shift+X");
  await expect.poll(() => section(page)).toMatchObject({ enabled: true });
  await page.mouse.click(exposed.x, exposed.y);
  await expect.poll(() => selectedTopoKey(page)).toBe(exposed.topoKey);
});

test("an idle section view renders zero frames", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await waitForCameraSettled(page);

  await page.keyboard.press("Shift+X");
  await expect.poll(() => section(page)).toMatchObject({ enabled: true });

  // Let the cut's own repaint land, then hold still: on-demand rendering means
  // an enabled section must schedule nothing further (the per-frame stencil
  // reconcile runs INSIDE a frame, it must never request one).
  await page.waitForTimeout(400);
  const before = await frames(page);
  await page.waitForTimeout(800);
  expect(await frames(page)).toBe(before);
});

/*
 * The cut is seeded from the SCENE, not from the origin.
 *
 * `SECTION_DEFAULT.offsetMm` is 0, and the unflipped cut keeps `axis · p <= offset`
 * — so on a model that does not straddle the origin (a sketch-on-XY extrude
 * occupies z ∈ [0, depth], the most ordinary shape there is) a cut at 0 discards
 * EVERYTHING: empty viewport, no cross-section, therefore no cap to explain it,
 * and only the slider to get back. Seeding the offset to the middle of the scene
 * makes the plane land inside the model whatever the model's position is.
 *
 * `?vpdemo=cyl` puts the box (x ∈ [-40, 40]) beside the bushing (x ∈ [60, 100]),
 * so the YZ scene span is asymmetric and its middle is emphatically not 0 —
 * which is what makes the seeded value observable at all. NOTE the guarantee
 * being asserted is scene-level ("the plane always cuts the model"), not
 * body-level: no single plane can intersect every body of a spread-out assembly.
 */
test("the cut seeds to the middle of the scene, so enabling it never erases the model", async ({
  page,
}) => {
  await page.goto("/?vpdebug&vpdemo=cyl");
  await expect(page.locator('[data-testid="viewport-canvas"] canvas')).toBeVisible();
  await waitForCameraSettled(page);
  await expect.poll(() => bodyIsPickable(page, BUSHING)).toBe(true);

  const bounds = await sceneBounds(page);
  const midX = (bounds.min[0] + bounds.max[0]) / 2;
  expect(midX).not.toBe(0); // the fixture has to be asymmetric or this proves nothing

  await page.keyboard.press("Shift+X");
  await page.getByRole("button", { name: "Viewport layers" }).click();
  await page.getByTestId("section-plane-yz").click();

  // The seeded offset IS the middle of the scene along the plane's axis. Before
  // the fix this read 0, which is outside a great many real models.
  await expect.poll(() => section(page)).toMatchObject({ plane: "YZ", offsetMm: midX });
  await expect.poll(() => section(page)).toMatchObject({ capVisible: true });
  expect(await bodyIsPickable(page, "body1")).toBe(true);

  // The counterfactual — a plane parked OFF the model, which is exactly what a
  // hardcoded 0 is for a part that sits above its sketch plane. Everything goes,
  // and there is no cap to say why.
  await setOffset(page, bounds.min[0] - 10);
  await expect.poll(() => section(page)).toMatchObject({ offsetMm: bounds.min[0] - 10 });
  // Nothing left to click anywhere on the canvas. (`capVisible` deliberately
  // stays true here: it reports that the cap is BEING DRAWN, and it is — over a
  // stencil that now sums to zero everywhere. Only picking can tell the
  // difference between "capped" and "empty".)
  expect(await bodyIsPickable(page, "body1")).toBe(false);
  expect(await bodyIsPickable(page, BUSHING)).toBe(false);

  // …and back to the seeded plane, the model returns.
  await setOffset(page, midX);
  await expect.poll(() => section(page)).toMatchObject({ capVisible: true });
  expect(await bodyIsPickable(page, "body1")).toBe(true);
});

test("a selection made BEFORE the cut is clipped with it, not left floating", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await waitForCameraSettled(page);
  await hideSeedSketches(page);

  // Select a face while the model is whole. The highlight overlays are
  // depthTest:false, so an unclipped one paints over the empty half.
  const face = await findFaceOnBody(page);
  await page.mouse.click(face.x, face.y);
  await expect.poll(() => selectedTopoKey(page)).toBe(face.topoKey);
  expect((await section(page)).clippedOverlays).toBe(0);

  await page.keyboard.press("Shift+X");

  await expect
    .poll(async () => (await section(page)).clippedOverlays)
    .toBeGreaterThan(0);

  await page.keyboard.press("Escape");
  await expect.poll(async () => (await section(page)).clippedOverlays).toBe(0);
});
