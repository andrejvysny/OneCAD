import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  hideSeedSketches,
  openEditorDebug,
  waitForCameraSettled,
  selectSketchTool,
  findFaceOnBody,
  getSketchSnapshot,
  getSketchEntityCount,
  planePointToClient,
  clickAtClient,
  bodyOptions,
  findExtrudeHandle,
  extrudeDebug,
  getFeatureLabels,
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

/** Hide every currently-visible sketch through the tree's own toggles. Seeded
 *  sketches fill the origin INSIDE the box and would arbitrate against a face
 *  pick; a finished FACE sketch lies flush on its host and would claim the
 *  double-click through the static-sketch branch. */
async function hideVisibleSketches(page: Page): Promise<void> {
  await hideSeedSketches(page);
}

/**
 * A canvas pixel where a model FACE is the ONLY thing under the pointer — no
 * datum, and (crucially, while the plane picker is up) no origin quad, since the
 * pick order deliberately gives those two the pointer first.
 */
async function findFaceClearOfPickerChrome(page: Page): Promise<{ x: number; y: number; topoKey: string }> {
  let found: { x: number; y: number; topoKey: string } | null = null;
  await expect(async () => {
    found = await page.evaluate(() => {
      const engine = (
        window as unknown as {
          __vpEngine?: {
            probePick(x: number, y: number): { kind: string; topoKey: string } | null;
            planePickerHitTest(x: number, y: number): string | null;

            sketchStaticHitTest(x: number, y: number): unknown;
          };
        }
      ).__vpEngine;
      const canvas = document.querySelector('[data-testid="viewport-canvas"] canvas') as HTMLCanvasElement | null;
      if (!engine || !canvas) return null;
      const rect = canvas.getBoundingClientRect();
      const step = 8;
      for (let y = rect.top + step; y <= rect.bottom - step; y += step) {
        for (let x = rect.left + step; x <= rect.right - step; x += step) {
          const hit = engine.probePick(x, y);
          if (!hit || hit.kind !== "face") continue;
          if (
            engine.planePickerHitTest(x, y) ||
            (window as unknown as { __datumVisuals?: { hitTest(x: number, y: number): string | null } })
              .__datumVisuals?.hitTest(x, y)
          )
            continue;
          if (engine.sketchStaticHitTest(x, y)) continue;
          return { x, y, topoKey: hit.topoKey };
        }
      }
      return null;
    });
    expect(found, "no unobstructed model face found under any scanned pixel").not.toBeNull();
  }).toPass({ timeout: 15_000, intervals: [150, 300, 600, 1_000] });
  return found as unknown as { x: number; y: number; topoKey: string };
}

/** The live `activeSketchId` (the document id the controller is driving). */
async function activeSketchId(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { viewport: { getState(): { activeSketchId: string | null } } };
    };
    return w.__stores?.viewport.getState().activeSketchId ?? null;
  });
}

/** The projection row's recorded host face (`SketchDto.hostFace` in the real lane). */
async function hostFaceOf(page: Page, sketchId: string): Promise<{ bodyId: string; elementId: string } | null> {
  return page.evaluate((id) => {
    const w = window as unknown as {
      __stores?: {
        document: {
          getState(): { sketches: Record<string, { hostFace?: { bodyId: string; elementId: string } }> };
        };
      };
    };
    return w.__stores?.document.getState().sketches[id]?.hostFace ?? null;
  }, sketchId);
}

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

/**
 * Drag the extrude depth handle to an exact client point and RELEASE (which keeps
 * the tool armed at that depth — MODEL-HARDEN W1). Retries a missed grab: the
 * handle scan and the pointerdown are a frame apart on a render-on-demand engine,
 * so an occasional press lands beside the handle and no drag starts at all.
 */
async function dragExtrudeTo(page: Page, target: { x: number; y: number }): Promise<void> {
  await expect(async () => {
    const handle = await findExtrudeHandle(page);
    await page.mouse.move(handle.x, handle.y);
    await page.mouse.down();
    try {
      await page.mouse.move(target.x, target.y, { steps: 8 });
      expect((await extrudeDebug(page))?.phase).toBe("dragging");
    } finally {
      await page.mouse.up();
    }
  }).toPass({ timeout: 20_000, intervals: [200, 500, 1_000] });
}

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
  await hideVisibleSketches(page);

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

  const extrudeBtn = page.getByRole("button", { name: "Extrude", exact: true });
  await extrudeBtn.click();
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();

  // ── (f) HOST-BOOLEAN: the op defaults to MODIFYING the host, not a new body ─
  // This sketch is hosted on that body's face, so the arm opens on Add against it
  // — the Shapr3D push/pull expectation. The segments themselves are behind the
  // chip's `⋯`; the COLLAPSED readout is what the user sees change, and it is
  // what this spec asserts, because a dismiss-on-outside-press popover cannot
  // stay open across a viewport drag.
  await expect(page.getByTestId("chip-mode-readout")).toHaveText("Add");
  expect((await extrudeDebug(page))?.booleanMode).toBe("Add");

  // Direction-aware, live: the +depth screen direction is (handle − plane point),
  // so MIRRORING the handle through the plane point is a drag INTO the host. That
  // makes the flip a deterministic input here rather than a camera-dependent guess.
  const armedHandle = await findExtrudeHandle(page);
  const zero = await planePointToClient(page, plane, centroid);
  const into = { x: 2 * zero.x - armedHandle.x, y: 2 * zero.y - armedHandle.y };

  await dragExtrudeTo(page, into);
  expect((await extrudeDebug(page))?.depth as number).toBeLessThan(0); // really went in
  expect((await extrudeDebug(page))?.booleanMode).toBe("Cut");
  await expect(page.getByTestId("chip-mode-readout")).toHaveText("Cut");

  // …and back out again — the flip is live in BOTH directions, so the commit
  // below is an additive push/pull.
  await dragExtrudeTo(page, armedHandle);
  expect((await extrudeDebug(page))?.depth as number).toBeGreaterThan(0);
  expect((await extrudeDebug(page))?.booleanMode).toBe("Add");
  await expect(page.getByTestId("chip-mode-readout")).toHaveText("Add");

  await page.keyboard.press("Enter"); // explicit confirm → commit
  await expect(extrudeBtn).not.toHaveAttribute("aria-pressed", "true", { timeout: 10_000 });

  // The host was MODIFIED: one timeline row reading Add, and NO new body. (The
  // mock's Add concats into the target — the numeric truth for a real fused solid
  // is the cargo gate, `sketch_on_face.rs`.)
  await expect.poll(() => getFeatureLabels(page)).toContain("Extrude (Add)");
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore);
});

