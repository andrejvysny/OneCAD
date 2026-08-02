import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtClient,
  dofPill,
  bodyOptions,
  extrudeDebug,
  getSketchSnapshot,
  planePointToClient,
  type SketchPlaneSnapshot,
} from "./helpers";

/*
 * W1-B — CONSTRUCTION GEOMETRY (mock lane).
 *
 * Construction geometry is still solved (it snaps, constrains, dimensions, and
 * is a legal revolve axis) but is EXCLUDED from region detection — so it never
 * closes a profile. `detectRegions` (src/ipc/mockSketch.ts) applies exactly that
 * filter, so these region assertions are real in the mock lane, not staged.
 *
 * Observability: the viewport is a WebGL canvas, so "is there a region here" is
 * asked through the engine's OWN pick of the persistent static sketch layer
 * (`sketchStaticHitTest`, exposed under `?vpdebug`) — a `sketchRegion` hit means
 * a selectable profile fill exists at that plane point; a `sketch`/null hit means
 * only lines (or nothing) are there. The construction FLAG itself is read from the
 * live session (`__stores.sketch`), which only exists while in sketch mode.
 *
 * The dashed material is not asserted here (no per-entity DOM); SketchObject's
 * construction branch is covered by the unit suite.
 */

interface SessionEntity {
  id: string;
  type: string;
  construction: boolean;
  p0?: [number, number];
  p1?: [number, number];
}

/** The live session's entities + construction flags + coords. Sketch mode only. */
function sketchEntities(page: Page): Promise<SessionEntity[]> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: {
        sketch: {
          getState(): {
            session: {
              entities: Array<{
                id: string;
                type: string;
                construction?: boolean;
                p0?: [number, number];
                p1?: [number, number];
              }>;
            } | null;
          };
        };
      };
    };
    const session = w.__stores?.sketch.getState().session;
    if (!session) throw new Error("sketchEntities: no live sketch session (call before finishing)");
    return session.entities.map((e) => ({
      id: e.id,
      type: e.type,
      construction: Boolean(e.construction),
      p0: e.p0,
      p1: e.p1,
    }));
  });
}

/** Kind of static-layer hit under a plane point: "sketchRegion" ⇒ a profile fill. */
async function staticHitKindAt(
  page: Page,
  plane: SketchPlaneSnapshot,
  point: { x: number; y: number },
): Promise<string | null> {
  const client = await planePointToClient(page, plane, point);
  return page.evaluate(
    ({ x, y }) =>
      (window as unknown as {
        __vpEngine?: { sketchStaticHitTest(x: number, y: number): { kind: string } | null };
      }).__vpEngine?.sketchStaticHitTest(x, y)?.kind ?? null,
    client,
  );
}

/** Axis-aligned centroid of a set of plane-space line endpoints. */
function centroidOf(lines: Array<{ p0: [number, number]; p1: [number, number] }>): {
  x: number;
  y: number;
} {
  const xs = lines.flatMap((l) => [l.p0[0], l.p1[0]]);
  const ys = lines.flatMap((l) => [l.p0[1], l.p1[1]]);
  return {
    x: (Math.min(...xs) + Math.max(...xs)) / 2,
    y: (Math.min(...ys) + Math.max(...ys)) / 2,
  };
}

/** Boot into a clean sketch: seed bodies + seed sketches hidden (no stray fills). */
async function freshSketch(page: Page): Promise<void> {
  await openEditorDebug(page);
  await bodyOptions(page).first().getByRole("switch").click();
  const visibleSeedSketches = page
    .getByRole("listbox", { name: "Sketches" })
    .locator('[role="switch"][aria-checked="true"]');
  while ((await visibleSeedSketches.count()) > 0) await visibleSeedSketches.first().click();
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);
}

const constructionButton = (page: Page) => page.getByRole("button", { name: /Construction/ });

