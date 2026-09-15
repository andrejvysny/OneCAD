/*
 * T8 — body EDGES must be hoverable and clickable, at every camera angle.
 *
 * The review's second cold-start session aimed five clicks at three different
 * silhouette edges to 1–3 px accuracy and selected the adjoining FACE every
 * time, with no hover pre-highlight to aim by; a first session at a different
 * camera had picked an edge on the first attempt. So the interesting assertion
 * is not "some pixel somewhere picks an edge" — a scan would always find one —
 * but "the pixel the edge is DRAWN at picks the edge", from more than one
 * camera. Every point below is therefore computed from the mock box's known
 * geometry through the engine's own `projectPoint`, the same ground truth
 * `filletChamfer.spec.ts` drives its direction gestures from.
 *
 * The session-32 inverse ("left-click on a body top face selected nothing, edge
 * click did") is the last case: a click well inside a face must select that
 * face, never an edge that happens to be near in world units.
 */
import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditorDebug, hideSeedSketches, waitForCameraSettled, CANVAS } from "./helpers";
import { findFaceOnBody } from "./helpers";
import { clickAt } from "./modelToolHelpers";

/** Mock box (80×60×30, origin-centred) midpoints, straight off `BOX_EDGE_PAIRS`. */
const EDGES: Record<string, [number, number, number]> = {
  // +x/+z top edge — the one the fillet spec arms on.
  "e:5": [40, 0, 15],
  // the vertical corner edge at (+x,+y).
  "e:10": [40, 30, 0],
};

interface Ref {
  kind: string;
  bodyId?: string;
  topoKey?: string;
}

async function selection(page: Page): Promise<Ref[]> {
  return page.evaluate(() => {
    const store = (window as unknown as { __stores?: { selection: { getState(): { selected: Ref[] } } } })
      .__stores?.selection;
    return store ? store.getState().selected : [];
  }) as Promise<Ref[]>;
}

async function hovered(page: Page): Promise<Ref | null> {
  return page.evaluate(() => {
    const store = (window as unknown as { __stores?: { selection: { getState(): { hover: Ref | null } } } })
      .__stores?.selection;
    return store ? store.getState().hover : null;
  }) as Promise<Ref | null>;
}

/**
 * Where the engine currently draws `world`, in CLIENT px — or null when it is
 * behind the camera or buried under the chrome.
 *
 * `projectPoint` answers in CANVAS-relative pixels, so the canvas rect's origin
 * has to be added back before the coordinate means anything to `page.mouse`.
 * (`filletChamfer.spec.ts` gets away without it because it only ever takes a
 * DIFFERENCE of two projected points, which the offset cancels out of.)
 */
async function onScreen(page: Page, world: [number, number, number]) {
  return page.evaluate((w) => {
    const engine = (
      window as unknown as { __vpEngine?: { projectPoint(p: number[]): { x: number; y: number } | null } }
    ).__vpEngine;
    if (!engine) throw new Error("__vpEngine unavailable (?vpdebug only)");
    const p = engine.projectPoint(w);
    const canvas = document.querySelector(
      '[data-testid="viewport-canvas"] canvas',
    ) as HTMLCanvasElement | null;
    if (!p || !canvas) return null;
    const rect = canvas.getBoundingClientRect();
    const at = { x: rect.left + p.x, y: rect.top + p.y };
    // The history rail and the inspector float OVER the canvas: a click computed
    // from a world point can land under one and never reach the viewport at all.
    return document.elementFromPoint(at.x, at.y) === canvas ? at : null;
  }, world);
}

/**
 * Orbit without touching the viewport with the left button (which would select).
 * Shift + wheel under the trackpad override is the orbit gesture
 * `navigation.spec.ts` covers; the preference store rehydrates asynchronously,
 * so the write has to wait for hydration or it is silently overwritten.
 */
async function orbit(page: Page, deltaY: number): Promise<void> {
  await page.waitForFunction(() => {
    const st = (window as unknown as { __stores?: { settings?: { persist?: { hasHydrated(): boolean } } } })
      .__stores?.settings;
    return !!st && (!st.persist || st.persist.hasHydrated());
  });
  await page.evaluate(() => {
    const st = (window as unknown as {
      __stores: { settings: { getState(): { setInputDevice(p: string): void } } };
    }).__stores.settings;
    st.getState().setInputDevice("trackpad");
  });
  const box = await page.locator(CANVAS).boundingBox();
  if (!box) throw new Error("no canvas box");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.keyboard.down("Shift");
  await page.mouse.wheel(0, deltaY);
  await page.keyboard.up("Shift");
  await waitForCameraSettled(page);
}

test.describe("body edge picking (T8)", () => {
  test.beforeEach(async ({ page }) => {
    // `?vpdemo` drives the mock box through the real `onDocumentChanged` ingest,
    // so there is genuine MESH1 geometry — faces AND fat edge lines — to raycast.
    await openEditorDebug(page, { mockBody: true });
    await hideSeedSketches(page); // a visible profile would arbitrate against the body
    await waitForCameraSettled(page);
  });

  for (const [topoKey, mid] of Object.entries(EDGES)) {
    test(`hovering ${topoKey} where it is drawn pre-highlights it`, async ({ page }) => {
      const at = await onScreen(page, mid);
      expect(at, `${topoKey} midpoint is not on screen and clear of chrome`).not.toBeNull();
      await page.mouse.move(at!.x, at!.y);
      await expect
        .poll(async () => (await hovered(page))?.kind, {
          message: `no hover pre-highlight on ${topoKey}`,
        })
        .toBe("edge");
      expect((await hovered(page))?.topoKey).toBe(topoKey);
    });

    test(`clicking ${topoKey} where it is drawn selects it`, async ({ page }) => {
      const at = await onScreen(page, mid);
      expect(at).not.toBeNull();
      await clickAt(page, at!);
      await expect.poll(async () => (await selection(page))[0]?.kind).toBe("edge");
      expect((await selection(page))[0]?.topoKey).toBe(topoKey);
    });
  }

  test("the same edge is still pickable after the camera orbits", async ({ page }) => {
    await orbit(page, 140);
    // Whichever of the two edges the new camera still shows clear of the chrome
    // — the point is that the answer does not depend on the camera pose.
    let picked = 0;
    for (const [topoKey, mid] of Object.entries(EDGES)) {
      const at = await onScreen(page, mid);
      if (!at) continue;
      await clickAt(page, at);
      await expect.poll(async () => (await selection(page))[0]?.kind).toBe("edge");
      expect((await selection(page))[0]?.topoKey).toBe(topoKey);
      picked++;
    }
    expect(picked, "no box edge was on screen after the orbit").toBeGreaterThan(0);
  });

  test("a click well inside a face still selects the face", async ({ page }) => {
    // The session-32 inverse. `findFaceOnBody` scans for a pixel the engine
    // itself reports as a face, so this cannot silently pass on an empty scene.
    const face = await findFaceOnBody(page, "body1");
    await clickAt(page, { x: face.x, y: face.y });
    await expect.poll(async () => (await selection(page))[0]?.kind).toBe("face");
    expect((await selection(page))[0]?.topoKey).toBe(face.topoKey);
  });
});
