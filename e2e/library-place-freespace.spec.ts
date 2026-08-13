import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { bodyOptions, waitForCameraSettled } from "./helpers";

/*
 * COMPONENT-LIBRARY WP-F3 — the two halves of the placement gesture the mock
 * lane could not reach before: AUTO-SIZE on a real hole, and a FREE-SPACE drop.
 *
 * Why they were unreachable, and what changed: `mockClient.classifyElement` only
 * ever answered `plane`, so the concentric branch and the hole RADIUS auto-size
 * runs on had no input in Playwright at all — the seed box has six flat faces
 * and nothing else. `?vpdemo=cyl` now publishes a bushing with a genuine Ø8.5
 * bore, and the mock classifier MEASURES that bore off the body's own MESH1
 * bytes (facet normals ⊥ a common axis + a least-squares circle fit). So the
 * chain under test here is real end to end: hover → classify → snap kind →
 * auto-size → ghost → commit → the record the inspector reads back.
 *
 * WHAT THIS SPEC DOES NOT CLAIM: that a free-space commit records no mate. That
 * fact has no UI surface to read — nothing renders a placement's mate — so it is
 * pinned where it is observable, in `placementController.test.ts` ("commits at
 * the ghost's transform with NO mate"). What IS observable here, and asserted,
 * is the consequence: the body lands at the ground-plane point under the cursor
 * with identity rotation, which is only where an UNSNAPPED placement can land.
 *
 * `retries: 0`, matching the rest of this suite.
 */

/** The `?vpdemo=cyl` bushing (see `mockClient.MOCK_DEMO_BORE_BODY_ID`). */
const BUSHING = "body_demo_bore";

function bodyIds(page: Page): Promise<string[]> {
  return page.evaluate(() =>
    Object.keys(
      (
        window as unknown as {
          __stores?: { document: { getState(): { bodies: Record<string, unknown> } } };
        }
      ).__stores?.document.getState().bodies ?? {},
    ),
  );
}

/** `ViewportEngine.previewBodies` size (`?vpdebug`) — >0 while a ghost is live. */
function previewBodyCount(page: Page): Promise<number> {
  return page.evaluate(
    () =>
      (window as unknown as { __vpEngine?: { previewBodies?: Map<string, unknown> } }).__vpEngine
        ?.previewBodies?.size ?? 0,
  );
}

/**
 * A rendered body's world extents, read off the SCENE graph — the same shape
 * `transform-body.spec.ts` uses, and for the same reason: a placement's whole
 * job is to put geometry somewhere, and the projection store carries no
 * coordinates. MESH1 buffers upload verbatim into an un-transformed
 * `bodiesRoot`, so a geometry-space bbox IS the world bbox. FACE meshes only —
 * the edge layer is a fat-line whose `position` attribute is a unit-quad
 * template, and folding it in clamps every body's bbox to ±1.
 */
async function bodyBounds(page: Page, bodyId: string): Promise<{ min: number[]; max: number[] } | null> {
  return page.evaluate((id) => {
    const engine = (
      window as unknown as {
        __vpEngine?: {
          bodiesRoot: {
            children: Array<{
              userData: { bodyId?: string };
              traverse(fn: (o: Record<string, unknown>) => void): void;
            }>;
          };
        };
      }
    ).__vpEngine;
    const node = engine?.bodiesRoot.children.find((c) => String(c.userData.bodyId ?? "") === id);
    if (!node) return null;
    const min = [Infinity, Infinity, Infinity];
    const max = [-Infinity, -Infinity, -Infinity];
    node.traverse((o) => {
      if (!o.isMesh || (o.userData as { kind?: string } | undefined)?.kind !== "face") return;
      const g = o.geometry as
        | { attributes?: { position?: { array: ArrayLike<number>; count: number } } }
        | undefined;
      const pos = g?.attributes?.position;
      if (!pos) return;
      for (let i = 0; i < pos.count; i++) {
        for (let a = 0; a < 3; a++) {
          const v = pos.array[i * 3 + a];
          if (v < min[a]) min[a] = v;
          if (v > max[a]) max[a] = v;
        }
      }
    });
    return Number.isFinite(min[0]) ? { min, max } : null;
  }, bodyId);
}

