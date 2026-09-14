import { promises as fs } from "node:fs";
import { test, expect } from "./fixtures";
import type { Page, TestInfo } from "@playwright/test";
import {
  CANVAS,
  hideSeedSketches,
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
} from "./helpers";
import { waitForRenderedFrame } from "./modelToolHelpers";

// See e2e/vph-hover-lifecycle.spec.ts's `attachJson` for why this writes the
// attachment file directly instead of relying on `testInfo.attach`.
async function attachJson(testInfo: TestInfo, name: string, value: unknown): Promise<void> {
  const body = JSON.stringify(value, null, 2);
  const path = testInfo.outputPath(name);
  await fs.writeFile(path, body, "utf8");
  testInfo.attachments.push({ name, contentType: "application/json", path });
  console.log(`[${name}]`, body);
}

/*
 * TEST-RES-05 — VP-HARDENING §3.3/§5 lifetime and upload accounting, two
 * lanes the hover probe (TEST-RES-01) doesn't cover: a preview session that
 * is built and torn down without ever committing, and a whole document's
 * worth of resources released on close.
 *
 * Both loops read the same `?vpdebug` `window.__vpEngine.debugResourceCounters()`
 * surface as TEST-RES-01 (renderer `info.memory.geometries` + the process-global
 * `ViewportCounters` in src/viewport/vph/instrumentation.ts: `leasesOpen`,
 * `meshEntriesBuilt/Retired`). `leasesOpen` is the application-side ownership
 * ledger PLAN.md's T3/T4/T6 run-log entries describe: every registry borrower
 * (preview, ghost, section, highlight) is a lease acquired via
 * `countLeaseAcquired`/`countLeaseReleased` (meshRegistry.ts), so a preview
 * that reserves on arm and never releases on cancel shows up here as a
 * monotone climb instead of a plateau.
 */

