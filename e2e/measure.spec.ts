import { test, expect, type Page } from "@playwright/test";
import { openEditorDebug, getFeatureLabels, waitForCameraSettled, CANVAS } from "./helpers";

/*
 * MEASURE V1a (W2-B) — the read-only inspection tool, mock lane.
 *
 * SHAPE ONLY, and deliberately so. `mockClient.elementInfo` SYNTHESIZES its
 * numbers from a hash of the pick (there is no kernel in this lane), so the only
 * honest assertions here are about the UI contract: a face pick shows an area
 * with `mm²`, a second pick adds a centre-to-centre distance in `mm` plus its Δ
 * components, and Esc clears everything. The NUMBERS are pinned against the real
 * OCCT worker in `src-tauri/tests/wire_contract.rs
 * measure_element_info_reports_exact_kernel_quantities` (top face area == 800,
 * centre == (−10,20,25)).
 *
 * Picks go through the engine's OWN raycast rather than a seeded store write,
 * because routing a pick to the measure tool (ViewportRoot `onPick`) is exactly
 * the wiring under test. The seeded mock document already publishes a real
 * MESH1 box, so `probePick` finds genuine face hits — no draw→extrude needed,
 * which keeps this spec free of the multi-step sketch flake the extrude specs
 * carry their own retries for.
 *
 * NOT covered here, deliberately: re-picking the SAME element (which refreshes
 * that pick rather than pairing it with itself). Nothing observable changes when
 * it happens, so an e2e assertion would either need a fixed sleep or pass
 * vacuously — `measureTool.test.ts` pins that rule directly instead.
 */

const MEASURE_BTN = { name: "Measure", exact: true } as const;

/**
 * Hit-scan the canvas for `n` screen points that raycast onto DIFFERENT faces.
 *
 * The camera MUST be settled first: the first body to load triggers a
 * `fitView()` tween (ViewportRoot), and a scan taken mid-tween records pixel
 * coordinates that no longer name the same face by the time the click lands —
 * two "distinct" points then resolve to one face and the second pick silently
 * becomes an idempotent refresh. Same discipline as `findExtrudeHandle`.
 */
async function findFacePoints(page: Page, n: number): Promise<Array<{ x: number; y: number }>> {
  await waitForCameraSettled(page);
  let found: Array<{ x: number; y: number }> | null = null;
  await expect(async () => {
    found = await page.evaluate((want) => {
      const engine = (
        window as unknown as {
          __vpEngine?: {
            probePick(x: number, y: number): { bodyId: string; kind: string; topoKey: string } | null;
          };
        }
      ).__vpEngine;
      const canvas = document.querySelector(
        '[data-testid="viewport-canvas"] canvas',
      ) as HTMLCanvasElement | null;
      if (!engine || !canvas) return null;
      const rect = canvas.getBoundingClientRect();
      const byKey = new Map<string, { x: number; y: number }>();
      const step = 8;
      for (let y = rect.top; y <= rect.bottom; y += step) {
        for (let x = rect.left; x <= rect.right; x += step) {
          const hit = engine.probePick(x, y);
          if (hit && hit.kind === "face" && !byKey.has(hit.topoKey)) {
            byKey.set(hit.topoKey, { x, y });
            if (byKey.size >= want) return [...byKey.values()];
          }
        }
      }
      return null;
    }, n);
    expect(found).not.toBeNull();
  }).toPass({ timeout: 10_000, intervals: [200, 400, 800] });
  return found as unknown as Array<{ x: number; y: number }>;
}

/**
 * Click a point and WAIT for the reading to settle at `expectSlots` labels.
 *
 * Each pick round-trips the mock client twice (promoteSelection + elementInfo),
 * so firing the next click before the previous one lands would race the overlay
 * update. The label count is the deterministic settle signal.
 */
async function clickPoint(page: Page, p: { x: number; y: number }): Promise<void> {
  await page.mouse.move(p.x, p.y);
  await page.mouse.down();
  await page.mouse.up();
}

async function pickFace(
  page: Page,
  p: { x: number; y: number },
  expectSlots: number,
): Promise<void> {
  await clickPoint(page, p);
  await expect(page.locator('[data-testid^="measure-label-"]')).toHaveCount(
    // slots + the pair summary once there are two of them
    expectSlots + (expectSlots === 2 ? 1 : 0),
  );
}

