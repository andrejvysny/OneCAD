import { test, expect } from "@playwright/test";
import {
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtClient,
  dofPill,
  bodyOptions,
  getSketchSnapshot,
  planePointToClient,
  commitExtrudeAtHandle,
} from "./helpers";

/*
 * Non-first-region extrude (mock lane): a separated circle is published first
 * and the rectangle second. This selects the rectangle's persistent fill,
 * invokes Extrude, then proves the committed mesh has the rectangle's exact
 * plane-local bounds. A body-count-only assertion would miss a first-region
 * fallback.
 *
 * ADAPTATION vs the original design note ("click inside the circle"): verified
 * against `detectRegions` (src/ipc/mockSketch.ts) — it scans ALL entities for
 * Circles first and pushes a region per circle, THEN appends the (at most one)
 * closed line/arc loop's region — so for one circle + one rectangle the mock
 * ALWAYS returns `regions = [circleRegion, rectRegion]` regardless of draw
 * order. The circle is therefore always `regions[0]` (the FIRST region), and
 * the rectangle is always `regions[1]` (the actual non-first one). Clicking the
 * circle would exercise the already-well-covered first-region path, so this
 * spec clicks the RECTANGLE instead to genuinely prove the non-first pick.
 *
 * Targeting: the click point is the rectangle's centroid computed from its
 * REAL line entities (`getSketchSnapshot`, read BEFORE finishing — the session
 * clears on leaving sketch mode) and projected through the live camera
 * (`planePointToClient`), not a guessed canvas-relative offset — `screenToPlane`
 * is a real perspective raycast, not affine, so a pixel-offset guess for a point
 * never directly clicked (like a shape's centroid) is unreliable (verified
 * empirically against an earlier, offset-based version of this spec).
 *
 * Camera-settle note (two different tweens):
 * entering the plane picker kicks off an animated re-home, so drawing must not
 * race it (`waitForCameraSettled` right after `enterSketchViaPlanePicker`).
 * SEPARATELY, `ViewportEngine.exitSketch()` — run when Enter finishes the sketch
 * — animates the camera BACK to whatever view was active before the sketch was
 * entered. The plane's own basis (origin/xAxis/yAxis) is fixed and safe to read
 * before finishing, but the click point must be projected through the camera
 * pose AFTER that restore tween completes — projecting with the sketch-mode
 * pose and clicking after the restore targets the wrong screen point. Hence
 * `waitForCameraSettled` again after Enter, before `planePointToClient`.
 */

interface PlaneBounds {
  minU: number;
  maxU: number;
  minV: number;
  maxV: number;
}

async function documentBodyIds(page: import("@playwright/test").Page): Promise<string[]> {
  return page.evaluate(() => {
    const store = (window as unknown as {
      __stores?: { document: { getState(): { bodies: Record<string, unknown> } } };
    }).__stores?.document;
    if (!store) throw new Error("document store unavailable");
    return Object.keys(store.getState().bodies);
  });
}

async function committedBodyPlaneBounds(
  page: import("@playwright/test").Page,
  previousBodyIds: string[],
  plane: {
    origin: [number, number, number];
    xAxis: [number, number, number];
    yAxis: [number, number, number];
  },
): Promise<PlaneBounds> {
  let bounds: PlaneBounds | null = null;
  await expect(async () => {
    bounds = await page.evaluate(
      ({ previousBodyIds, plane }) => {
        const w = window as unknown as {
          __stores?: { document: { getState(): { bodies: Record<string, unknown> } } };
          __vpEngine?: {
            bodiesRoot: {
              children: Array<{
                userData: { bodyId?: string };
                children: Array<{
                  geometry?: { attributes?: { position?: { array: ArrayLike<number> } } };
                  matrixWorld: { elements: number[] };
                  updateMatrixWorld(force: boolean): void;
                }>;
              }>;
            };
          };
        };
        const ids = Object.keys(w.__stores?.document.getState().bodies ?? {});
        const bodyId = ids.find((id) => !previousBodyIds.includes(id));
        const group = w.__vpEngine?.bodiesRoot.children.find((child) => child.userData.bodyId === bodyId);
        const mesh = group?.children.find((child) => child.geometry?.attributes?.position);
        const positions = mesh?.geometry?.attributes?.position?.array;
        if (!mesh || !positions) return null;
        mesh.updateMatrixWorld(true);
        const m = mesh.matrixWorld.elements;
        const [ox, oy, oz] = plane.origin;
        const [xx, xy, xz] = plane.xAxis;
        const [yx, yy, yz] = plane.yAxis;
        let minU = Infinity;
        let maxU = -Infinity;
        let minV = Infinity;
        let maxV = -Infinity;
        for (let i = 0; i < positions.length; i += 3) {
          const px = Number(positions[i]);
          const py = Number(positions[i + 1]);
          const pz = Number(positions[i + 2]);
          const wx = m[0] * px + m[4] * py + m[8] * pz + m[12];
          const wy = m[1] * px + m[5] * py + m[9] * pz + m[13];
          const wz = m[2] * px + m[6] * py + m[10] * pz + m[14];
          const dx = wx - ox;
          const dy = wy - oy;
          const dz = wz - oz;
          const u = dx * xx + dy * xy + dz * xz;
          const v = dx * yx + dy * yy + dz * yz;
          minU = Math.min(minU, u);
          maxU = Math.max(maxU, u);
          minV = Math.min(minV, v);
          maxV = Math.max(maxV, v);
        }
        return { minU, maxU, minV, maxV };
      },
      { previousBodyIds, plane },
    );
    expect(bounds).not.toBeNull();
  }).toPass({ timeout: 10_000, intervals: [100, 200, 400] });
  return bounds as unknown as PlaneBounds;
}

