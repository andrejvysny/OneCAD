import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  openEditorDebug,
  findFaceOnBody,
  clickAtClient,
  bodyOptions,
} from "./helpers";

/*
 * WP-C T3 — the HOLE tool, end to end in the mock lane.
 *
 * The gesture is new to this codebase: activate → click a PLANAR FACE (a genuine
 * raycast against the demo box, via `findFaceOnBody` → the engine's own
 * `probePick`) → armed with a chip cluster whose SHAPE depends on the hole
 * profile → ✓ commits → the row re-opens seeded.
 *
 * SCOPE NOTE (why this asserts a record round-trip and not a volume, exactly as
 * `extrude-draft.spec.ts` does for the draft angle): the mock lane has no CSG. A
 * hole is a boolean subtraction of a cylinder from the host mesh, and the mock's
 * `mutateOp` deliberately re-emits the host UNTOUCHED rather than shipping a
 * second, worse CSG implementation — so a drilled and an undrilled mock body are
 * geometrically identical here and any mesh assertion would be vacuous. The
 * KERNEL geometry is pinned exactly, against analytic volumes, in
 * `worker/tests/test_hole_op.cpp` (through-all / blind / counterbore /
 * countersink, GProp on the real BRep at 1e-6 relative) and in
 * `src-tauri/tests/hole_ops.rs` (the same volumes via `QueryMassProperties`, plus
 * the re-edit, the point fence, and save→reopen). What THIS spec owns is the
 * chain the kernel never sees: face pick → planarity gate → chips → committed
 * record → history row → re-edit seed.
 */

/** The model-tool debug surface (`?vpdebug`) — the hole slice of it. */
interface HoleDebug {
  holePhase?: string;
  holeType?: string;
  holeDiameter?: number;
  holeDepth?: number | null;
  holePoint?: [number, number, number] | null;
  holeCbDiameter?: number;
  holeCsDiameter?: number;
  holeCsAngleDeg?: number;
  holeEdit?: string | null;
}

async function holeDebug(page: Page): Promise<HoleDebug | undefined> {
  return page.evaluate(
    () => (window as unknown as { __extrudePreview?: HoleDebug }).__extrudePreview,
  );
}

async function lastFeature(page: Page): Promise<{ id: string; label: string; valueText: string }> {
  const f = await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: {
        document: { getState(): { features: Array<{ id: string; label: string; valueText: string }> } };
      };
    };
    const all = w.__stores?.document.getState().features ?? [];
    return all[all.length - 1];
  });
  if (!f) throw new Error("the projection has no features");
  return f;
}

/** The cluster's primary Ø field (the shared `DimensionInput`). */
function diameterInput(page: Page) {
  return page.getByLabel("Dimension value");
}

/**
 * The pixel on the SAME face that is FARTHEST from `from` — the second click a
 * "move the hole" gesture needs. Distance matters for a mundane reason: the armed
 * chip cluster is ~400px wide and is anchored AT the hole, so a nearby second
 * click lands on the chip DOM and the controller correctly ignores it (that
 * exclusion is what keeps the ✓ clickable).
 */
async function farthestPixelOnFace(
  page: Page,
  from: { x: number; y: number; bodyId: string; topoKey: string },
): Promise<{ x: number; y: number }> {
  const found = await page.evaluate((f) => {
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
    let best: { x: number; y: number } | null = null;
    let bestD = 0;
    for (let y = rect.top + 4; y <= rect.bottom - 4; y += 4) {
      for (let x = rect.left + 4; x <= rect.right - 4; x += 4) {
        const hit = engine.probePick(x, y);
        if (!hit || hit.kind !== "face") continue;
        if (hit.bodyId !== f.bodyId || hit.topoKey !== f.topoKey) continue;
        const d = Math.hypot(x - f.x, y - f.y);
        if (d > bestD) {
          bestD = d;
          best = { x, y };
        }
      }
    }
    return bestD >= 120 ? best : null;
  }, from);
  if (!found) throw new Error("no second pixel far enough on the same face");
  return found;
}

