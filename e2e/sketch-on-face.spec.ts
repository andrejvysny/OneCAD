import { test, expect, type Page } from "@playwright/test";
import {
  openEditorDebug,
  waitForCameraSettled,
  selectSketchTool,
  findFaceOnBody,
  getSketchSnapshot,
  getSketchEntityCount,
  planePointToClient,
  clickAtClient,
  bodyOptions,
  commitExtrudeAtHandle,
} from "./helpers";

/*
 * SKETCH ON A MODEL FACE — the whole W2 loop in the mock lane.
 *
 * pick a real face → S → the projected boundary is there and LOCKED → dragging it
 * is refused (with a hint) → deleting it is refused (with a hint) → the boundary
 * still bounds a region you can extrude off.
 *
 * The face pick is a GENUINE raycast (`findFaceOnBody` → the engine's own
 * `probePick`), not a seeded selection: the flow under test IS "the user clicked a
 * face", and seeding the store would skip the promote → faceSketchPlane →
 * enterSketch(newOnFace) chain that carries the topoKey.
 *
 * MOCK FALSE-GREEN RULE. Every assertion here is STRUCTURAL — counts, presence,
 * "unchanged", "a hint appeared". The mock lane's face geometry is analytic
 * (`src/ipc/mockFaceGeometry.ts`), not kernel output, so asserting numeric
 * geometry here would pin the mock to itself. Numeric truth for a projected face
 * boundary lives in the cargo/ctest gates.
 */

/** Ids of the `referenceLocked` entities in the live session (the projected boundary). */
async function lockedEntityIds(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: {
        sketch: { getState(): { session: { entities: Array<{ id: string; referenceLocked?: boolean }> } | null } };
      };
    };
    const session = w.__stores?.sketch.getState().session;
    if (!session) throw new Error("no live sketch session");
    return session.entities.filter((e) => e.referenceLocked).map((e) => e.id);
  });
}

/** Every Line entity's endpoints, in plane (u,v) — the drag/click targets. */
async function lineEndpoints(page: Page): Promise<Array<[number, number]>> {
  const snap = await getSketchSnapshot(page);
  return snap.lines.flatMap((l) => [l.p0, l.p1]);
}

const lockedHint = (page: Page) => page.getByText(/Reference geometry is locked/);

/** Clear the status hint so the NEXT guard's hint is proof of that guard firing
 *  (the two refusals share one message by design). */
async function clearStatusHint(page: Page): Promise<void> {
  await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { viewport: { getState(): { setStatusHint(m: string | null): void } } };
    };
    w.__stores?.viewport.getState().setStatusHint(null);
  });
  await expect(page.getByText(/Reference geometry is locked/)).toHaveCount(0);
}

