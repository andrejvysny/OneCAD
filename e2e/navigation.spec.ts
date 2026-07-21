/*
 * Viewport navigation e2e.
 *
 * IMPORTANT: `page.mouse.wheel()` synthesises a MOUSE-shaped event stream —
 * integer deltas, no horizontal component, slow cadence. Under the "auto"
 * preference the classifier therefore (correctly) routes it as a mouse, so a
 * synthetic Shift+wheel ZOOMS rather than orbiting. Asserting orbit from that
 * would be testing the wrong path.
 *
 * So the trackpad gestures are exercised with the device override pinned to
 * "trackpad"; auto-classification itself is covered by navInput.test.ts, and a
 * real pinch (which Playwright cannot synthesise at all) stays a manual check.
 */
import { test, expect, type Page } from "@playwright/test";
import { openEditorDebug, waitForCameraSettled, CANVAS } from "./helpers";

interface CamSnapshot {
  distance: number;
  target: [number, number, number];
  dir: [number, number, number];
}

/** Read camera state through the ?vpdebug engine handle. */
async function camera(page: Page): Promise<CamSnapshot> {
  return page.evaluate(() => {
    const e = (window as unknown as { __vpEngine?: any }).__vpEngine;
    const t = e.controls.getTarget();
    const d = e.getViewDirection();
    return {
      distance: e.getCameraDistance(),
      target: [t.x, t.y, t.z],
      dir: [d.x, d.y, d.z],
    };
  }) as Promise<CamSnapshot>;
}

/**
 * Pin the input-device preference.
 *
 * `settingsStore` persists through `zustand/persist`, which REHYDRATES
 * ASYNCHRONOUSLY. Writing before hydration lands means hydration silently
 * overwrites the write with whatever a previous test persisted — which showed up
 * as a trackpad test being routed as a mouse. Wait for hydration first, then
 * write, then confirm it stuck.
 */
async function setDevicePref(page: Page, pref: "auto" | "trackpad" | "mouse"): Promise<void> {
  await page.waitForFunction(() => {
    const st = (window as unknown as { __stores?: any }).__stores?.settings;
    return !!st && (!st.persist || st.persist.hasHydrated());
  });
  const applied = await page.evaluate((p) => {
    const st = (window as unknown as { __stores?: any }).__stores.settings;
    st.getState().setInputDevice(p);
    return st.getState().navigation.inputDevice;
  }, pref);
  expect(applied).toBe(pref);
}

/** Centre of the viewport, in page coordinates. */
async function centre(page: Page): Promise<{ x: number; y: number }> {
  const box = await page.locator(CANVAS).boundingBox();
  if (!box) throw new Error("no canvas box");
  return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
}

function dist(a: [number, number, number], b: [number, number, number]): number {
  return Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
}

test.describe("viewport navigation", () => {
  test.beforeEach(async ({ page }) => {
    // Start every test from the default preference — the persisted settings blob
    // would otherwise carry one test's override into the next.
    await page.addInitScript(() => localStorage.removeItem("onecad.settings"));
    await openEditorDebug(page);
    // Mesh ingestion triggers an animated fit; measuring mid-tween reads a
    // camera that is still moving on its own.
    await waitForCameraSettled(page);
    const c = await centre(page);
    await page.mouse.move(c.x, c.y);
  });

  test("mouse wheel zooms under the auto preference", async ({ page }) => {
    const before = await camera(page);
    await page.mouse.wheel(0, 240);
    const after = await camera(page);
    expect(after.distance).toBeGreaterThan(before.distance);
  });

  test("mouse wheel up moves the camera closer", async ({ page }) => {
    const before = await camera(page);
    await page.mouse.wheel(0, -240);
    const after = await camera(page);
    expect(after.distance).toBeLessThan(before.distance);
  });

  test("shift + synthetic wheel still zooms (it is a mouse stream)", async ({ page }) => {
    const before = await camera(page);
    await page.keyboard.down("Shift");
    await page.mouse.wheel(0, 240);
    await page.keyboard.up("Shift");
    const after = await camera(page);
    expect(after.distance).toBeGreaterThan(before.distance);
    expect(dist(after.dir, before.dir)).toBeCloseTo(0, 6);
  });

  test("the mouse override forces zoom even for trackpad-shaped input", async ({ page }) => {
    await setDevicePref(page, "mouse");
    const before = await camera(page);
    await page.mouse.wheel(0, 240);
    const after = await camera(page);
    expect(after.distance).toBeGreaterThan(before.distance);
    expect(dist(after.target, before.target)).toBeCloseTo(0, 6);
  });

  test("the trackpad override pans on a plain wheel", async ({ page }) => {
    await setDevicePref(page, "trackpad");
    const before = await camera(page);
    await page.mouse.wheel(0, 120);
    const after = await camera(page);
    expect(dist(after.target, before.target)).toBeGreaterThan(0);
    expect(after.distance).toBeCloseTo(before.distance, 6);
  });

  test("the trackpad override orbits on shift + wheel", async ({ page }) => {
    await setDevicePref(page, "trackpad");
    const before = await camera(page);
    await page.keyboard.down("Shift");
    await page.mouse.wheel(0, 120);
    await page.keyboard.up("Shift");
    const after = await camera(page);
    expect(dist(after.dir, before.dir)).toBeGreaterThan(0);
    expect(dist(after.target, before.target)).toBeCloseTo(0, 6);
    expect(after.distance).toBeCloseTo(before.distance, 6);
  });

  test("horizontal wheel motion pans under the trackpad override", async ({ page }) => {
    await setDevicePref(page, "trackpad");
    const before = await camera(page);
    await page.mouse.wheel(120, 0);
    const after = await camera(page);
    expect(dist(after.target, before.target)).toBeGreaterThan(0);
  });

  test("wheeling over the canvas selects nothing", async ({ page }) => {
    await page.mouse.wheel(0, 240);
    // A selection would surface in the inspector; nothing should be picked.
    await expect(page.getByText("Select geometry to constrain.")).toBeVisible();
  });

  test("the page itself never scrolls", async ({ page }) => {
    await page.mouse.wheel(0, 500);
    const scrolled = await page.evaluate(() => window.scrollY || document.body.scrollTop || 0);
    expect(scrolled).toBe(0);
  });

  test("a right-button drag pans rather than orbiting", async ({ page }) => {
    const c = await centre(page);
    const before = await camera(page);
    await page.mouse.move(c.x, c.y);
    await page.mouse.down({ button: "right" });
    await page.mouse.move(c.x + 80, c.y, { steps: 4 });
    await page.mouse.up({ button: "right" });
    const after = await camera(page);
    expect(dist(after.target, before.target)).toBeGreaterThan(0);
    expect(dist(after.dir, before.dir)).toBeCloseTo(0, 6);
  });

  test("orbiting near the top view does not self-align", async ({ page }) => {
    await setDevicePref(page, "trackpad");
    // Drive the pitch up towards the pole in steps, recording the view direction.
    await page.keyboard.down("Shift");
    for (let i = 0; i < 12; i++) await page.mouse.wheel(0, -60);
    const near = await camera(page);
    await page.mouse.wheel(0, -30);
    const past = await camera(page);
    await page.keyboard.up("Shift");
    // Crossing the former pole threshold used to snap the basis; the direction
    // must keep changing smoothly instead of jumping.
    expect(dist(past.dir, near.dir)).toBeLessThan(0.2);
  });
});
