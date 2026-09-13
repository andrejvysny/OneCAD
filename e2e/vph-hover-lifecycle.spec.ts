import { promises as fs } from "node:fs";
import { test, expect } from "./fixtures";
import type { Page, TestInfo } from "@playwright/test";
import { openEditorDebug } from "./helpers";
import { findFacePoint, waitForRenderedFrame } from "./modelToolHelpers";

// See e2e/vph-line-width.spec.ts's `attachJson` — `testInfo.attach(name,
// {body})` is never written to disk under this project's "list"/"line"
// reporter (e2e/fixtures.ts documents why); write the file and push it onto
// `testInfo.attachments` directly, and echo to stdout for a `tee`'d run log.
async function attachJson(testInfo: TestInfo, name: string, value: unknown): Promise<void> {
  const body = JSON.stringify(value, null, 2);
  const path = testInfo.outputPath(name);
  await fs.writeFile(path, body, "utf8");
  testInfo.attachments.push({ name, contentType: "application/json", path });
  console.log(`[${name}]`, body);
}

/*
 * TEST-RES-01 — VP-HARDENING R02 baseline probe.
 *
 * Repeatedly hovers between two distinct faces of the mock box body and reads
 * the engine's own resource bookkeeping (`?vpdebug`'s
 * `window.__vpEngine.debugResourceCounters()`) every 50 iterations, over 300
 * REAL rendered hovers. A face highlight is a per-hover wrapper object built
 * around the picked geometry; if it is never disposed on hover-away, renderer
 * geometry count keeps climbing instead of plateauing once warmed up — a
 * classic never-torn-down-overlay leak that a single hover/unhover pair can't
 * show.
 *
 * History: at the design baseline (65b4c60) this measured +2 renderer
 * geometries per hover with no plateau (baseline/vph-g-lane-probes-at-65b4c60.log,
 * 11 → 611 over 300 hovers). After WP03 (owned, cached face overlays; leased
 * whole-body overlays; frame-ordered retirement) the count holds flat after
 * warm-up (11 → 13, then 13 at every sample). The test states the required
 * behaviour — count at i=300 equals count at i=100 — and preserves the series
 * via `testInfo.attach`.
 */

interface ResourceCounters {
  renderer: { geometries: number; textures: number; programs: number } | null;
  viewport: {
    meshEntriesBuilt: number;
    meshEntriesRetired: number;
    pickQueries: number;
    [k: string]: number;
  };
}

interface ResourceSample {
  i: number;
  geometries: number | null;
  textures: number | null;
  meshEntriesBuilt: number;
  meshEntriesRetired: number;
  pickQueries: number;
}

function readCounters(page: Page): Promise<ResourceCounters> {
  return page.evaluate(() => {
    const engine = (
      window as unknown as { __vpEngine?: { debugResourceCounters(): ResourceCounters } }
    ).__vpEngine;
    if (!engine) throw new Error("no __vpEngine — boot with openEditorDebug (?vpdebug)");
    return engine.debugResourceCounters();
  });
}

function toSample(i: number, c: ResourceCounters): ResourceSample {
  return {
    i,
    geometries: c.renderer?.geometries ?? null,
    textures: c.renderer?.textures ?? null,
    meshEntriesBuilt: c.viewport.meshEntriesBuilt,
    meshEntriesRetired: c.viewport.meshEntriesRetired,
    pickQueries: c.viewport.pickQueries,
  };
}

const HOVER_ITERATIONS = 300;
const SAMPLE_EVERY = 50;

test("TEST-RES-01: renderer geometry count plateaus after warm-up across 300 rendered hovers", async ({
  page,
}, testInfo) => {
  test.setTimeout(180_000);
  // R02 baseline (65b4c60): +2 renderer geometries per hover, no plateau
  // (docs/qa/viewport-hardening/baseline/vph-g-lane-probes-at-65b4c60.log).
  // WP03 made face overlays owned + cached and whole-body overlays leased, so
  // this is now a plain pass: geometries hold flat after warm-up.

  await openEditorDebug(page, { mockBody: true });
  // Two distinct faces of the seeded box body — visible together in the
  // default isometric view. `f:2` (+Y, BACK) is occluded from ISO — measured
  // directly: `findFacePoint(page, "body1", "f:2")` times out with "no screen
  // point picks body1/f:2". The default ISO view shows RIGHT(+X)/FRONT(-Y)/
  // TOP(+Z) (src/features/viewcube/ViewCube.tsx `FACES`), i.e. BOX_FACES
  // f:0/f:3/f:4 — use two of those instead.
  const a = await findFacePoint(page, "body1", "f:0");
  const b = await findFacePoint(page, "body1", "f:4");

  const before = await readCounters(page);
  const samples: ResourceSample[] = [];

  for (let i = 1; i <= HOVER_ITERATIONS; i++) {
    await page.mouse.move(a.x, a.y);
    await waitForRenderedFrame(page);
    await page.mouse.move(b.x, b.y);
    await waitForRenderedFrame(page);

    if (i % SAMPLE_EVERY === 0) {
      samples.push(toSample(i, await readCounters(page)));
    }
  }

  await attachJson(testInfo, "hover-lifecycle-series.json", { before: toSample(0, before), samples });

  const at50 = samples.find((s) => s.i === 50);
  const at100 = samples.find((s) => s.i === 100);
  const at300 = samples.find((s) => s.i === 300);
  if (!at50 || !at100 || !at300) throw new Error("missing i=50, i=100, or i=300 sample");

  // Plain assertions — must pass TODAY, independent of the R02 baseline: the
  // loop is genuinely hovering (pick queries advance across the run) and mesh
  // ingestion is not re-running per hover (meshEntriesBuilt is flat ACROSS THE
  // LOOP itself — compared sample-to-sample, not against the pre-loop `before`
  // baseline, which still includes the initial body ingest settling).
  expect(at300.pickQueries).toBeGreaterThan(before.viewport.pickQueries);
  expect(at300.meshEntriesBuilt).toBe(at50.meshEntriesBuilt);

  // R02 baseline (expected to fail today): geometry count should plateau
  // after warm-up instead of climbing from i=100 to i=300.
  expect(at300.geometries).toBe(at100.geometries);
});
