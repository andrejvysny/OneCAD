import { test, expect } from "./fixtures";
import type { Page } from "@playwright/test";
import {
  hideSeedSketches,
  openEditorDebug,
  enterSketchViaPlanePicker,
  waitForCameraSettled,
  selectSketchTool,
  clickAt,
  clickAtAwaitingDofChange,
  clickAtClient,
  dofPill,
  bodyOptions,
  extrudeDebug,
  commitExtrudeAtHandle,
  getSketchCircle,
  getSketchSnapshot,
  planePointToClient,
  type SketchPlaneSnapshot,
} from "./helpers";

/*
 * A nested loop defines two selectable planar cells:
 *   - the circle disc;
 *   - the rectangle cell with the circle as a hole.
 *
 * Both tests select through the persistent static fill, invoke Extrude, and
 * inspect the committed face mesh. The projected cap must cover the circle
 * centre for the disc and must not cover it for the annulus. Body count alone
 * would not detect a silent first-region fallback.
 */

interface NestedSketch {
  plane: SketchPlaneSnapshot;
  center: { x: number; y: number };
  annulusPoint: { x: number; y: number };
}

/** Draw the nested profile and STOP inside the sketch session, before Enter. */
async function drawNestedSketchOpen(page: Page): Promise<NestedSketch> {
  await openEditorDebug(page);
  await bodyOptions(page).first().getByRole("switch").click();
  await hideSeedSketches(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -220, -140);
  await clickAt(page, 220, 140);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  await selectSketchTool(page, "Circle");
  await clickAt(page, 0, 0);
  await clickAtAwaitingDofChange(page, 60, 0);

  const snap = await getSketchSnapshot(page);
  const circle = await getSketchCircle(page);
  const xs = snap.lines.flatMap((line) => [line.p0[0], line.p1[0]]);
  const annulusPoint = {
    x: Math.min(...xs) + (Math.max(...xs) - Math.min(...xs)) * 0.15,
    y: circle.center[1],
  };
  return {
    plane: snap.plane,
    center: { x: circle.center[0], y: circle.center[1] },
    annulusPoint,
  };
}

async function drawNestedSketch(page: Page): Promise<NestedSketch> {
  const sketch = await drawNestedSketchOpen(page);
  await page.keyboard.press("Enter");
  await waitForCameraSettled(page);
  return sketch;
}

/**
 * The LIVE closure fills currently drawn by the open session: how many meshes,
 * and whether their triangles cover a given plane (u,v) point.
 *
 * The canvas has no per-entity DOM, so this reads the engine's own scene under
 * `?vpdebug`. Fill geometry is plane-LOCAL — the same (u,v) frame the sketch
 * snapshot reports — so no basis transform is needed here.
 */
async function liveFillCoverage(
  page: Page,
  point: { x: number; y: number },
): Promise<{ meshes: number; covered: boolean }> {
  return page.evaluate((p) => {
    const w = window as unknown as {
      __vpEngine?: {
        sketchRoot: {
          traverse(cb: (o: {
            name: string;
            geometry?: {
              index?: { array: ArrayLike<number> } | null;
              attributes?: { position?: { array: ArrayLike<number> } };
            };
          }) => void): void;
        };
      };
    };
    if (!w.__vpEngine) throw new Error("__vpEngine unavailable (needs ?vpdebug)");
    let meshes = 0;
    let covered = false;
    w.__vpEngine.sketchRoot.traverse((o) => {
      if (o.name !== "sketchActiveFill") return;
      meshes++;
      const pos = o.geometry?.attributes?.position?.array;
      const idx = o.geometry?.index?.array;
      if (!pos || !idx) return;
      const at = (v: number): [number, number] => [Number(pos[v * 3]), Number(pos[v * 3 + 1])];
      for (let t = 0; t < idx.length; t += 3) {
        const a = at(Number(idx[t]));
        const b = at(Number(idx[t + 1]));
        const c = at(Number(idx[t + 2]));
        const sign = (q: [number, number], r: [number, number]): number =>
          (p.x - r[0]) * (q[1] - r[1]) - (q[0] - r[0]) * (p.y - r[1]);
        const d1 = sign(a, b);
        const d2 = sign(b, c);
        const d3 = sign(c, a);
        const neg = d1 < 0 || d2 < 0 || d3 < 0;
        const pos3 = d1 > 0 || d2 > 0 || d3 > 0;
        if (!(neg && pos3)) covered = true;
      }
    });
    return { meshes, covered };
  }, point);
}

async function documentBodyIds(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const store = (window as unknown as {
      __stores?: { document: { getState(): { bodies: Record<string, unknown> } } };
    }).__stores?.document;
    if (!store) throw new Error("document store unavailable");
    return Object.keys(store.getState().bodies);
  });
}

async function selectRegionAndArm(
  page: Page,
  plane: SketchPlaneSnapshot,
  point: { x: number; y: number },
): Promise<void> {
  const client = await planePointToClient(page, plane, point);
  await expect
    .poll(() =>
      page.evaluate(
        ({ x, y }) => Boolean((window as unknown as { __vpEngine?: { sketchStaticHitTest(x: number, y: number): unknown } })
          .__vpEngine?.sketchStaticHitTest(x, y)),
        client,
      ),
    )
    .toBe(true);
  await clickAtClient(page, client.x, client.y);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as {
            __stores?: { selection: { getState(): { selected: Array<{ kind: string }> } } };
          }).__stores?.selection.getState().selected[0]?.kind,
      ),
    )
    .toBe("sketchRegion");
  await page.getByRole("button", { name: "Extrude", exact: true }).click();
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();
}

