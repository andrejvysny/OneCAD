import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtClient,
  clickAtAwaitingDofChange,
  dofPill,
  bodyOptions,
  sketchOptions,
  extrudeDebug,
  getFeatureLabels,
  getSketchSnapshot,
  getSketchCircle,
  planePointToClient,
  commitExtrudeAtHandle,
} from "./helpers";

/*
 * Boolean kernel-preview LIFECYCLE (mock lane) — the standalone Boolean tool.
 *
 * Picking the tool body is what makes a standalone Boolean well-formed, so that is
 * the moment the shared preview lane opens (mirrors revolve-preview.spec.ts for
 * Revolve): the controller claims it (`previewOwner: "boolean"`), opens ONE
 * session, and the engine hides BOTH source bodies the moment the exact candidate
 * (or, on the mock lane, its truthful `replacedBodyIds` half) lands.
 *
 * GEOMETRY IS DELIBERATELY NOT ASSERTED. The mock client has no CSG —
 * `localSolver` publishes `{bodies: [], replacedBodyIds: [toolBodyId]}` for a
 * Boolean session (the ONE truthful half a preview with no kernel can offer: the
 * tool body IS consumed), which `ViewportEngine.setPreviewReplacedBodyIds` hides
 * regardless of whether the mesh came from a real worker or nothing at all. That
 * preview-volume == commit-volume equivalence is proven against the REAL worker in
 * `src-tauri/tests/preview_boolean.rs`; these specs prove the lane LIFECYCLE the
 * real client shares with the mock: arm → tool body hidden → Esc restores it, and
 * arm → Apply → the tool body is CONSUMED (one surviving body row).
 */

async function documentBodyIds(page: Page): Promise<string[]> {
  return page.evaluate(
    () =>
      Object.keys(
        (
          window as unknown as {
            __stores?: { document: { getState(): { bodies: Record<string, unknown> } } };
          }
        ).__stores?.document.getState().bodies ?? {},
      ),
  );
}

/**
 * Wait until the point is genuinely pickable — the ATOMIC tri-state, not a bare
 * hit test.
 *
 * A poll can only ever say "it was hittable a moment ago": the sketch layer tears
 * its fills out for two awaited round trips on every reload, so the window can open
 * between this poll and the click. That is why the previous version of this helper
 * passed immediately before the failure it was supposed to prevent. The real fix is
 * in the product — an unsettled pick DEFERS instead of clearing the selection
 * (`ViewportRoot.runPick`) — and this helper now reads the same tri-state so it
 * cannot report ready while a refill is in flight.
 */
async function sketchHitTestReady(page: Page, client: { x: number; y: number }): Promise<void> {
  await expect
    .poll(() =>
      page.evaluate(
        ({ x, y }) =>
          (
            window as unknown as {
              __vpEngine?: {
                sketchStaticPickState(x: number, y: number): { state: string };
              };
            }
          ).__vpEngine?.sketchStaticPickState(x, y).state ?? "no-engine",
        client,
      ),
    )
    .toBe("hit");
}

/** Extrude the region at `sketchId`/plane point `pt` (already selected via a
 *  static-sketch click) and return the newly committed body id. */
async function extrudeRegionAt(
  page: Page,
  plane: Awaited<ReturnType<typeof getSketchSnapshot>>["plane"],
  pt: { x: number; y: number },
  before: string[],
): Promise<string> {
  // The camera MUST be settled before a plane point is projected to client
  // coordinates. Re-showing a sketch (which `twoBodies` does between the two
  // extrudes) schedules the debounced auto-fit, and a fit that starts after the
  // projection invalidates the coordinate — `waitForCameraSettled` exists for
  // exactly this and covers the scheduled-but-not-started window.
  //
  // Measured: without it the second pick reports `state:"miss"` with NO body hit
  // and NO sketch hit, i.e. the ray lands on empty space, and Extrude then arms
  // multi-select. It is not a hit-precedence or refill problem.
  await waitForCameraSettled(page);
  const client = await planePointToClient(page, plane, pt);
  await sketchHitTestReady(page, client);
  await clickAtClient(page, client.x, client.y);
  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();
  await commitExtrudeAtHandle(page);
  const after = await documentBodyIds(page);
  const created = after.find((id) => !before.includes(id));
  if (!created) throw new Error("extrude commit produced no new body");
  return created;
}

/**
 * Build TWO real (clickable) bodies from ONE sketch — a rectangle and a
 * well-separated circle (the disjoint-region layout `extrude-multiselect.spec.ts`
 * proves stays non-ambiguous). Each region is extruded and committed SEPARATELY
 * (V1 Extrude is single-profile — a multi-select is rejected), re-showing the
 * sketch between the two commits (a successful extrude auto-hides its consumed
 * sketch). Returns `[targetBodyId, toolBodyId]`.
 */
