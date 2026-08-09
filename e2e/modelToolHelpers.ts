/*
 * Helpers for the value-drag model tools (fillet / chamfer / shell) in the mock
 * lane. Kept OUT of `helpers.ts` so the edge-op wave owns one file; Playwright
 * forbids a spec importing another spec, hence a module of its own.
 */
import { expect, type Page } from "@playwright/test";
import { CANVAS } from "./helpers";

/**
 * Seed a face/edge selection straight into the selection store.
 *
 * There is no per-entity DOM in the WebGL viewport and the mock lane publishes no
 * body meshes to raycast against, so an edge pick cannot be produced by clicking.
 * The store IS the seam the controller reads (`armEdgeOpFromSelection` /
 * `armShellFromSelection`), and `__stores` is the dev-only surface the other specs
 * already drive.
 */
export async function seedSelection(
  page: Page,
  refs: Array<Record<string, unknown>>,
): Promise<void> {
  await page.evaluate((entries) => {
    const w = window as unknown as {
      __stores?: { selection: { getState(): { set(refs: unknown[]): void } } };
    };
    if (!w.__stores) throw new Error("__stores unavailable (dev build only)");
    w.__stores.selection.getState().set(entries);
  }, refs);
}

/**
 * The model-tool debug surface (`?vpdebug`), which carries both value-drag
 * phases. `edgeOpAuto`/`edgeOpAxisSource` expose the FILLET-CHAMFER-UNIFY rule
 * that binds them: the drag direction may re-type the edge op ONLY on the
 * bisector tier. The seeded mock document publishes the box body's MESH1
 * through the ingest path, so `body1`'s edges DO reach that tier here.
 */
interface ToolDebugSurface {
  filletPhase?: string;
  shellPhase?: string;
  edgeOpKind?: string;
  edgeOpAuto?: boolean;
  edgeOpAxisSource?: string;
  /** The CHAMFER second leg (`null` = equal-leg — SCHEMA §7.3, 2026-08-03). */
  edgeOpDistance2?: number | null;
  // ── OffsetFace (SCHEMA §7.3 + §7.6) ────────────────────────────────────────
  offsetFacePhase?: string;
  offsetDistance?: number;
  offsetDistanceType?: string;
  offsetChainTangent?: boolean;
  /**
   * The FROZEN operative closure the record will hold — picks ∪ the worker's G1
   * tangent chain. Distinct from the number of faces the user selected, which is
   * the entire point of the `PrepareOffsetFace` handshake and has no other surface.
   */
  offsetFaceCount?: number;
  /**
   * The document revision the newest successful handshake answered against, or
   * `null` when there is none. This IS the commit gate: a ✓ with no valid
   * handshake, or one frozen against an older head, is refused.
   */
  offsetPrepared?: number | null;
  /** True when the 3D arrow + ghost lane is unavailable (curved / diverging faces). */
  offsetDegraded?: boolean;
  offsetValueError?: boolean;
}

export async function toolPhases(page: Page): Promise<ToolDebugSurface | undefined> {
  return page.evaluate(
    () => (window as unknown as { __extrudePreview?: ToolDebugSurface }).__extrudePreview,
  );
}

/**
 * A press-drag-release inside the canvas. An armed edge op / shell claims EVERY
 * viewport press as a value drag (there is no handle to hit-test — which is
 * exactly why these two tools have no click-away commit), so the offset only has
 * to clear the tool chip, which is anchored at the pick's world point.
 */
export async function dragInCanvas(page: Page, dx: number, dy: number): Promise<void> {
  const box = await page.locator(CANVAS).boundingBox();
  if (!box) throw new Error("viewport canvas has no bounding box");
  const x = box.x + box.width / 2 + dx;
  const y = box.y + box.height / 2 + dy;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x, y - 40, { steps: 4 });
  await page.mouse.up();
}

/**
 * Press at the canvas CENTRE and drag by `(dx, dy)` before releasing. Unlike
 * `dragInCanvas` (which offsets the press point and always drags 40px up), the
 * delta here is the whole gesture — the edge-op direction lane measures the
 * value from the press point, so the test has to own both ends of it.
 */
export async function dragBy(page: Page, dx: number, dy: number): Promise<void> {
  const box = await page.locator(CANVAS).boundingBox();
  if (!box) throw new Error("viewport canvas has no bounding box");
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx, y + dy, { steps: 4 });
  await page.mouse.up();
}

/** One placement-gizmo handle (WP-B W2), as `hitTransformGizmo` reports it. */
export interface GizmoHandle {
  kind: "axis" | "plane" | "ring";
  axis: "X" | "Y" | "Z";
}

/**
 * Hit-scan the canvas for a real client point that grabs `want` on the placement
 * gizmo, through the engine's OWN `hitTransformGizmo` raycast (exposed at
 * `window.__vpEngine` under `?vpdebug`).
 *
 * Same rationale as `findExtrudeHandle`: the gizmo is screen-scaled 3D geometry
 * at a world pivot, so its arms land wherever the camera puts them and there is
 * no closed-form pixel offset to click. Scanning with the exact raycast the
 * pointer handlers use is the only way to produce a GENUINE grab — and it also
 * proves the arbitration, since a point that scans as the Z ring is a point the
 * real pointerdown will classify as the Z ring.
 */