async function committedCapCovers(
  page: Page,
  previousBodyIds: string[],
  plane: SketchPlaneSnapshot,
  point: { x: number; y: number },
): Promise<boolean> {
  let result: boolean | null = null;
  await expect(async () => {
    result = await page.evaluate(
      ({ previousBodyIds, plane, point }) => {
        const w = window as unknown as {
          __stores?: { document: { getState(): { bodies: Record<string, unknown> } } };
          __vpEngine?: {
            bodiesRoot: {
              children: Array<{
                userData: { bodyId?: string };
                children: Array<{
                  userData: { kind?: string };
                  geometry?: {
                    index?: { array: ArrayLike<number> } | null;
                    attributes?: { position?: { array: ArrayLike<number> } };
                  };
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
        const mesh = group?.children.find((child) => child.userData.kind === "face");
        const positions = mesh?.geometry?.attributes?.position?.array;
        if (!mesh || !positions) return null;
        mesh.updateMatrixWorld(true);
        const indices = mesh.geometry?.index?.array;
        const m = mesh.matrixWorld.elements;
        const [ox, oy, oz] = plane.origin;
        const [xx, xy, xz] = plane.xAxis;
        const [yx, yy, yz] = plane.yAxis;
        const uv = (vertex: number): [number, number] => {
          const i = vertex * 3;
          const px = Number(positions[i]);
          const py = Number(positions[i + 1]);
          const pz = Number(positions[i + 2]);
          const wx = m[0] * px + m[4] * py + m[8] * pz + m[12];
          const wy = m[1] * px + m[5] * py + m[9] * pz + m[13];
          const wz = m[2] * px + m[6] * py + m[10] * pz + m[14];
          const dx = wx - ox;
          const dy = wy - oy;
          const dz = wz - oz;
          return [dx * xx + dy * xy + dz * xz, dx * yx + dy * yy + dz * yz];
        };
        const triangleCount = indices ? Math.floor(indices.length / 3) : Math.floor(positions.length / 9);
        for (let tri = 0; tri < triangleCount; tri++) {
          const vertex = (corner: number): number =>
            indices ? Number(indices[tri * 3 + corner]) : tri * 3 + corner;
          const a = uv(vertex(0));
          const b = uv(vertex(1));
          const c = uv(vertex(2));
          const area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
          if (Math.abs(area) < 1e-7) continue;
          const sign = (
            p: { x: number; y: number },
            p0: [number, number],
            p1: [number, number],
          ): number => (p.x - p1[0]) * (p0[1] - p1[1]) - (p0[0] - p1[0]) * (p.y - p1[1]);
          const d1 = sign(point, a, b);
          const d2 = sign(point, b, c);
          const d3 = sign(point, c, a);
          const hasNegative = d1 < -1e-6 || d2 < -1e-6 || d3 < -1e-6;
          const hasPositive = d1 > 1e-6 || d2 > 1e-6 || d3 > 1e-6;
          if (!(hasNegative && hasPositive)) return true;
        }
        return false;
      },
      { previousBodyIds, plane, point },
    );
    expect(result).not.toBeNull();
  }).toPass({ timeout: 10_000, intervals: [100, 200, 400] });
  return result as unknown as boolean;
}

test("the LIVE closure fill leaves the donut's hole unpainted", async ({ page }) => {
  // Mid-session, before any region exists: the fills are the only "this closes"
  // signal on screen, and the v1 pass painted the circle's disc on top of the
  // rectangle's, so the hole read as material.
  const sketch = await drawNestedSketchOpen(page);
  await expect.poll(async () => (await liveFillCoverage(page, sketch.annulusPoint)).meshes).toBe(1);
  expect((await liveFillCoverage(page, sketch.annulusPoint)).covered).toBe(true);
  expect((await liveFillCoverage(page, sketch.center)).covered).toBe(false);
});

test("nested inner disc is independently selectable and commits a solid cap", async ({ page }) => {
  const sketch = await drawNestedSketch(page);
  const bodyIdsBefore = await documentBodyIds(page);
  const bodiesBefore = await bodyOptions(page).count();

  await selectRegionAndArm(page, sketch.plane, sketch.center);
  await expect.poll(async () => (await extrudeDebug(page))?.profileHoleCounts).toEqual([0]);
  await commitExtrudeAtHandle(page);

  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
  expect(await committedCapCovers(page, bodyIdsBefore, sketch.plane, sketch.center)).toBe(true);
});

test("nested outer cell keeps the inner loop as a hole in preview and commit", async ({ page }) => {
  const sketch = await drawNestedSketch(page);
  const bodyIdsBefore = await documentBodyIds(page);
  const bodiesBefore = await bodyOptions(page).count();

  await selectRegionAndArm(page, sketch.plane, sketch.annulusPoint);
  await expect.poll(async () => (await extrudeDebug(page))?.profileHoleCounts).toEqual([1]);
  await commitExtrudeAtHandle(page);

  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
  expect(await committedCapCovers(page, bodyIdsBefore, sketch.plane, sketch.center)).toBe(false);
});