interface ResourceCounters {
  renderer: { geometries: number; textures: number; programs: number } | null;
  viewport: {
    meshEntriesBuilt: number;
    meshEntriesRetired: number;
    leasesOpen: number;
    [k: string]: number;
  };
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

/** The FE log ring, filtered to `err`-level `vp` events (see src/debug/log.ts's
 *  `window.__logsDump`). Used to assert the meshRegistry leak tripwire
 *  (`meshRegistry.ts`'s `disposeAll`, message "meshRegistry leak tripwire
 *  after disposeAll") never fires during either loop. */
async function tripwireHits(page: Page): Promise<string[]> {
  const dump = await page.evaluate(
    () => (window as unknown as { __logsDump?: () => string }).__logsDump?.() ?? "[]",
  );
  const events = JSON.parse(dump) as Array<{ level?: string; msg?: string }>;
  return events
    .filter((e) => e.level === "err" && (e.msg ?? "").includes("leak tripwire"))
    .map((e) => e.msg ?? "");
}

/**
 * Draw one rectangle sketch on a clean document, finish it, and return the
 * client-pixel point over its filled region — the same profile-first flow as
 * e2e/extrude-draft.spec.ts's `armExtrudeOnFreshRectangle`, split so the
 * preview loop can re-click the SAME region point on every cycle instead of
 * redrawing the sketch.
 *
 * Does NOT boot the editor itself — a document-cycle loop reopens a document
 * in-app (`closeAndReopen`'s "New project" click) and calling
 * `openEditorDebug` again here would `page.goto()` a second, unwanted full
 * page reload on top of that fresh editor (measured: back-to-back "engine
 * dispose" events and an unsettled camera hang around cycle 15/50). Callers
 * that need the initial boot call `openEditorDebug` themselves first.
 *
 * The seed body (`body1`) exists ONLY on the very first document a boot
 * produces — `mockClient.newDocument()` hands back whatever the in-memory
 * projection currently holds, and a close empties it, so every document
 * `closeAndReopen` produces is genuinely bodyless (measured: an in-app
 * "New project" leaves an empty "Bodies" listbox — real new-document
 * behaviour, not a mock-lane gap). `bodyOptions(page).first()` would hang
 * forever waiting for a row that will never appear, so the seed-hiding step
 * is conditional on one actually being there.
 */
async function drawRectangleRegion(page: Page): Promise<{ x: number; y: number }> {
  const seedBodies = bodyOptions(page);
  if ((await seedBodies.count()) > 0) {
    await seedBodies.first().getByRole("switch").click();
  }
  await hideSeedSketches(page);
  await enterSketchViaPlanePicker(page);
  await waitForCameraSettled(page);

  await selectSketchTool(page, "Rectangle");
  await clickAt(page, -180, -110);
  await clickAt(page, 60, 90);
  await expect(dofPill(page)).toHaveText(/^DOF: [1-9]\d*$/);

  const snap = await getSketchSnapshot(page);
  const sum = snap.lines.reduce(
    (acc, l) => ({ x: acc.x + l.p0[0] + l.p1[0], y: acc.y + l.p0[1] + l.p1[1] }),
    { x: 0, y: 0 },
  );
  const centroid = { x: sum.x / (snap.lines.length * 2), y: sum.y / (snap.lines.length * 2) };

  await page.keyboard.press("Enter");
  await waitForCameraSettled(page);
  const client = await planePointToClient(page, snap.plane, centroid);
  await expect
    .poll(() =>
      page.evaluate(
        ({ x, y }) =>
          Boolean(
            (window as unknown as { __vpEngine?: { sketchStaticHitTest(x: number, y: number): unknown } })
              .__vpEngine?.sketchStaticHitTest(x, y),
          ),
        client,
      ),
    )
    .toBe(true);
  return client;
}

const extrudeButton = (page: Page) => page.getByRole("button", { name: "Extrude", exact: true });

/** Select the sketch region and arm Extrude — a real kernel-preview session
 *  opens (`ModelToolController`'s shared preview lane). */
async function armExtrudeOnRegion(page: Page, region: { x: number; y: number }): Promise<void> {
  await clickAtClient(page, region.x, region.y);
  await extrudeButton(page).click();
  await expect(page.getByText(/^Drag the arrow to set depth/)).toBeVisible();
  await waitForRenderedFrame(page); // the preview body actually renders once
}

/** Cancel the armed tool. Esc is the tool-wide cancel key — ExtrudeChipControls.tsx
 *  documents that the controller owns it from a capture-phase window listener
 *  so it always means "cancel", never "dismiss the chip popover". */
async function cancelArmedTool(page: Page): Promise<void> {
  await page.keyboard.press("Escape");
  await expect(extrudeButton(page)).not.toHaveAttribute("aria-pressed", "true");
}

const PREVIEW_CYCLES = 100;
const PREVIEW_SAMPLE_EVERY = 10;

test("TEST-RES-05: 100 extrude preview apply/cancel cycles return to baseline (leases, geometries)", async ({
  page,
}, testInfo) => {
  test.setTimeout(180_000);

  await openEditorDebug(page);
  const region = await drawRectangleRegion(page);

  // One warm-up cycle before the baseline is captured — first arm allocates
  // the preview session's steady-state resources (chip mount, preview
  // material, etc.) that a resource test should not count as growth. A
  // rendered frame after the cancel is required too: retirement is
  // frame-ordered (PLAN.md PR-03B, meshRegistry.ts's `flushDisposals`), so the
  // freed geometry from THIS cancel is still pending disposal for one frame —
  // without this the baseline undercounts by exactly what every subsequent
  // sample already includes (measured: 24 vs. the true 26 plateau).
  await armExtrudeOnRegion(page, region);
  await cancelArmedTool(page);
  await waitForRenderedFrame(page);
  const baseline = await readCounters(page);

  const samples: Array<{ i: number; geometries: number | null; leasesOpen: number; meshEntriesBuilt: number; meshEntriesRetired: number }> = [];

  for (let i = 1; i <= PREVIEW_CYCLES; i++) {
    await armExtrudeOnRegion(page, region);
    await cancelArmedTool(page);
    // Same frame-ordered-retirement reasoning as the baseline capture above:
    // without this, a sample taken before the cancel's disposal frame has run
    // reads the outgoing preview geometry as still live, which measured as a
    // spurious ±2 oscillation (26/24/26/24…) instead of a real trend.
    await waitForRenderedFrame(page);

    if (i % PREVIEW_SAMPLE_EVERY === 0) {
      const c = await readCounters(page);
      samples.push({
        i,
        geometries: c.renderer?.geometries ?? null,
        leasesOpen: c.viewport.leasesOpen,
        meshEntriesBuilt: c.viewport.meshEntriesBuilt,
        meshEntriesRetired: c.viewport.meshEntriesRetired,
      });
    }
  }

  const tripwires = await tripwireHits(page);
  await attachJson(testInfo, "preview-lifecycle-series.json", {
    baseline: {
      geometries: baseline.renderer?.geometries ?? null,
      leasesOpen: baseline.viewport.leasesOpen,
    },
    samples,
    tripwires,
  });

  expect(tripwires).toEqual([]);

  // Plateau, not monotone growth: EVERY sampled cycle returns exactly to the
  // post-warm-up baseline once cancelled — a preview that leaked a lease or a
  // geometry on Esc would show a climbing series here.
  for (const s of samples) {
    expect(s.leasesOpen, `leasesOpen at i=${s.i}`).toBe(baseline.viewport.leasesOpen);
    expect(s.geometries, `geometries at i=${s.i}`).toBe(baseline.renderer?.geometries ?? null);
  }
});

const DOC_CYCLES = 50;
const DOC_SAMPLE_EVERY = 10;

const closeProjectButton = (page: Page) => page.getByRole("button", { name: "Close project" });
const dialog = (page: Page) => page.getByRole("dialog");
const newProjectButton = (page: Page) => page.getByRole("button", { name: "New project" });

/**
 * Draw a rectangle, finish it, arm Extrude on its region and commit at the
 * default depth (`DEFAULT_EXTRUDE_DEPTH`, modelToolMachine.ts) — pressing
 * Enter right after arming confirms the tool with no drag needed (the same
 * "Enter confirms whichever model tool is armed" ladder rule
 * ModelToolController.ts documents), so one committed body costs one arm +
 * one keypress per document cycle.
 */
async function extrudeOneBody(page: Page): Promise<void> {
  const region = await drawRectangleRegion(page);
  const bodiesBefore = await bodyOptions(page).count();
  await armExtrudeOnRegion(page, region);
  await page.keyboard.press("Enter");
  await expect(extrudeButton(page)).not.toHaveAttribute("aria-pressed", "true");
  await expect(bodyOptions(page)).toHaveCount(bodiesBefore + 1);
}

/** Close the dirty document (discarding, never saving — the mock lane has no
 *  real persistence target) and start a fresh one, mirroring
 *  e2e/unsaved-guard.spec.ts's close flow. */
async function closeAndReopen(page: Page): Promise<void> {
  await closeProjectButton(page).click();
  await expect(dialog(page)).toBeVisible();
  await dialog(page)
    .getByRole("button", { name: /Don.t Save/ })
    .click();
  await expect(dialog(page)).toHaveCount(0);
  await expect(newProjectButton(page)).toBeVisible();
  await newProjectButton(page).click();
  await expect(page.locator(`${CANVAS} canvas`)).toBeVisible({ timeout: 20_000 });
}

test("TEST-RES-05: 50 new-document/close cycles return to the empty-document baseline", async ({
  page,
}, testInfo) => {
  // Measured: 48/50 cycles at ~4.9s/cycle landed right at a 240s budget,
  // cutting the run off mid-handoff on the close/reopen engine dispose+init
  // pair and reading as a false "window.__vpEngine missing" failure.
  test.setTimeout(360_000);

  await openEditorDebug(page);
  // Warm-up cycle: the very first document's boot settling (initial seed body
  // ingest) is not "growth" a resource test should charge to the loop.
  await extrudeOneBody(page);
  await closeAndReopen(page);
  const baseline = await readCounters(page);

  const samples: Array<{
    i: number;
    geometries: number | null;
    leasesOpen: number;
    meshEntriesRetired: number;
    retiredDelta: number;
  }> = [];
  let prevRetired = baseline.viewport.meshEntriesRetired;

  for (let i = 1; i <= DOC_CYCLES; i++) {
    await extrudeOneBody(page);
    await closeAndReopen(page);

    if (i % DOC_SAMPLE_EVERY === 0) {
      const c = await readCounters(page);
      samples.push({
        i,
        geometries: c.renderer?.geometries ?? null,
        leasesOpen: c.viewport.leasesOpen,
        meshEntriesRetired: c.viewport.meshEntriesRetired,
        retiredDelta: c.viewport.meshEntriesRetired - prevRetired,
      });
      prevRetired = c.viewport.meshEntriesRetired;
    }
  }

  const tripwires = await tripwireHits(page);
  await attachJson(testInfo, "document-lifecycle-series.json", {
    baseline: {
      geometries: baseline.renderer?.geometries ?? null,
      leasesOpen: baseline.viewport.leasesOpen,
      meshEntriesRetired: baseline.viewport.meshEntriesRetired,
    },
    samples,
    tripwires,
  });

  expect(tripwires).toEqual([]);

  // Every close RETIRES what the cycle built: the retirement counter must grow
  // on every sampled window — at least one entry per closed document — and by
  // a bounded amount. The exact count is NOT constant: a cycle republishes its
  // bodies a varying number of times (the mock lane's regen/colour reloads each
  // swap an entry), so each swap retires one more entry; 2026-09-14 measured
  // 30/26/25/26/28 per ten closes with geometries and leases flat at baseline.
  // Leak detection is the two plateau assertions below, not this counter.
  for (const s of samples) {
    expect(s.retiredDelta, `meshEntriesRetired delta at i=${s.i}`).toBeGreaterThanOrEqual(DOC_SAMPLE_EVERY);
    expect(s.retiredDelta, `meshEntriesRetired delta at i=${s.i}`).toBeLessThanOrEqual(DOC_SAMPLE_EVERY * 8);
  }

  // Application resources return to the empty-document baseline every time —
  // a fresh `New project` document looks the same on the renderer/lease
  // ledger no matter how many prior documents were opened and closed.
  for (const s of samples) {
    expect(s.leasesOpen, `leasesOpen at i=${s.i}`).toBe(baseline.viewport.leasesOpen);
    expect(s.geometries, `geometries at i=${s.i}`).toBe(baseline.renderer?.geometries ?? null);
  }
});