/**
 * Hit-scan for a client pixel over one NAMED face of one body, through the
 * engine's own `probePick` — `findFaceOnBody` finds any face, and the bore is
 * a specific one (`f:1` on the bushing). Same rationale as every other scan
 * helper: a face lands wherever the camera puts it.
 */
async function findNamedFace(
  page: Page,
  bodyId: string,
  topoKey: string,
): Promise<{ x: number; y: number }> {
  await waitForCameraSettled(page);
  let found: { x: number; y: number } | null = null;
  await expect(async () => {
    found = await page.evaluate(
      ({ bodyId, topoKey }) => {
        const engine = (
          window as unknown as {
            __vpEngine?: {
              probePick(x: number, y: number): { bodyId: string; topoKey: string } | null;
            };
          }
        ).__vpEngine;
        const canvas = document.querySelector(
          '[data-testid="viewport-canvas"] canvas',
        ) as HTMLCanvasElement | null;
        if (!engine || !canvas) return null;
        const rect = canvas.getBoundingClientRect();
        const step = 4;
        for (let y = rect.top + step; y <= rect.bottom - step; y += step) {
          for (let x = rect.left + step; x <= rect.right - step; x += step) {
            const hit = engine.probePick(x, y);
            if (hit && hit.bodyId === bodyId && hit.topoKey === topoKey) return { x, y };
          }
        }
        return null;
      },
      { bodyId, topoKey },
    );
    expect(found, `no pixel over ${bodyId} ${topoKey}`).not.toBeNull();
  }).toPass({ timeout: 20_000, intervals: [200, 400, 800, 1_500] });
  return found as unknown as { x: number; y: number };
}

/**
 * A client pixel over EMPTY space whose camera ray still reaches the ground
 * plane, plus the world point it lands on. Targeting only — the ray→z=0 solve
 * is repeated here because the test has to know where to LOOK for the committed
 * body; the behaviour it proves is the body being there.
 */
async function findFreeSpacePoint(
  page: Page,
): Promise<{ x: number; y: number; world: [number, number, number] }> {
  await waitForCameraSettled(page);
  let found: { x: number; y: number; world: [number, number, number] } | null = null;
  await expect(async () => {
    found = await page.evaluate(() => {
      const engine = (
        window as unknown as {
          __vpEngine?: {
            probePick(x: number, y: number): unknown;
            screenRay(x: number, y: number): { origin: number[]; dir: number[] } | null;
          };
        }
      ).__vpEngine;
      const canvas = document.querySelector(
        '[data-testid="viewport-canvas"] canvas',
      ) as HTMLCanvasElement | null;
      if (!engine || !canvas) return null;
      const rect = canvas.getBoundingClientRect();
      const step = 8;
      // Bottom-up: below the model the camera looks DOWN at the ground plane,
      // so a ray there crosses z = 0 in front of the eye rather than behind it.
      for (let y = rect.bottom - step; y >= rect.top + step; y -= step) {
        for (let x = rect.right - step; x >= rect.left + step; x -= step) {
          if (engine.probePick(x, y)) continue;
          const ray = engine.screenRay(x, y);
          if (!ray || Math.abs(ray.dir[2]) < 1e-9) continue;
          const t = -ray.origin[2] / ray.dir[2];
          if (!(t > 0)) continue;
          return {
            x,
            y,
            world: [
              ray.origin[0] + ray.dir[0] * t,
              ray.origin[1] + ray.dir[1] * t,
              ray.origin[2] + ray.dir[2] * t,
            ] as [number, number, number],
          };
        }
      }
      return null;
    });
    expect(found, "no empty pixel whose ray reaches the ground plane").not.toBeNull();
  }).toPass({ timeout: 20_000, intervals: [200, 400, 800, 1_500] });
  return found as unknown as { x: number; y: number; world: [number, number, number] };
}

/** The id of the committed `PlaceComponent` row, or `null` before it lands. */
function placeComponentFeatureId(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { document: { getState(): { features: Array<{ id: string; opType?: string }> } } };
    };
    const rows = w.__stores?.document.getState().features ?? [];
    return rows.find((f) => f.opType === "PlaceComponent")?.id ?? null;
  });
}