/** Activate the tool and click a real face on the demo box. */
async function armHoleOnBox(
  page: Page,
): Promise<{ x: number; y: number; bodyId: string; topoKey: string }> {
  await page.getByRole("button", { name: "Hole", exact: true }).click();
  await expect(page.getByText(/Click a flat face/)).toBeVisible();
  const face = await findFaceOnBody(page);
  await clickAtClient(page, face.x, face.y);
  await expect.poll(async () => (await holeDebug(page))?.holePhase).toBe("armed");
  return face;
}

/**
 * Reach the FULL timeline: the body state shows only that body's rows, the
 * feature state shows every one (InspectorPanel BodyState vs FeatureState) — and
 * a hole's row lives on whichever body the raycast happened to hit.
 */
async function openFullTimeline(page: Page): Promise<void> {
  await bodyOptions(page).first().click();
  await page.locator('[data-testid^="history-row-"]').first().click();
}

test.beforeEach(async ({ page }) => {
  // `vpdemo` drives the mock BOX body through the real ingest path, so there is
  // genuine geometry to raycast a face out of.
  await openEditorDebug(page, { mockBody: true });
});

test("the tool waits for a face click before it arms anything", async ({ page }) => {
  await page.getByRole("button", { name: "Hole", exact: true }).click();
  await expect.poll(async () => (await holeDebug(page))?.holePhase).toBe("facePick");
  // Nothing is armed, so no cluster and no ✓ — there is nothing to confirm yet.
  await expect(page.getByTestId("chip-confirm")).toHaveCount(0);
  await expect(page.getByTestId("chip-hole-simple")).toHaveCount(0);
});

test("a face click arms the hole at the picked point with the full cluster", async ({ page }) => {
  await armHoleOnBox(page);

  const d = await holeDebug(page);
  expect(d?.holeType).toBe("simple");
  expect(d?.holePoint, "the click's world point is frozen into the record").not.toBeNull();
  // A fresh hole is THROUGH-ALL: the commonest hole, and the one end condition
  // that needs no number.
  expect(d?.holeDepth).toBeNull();

  await expect(page.getByTestId("chip-hole-simple")).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("chip-hole-depth")).toHaveValue("Thru");
  await expect(page.getByTestId("chip-hole-std")).toBeVisible();
  await expect(page.getByTestId("chip-confirm")).toBeVisible();
  // The conditional pairs belong to the other two profiles and must NOT be here.
  await expect(page.getByTestId("chip-hole-cb-diameter")).toHaveCount(0);
  await expect(page.getByTestId("chip-hole-cs-diameter")).toHaveCount(0);
});

test("the Ø chip authors the diameter, ✓ commits, and the row reads it back", async ({ page }) => {
  await armHoleOnBox(page);
  const bodiesBefore = await bodyOptions(page).count();

  await diameterInput(page).fill("5.5");
  await page.keyboard.press("Tab"); // blur commits the value, Enter would commit the OP
  await expect.poll(async () => (await holeDebug(page))?.holeDiameter).toBe(5.5);

  await page.getByTestId("chip-confirm").click();
  await expect.poll(async () => (await holeDebug(page))?.holePhase).toBe("idle");

  const { label, valueText } = await lastFeature(page);
  expect(label).toBe("Hole");
  // Mirrors `dto.rs feature_value_text` for Hole exactly.
  expect(valueText).toBe("Ø5.5");
  // A hole MODIFIES its host — SCHEMA §7.3 mints nothing, so the body count holds.
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore);
});

test("the profile segments swap which conditional pair is on screen", async ({ page }) => {
  await armHoleOnBox(page);

  await page.getByTestId("chip-hole-counterbore").click();
  await expect.poll(async () => (await holeDebug(page))?.holeType).toBe("counterbore");
  await expect(page.getByTestId("chip-hole-cb-diameter")).toBeVisible();
  await expect(page.getByTestId("chip-hole-cb-depth")).toBeVisible();
  await expect(page.getByTestId("chip-hole-cs-diameter")).toHaveCount(0);

  await page.getByTestId("chip-hole-countersink").click();
  await expect.poll(async () => (await holeDebug(page))?.holeType).toBe("countersink");
  await expect(page.getByTestId("chip-hole-cs-diameter")).toBeVisible();
  await expect(page.getByTestId("chip-hole-cb-diameter")).toHaveCount(0);
  // Only the four SCHEMA §7.3 included angles are offered — never a free field.
  for (const a of [82, 90, 100, 120]) {
    await expect(page.getByTestId(`chip-hole-cs-${a}`)).toBeVisible();
  }
  await page.getByTestId("chip-hole-cs-82").click();
  await expect.poll(async () => (await holeDebug(page))?.holeCsAngleDeg).toBe(82);
});

