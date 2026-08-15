import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import { openEditorDebug, waitForCameraSettled, extrudeDebug, clickAtClient } from "./helpers";

/*
 * Gear Generator G1 — the title-bar "Generators ▾" lane, end to end in the mock
 * client (no Tauri, no C++ worker).
 *
 * PLACEMENT. A gear has TWO placement modes (SCHEMA §7.3) and these specs drive
 * the FRAME one: a click that hits no face falls through `tryPickGearFace` →
 * `tryPickGearGround`, which drops the gear on the world XY ground plane with an
 * identity frame. That is the path that exists precisely so a gear can be
 * authored with no host body to click, so it needs no prior sketch/extrude —
 * which is also what keeps this spec short.
 *
 * THE SEEDED BOX IS IN THE SCENE. `openEditorDebug` without `vpdemo` still
 * publishes `body1`'s MESH1 through the ingest path (same fact
 * `modelToolHelpers.ts` relies on for its edge tier), so the canvas CENTRE picks
 * the box's top face and would exercise the FACE branch instead. `findGroundPoint`
 * therefore locates a real empty-space pixel through the engine's own
 * `probePick`/`screenRay`, and `expectGearOnGroundPlane` proves which branch ran
 * by reading the armed chip's world anchor: z == 0 is the XY datum, the box top
 * is z == 15.
 *
 * WHAT THE MOCK ACTUALLY OWNS. `mockClient` mints no involute geometry — that is
 * pinned against the real kernel by the worker's own ctests. What this lane owns
 * is the UI CHAIN: menu → placement pick → chip + properties panel → committed
 * record + minted body. So the assertions are all about that chain, read off
 * `window.__stores` (dev-only) rather than pixels.
 *
 * NO RE-CLICK POLLING anywhere: the placement click happens exactly once and the
 * spec then waits on `__extrudePreview.gearPhase` / a store predicate. A blind
 * re-click loop would place a SECOND gear (each later click moves the armed one)
 * and hide a real arming regression behind a retry.
 */

/** The gear tool's registry id (`modules/modeling/ids.ts` `ModelingModelTools.gear`). */
const GEAR_TOOL_ID = "onecad.modeling.tool.model.gear";

async function documentBodyIds(page: Page): Promise<string[]> {
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

/** Every committed feature's `{ opType, label, valueText }` — the timeline
 *  projection, not the inspector's capped on-screen slice. */
async function gearFeatures(
  page: Page,
): Promise<Array<{ opType: string; label: string; valueText: string }>> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: {
        document: {
          getState(): {
            features: Array<{ opType?: string; label: string; valueText?: string }>;
          };
        };
      };
    };
    return (w.__stores?.document.getState().features ?? [])
      .filter((f) => f.opType === "Gear")
      .map((f) => ({ opType: f.opType ?? "", label: f.label, valueText: f.valueText ?? "" }));
  });
}

/**
 * A real client pixel that takes the FREE-SPACE branch: `probePick` misses every
 * face there, the camera ray still reaches `z = 0` (`groundPlanePoint` refuses a
 * ray parallel to or pointing away from the ground plane), and the canvas — not
 * a floating panel — is the topmost element, which is the arbitration a genuine
 * mouse click goes through.
 *
 * Scanned rather than hardcoded, for `findFacePoint`'s reason inverted: the box
 * lands wherever the fitted camera puts it, so "a pixel that is empty space" has
 * no closed-form offset. Asking the engine's OWN raycasts makes a point that
 * scans as empty a point the real pointerup also classifies as empty. The
 * candidate closest to the canvas centre wins — the farther out a pixel is, the
 * shallower its ground angle and the nearer the floating chrome.
 */
async function findGroundPoint(page: Page): Promise<{ x: number; y: number }> {
  let found: { x: number; y: number } | null = null;
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
      const cx = rect.left + rect.width / 2;
      const cy = rect.top + rect.height / 2;
      // Candidates are ordered centre-outward FIRST and raycast second, so the
      // first acceptable pixel is also the closest one and the scan stops there
      // — a full grid sweep is ~9k raycasts and measurably slowed the test.
      const step = 10;
      const candidates: Array<{ x: number; y: number }> = [];
      for (let y = rect.top + 40; y <= rect.bottom - 40; y += step) {
        for (let x = rect.left + 40; x <= rect.right - 40; x += step) candidates.push({ x, y });
      }
      candidates.sort(
        (a, b) => (a.x - cx) ** 2 + (a.y - cy) ** 2 - ((b.x - cx) ** 2 + (b.y - cy) ** 2),
      );
      for (const c of candidates) {
        if (engine.probePick(c.x, c.y)) continue; // a face is under this pixel
        const ray = engine.screenRay(c.x, c.y);
        if (!ray) continue;
        // `groundPlanePoint`'s own domain, inlined: the ray must actually travel
        // forward onto z = 0.
        const dz = ray.dir[2];
        if (!Number.isFinite(dz) || Math.abs(dz) < 1e-9) continue;
        if (-ray.origin[2] / dz <= 0) continue;
        if (document.elementFromPoint(c.x, c.y) !== canvas) continue; // occluded by DOM
        return c;
      }
      return null;
    });
    expect(found, "no canvas pixel is both empty space and over the ground plane").not.toBeNull();
  }).toPass({ timeout: 15_000, intervals: [200, 400, 800] });
  return found as unknown as { x: number; y: number };
}