/** Arm the SHCS fixture from the library panel. */
async function armScrew(page: Page) {
  await page.getByTestId("sidebar-tab-library").click();
  const card = page.getByTestId("library-card").filter({ hasText: "Socket Head Cap Screw" });
  await expect(card).toBeVisible();
  await card.click();
  await expect(card).toHaveAttribute("data-armed", "true");
  return card;
}

test("hovering a real bore auto-sizes the screw, and the committed record keeps that size", async ({
  page,
}) => {
  // `?vpdemo=cyl` — box PLUS the bored bushing; `?mocklibrary=1` — the catalog.
  await page.goto("/?vpdebug&vpdemo=cyl&mocklibrary=1");
  await expect(page.locator('[data-testid="viewport-canvas"] canvas')).toBeVisible();

  await armScrew(page);
  // Armed, nothing measured yet: the hint offers both lanes and names no size.
  await expect(page.getByText(/snap to a target or drop in free space/)).toBeVisible();

  const bore = await findNamedFace(page, BUSHING, "f:1");
  await page.mouse.move(bore.x, bore.y);

  // Ø8.5 is the M8 clearance hole, so the largest declared size that fits is
  // M8 — NOT the armed M6. The hint naming it is auto-size firing.
  await expect(page.getByText(/M8 — A flips/)).toBeVisible({ timeout: 10_000 });
  await expect.poll(() => previewBodyCount(page), { timeout: 10_000 }).toBeGreaterThan(0);

  await page.mouse.down();
  await page.mouse.up();
  await expect.poll(() => previewBodyCount(page)).toBe(0);

  // Read the size back off the RECORD, through the inspector's component
  // section: selecting the placed feature loads its stored params and renders
  // the live designation. M8 there means the commit carried what the ghost showed.
  await expect.poll(() => placeComponentFeatureId(page), { timeout: 10_000 }).not.toBeNull();
  const featureId = (await placeComponentFeatureId(page))!;

  // Back to the model tree, select the placed BODY, then step through its
  // history to a FEATURE selection — the inspector caps a body's history at
  // three rows and only a feature selection shows the whole timeline. Same
  // two-step every history spec here uses (`extrude-draft`, `filletChamfer`).
  await page.getByTestId("sidebar-tab-model").click();
  await bodyOptions(page).last().click();
  await page.getByTestId("history-row-f3").click();
  await page.getByTestId(`history-row-${featureId}`).click();
  await expect(page.getByText("ISO 4762 M8x20")).toBeVisible({ timeout: 10_000 });
});

test("a click in free space drops the component on the ground plane", async ({ page }) => {
  await page.goto("/?vpdebug&vpdemo&mocklibrary=1");
  await expect(page.locator('[data-testid="viewport-canvas"] canvas')).toBeVisible();

  const card = await armScrew(page);
  const before = await bodyIds(page);
  expect(before).toHaveLength(1); // the vpdemo box, nothing placed yet

  const spot = await findFreeSpacePoint(page);
  await page.mouse.move(spot.x, spot.y);
  // The ghost FOLLOWS with nothing under the cursor — the WP-1.5 scope cut hid
  // it until a target classified, which is exactly what this closes.
  await expect.poll(() => previewBodyCount(page), { timeout: 10_000 }).toBeGreaterThan(0);

  await page.mouse.down();
  await page.mouse.up();

  await expect.poll(() => previewBodyCount(page)).toBe(0);
  await expect(card).not.toHaveAttribute("data-armed", "true");
  await expect.poll(() => bodyIds(page)).toHaveLength(2);

  const placed = (await bodyIds(page)).find((id) => !before.includes(id))!;
  await expect.poll(() => bodyBounds(page, placed), { timeout: 10_000 }).not.toBeNull();
  const { min, max } = (await bodyBounds(page, placed))!;
  // The M6 SHCS stub is seated at its own origin: head up to +6, shank down to
  // -20, both within Ø10 (`mockMeshes.placeComponentGhostMesh`). Identity
  // rotation + the ground point is the only placement that produces this.
  expect((min[0] + max[0]) / 2).toBeCloseTo(spot.world[0], 1);
  expect((min[1] + max[1]) / 2).toBeCloseTo(spot.world[1], 1);
  expect(min[2]).toBeCloseTo(-20, 1);
  expect(max[2]).toBeCloseTo(6, 1);
});