test("a standards pick fills raw millimetres into the chips", async ({ page }) => {
  await armHoleOnBox(page);
  await page.getByTestId("chip-hole-counterbore").click();

  await expect(page.getByTestId("chip-hole-std-panel")).toHaveCount(0);
  await page.getByTestId("chip-hole-std").click();
  await page.getByTestId("chip-hole-std-M6-normal").click();

  // ISO 273 medium clearance for M6 + the DIN 974-1 row-1 SHCS counterbore.
  await expect.poll(async () => (await holeDebug(page))?.holeDiameter).toBe(6.6);
  await expect.poll(async () => (await holeDebug(page))?.holeCbDiameter).toBe(11);
  await expect(diameterInput(page)).toHaveValue("6.6");
  await expect(page.getByTestId("chip-hole-cb-diameter")).toHaveValue("11");
  await expect(page.getByTestId("chip-hole-cb-depth")).toHaveValue("6.8");
  // The picker is a one-shot filler, not a mode.
  await expect(page.getByTestId("chip-hole-std-panel")).toHaveCount(0);
});

test("clicking the face again MOVES the armed hole instead of starting a second one", async ({
  page,
}) => {
  const face = await armHoleOnBox(page);
  const first = (await holeDebug(page))?.holePoint;
  expect(first).not.toBeNull();

  const second = await farthestPixelOnFace(page, face);
  await clickAtClient(page, second.x, second.y);
  await expect.poll(async () => (await holeDebug(page))?.holePhase).toBe("armed");
  await expect
    .poll(async () => JSON.stringify((await holeDebug(page))?.holePoint))
    .not.toBe(JSON.stringify(first));

  // Still ONE hole: the commit writes a single row.
  await page.getByTestId("chip-confirm").click();
  await expect.poll(async () => (await holeDebug(page))?.holePhase).toBe("idle");
  const labels = await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { document: { getState(): { features: Array<{ label: string }> } } };
    };
    return (w.__stores?.document.getState().features ?? []).map((f) => f.label);
  });
  expect(labels.filter((l) => l === "Hole")).toHaveLength(1);
});

test("the ✕ cancels the armed hole without writing a row", async ({ page }) => {
  await armHoleOnBox(page);
  await page.getByTestId("chip-cancel").click();
  await expect.poll(async () => (await holeDebug(page))?.holePhase).toBe("idle");
  const labels = await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { document: { getState(): { features: Array<{ label: string }> } } };
    };
    return (w.__stores?.document.getState().features ?? []).map((f) => f.label);
  });
  expect(labels).not.toContain("Hole");
});

/*
 * A COMMITTED countersink. The spec above only ever *configures* one — it clicks
 * the chip, reads `holeType` back and moves on — so before this test the browser
 * lane had no evidence that a countersink survives the commit at all, and the
 * counterbore was the only dressed profile that ever reached a history row.
 *
 * The detour through counterbore is the load-bearing part. `holeParamsOf` emits
 * the INACTIVE profile's block as `null` rather than omitting it, so a record
 * committed as a countersink must carry `cbDiameter: null` — and `holeFsmFromParams`
 * falls back to `DEFAULT_HOLE_CB_DIAMETER` (11) for a null. Authoring a counterbore
 * at 15 first, then flipping, makes that fallback observable: an 11 on the way back
 * proves the commit really dropped the counterbore numbers, and a 15 would mean a
 * stale `cb*` rode along on a countersink — the exact shape `holeStandards.ts`
 * warns the Rust session rejects.
 */