test("W3(b): the plane picker accepts a body FACE — S with nothing selected, then click the part", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await hideVisibleSketches(page);

  // Nothing selected ⇒ `S` is the bare new-sketch intent, so the picker comes up
  // instead of the sketch-on-the-selected-face shortcut the first spec covers.
  await page.evaluate(() => {
    const w = window as unknown as { __stores?: { selection: { getState(): { clear(): void } } } };
    w.__stores?.selection.getState().clear();
  });
  await page.keyboard.press("s");
  await expect(page.getByText(/Select a plane to start the sketch/)).toBeVisible({ timeout: 10_000 });
  await waitForCameraSettled(page); // the picker re-homes an axis-aligned camera

  const face = await findFaceClearOfPickerChrome(page);
  expect(face.topoKey).toMatch(/^f:\d+$/);

  // Hovering names the face in the prompt — the affordance that tells the user a
  // body is pickable here at all. NO planarity check runs on hover (it would cost
  // a backend round-trip per move); the click below is what validates.
  await page.mouse.move(face.x, face.y);
  await expect(page.getByText(/— click to sketch on it/)).toBeVisible();

  await clickAtClient(page, face.x, face.y);
  await expect(page.getByText(/^Editing /)).toBeVisible({ timeout: 10_000 });
  await waitForCameraSettled(page);

  // Identical outcome to the selected-face route: the host's projected boundary
  // arrives WITH the session, locked. Same command, reached a different way.
  expect(await lockedEntityIds(page)).toHaveLength(4);
  expect(await getSketchEntityCount(page)).toBe(4);

  const sid = await activeSketchId(page);
  expect(sid).toBeTruthy();
  expect(await hostFaceOf(page, sid as string)).toMatchObject({ bodyId: expect.any(String) });
});

/** The live selection array (`__stores.selection`, dev-only). */
async function selectedRefs(page: Page): Promise<Array<{ kind: string; id: string }>> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { selection: { getState(): { selected: Array<{ kind: string; id: string }> } } };
    };
    return w.__stores?.selection.getState().selected ?? [];
  });
}

/** The live tool mode (`__stores.tool`, dev-only). */
async function toolMode(page: Page): Promise<string | undefined> {
  return page.evaluate(() => {
    const w = window as unknown as { __stores?: { tool: { getState(): { mode: string } } } };
    return w.__stores?.tool.getState().mode;
  });
}

test("double-clicking a face selects the whole body, not a sketch (moved to S)", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await hideVisibleSketches(page);

  const face = await findFaceOnBody(page);

  // A plain click first selects just the FACE — proves the double-click below
  // REPLACES that, it does not merely add to it.
  await clickAtClient(page, face.x, face.y);
  expect((await selectedRefs(page))[0]?.kind).toBe("face");

  await page.mouse.dblclick(face.x, face.y);

  expect(await selectedRefs(page)).toEqual([{ kind: "body", id: face.bodyId }]);
  expect(await toolMode(page), "double-click must never enter sketch mode").toBe("model");
  await expect(page.getByText(/^Editing /)).toHaveCount(0);

  // Sketch-from-face now lives on S with a face selected (unaffected by this
  // change) — re-picking the face and pressing S still opens a sketch on it.
  await clickAtClient(page, face.x, face.y);
  await page.keyboard.press("s");
  await expect(page.getByText(/^Editing /)).toBeVisible({ timeout: 10_000 });
});

test("shift+double-click a face toggles its body into, then out of, the selection", async ({ page }) => {
  await openEditorDebug(page, { mockBody: true });
  await hideVisibleSketches(page);

  const face = await findFaceOnBody(page);

  // Seed an unrelated selection so "extend" is distinguishable from "replace".
  // Its identity is opaque to selectionStore.set — it need not resolve to a real
  // datum, only to stay untouched by the double-click below.
  const seed = { kind: "datum", id: "seed-for-extend-test" };
  await page.evaluate((ref) => {
    const w = window as unknown as {
      __stores?: { selection: { getState(): { set(refs: unknown[]): void } } };
    };
    w.__stores?.selection.getState().set([ref]);
  }, seed);

  await page.keyboard.down("Shift");
  await page.mouse.dblclick(face.x, face.y);
  await page.keyboard.up("Shift");

  expect(await selectedRefs(page)).toEqual(
    expect.arrayContaining([seed, { kind: "body", id: face.bodyId }]),
  );
  expect((await selectedRefs(page)).length).toBe(2);

  // Shift+double-click the SAME body again removes it (toggle), leaving the seed.
  await page.keyboard.down("Shift");
  await page.mouse.dblclick(face.x, face.y);
  await page.keyboard.up("Shift");

  expect(await selectedRefs(page)).toEqual([seed]);
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