test("X draws construction geometry that never closes a region; a real profile still arms extrude", async ({
  page,
}) => {
  await freshSketch(page);

  // X with nothing selected turns the STICKY construction draw mode on; the chrome
  // toggle mirrors it (one piece of state, two surfaces).
  await expect(constructionButton(page)).toHaveAttribute("aria-pressed", "false");
  await page.keyboard.press("x");
  await expect(constructionButton(page)).toHaveAttribute("aria-pressed", "true");

  // Rect A — drawn entirely in construction mode.
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -230, -130);
  await clickAt(page, -80, -30);
  await expect.poll(() => sketchEntities(page).then((e) => e.length)).toBe(4);
  expect((await sketchEntities(page)).every((e) => e.construction)).toBe(true);
  // Still SOLVED: the auto-constrained construction rect consumes DOF like real
  // geometry (it is excluded from regions, not from the solver).
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  // X again → back to real geometry (sticky, so this is the only way out).
  await page.keyboard.press("x");
  await expect(constructionButton(page)).toHaveAttribute("aria-pressed", "false");

  // Rect B — real, disjoint from A.
  await clickAt(page, 40, 20);
  await clickAt(page, 220, 130);
  await expect.poll(() => sketchEntities(page).then((e) => e.length)).toBe(8);
  const flags = await sketchEntities(page);
  expect(flags.slice(0, 4).every((e) => e.construction)).toBe(true);
  expect(flags.slice(4).every((e) => !e.construction)).toBe(true);

  // Snapshot geometry BEFORE finishing (the session clears on leaving sketch mode).
  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(8);
  const constructionCentroid = centroidOf(snap.lines.slice(0, 4));
  const realCentroid = centroidOf(snap.lines.slice(4));

  await page.keyboard.press("Enter");
  await expect(page.getByText(/^Editing /)).toHaveCount(0);
  await waitForCameraSettled(page);

  // The construction rect has NO selectable profile fill; the real one does.
  expect(await staticHitKindAt(page, snap.plane, constructionCentroid)).not.toBe("sketchRegion");
  await expect
    .poll(() => staticHitKindAt(page, snap.plane, realCentroid))
    .toBe("sketchRegion");

  // Exactly one region ⇒ picking it arms Extrude directly (no multi-select).
  const realClient = await planePointToClient(page, snap.plane, realCentroid);
  await clickAtClient(page, realClient.x, realClient.y);
  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Extrude", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

test("X on a selected profile edge flips it to construction and the region disappears", async ({
  page,
}) => {
  await freshSketch(page);

  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -150, -100);
  await clickAt(page, 150, 100);
  await expect.poll(() => sketchEntities(page).then((e) => e.length)).toBe(4);

  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(4);
  const centroid = centroidOf(snap.lines);
  // Midpoint of the rectangle's bottom edge — a real click target on that entity.
  const ys = snap.lines.flatMap((l) => [l.p0[1], l.p1[1]]);
  const minV = Math.min(...ys);
  const bottom = snap.lines.find(
    (l) => Math.abs(l.p0[1] - minV) < 1e-3 && Math.abs(l.p1[1] - minV) < 1e-3,
  );
  if (!bottom) throw new Error("no bottom edge in the rectangle snapshot");
  const edgeMid = { x: (bottom.p0[0] + bottom.p1[0]) / 2, y: minV };

  // Select that edge, then X — with a NON-empty selection X flips the picked
  // entities instead of toggling the draw mode.
  await selectSketchTool(page, "Select");
  const edgeClient = await planePointToClient(page, snap.plane, edgeMid);
  await clickAtClient(page, edgeClient.x, edgeClient.y);
  await page.keyboard.press("x");

  await expect
    .poll(() => sketchEntities(page).then((e) => e.filter((x) => x.construction).length))
    .toBe(1);
  // The draw mode was NOT toggled (the selection branch owns the key press).
  await expect(constructionButton(page)).toHaveAttribute("aria-pressed", "false");

  await page.keyboard.press("Enter");
  await expect(page.getByText(/^Editing /)).toHaveCount(0);
  await waitForCameraSettled(page);

  // The loop no longer closes ⇒ no profile fill, and Extrude refuses to arm.
  expect(await staticHitKindAt(page, snap.plane, centroid)).not.toBe("sketchRegion");
  const extrudeBtn = page.getByRole("button", { name: "Extrude", exact: true });
  await page.getByRole("listbox", { name: "Sketches" }).getByRole("option").last().click();
  await extrudeBtn.click();
  // SEAM (pre-existing, not W1-B): `enterRegionPick` sets the "No closed region to
  // extrude" hint and THEN calls setTool("select"), whose tool-change subscriber
  // clears it — so the refusal is only observable as the fallback to Select.
  await expect(extrudeBtn).toHaveAttribute("aria-pressed", "false");
  await expect(page.getByText(/^Drag the arrow to set depth/)).toHaveCount(0);
});

test("a construction line is a valid revolve axis (the centerline workflow)", async ({ page }) => {
  await freshSketch(page);

  // A real rectangle profile…
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -60, -110);
  await clickAt(page, 180, 110);
  await expect.poll(() => sketchEntities(page).then((e) => e.length)).toBe(4);

  // …plus a CONSTRUCTION centerline well clear to its left (so it splits nothing).
  await page.keyboard.press("x");
  await expect(constructionButton(page)).toHaveAttribute("aria-pressed", "true");
  await selectSketchTool(page, "Line");
  await clickAt(page, -210, -140);
  await clickAt(page, -210, 140);
  await page.keyboard.press("Enter"); // end the line chain (a gesture is in progress)
  await expect.poll(() => sketchEntities(page).then((e) => e.length)).toBe(5);

  // Identify the centerline by its FLAG (not by guessed geometry): auto-constrain
  // may snap it H/V, which moves its endpoints off the clicked coordinates.
  const entities = await sketchEntities(page);
  const construction = entities.filter((e) => e.construction);
  expect(construction).toHaveLength(1);
  const axisLine = construction[0];
  if (!axisLine.p0 || !axisLine.p1) throw new Error("the centerline has no endpoints");
  const axisMid = {
    x: (axisLine.p0[0] + axisLine.p1[0]) / 2,
    y: (axisLine.p0[1] + axisLine.p1[1]) / 2,
  };

  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(5);

  await page.keyboard.press("Enter");
  await expect(page.getByText(/^Editing /)).toHaveCount(0);
  await waitForCameraSettled(page);

  // The construction line left the REGION set (the rectangle alone is the profile)…
  await page.getByRole("listbox", { name: "Sketches" }).getByRole("option").last().click();
  await page.getByRole("button", { name: "Revolve", exact: true }).click();
  await expect(page.getByText(/Pick axis line/)).toBeVisible();

  // …but it is still offered — and accepted — as the revolve AXIS.
  const axisClient = await planePointToClient(page, snap.plane, axisMid);
  await clickAtClient(page, axisClient.x, axisClient.y);
  await expect(page.getByText(/Drag to set angle/)).toBeVisible();
  expect((await extrudeDebug(page))?.revolvePhase).toBe("armed");
  await expect(page.getByLabel("Dimension value")).toHaveValue("360");
});