test("a countersink COMMITS, and the row re-opens as a countersink with no stale counterbore", async ({
  page,
}) => {
  await armHoleOnBox(page);

  // Author a full COUNTERBORE first, so there are real cb numbers to leak.
  await page.getByTestId("chip-hole-counterbore").click();
  await page.getByTestId("chip-hole-std").click();
  await page.getByTestId("chip-hole-std-M8-close").click();
  await expect.poll(async () => (await holeDebug(page))?.holeCbDiameter).toBe(15);

  // …then flip to COUNTERSINK and dress that block instead. M8 is deliberate: its
  // DIN 74 recess is 16.4, which differs from the M6 default (12.4) the seed would
  // fall back to, so a restored value cannot be confused with a default one.
  await page.getByTestId("chip-hole-countersink").click();
  await page.getByTestId("chip-hole-std").click();
  await page.getByTestId("chip-hole-std-M8-close").click();
  await expect.poll(async () => (await holeDebug(page))?.holeCsDiameter).toBe(16.4);
  await page.getByTestId("chip-hole-cs-100").click();
  await expect.poll(async () => (await holeDebug(page))?.holeCsAngleDeg).toBe(100);

  await page.getByTestId("chip-confirm").click();
  await expect.poll(async () => (await holeDebug(page))?.holePhase).toBe("idle");

  const { id, label, valueText } = await lastFeature(page);
  expect(label).toBe("Hole");
  expect(valueText).toBe("Ø8.4"); // ISO 273 close clearance for M8

  await openFullTimeline(page);
  const row = page.getByTestId(`history-row-${id}`);
  await expect(row).toBeVisible();
  await row.dblclick();

  await expect.poll(async () => (await holeDebug(page))?.holePhase).toBe("armed");
  await expect.poll(async () => (await holeDebug(page))?.holeEdit).toBe(id);
  await expect.poll(async () => (await holeDebug(page))?.holeType).toBe("countersink");
  await expect.poll(async () => (await holeDebug(page))?.holeDiameter).toBe(8.4);
  await expect.poll(async () => (await holeDebug(page))?.holeCsDiameter).toBe(16.4);
  await expect.poll(async () => (await holeDebug(page))?.holeCsAngleDeg).toBe(100);
  // The counterbore authored before the flip is GONE from the record: 11 is the
  // default the null falls back to, 15 would be the leak.
  await expect.poll(async () => (await holeDebug(page))?.holeCbDiameter).toBe(11);

  await expect(page.getByTestId("chip-hole-cs-diameter")).toHaveValue("16.4");
  await expect(page.getByTestId("chip-hole-cs-100")).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("chip-hole-cb-diameter")).toHaveCount(0);
});

test("double-clicking the row re-arms the hole seeded from the stored params", async ({ page }) => {
  await armHoleOnBox(page);
  // Author a fully dressed counterbore so the re-edit has every field to restore.
  await page.getByTestId("chip-hole-counterbore").click();
  await page.getByTestId("chip-hole-std").click();
  await page.getByTestId("chip-hole-std-M8-close").click();
  await page.getByTestId("chip-hole-depth").fill("18");
  await page.keyboard.press("Tab");
  await expect.poll(async () => (await holeDebug(page))?.holeDepth).toBe(18);

  await page.getByTestId("chip-confirm").click();
  await expect.poll(async () => (await holeDebug(page))?.holePhase).toBe("idle");
  const { id } = await lastFeature(page);

  await openFullTimeline(page);
  const row = page.getByTestId(`history-row-${id}`);
  await expect(row).toBeVisible();
  await row.dblclick();

  await expect.poll(async () => (await holeDebug(page))?.holePhase).toBe("armed");
  await expect.poll(async () => (await holeDebug(page))?.holeEdit).toBe(id);
  // Every field comes back — including the frozen seat, which is never re-picked.
  await expect.poll(async () => (await holeDebug(page))?.holeType).toBe("counterbore");
  await expect.poll(async () => (await holeDebug(page))?.holeDiameter).toBe(8.4);
  await expect.poll(async () => (await holeDebug(page))?.holeDepth).toBe(18);
  await expect.poll(async () => (await holeDebug(page))?.holePoint).not.toBeNull();
  await expect(page.getByTestId("chip-hole-cb-diameter")).toHaveValue("15");
});