/**
 * The measured element ids, oldest first — read from the dev-only `__stores`
 * surface. The rendered LABELS cannot stand in for identity here: the mock
 * synthesizes its magnitudes, so two different faces can legitimately format to
 * the same string, which would make an "it changed" text assertion vacuous.
 */
async function measuredIds(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { measure: { getState(): { picks: Array<{ elementId: string }> } } };
    };
    return (w.__stores?.measure.getState().picks ?? []).map((p) => p.elementId);
  });
}

async function armMeasure(page: Page): Promise<void> {
  const btn = page.getByRole("button", MEASURE_BTN);
  await btn.click();
  await expect(btn).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByText("Select a face or edge to measure")).toBeVisible();
}

test("measuring one face reports its AREA, a second adds a centre-to-centre distance", async ({
  page,
}) => {
  await openEditorDebug(page);
  await expect(page.locator(CANVAS)).toBeVisible();
  const [pointA, pointB] = await findFacePoints(page, 2);

  await armMeasure(page);

  // ── first pick: an area label, in mm² ──
  await pickFace(page, pointA, 1);
  const first = page.getByTestId("measure-label-0");
  await expect(first).toBeVisible();
  await expect(first).toContainText(/^Area \d/);
  await expect(first).toContainText("mm²");
  // One pick is not a relationship — no distance yet.
  await expect(page.getByTestId("measure-label-sum")).toHaveCount(0);
  await expect(
    page.getByText("Select a second face or edge to measure between them"),
  ).toBeVisible();

  // ── second pick: the pair summary appears ──
  await pickFace(page, pointB, 2);
  await expect(page.getByTestId("measure-label-1")).toBeVisible();
  const sum = page.getByTestId("measure-label-sum");
  await expect(sum).toBeVisible();
  // "center ↔ center", never "centroid": the kernel value is a BOUNDING-BOX
  // centre, and the label has to say which distance this is.
  await expect(page.getByTestId("measure-distance")).toContainText(/^Center ↔ center [-\d.]+ mm$/);
  await expect(sum).toContainText("ΔX");
  await expect(sum).toContainText("ΔY");
  await expect(sum).toContainText("ΔZ");

  // ── Esc clears the whole reading ──
  await page.keyboard.press("Escape");
  await expect(page.getByRole("button", MEASURE_BTN)).toHaveAttribute("aria-pressed", "false");
  await expect(page.getByTestId("measure-label-0")).toHaveCount(0);
  await expect(page.getByTestId("measure-label-1")).toHaveCount(0);
  await expect(page.getByTestId("measure-label-sum")).toHaveCount(0);
});

test("measure writes NOTHING to the timeline", async ({ page }) => {
  await openEditorDebug(page);
  const [pointA, pointB] = await findFacePoints(page, 2);
  const before = await getFeatureLabels(page);

  await armMeasure(page);
  await pickFace(page, pointA, 1);
  await pickFace(page, pointB, 2);
  await expect(page.getByTestId("measure-label-sum")).toBeVisible();

  expect(await getFeatureLabels(page)).toEqual(before);
});

test("a third pick replaces the OLDEST — the reading is always the last two", async ({ page }) => {
  await openEditorDebug(page);
  // An isometric box shows three faces; the tool must keep the newest two of them.
  const [a, b, c] = await findFacePoints(page, 3);

  await armMeasure(page);
  await pickFace(page, a, 1);
  await pickFace(page, b, 2);
  const [idA, idB] = await measuredIds(page);
  expect(idA).not.toBe(idB); // the scan really found two distinct faces

  // Third DISTINCT face: A (the oldest) is evicted, B slides down a slot.
  //
  // The label-count gate cannot settle this pick — the overlay is ALREADY at two
  // slots + a summary, so it would pass before the third round-trip even lands.
  // Poll the ids themselves, which is the thing that actually changes.
  await clickPoint(page, c);
  await expect.poll(async () => (await measuredIds(page))[0]).toBe(idB);
  const after = await measuredIds(page);
  expect(after).toHaveLength(2);
  expect(after).not.toContain(idA);
  expect(after[1]).not.toBe(idB); // the third face really is a new element
  // Still exactly two slots + one summary on screen — never three readings.
  await expect(page.locator('[data-testid^="measure-label-"]')).toHaveCount(3);
});