/** `Generators ▾` → Gear, then ONE click in empty space to seat it on the XY
 *  ground plane. Returns once the FSM reports `armed`. */
async function armGearOnGround(page: Page): Promise<void> {
  await page.getByTestId("generators-menu-trigger").click();
  await page.getByTestId(`generators-menu-${GEAR_TOOL_ID}`).click();

  // The tool is waiting for a placement before the click is worth making —
  // clicking into `idle` would be swallowed by `tryPickGearFace`'s phase guard.
  await expect.poll(async () => (await extrudeDebug(page))?.gearPhase).toBe("facePick");

  const p = await findGroundPoint(page);
  await clickAtClient(page, p.x, p.y);

  await expect.poll(async () => (await extrudeDebug(page))?.gearPhase).toBe("armed");
}

/** The armed chip's world anchor IS `gear.point` (`showGear(…, anchor, …)`), so
 *  `z == 0` proves the frozen-FRAME branch ran rather than a face pick — the
 *  seeded box's flat seats sit at z = ±15, never on the ground plane. */
async function expectGearOnGroundPlane(page: Page): Promise<void> {
  const worldPos = await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { toolChip: { getState(): { kind: string; worldPos: number[] | null } } };
    };
    const s = w.__stores?.toolChip.getState();
    return s?.kind === "gear" ? s.worldPos : null;
  });
  expect(worldPos, "no armed gear chip").not.toBeNull();
  expect((worldPos as number[])[2]).toBeCloseTo(0, 6);
}

test("gear: Generators ▾ → Gear → click empty space → Apply mints a gear body + record", async ({
  page,
}) => {
  await openEditorDebug(page);
  await waitForCameraSettled(page);

  const bodiesBefore = await documentBodyIds(page);
  expect(await gearFeatures(page)).toHaveLength(0);

  await armGearOnGround(page);

  // The gear went onto the XY datum, not onto the seeded box's top face.
  await expectGearOnGroundPlane(page);

  // Both views onto the armed FSM are up, at the 20T m2 default (`gearInit`).
  await expect(page.getByTestId("chip-gear-summary")).toHaveText("Gear · 20T m2");
  await expect(page.getByTestId("gear-properties-panel")).toBeVisible();

  await page.getByTestId("gear-properties-apply").click();

  // A committed Gear record, and a body the WORKER minted (D1 `body_<opId>`) —
  // not a re-emitted host, because a gear has no target body.
  await expect.poll(async () => await gearFeatures(page)).toEqual([
    { opType: "Gear", label: "Gear", valueText: "20T m2" },
  ]);
  const bodiesAfter = await documentBodyIds(page);
  expect(bodiesAfter).toHaveLength(bodiesBefore.length + 1);
  const minted = bodiesAfter.filter((id) => !bodiesBefore.includes(id));
  expect(minted).toHaveLength(1);
  expect(minted[0]).toMatch(/^body_/);

  // The tool tore itself down: chip cleared, FSM back to idle.
  await expect(page.getByTestId("gear-properties-panel")).toHaveCount(0);
  await expect.poll(async () => (await extrudeDebug(page))?.gearPhase).toBe("idle");
});

test("gear: a teeth edit in the properties panel reaches the committed record", async ({ page }) => {
  await openEditorDebug(page);
  await waitForCameraSettled(page);
  await armGearOnGround(page);

  // `NumField` commits on blur; Enter blurs it (see GearPropertiesPanel).
  await page.getByTestId("gear-prop-teeth").fill("24");
  await page.getByTestId("gear-prop-teeth").press("Enter");

  // The chip is a second VIEW onto the same `toolChipStore` slice the panel
  // writes — if it moved, the FSM moved.
  await expect(page.getByTestId("chip-gear-summary")).toHaveText("Gear · 24T m2");

  await page.getByTestId("gear-properties-apply").click();

  await expect.poll(async () => (await gearFeatures(page)).map((f) => f.valueText)).toEqual([
    "24T m2",
  ]);
});