export async function findGizmoHandle(page: Page, want: GizmoHandle): Promise<{ x: number; y: number }> {
  let found: { x: number; y: number } | null = null;
  await expect(async () => {
    found = await page.evaluate((target) => {
      const engine = (
        window as unknown as {
          __vpEngine?: { hitTransformGizmo(x: number, y: number): GizmoHandle | null };
        }
      ).__vpEngine;
      const canvas = document.querySelector(
        '[data-testid="viewport-canvas"] canvas',
      ) as HTMLCanvasElement | null;
      if (!engine || !canvas) return null;
      const rect = canvas.getBoundingClientRect();
      const step = 3; // the arms are only a few px wide on screen
      for (let y = rect.top; y <= rect.bottom; y += step) {
        for (let x = rect.left; x <= rect.right; x += step) {
          const hit = engine.hitTransformGizmo(x, y);
          if (hit && hit.kind === target.kind && hit.axis === target.axis) return { x, y };
        }
      }
      return null;
    }, want);
    expect(found).not.toBeNull();
  }).toPass({ timeout: 15_000, intervals: [200, 400, 800] });
  return found as unknown as { x: number; y: number };
}

/**
 * Press at `from`, move to `to` in steps, release. `modifiers` are held for the
 * WHOLE gesture — Alt at press is what turns a placement drag into a copy.
 */
export async function dragFromTo(
  page: Page,
  from: { x: number; y: number },
  to: { x: number; y: number },
  modifiers: Array<"Alt" | "Shift"> = [],
): Promise<void> {
  for (const m of modifiers) await page.keyboard.down(m);
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(to.x, to.y, { steps: 8 });
  await page.mouse.up();
  for (const m of modifiers) await page.keyboard.up(m);
}

/**
 * Hit-scan the canvas for a client point that picks face `topoKey` of `bodyId`,
 * through the engine's OWN `probePick` raycast (`window.__vpEngine`, `?vpdebug`).
 *
 * Same rationale as `findGizmoHandle`: there is no per-face DOM in a WebGL
 * viewport and the camera decides where a face lands on screen, so the only way
 * to produce a GENUINE face click is to ask the very raycast the controller's
 * pointer handler will run. A point that scans as `body1/f:0` is a point the
 * real pointerup classifies as `body1/f:0`.
 *
 * Faces are large, so the 8px step is plenty (the gizmo scan needs 3px because
 * its arms are a few pixels wide). Only primitives cross the bridge — a
 * `PickHit` carries a `THREE.Vector3` that Playwright cannot serialise.
 *
 * THE CANVAS MUST BE THE TOPMOST ELEMENT at the returned point. The canvas runs
 * the full width of the shell and the inspector panel floats OVER its right-hand
 * 260px, so a raycast happily reports geometry at points where a real mouse
 * click would land on the panel instead and never reach the viewport's pointer
 * handlers at all. `elementFromPoint` is the same arbitration the browser does.
 */
export async function findFacePoint(
  page: Page,
  bodyId: string,
  topoKey: string,
): Promise<{ x: number; y: number }> {
  let found: { x: number; y: number } | null = null;
  await expect(async () => {
    found = await page.evaluate(
      ({ bodyId: wantBody, topoKey: wantFace }) => {
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
        const step = 8;
        for (let y = rect.top + step; y <= rect.bottom - step; y += step) {
          for (let x = rect.left + step; x <= rect.right - step; x += step) {
            const hit = engine.probePick(x, y);
            if (!hit || hit.kind !== "face" || hit.bodyId !== wantBody || hit.topoKey !== wantFace) {
              continue;
            }
            if (document.elementFromPoint(x, y) !== canvas) continue; // occluded by DOM
            return { x, y };
          }
        }
        return null;
      },
      { bodyId, topoKey },
    );
    expect(found, `no screen point picks ${bodyId}/${topoKey}`).not.toBeNull();
  }).toPass({ timeout: 15_000, intervals: [200, 400, 800] });
  return found as unknown as { x: number; y: number };
}

/** A press + release at one point — a CLICK, not a drag (align picks, face picks). */
export async function clickAt(page: Page, at: { x: number; y: number }): Promise<void> {
  await page.mouse.move(at.x, at.y);
  await page.mouse.down();
  await page.mouse.up();
}

/** Wait until the named model tool reports `armed` on the debug surface. */
export async function expectArmed(
  page: Page,
  which: "fillet" | "shell" | "offsetFace",
): Promise<void> {
  await expect
    .poll(async () => {
      const d = await toolPhases(page);
      if (which === "fillet") return d?.filletPhase;
      if (which === "shell") return d?.shellPhase;
      return d?.offsetFacePhase;
    })
    .toBe("armed");
}

/**
 * Open the armed EXTRUDE chip's `⋯` overflow, where the end conditions, draft,
 * symmetric toggle and boolean segments now live — the collapsed chip carries a
 * dimension and nothing else.
 *
 * Idempotent: the button toggles, so it is only clicked when the panel is closed.
 */
export async function openExtrudeOverflow(page: Page): Promise<void> {
  const panel = page.getByTestId("chip-overflow-panel");
  if (await panel.isVisible().catch(() => false)) return;
  await page.getByTestId("chip-overflow").click();
  await expect(panel).toBeVisible();
}

/** Close the extrude `⋯` overflow if it is open (a floating panel can cover the
 *  arrow, and a press on the panel is deliberately NOT a depth grab). */
export async function closeExtrudeOverflow(page: Page): Promise<void> {
  const panel = page.getByTestId("chip-overflow-panel");
  if (!(await panel.isVisible().catch(() => false))) return;
  await page.getByTestId("chip-overflow").click();
  await expect(panel).toBeHidden();
}