test("sketch on a picked face: the projected boundary is seeded, locked, and extrudable", async ({ page }) => {
  // `mockBody` publishes the mock BOX through the real ingest path so there is a
  // face to raycast against (the mock lane has no meshes otherwise).
  await openEditorDebug(page, { mockBody: true });

  // Hide the seeded sketches: their static fills sit at the origin, INSIDE the
  // box, and would arbitrate against the face pick. The body stays visible.
  const visibleSeedSketches = page
    .getByRole("listbox", { name: "Sketches" })
    .locator('[role="switch"][aria-checked="true"]');
  while ((await visibleSeedSketches.count()) > 0) await visibleSeedSketches.first().click();

  // ── (a) pick a real model face ─────────────────────────────────────────────
  const face = await findFaceOnBody(page);
  expect(face.topoKey).toMatch(/^f:\d+$/);
  await clickAtClient(page, face.x, face.y);

  const picked = await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { selection: { getState(): { selected: Array<{ kind: string; topoKey?: string }> } } };
    };
    return w.__stores?.selection.getState().selected[0] ?? null;
  });
  expect(picked?.kind).toBe("face");
  expect(picked?.topoKey).toBe(face.topoKey);

  // ── (b) S → sketch mode ON that face, seeded with the projected boundary ───
  await page.keyboard.press("s");
  await expect(page.getByText(/^Editing /)).toBeVisible({ timeout: 10_000 });
  await waitForCameraSettled(page);

  // The boundary arrives with the session — nothing was drawn.
  const locked = await lockedEntityIds(page);
  expect(locked.length, "the host face's projected boundary should be seeded as locked geometry").toBe(4);
  expect(await getSketchEntityCount(page)).toBe(4);

  // The pins are machine constraints and must NOT clutter the inspector, even
  // though the session still carries them (the marshaller depends on that).
  await expect(page.getByTestId(/^constraint-row-/)).toHaveCount(0);

  // ── (c) dragging a projected corner is REFUSED, loudly ─────────────────────
  await selectSketchTool(page, "Select");
  await clearStatusHint(page);
  const snapBefore = await getSketchSnapshot(page);
  const corner = (await lineEndpoints(page))[0];
  const cornerPt = await planePointToClient(page, snapBefore.plane, { x: corner[0], y: corner[1] });

  await page.mouse.move(cornerPt.x, cornerPt.y);
  await page.mouse.down();
  await page.mouse.move(cornerPt.x + 60, cornerPt.y + 40, { steps: 6 });
  await page.mouse.up();

  await expect(lockedHint(page)).toBeVisible();
  const snapAfterDrag = await getSketchSnapshot(page);
  expect(snapAfterDrag.lines, "a locked corner must not move").toEqual(snapBefore.lines);
  expect(await lockedEntityIds(page)).toEqual(locked);

  // ── (d) Delete on a locked selection is REFUSED, loudly ────────────────────
  await clearStatusHint(page);
  // Click the midpoint of a projected edge to select the LINE (not a handle).
  const [a, b] = [snapBefore.lines[0].p0, snapBefore.lines[0].p1];
  const midPt = await planePointToClient(page, snapBefore.plane, {
    x: (a[0] + b[0]) / 2,
    y: (a[1] + b[1]) / 2,
  });
  await clickAtClient(page, midPt.x, midPt.y);
  await page.keyboard.press("Delete");

  // The hint is itself the proof the LINE was selected: an empty sketch selection
  // makes Delete fall through untouched (useShortcuts) and nothing is reported.
  await expect(lockedHint(page)).toBeVisible();
  expect(await getSketchEntityCount(page), "a locked entity must survive Delete").toBe(4);
  expect(await lockedEntityIds(page)).toEqual(locked);

  // ── (e) the projected region is a real profile: extrude off it ─────────────
  // Centroid of the boundary: every corner is shared by exactly two lines, so the
  // raw-endpoint average equals the corner average (same trick as multiregion).
  const pts = await lineEndpoints(page);
  const centroid = {
    x: pts.reduce((s, p) => s + p[0], 0) / pts.length,
    y: pts.reduce((s, p) => s + p[1], 0) / pts.length,
  };
  const plane = snapBefore.plane;
  const bodiesBefore = await bodyOptions(page).count();

  await page.keyboard.press("Enter"); // finish the sketch
  await waitForCameraSettled(page);

  const regionPt = await planePointToClient(page, plane, centroid);
  await expect
    .poll(() =>
      page.evaluate(
        ({ x, y }) =>
          Boolean(
            (window as unknown as { __vpEngine?: { sketchStaticHitTest(x: number, y: number): unknown } }).__vpEngine
              ?.sketchStaticHitTest(x, y),
          ),
        regionPt,
      ),
    )
    .toBe(true);
  await clickAtClient(page, regionPt.x, regionPt.y);

  const selectedRegion = await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { selection: { getState(): { selected: Array<{ kind: string; regionId?: string }> } } };
    };
    return w.__stores?.selection.getState().selected[0] ?? null;
  });
  // Locked geometry DOES bound regions — the deliberate contrast with
  // construction geometry. Without that, a face sketch would be decorative.
  expect(selectedRegion?.kind).toBe("sketchRegion");
  expect(selectedRegion?.regionId).toBeTruthy();

  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();
  await commitExtrudeAtHandle(page);

  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
});

test("a NON-PLANAR face refuses to host a sketch and falls back to the plane picker", async ({ page }) => {
  // The cylinder's curved side face is the mock lane's stand-in for every
  // non-planar host; the backend refuses it, and the frontend must report that
  // rather than sketching on an approximated plane.
  await openEditorDebug(page, { mockBody: true });

  await page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { selection: { getState(): { set(refs: unknown[]): void } } };
    };
    // body2 is the mock CYLINDER; f:0 is its curved side. There is no cylinder in
    // the scene to raycast, so the pick is seeded — the refusal is what is under
    // test here, not the picking (which the first spec covers for real).
    w.__stores?.selection.getState().set([
      { kind: "face", id: "body2#f:0", bodyId: "body2", topoKey: "f:0", elementId: "el_side" },
    ]);
  });

  await page.keyboard.press("s");
  // The refusal RIDES INTO the picker prompt: publishing it as its own hint would
  // be overwritten the instant the picker writes its prompt, and the user would
  // be told nothing at all. Not stuck either — the world planes are still there.
  await expect(page.getByText(/Cannot sketch on that face.*Select a plane/)).toBeVisible({
    timeout: 10_000,
  });
});
