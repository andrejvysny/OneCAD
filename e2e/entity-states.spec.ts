import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  clickAt,
  clickAtAwaitingDofChange,
  clickAtClient,
  findFaceOnBody,
  getSketchEntityCount,
  hideSeedSketches,
  openEditorDebug,
  selectSketchTool,
  waitForCameraSettled,
} from "./helpers";

/*
 * PER-ENTITY constrained state end to end (SCHEMA §7.4 `entityStates`).
 *
 * The map flows client → sketchStore → ViewportEngine → SketchObject, where each
 * entity the solver diagnosed gets its own color and an entity the solver said
 * NOTHING about keeps the whole-sketch tint. This spec drives the mock lane, so
 * it asserts exactly what that lane can HONESTLY produce.
 *
 * WHAT THE MOCK CAN SAY, and why it is only this:
 *   - `fullyConstrained` for `referenceLocked` geometry — a projected host-face
 *     boundary is pinned by machine `Fixed` constraints and provably cannot
 *     move. That is the case this spec drives.
 *   - ABSENCE for everything else. This lane's DOF is a coarse Σ-heuristic with
 *     no Jacobian behind it, so it deliberately never claims a per-entity answer
 *     for user geometry (`mockSketch.mockEntityStates`).
 *
 * WHY THERE IS NO `conflicting` CASE HERE. The mock CAN project a conflict onto
 * the entities a clashing constraint names, but no browser flow can hold that
 * state still: every frontend constraint path is reject-on-conflict — it
 * re-upserts the PRIOR constraints and writes back the restored solve, so the
 * conflicting result never reaches a store or the engine at all. Inventing a
 * flow that parks a red sketch on screen would be testing a UI the product does
 * not have. That projection is pinned where it is real instead:
 * `src/ipc/mockSketch.test.ts` (the derivation, including the deliberate
 * over-attribution), `src/ipc/localSolver.test.ts` (the lane carrying it on
 * upsert/begin/end) and `src/viewport/engine/SketchObject.test.ts` (the color).
 *
 * ASSERTIONS ARE STRUCTURAL, per the mock false-green rule: the store map by
 * content, and the viewport by MATERIAL IDENTITY (which shared LineMaterial each
 * stroke draws with) rather than by color literal — there is no raw hex in this
 * repo outside `tokens.css`, and a spec that hardcoded one would be pinning the
 * theme, not the state.
 */

/** The live per-entity map (`__stores.sketch`, dev-only). */
async function entityStates(page: Page): Promise<Record<string, string>> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { sketch: { getState(): { entityStates: Record<string, string> } } };
    };
    const states = w.__stores?.sketch.getState().entityStates;
    if (!states) throw new Error("no sketch store");
    return states;
  });
}

/** Ids of the `referenceLocked` entities in the live session. */
async function lockedIds(page: Page): Promise<string[]> {
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

/**
 * How many DISTINCT materials the active session's committed strokes are drawn
 * with, and how many strokes there are — read off the engine's own scene graph.
 * The materials are shared per state, so distinct-material-count IS the number
 * of visually distinct entity states on screen.
 */
async function strokeMaterials(page: Page): Promise<{ strokes: number; distinct: number }> {
  return page.evaluate(() => {
    const engine = (window as unknown as { __vpEngine?: { sketchRoot: unknown } }).__vpEngine;
    if (!engine) throw new Error("no __vpEngine (needs ?vpdebug)");
    const mats = new Set<unknown>();
    let strokes = 0;
    const walk = (o: unknown): void => {
      const node = o as {
        children?: unknown[];
        material?: unknown;
        isLine2?: boolean;
        userData?: Record<string, unknown>;
      };
      // Committed entity strokes only: not the selection halo, not the dimension
      // witnesses (both are separate underlays with their own materials).
      if (node.isLine2 && !node.userData?.selectionHalo && !node.userData?.dimLineWitness) {
        strokes++;
        mats.add(node.material);
      }
      for (const child of node.children ?? []) walk(child);
    };
    walk(engine.sketchRoot);
    return { strokes, distinct: mats.size };
  });
}

test("a face sketch's projected boundary is fullyConstrained; the line you draw is unknown", async ({
  page,
}) => {
  await openEditorDebug(page, { mockBody: true });
  // The seeded sketches' static fills sit at the origin, inside the box, and
  // would arbitrate against the face pick. The body stays visible.
  await hideSeedSketches(page);

  const face = await findFaceOnBody(page);
  await clickAtClient(page, face.x, face.y);
  await page.keyboard.press("s");
  await expect(page.getByText(/^Editing /)).toBeVisible({ timeout: 10_000 });
  await waitForCameraSettled(page);

  // ── (a) the entering session SEEDS the map ─────────────────────────────────
  const locked = await lockedIds(page);
  expect(locked, "the host face's projected boundary should be seeded as locked").toHaveLength(4);
  expect(await getSketchEntityCount(page)).toBe(4);

  const seeded = await entityStates(page);
  expect(Object.keys(seeded).slice().sort()).toEqual(locked.slice().sort());
  for (const id of locked) expect(seeded[id]).toBe("fullyConstrained");

  // ── (b) the boundary keeps its REFERENCE color, not the fully-constrained one ─
  // Precedence head (§ SketchObject.rebuildEntities): reference geometry outranks
  // the solver's answer, because "not yours to move" is a stronger statement than
  // "pinned down". Four locked strokes ⇒ ONE material, which is the reference one.
  const before = await strokeMaterials(page);
  expect(before.strokes).toBe(4);
  expect(before.distinct, "locked geometry must not split by constrained state").toBe(1);

  // ── (c) a line the user draws is ABSENT from the map — unknown, not blue-by-default ─
  await selectSketchTool(page, "Line");
  await clickAt(page, -40, -30);
  await clickAtAwaitingDofChange(page, 40, 30);
  await page.keyboard.press("Escape");

  expect(await getSketchEntityCount(page)).toBe(5);
  const after = await entityStates(page);
  // The map is REPLACED on every solve, and the boundary is still the only thing
  // this lane can honestly speak for.
  expect(Object.keys(after).slice().sort()).toEqual(locked.slice().sort());
  for (const id of locked) expect(after[id]).toBe("fullyConstrained");

  // Five strokes now, in TWO materials: the four locked ones (reference) and the
  // drawn line, which falls back to the whole-sketch tint because it is unknown.
  const drawn = await strokeMaterials(page);
  expect(drawn.strokes).toBe(5);
  expect(drawn.distinct).toBe(2);
});