async function twoBodies(page: Page): Promise<[string, string]> {
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -260, -150);
  await clickAt(page, -60, 50);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  await selectSketchTool(page, "Circle");
  await clickAt(page, 120, -80); // center
  await clickAtAwaitingDofChange(page, 200, -80); // radius 80 → commits the circle

  // Snapshot BEFORE finishing — sketchStore.session clears once mode leaves "sketch".
  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(4); // the rectangle only — circle isn't a Line entity
  const circle = await getSketchCircle(page);
  const rectSum = snap.lines.reduce(
    (acc, l) => ({ x: acc.x + l.p0[0] + l.p1[0], y: acc.y + l.p0[1] + l.p1[1] }),
    { x: 0, y: 0 },
  );
  const rectCentroid = { x: rectSum.x / (snap.lines.length * 2), y: rectSum.y / (snap.lines.length * 2) };
  const circleCentroid = { x: circle.center[0], y: circle.center[1] };

  const beforeAny = await documentBodyIds(page);

  await page.keyboard.press("Enter"); // finish → model Select
  await waitForCameraSettled(page);

  const targetId = await extrudeRegionAt(page, snap.plane, rectCentroid, beforeAny);
  const afterTarget = await documentBodyIds(page);

  // The rectangle's extrude auto-hid the (now consumed-looking) sketch — re-show it
  // so the circle's static fill is clickable for the second extrude.
  await sketchOptions(page).last().getByRole("switch").click();
  const toolId = await extrudeRegionAt(page, snap.plane, circleCentroid, afterTarget);

  return [targetId, toolId];
}

async function setupTwoBodies(page: Page): Promise<[string, string]> {
  await openEditorDebug(page);
  await bodyOptions(page).first().getByRole("switch").click(); // hide the seeded body
  const visibleSeedSketches = page
    .getByRole("listbox", { name: "Sketches" })
    .locator('[role="switch"][aria-checked="true"]');
  while ((await visibleSeedSketches.count()) > 0) await visibleSeedSketches.first().click();

  const [targetId, toolId] = await twoBodies(page);
  expect(targetId).not.toBe(toolId);
  await expect(bodyOptions(page)).toHaveCount(3); // seeded (hidden) + target + tool
  return [targetId, toolId];
}

/** Hit-scan the canvas for a real screen point that raycasts onto `bodyId` — the
 *  engine's OWN `probePick` (exposed at `window.__vpEngine` under `?vpdebug`),
 *  mirroring `helpers.ts findExtrudeHandle`'s scanning approach. */
async function findBodyScreenPoint(page: Page, bodyId: string): Promise<{ x: number; y: number }> {
  let found: { x: number; y: number } | null = null;
  await expect(async () => {
    found = await page.evaluate((id) => {
      const engine = (
        window as unknown as {
          __vpEngine?: { probePick(x: number, y: number): { bodyId: string } | null };
        }
      ).__vpEngine;
      const canvas = document.querySelector(
        '[data-testid="viewport-canvas"] canvas',
      ) as HTMLCanvasElement | null;
      if (!engine || !canvas) return null;
      const rect = canvas.getBoundingClientRect();
      const step = 8;
      for (let y = rect.top; y <= rect.bottom; y += step) {
        for (let x = rect.left; x <= rect.right; x += step) {
          const hit = engine.probePick(x, y);
          if (hit && hit.bodyId === id) return { x, y };
        }
      }
      return null;
    }, bodyId);
    expect(found).not.toBeNull();
  }).toPass({ timeout: 10_000, intervals: [200, 400, 800] });
  return found as unknown as { x: number; y: number };
}

/** The engine's OWN runtime visibility for a body's scene group — what
 *  `setPreviewReplacedBodyIds` flips, independent of the tree's `visible` flag. */
async function bodySceneVisible(page: Page, bodyId: string): Promise<boolean | null> {
  return page.evaluate((id) => {
    const engine = (
      window as unknown as {
        __vpEngine?: {
          bodiesRoot: { children: Array<{ userData: { bodyId?: string }; visible: boolean }> };
        };
      }
    ).__vpEngine;
    const child = engine?.bodiesRoot.children.find((c) => c.userData.bodyId === id);
    return child ? child.visible : null;
  }, bodyId);
}

/**
 * Click the chip's "Apply" button (shared by boolean / pattern / mirror —
 * `ModelToolChips.tsx ApplyButton`). `page.getByRole("button", { name: "Apply" })
 * .click()` is unreliable here — verified empirically: the button is genuinely
 * mounted (correct role/name/rect, receiving pointer events, `elementFromPoint`
 * resolves to itself) yet Playwright's locator-click occasionally never settles,
 * while a coordinate click on the SAME rect lands immediately. Root cause not
 * pinned (no other e2e spec exercises this shared button); the coordinate click
 * is the robust workaround, mirroring `findBodyScreenPoint`'s own raw-canvas
 * approach for the same class of overlay-positioned element.
 *
 * The retry asserts the OUTCOME, not the rect. The previous version only checked
 * that a rect existed, so the click itself could silently miss (the chip is
 * overlay-positioned and can move between measuring and clicking) and the helper
 * still returned "successfully" — leaving the caller to time out on a lane that
 * was never applied. Re-clicking is safe because each attempt re-checks first:
 * once the lane has closed the loop exits without touching the button again.
 */
