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

/** Wait until the named model tool reports `armed` on the debug surface. */
export async function expectArmed(page: Page, which: "fillet" | "shell"): Promise<void> {
  await expect
    .poll(async () => {
      const d = await toolPhases(page);
      return which === "fillet" ? d?.filletPhase : d?.shellPhase;
    })
    .toBe("armed");
}