test("multi-region sketch: picking the non-first region (the rectangle) arms and commits an extrude", async ({
  page,
}) => {
  await openEditorDebug(page);
  await bodyOptions(page).first().getByRole("switch").click();
  const visibleSeedSketches = page
    .getByRole("listbox", { name: "Sketches" })
    .locator('[role="switch"][aria-checked="true"]');
  while ((await visibleSeedSketches.count()) > 0) await visibleSeedSketches.first().click();
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  // Rectangle — its own closed loop, well clear of the circle drawn below.
  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -260, -150);
  await clickAt(page, -60, 50);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  // Circle — separated in screen (and therefore plane) space from the rectangle.
  await selectSketchTool(page, "Circle");
  await clickAt(page, 120, -80); // center
  await clickAt(page, 200, -80); // radius point (radius 80)
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  // Snapshot BEFORE finishing: sketchStore.session clears once mode leaves "sketch".
  const snap = await getSketchSnapshot(page);
  expect(snap.lines).toHaveLength(4); // the rectangle only — circle isn't a Line entity
  // The rectangle's centroid: each of its 4 unique corners is shared by exactly 2
  // lines, so averaging ALL 8 raw endpoints (each corner counted twice, uniformly)
  // equals the average of the 4 unique corners — no dedup needed.
  const sum = snap.lines.reduce(
    (acc, l) => ({ x: acc.x + l.p0[0] + l.p1[0], y: acc.y + l.p0[1] + l.p1[1] }),
    { x: 0, y: 0 },
  );
  const centroid = { x: sum.x / (snap.lines.length * 2), y: sum.y / (snap.lines.length * 2) };
  const rectangleBounds: PlaneBounds = {
    minU: Math.min(...snap.lines.flatMap((line) => [line.p0[0], line.p1[0]])),
    maxU: Math.max(...snap.lines.flatMap((line) => [line.p0[0], line.p1[0]])),
    minV: Math.min(...snap.lines.flatMap((line) => [line.p0[1], line.p1[1]])),
    maxV: Math.max(...snap.lines.flatMap((line) => [line.p0[1], line.p1[1]])),
  };

  const bodiesBefore = await bodyOptions(page).count();
  const bodyIdsBefore = await documentBodyIds(page);

  // Finish leaves model Select active. Static fill publication is asynchronous.
  await page.keyboard.press("Enter");
  await waitForCameraSettled(page);

  const clientPt = await planePointToClient(page, snap.plane, centroid);
  await expect
    .poll(() =>
      page.evaluate(
        ({ x, y }) => Boolean((window as unknown as { __vpEngine?: { sketchStaticHitTest(x: number, y: number): unknown } })
          .__vpEngine?.sketchStaticHitTest(x, y)),
        clientPt,
      ),
    )
    .toBe(true);
  await clickAtClient(page, clientPt.x, clientPt.y);
  const selectedRegion = await page.evaluate(() => {
    const selection = (window as unknown as {
      __stores?: {
        selection: {
          getState(): {
            selected: Array<{ kind: string; sketchId?: string; regionId?: string }>;
          };
        };
      };
    }).__stores?.selection.getState().selected;
    return selection?.[0] ?? null;
  });
  expect(selectedRegion?.kind).toBe("sketchRegion");
  expect(selectedRegion?.sketchId).toBeTruthy();
  expect(selectedRegion?.regionId).toBeTruthy();
  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();

  await commitExtrudeAtHandle(page);
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
  await expect(bodyOptions(page).last()).toHaveAttribute("aria-selected", "true");

  const committed = await committedBodyPlaneBounds(page, bodyIdsBefore, snap.plane);
  expect(committed.minU).toBeCloseTo(rectangleBounds.minU, 3);
  expect(committed.maxU).toBeCloseTo(rectangleBounds.maxU, 3);
  expect(committed.minV).toBeCloseTo(rectangleBounds.minV, 3);
  expect(committed.maxV).toBeCloseTo(rectangleBounds.maxV, 3);
});