async function clickApplyButton(page: Page): Promise<void> {
  const laneClosed = async (): Promise<boolean> =>
    ((await extrudeDebug(page))?.previewOwner ?? null) === null;
  await expect(async () => {
    if (await laneClosed()) return;
    const rect = await page.evaluate(() => {
      const apply = Array.from(document.querySelectorAll("button")).find(
        (b) => b.textContent === "Apply",
      );
      return apply ? apply.getBoundingClientRect().toJSON() : null;
    });
    expect(rect).not.toBeNull();
    const r = rect as unknown as { x: number; y: number; width: number; height: number };
    await page.mouse.click(r.x + r.width / 2, r.y + r.height / 2);
    expect(await laneClosed()).toBe(true);
  }).toPass({ timeout: 15_000, intervals: [200, 400, 800, 1200] });
}

test("boolean preview: picking the tool body opens the lane and hides it; Esc restores it", async ({
  page,
}) => {
  const [targetId, toolId] = await setupTwoBodies(page);

  await bodyOptions(page).nth(1).click(); // select the target
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as unknown as {
              __stores?: { selection: { getState(): { selected: Array<{ kind: string; id: string }> } } };
            }
          ).__stores?.selection.getState().selected[0]?.id,
      ),
    )
    .toBe(targetId);

  await page.getByRole("button", { name: "Combine", exact: true }).click();
  await expect(page.getByText("Pick the tool body to combine")).toBeVisible();
  // No draft is well-formed yet (target alone) — the lane MUST still be free.
  expect((await extrudeDebug(page))?.previewOwner).toBeNull();
  expect((await extrudeDebug(page))?.previewSessionCount).toBe(0);

  // Pick the tool body on canvas — a real raycast click (pickBooleanTool's armGen path).
  const toolPoint = await findBodyScreenPoint(page, toolId);
  await clickAtClient(page, toolPoint.x, toolPoint.y);
  await expect(page.getByText(/Choose Union \/ Cut \/ Intersect/)).toBeVisible();

  await expect
    .poll(async () => (await extrudeDebug(page))?.previewOwner, { timeout: 5_000 })
    .toBe("boolean");
  expect((await extrudeDebug(page))?.previewSessionCount).toBe(1);
  expect((await extrudeDebug(page))?.sessionId).not.toBeNull();

  // The mock lane's one truthful preview half: the tool body is hidden the moment
  // its exact (mesh-less) candidate result lands.
  await expect.poll(async () => bodySceneVisible(page, toolId), { timeout: 5_000 }).toBe(false);
  await expect.poll(async () => bodySceneVisible(page, targetId)).toBe(true); // target stays shown

  // Esc cancels: the lane releases (`endPreview(false)`) and the tool body's
  // visibility is restored exactly as `closePreviewSessions` promises.
  await page.keyboard.press("Escape");
  await expect.poll(async () => (await extrudeDebug(page))?.previewOwner).toBeNull();
  await expect.poll(async () => bodySceneVisible(page, toolId)).toBe(true);
  await expect(bodyOptions(page)).toHaveCount(3); // nothing committed, nothing removed
});

test("boolean commit: two bodies → pick target + tool → Apply lands ONE surviving body row", async ({
  page,
}) => {
  const [targetId, toolId] = await setupTwoBodies(page);

  await bodyOptions(page).nth(1).click();
  await page.getByRole("button", { name: "Combine", exact: true }).click();
  const toolPoint = await findBodyScreenPoint(page, toolId);
  await clickAtClient(page, toolPoint.x, toolPoint.y);
  await expect
    .poll(async () => (await extrudeDebug(page))?.previewOwner, { timeout: 5_000 })
    .toBe("boolean");

  await clickApplyButton(page);

  // The lane releases and the tool body is CONSUMED (the mock removes it — no real
  // fusion — same bookkeeping a real commit performs): the surviving row is exactly
  // the target among the two just created, plus the hidden seed.
  await expect
    .poll(async () => (await extrudeDebug(page))?.previewOwner, { timeout: 5_000 })
    .toBeNull();
  await expect(bodyOptions(page)).toHaveCount(2);
  const remaining = await documentBodyIds(page);
  expect(remaining).toContain(targetId);
  expect(remaining).not.toContain(toolId);
  await expect.poll(async () => await getFeatureLabels(page)).toContain("Union");
  await expect(page.getByRole("button", { name: "Combine", exact: true })).not.toHaveAttribute(
    "aria-pressed",
    "true",
  );
});
