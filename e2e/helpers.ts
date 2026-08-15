/*
 * Shared e2e helpers: boot into the editor, enter a sketch via the REAL plane
 * picker, and drive the WebGL canvas with deliberate pointer "clicks".
 *
 * The viewport is a WebGL canvas with no per-entity DOM, so every assertion in
 * the specs goes through store-driven chrome instead: the status-bar DOF pill,
 * the sketch chrome bar ("Editing …" + "Under-constrained · DOF N"), the
 * inspector Constraints list, the model tree, and the toolbar aria-pressed
 * state. This module only provides the interaction primitives; the specs own the
 * assertions.
 *
 * Selectors used (all pre-existing in the app — no test-only attributes added):
 *   - [data-testid="viewport-canvas"]  — ViewportRoot container (engine appends
 *                                        its <canvas> here; fills the same box).
 *   - [data-plane-pick-label]          — PlanePicker hover chip (HTML overlay).
 *   - role=button aria-label/aria-pressed — FloatingToolbar tools.
 *   - status-bar / chrome / inspector text — see specs.
 */
import { type Page, type Locator, expect } from "@playwright/test";

export const CANVAS = '[data-testid="viewport-canvas"]';

export type SketchToolLabel =
  | "Line"
  | "Rectangle"
  | "Center rectangle"
  | "Circle"
  | "Ellipse"
  | "Arc"
  | "3-point arc"
  | "Polygon"
  | "Slot"
  | "Point"
  | "Select"
  | "Extend";

async function bootEditor(page: Page, path: string): Promise<void> {
  await page.goto(path);
  // `?vpdemo` (App.tsx `forceEditor`) boots straight into the editor shell, so
  // there is no start screen to click through — clicking would hang forever.
  if (!path.includes("vpdemo")) {
    await page.getByRole("button", { name: "New project" }).click();
  }
  // The editor shell (and thus the viewport canvas) only mounts after the mock
  // newDocument() resolves and appStore flips screen → editor.
  await expect(page.locator(`${CANVAS} canvas`)).toBeVisible();
}

/** Start screen → new document → live editor with a ready WebGL engine. */
export async function openEditor(page: Page): Promise<void> {
  await bootEditor(page, "/");
}

/**
 * Same boot, but with `?vpdebug` in the URL so `ViewportEngine` exposes
 * `window.__vpEngine` (its own `debug` gate — see ViewportEngine.ts /
 * ViewportRoot.tsx `hasFlag("vpdebug")`) AND `ModelToolController` exposes its
 * `window.__extrudePreview` debug surface (`phase`/`revolvePhase`). The Wave 1
 * extrude commit is a REAL drag on the 3D depth handle (to set depth) followed by
 * an explicit confirm (Enter) — release alone now keeps the tool armed. The
 * handle's on-screen position depends on the camera + sketch plane with no
 * closed-form pixel offset. `findExtrudeHandle` below hit-scans the canvas
 * through the engine's OWN `hitExtrudeHandle` raycast to find a real client
 * point to click — this flag is what exposes that raycast to the page.
 */
export async function openEditorDebug(page: Page, opts?: { mockBody?: boolean }): Promise<void> {
  // `?vpdemo` additionally drives the mock BOX body through the full
  // `onDocumentChanged` ingest path, so there is real body geometry in the scene
  // to raycast against (the mock lane publishes no meshes otherwise). Specs that
  // need to click a model FACE — not just a sketch — ask for it.
  await bootEditor(page, opts?.mockBody ? "/?vpdebug&vpdemo" : "/?vpdebug");
}

/** The viewport container's bounding box (the canvas fills the same rect). */
async function canvasBox(page: Page): Promise<{ x: number; y: number; width: number; height: number }> {
  const box = await page.locator(CANVAS).boundingBox();
  if (!box) throw new Error("viewport canvas has no bounding box");
  return box;
}

/** Canvas-center-relative offset (px) → absolute page coordinates. Exported so a
 *  spec that needs the raw client pixel (e.g. to feed `screenToPlane` for a
 *  ground-truth read of what a pixel offset maps to) can compute it without
 *  duplicating the canvas-box lookup. */
export async function toPage(page: Page, dx: number, dy: number): Promise<{ x: number; y: number }> {
  const box = await canvasBox(page);
  return { x: box.x + box.width / 2 + dx, y: box.y + box.height / 2 + dy };
}

/**
 * A deliberate click at a canvas-center-relative offset: move → down → up with
 * no intervening drag, so the SketchController classifies it as a tap (its
 * wasClick gate rejects moves > 4px). Offsets stay within the central drawing
 * zone that is clear of the floating tree/inspector/toolbar panels.
 */
export async function clickAt(page: Page, dx: number, dy: number): Promise<void> {
  const p = await toPage(page, dx, dy);
  await page.mouse.move(p.x, p.y);
  await page.mouse.down();
  await page.mouse.up();
}

/**
 * Select a sketch drawing tool from the floating toolbar and confirm it armed.
 *
 * Since the tool FAMILIES landed (`d81f758`) the toolbar shows one slot per
 * family — Rectangle and Center rectangle share a slot, as do Circle and
 * Ellipse — so a non-default member has no button of its own until it is
 * picked from the family's flyout. The slot then REMEMBERS it, which is why the
 * direct hit is tried first: it covers both the default member and any member
 * already made sticky by an earlier call in the same test.
 *
 * The flyout is discovered rather than hardcoded (open each `… options`
 * chevron, look for the menuitem, close it again if absent) so adding a family
 * member does not mean editing a table here.
 */
export async function selectSketchTool(page: Page, label: SketchToolLabel): Promise<void> {
  const btn = page.getByRole("button", { name: label, exact: true });
  // `count()` does NOT auto-wait, so it must never be the first thing asked of a
  // toolbar that may still be mounting — waitFor is what distinguishes "this tool
  // lives behind a flyout" from "the toolbar has not painted yet".
  try {
    await btn.waitFor({ state: "visible", timeout: 2_000 });
  } catch {
    await pickFromToolFlyout(page, label);
  }
  await btn.click();
  await expect(btn).toHaveAttribute("aria-pressed", "true");
}

/** Open the family flyout that owns `label` and make it the slot's sticky tool. */
async function pickFromToolFlyout(page: Page, label: string): Promise<void> {
  const chevrons = page.getByRole("button", { name: /\soptions$/ });
  await chevrons.first().waitFor({ state: "visible" });
  const count = await chevrons.count();
  for (let i = 0; i < count; i += 1) {
    const chevron = chevrons.nth(i);
    await chevron.click();
    // The row renders title and shortcut as adjacent spans with no separator, so
    // its accessible name is "EllipseO" / "Center rectangle⇧R" — matching the
    // TITLE span exactly is the only stable hook that survives a shortcut change.
    const item = page
      .getByRole("menuitem")
      .filter({ has: page.locator(`span:text-is(${JSON.stringify(label)})`) });
    if (await item.isVisible().catch(() => false)) {
      await item.click();
      return;
    }
    await chevron.click(); // wrong family — close it and try the next
  }
  throw new Error(`no toolbar button or flyout member named "${label}"`);
}

/** The status-bar DOF pill (e.g. "DOF: 3"). Sketch mode always shows it. */
export function dofPill(page: Page): Locator {
  // Only the status-bar pill starts with "DOF:"; the chrome/inspector use "DOF N".
  return page.getByText(/^DOF: \d/);
}

/** Sketch rows in the model tree (the mock seeds three; entering adds one). */
export function sketchOptions(page: Page): Locator {
  return page.getByRole("listbox", { name: "Sketches" }).getByRole("option");
}

/**
 * Click, then wait for the DOF pill to change — a settle gate for the line tool.
 * Each committed segment round-trips through the ASYNC mock sketchUpsert, and
 * successive commits read the live session at start; firing the next click
 * before the previous commit lands would let them clobber each other (and skip
 * the shared-endpoint coincidence). Adding a segment always shifts DOF by ≥1, so
 * this polls deterministically with no fixed sleep.
 */
export async function clickAtAwaitingDofChange(page: Page, dx: number, dy: number): Promise<void> {
  const pill = dofPill(page);
  const before = (await pill.textContent()) ?? "";
  await clickAt(page, dx, dy);
  await expect(pill).not.toHaveText(before);
}

/**
 * Enter a sketch through the real user path: model-mode "New sketch" → plane
 * picker → hover a plane quad (await the overlay chip) → click it.
 *
 * The hover is polled: the SketchController that owns planePickerHover is created
 * only once the engine finishes initialising, so the first pointermove may land
 * before the listener (or the picker) exists. toPass() re-hovers until the chip
 * appears, which doubles as the engine-ready gate.
 */
export async function enterSketchViaPlanePicker(page: Page): Promise<void> {
  // Settle the MODEL camera first. `enterSketch` aims along the plane normal at
  // `controls.getDistance()` — the sketch view INHERITS the model camera's
  // distance — and that distance sets `planePixelWorld`, which in turn sets the
  // draw tools' screen-constant reject radius (`minSize`) and the zoom-adaptive
  // dimension quantum. Entering while the initial-load fit is still moving
  // therefore makes both of those non-deterministic run to run, which is what
  // made `sketch-degenerate` and `live-dim-mouse-rounding` flaky. Callers that
  // settle AFTER entering are too late: the distance has already been captured.
  await waitForCameraSettled(page);
  await page.getByRole("button", { name: "New sketch", exact: true }).click();
  // Plane-pick phase chrome (activeSketchId still null).
  await expect(page.getByText("Select a sketch plane")).toBeVisible();

  const center = await toPage(page, 0, 0);
  const chip = page.locator("[data-plane-pick-label]");
  await expect(async () => {
    // Two moves guarantee a fresh pointermove delta reaches the controller.
    await page.mouse.move(center.x, center.y - 1);
    await page.mouse.move(center.x, center.y);
    expect(await chip.isVisible()).toBe(true);
  }).toPass({ timeout: 20_000, intervals: [150, 250, 400, 600] });

  // Click the highlighted quad → creates the sketch on that plane.
  await page.mouse.down();
  await page.mouse.up();

  // Session is live: chrome swaps to "Editing …" and Line is the default tool.
  await expect(page.getByText(/^Editing /)).toBeVisible();
  await expect(page.getByRole("button", { name: "Line", exact: true })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
}

/** Body rows in the model tree (mirrors sketchOptions — ModelTreePanel's "Bodies" listbox). */
export function bodyOptions(page: Page): Locator {
  return page.getByRole("listbox", { name: "Bodies" }).getByRole("option");
}

/** Datum rows in the model tree (DATUM W1 — ModelTreePanel's "Datums" listbox). */
export function datumOptions(page: Page): Locator {
  return page.getByRole("listbox", { name: "Datums" }).getByRole("option");
}

/**
 * Hit-scan the canvas for a real client pixel over the plane-picker quad of a
 * given repo `kind`, via the engine's OWN `planePickerHitTest` raycast exposed
 * at `window.__vpEngine` (see openEditorDebug).
 *
 * A center click is not enough here: all three origin quads MEET at the world
 * origin, so which one the ray finds first depends on the settled camera pose.
 * Scanning with the exact method the pointer handlers use is the only way to
 * target a NAMED plane. Wrapped in `toPass` because the picker's quads are only
 * sized on its first post-arm update (render-on-demand).
 */
export async function findPlaneQuad(
  page: Page,
  kind: "XY" | "XZ" | "YZ",
): Promise<{ x: number; y: number }> {
  let found: { x: number; y: number } | null = null;
  await expect(async () => {
    found = await page.evaluate((want) => {
      const engine = (
        window as unknown as {
          __vpEngine?: { planePickerHitTest(x: number, y: number): string | null };
        }
      ).__vpEngine;
      const canvas = document.querySelector('[data-testid="viewport-canvas"] canvas') as HTMLCanvasElement | null;
      if (!engine || !canvas) return null;
      const rect = canvas.getBoundingClientRect();
      const step = 8;
      for (let y = rect.top + step; y <= rect.bottom - step; y += step) {
        for (let x = rect.left + step; x <= rect.right - step; x += step) {
          if (engine.planePickerHitTest(x, y) === want) return { x, y };
        }
      }
      return null;
    }, kind);
    expect(found).not.toBeNull();
  }).toPass({ timeout: 15_000, intervals: [150, 300, 600, 1_000] });
  return found as unknown as { x: number; y: number };
}

/**
 * Hit-scan the canvas for a real client pixel over a DATUM plane quad, via the
 * datum layer's own `hitTest` (published at `window.__datumVisuals` — the layer
 * is a viewport CONTRIBUTION now, so the probe follows it off the engine).
 * Returns the pixel plus the datum id found there.
 * Same rationale as `findPlaneQuad`: a datum sits at an arbitrary offset with no
 * closed-form screen position.
 */
export async function findDatumQuad(page: Page): Promise<{ x: number; y: number; id: string }> {
  let found: { x: number; y: number; id: string } | null = null;
  await expect(async () => {
    found = await page.evaluate(() => {
      const engine = (
        window as unknown as { __datumVisuals?: { hitTest(x: number, y: number): string | null } }
      ).__datumVisuals;
      const canvas = document.querySelector('[data-testid="viewport-canvas"] canvas') as HTMLCanvasElement | null;
      if (!engine || !canvas) return null;
      const rect = canvas.getBoundingClientRect();
      const step = 8;
      for (let y = rect.top + step; y <= rect.bottom - step; y += step) {
        for (let x = rect.left + step; x <= rect.right - step; x += step) {
          const id = engine.hitTest(x, y);
          if (id) return { x, y, id };
        }
      }
      return null;
    });
    expect(found).not.toBeNull();
  }).toPass({ timeout: 15_000, intervals: [150, 300, 600, 1_000] });
  return found as unknown as { x: number; y: number; id: string };
}

/**
 * Hit-scan the canvas for a real client pixel over a model FACE, through the
 * engine's OWN `probePick` raycast (exposed at `window.__vpEngine` — see
 * `openEditorDebug`). Returns the pixel plus the face's body + `topoKey`.
 *
 * Same rationale as `findDatumQuad` / `findExtrudeHandle`: a body's faces land
 * wherever the camera puts them, with no closed-form screen position, so the only
 * robust target is one found with the exact raycast the real pointer handlers use.
 * This produces a GENUINE pick — the spec clicks it for real rather than seeding
 * the selection store, which is the whole point when the flow under test is
 * "pick a face, then sketch on it".
 *
 * Requires body geometry in the scene (`openEditorDebug(page, { mockBody: true })`).
 */
export async function findFaceOnBody(
  page: Page,
  bodyId?: string,
): Promise<{ x: number; y: number; bodyId: string; topoKey: string }> {
  // The returned pixel is only meaningful for the camera it was probed under, and
  // the initial-load auto-fit is debounced — probing mid-fit yields a point on a
  // face that is about to move (or shrink), which is how "no second pixel far
  // enough on the same face" showed up intermittently. Settle first.
  await waitForCameraSettled(page);
  let found: { x: number; y: number; bodyId: string; topoKey: string } | null = null;
  await expect(async () => {
    found = await page.evaluate((want) => {
      const engine = (
        window as unknown as {
          __vpEngine?: {
            probePick(x: number, y: number): { bodyId: string; kind: string; topoKey: string } | null;
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
          if (want && hit.bodyId !== want) continue;
          return { x, y, bodyId: hit.bodyId, topoKey: hit.topoKey };
        }
      }
      return null;
    }, bodyId ?? null);
    expect(found, "no model face found under any scanned pixel").not.toBeNull();
  }).toPass({ timeout: 15_000, intervals: [150, 300, 600, 1_000] });
  return found as unknown as { x: number; y: number; bodyId: string; topoKey: string };
}

/** The datum-plane names in the projection store (`__stores.document`, dev-only). */
export async function getDatumNames(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { document: { getState(): { datums: Record<string, { name: string }> } } };
    };
    return Object.values(w.__stores?.document.getState().datums ?? {}).map((d) => d.name);
  });
}

/**
 * The constraint-discovery menu's button grid (ConstraintMenu.tsx —
 * role="toolbar" aria-label="Constraints"), opened from its "Constraints"
 * trigger in `SketchChromeBar` if not already open. ConstraintContextChips
 * renders a SECOND floating row with the exact same button labels near the
 * selection centroid whenever the applicable set is non-empty, so any
 * geometric/dimensional constraint button lookup MUST be scoped to this
 * one — an unscoped `getByRole("button", { name: "Horizontal" })` resolves
 * to two elements (strict-mode violation) as soon as something is selected.
 *
 * The menu is a Popover (Sketcher UX cleanup, Track A3 — it replaced a
 * persistent, always-mounted toolbar) and closes on any outside click,
 * INCLUDING a canvas click that changes the selection. Call this again
 * — not a cached reference — right before each interaction that follows
 * an intervening canvas/keyboard click, so it re-opens if it closed.
 */
export async function constraintToolbar(page: Page): Promise<Locator> {
  const toolbar = page.getByRole("toolbar", { name: "Constraints" });
  if (!(await toolbar.isVisible().catch(() => false))) {
    await page.getByRole("button", { name: "Constraints" }).click();
    await expect(toolbar).toBeVisible();
  }
  return toolbar;
}

/**
 * Wait for the orbit camera's tween to finish (CadOrbitControls — a 250ms
 * `animateTo`). TWO separate spots kick off an ANIMATED camera move around
 * plane-picker entry: `ViewportEngine.setPlanePickerVisible` re-homes with
 * `homeView(true)` when the picker opens, and — once the plane is actually
 * picked — `ViewportEngine.enterSketch` saves the current view then calls
 * `controls.viewAlongNormal(..., true)` to look straight at the new sketch
 * plane. `screenToPlane`/`planePointToClient` both depend on the camera's
 * CURRENT matrices, so a click or projection issued mid-tween reads/targets a
 * moving pose — confirmed as the root cause of an intermittent miss in a real
 * run (a `screenToPlaneOn` round-trip matched the source point exactly on
 * settled runs, and was off by 10+ plane units on racing ones). A single
 * "is `tween` null right now" check is not enough: it can land in the narrow
 * gap between the FIRST tween finishing and the SECOND one starting, so this
 * requires `tween` to read null continuously for a window comfortably longer
 * than one tween (350ms > CadOrbitControls' 250ms) before returning — any
 * tween observed during that window resets the wait. Call this once right
 * after `enterSketchViaPlanePicker`, before any select-tool click or
 * `planePointToClient` projection.
 */
export async function waitForCameraSettled(page: Page): Promise<void> {
  const STABLE_WINDOW_MS = 350;
  // "No tween in flight" is NOT enough: the initial-load auto-fit is debounced,
  // so a fit can be scheduled-but-not-started and would begin moving the camera
  // after this helper already returned — invalidating any client coordinate the
  // caller computed from a probe. `autoFitPending` covers that window.
  const isSettled = (): Promise<boolean> =>
    page.evaluate(() => {
      const engine = (
        window as unknown as {
          __vpEngine?: { controls?: { tween: unknown } | null; autoFitPending?: boolean };
        }
      ).__vpEngine;
      return !engine?.controls?.tween && !engine?.autoFitPending;
    });

  await expect(async () => {
    const stableSince = Date.now();
    while (Date.now() - stableSince < STABLE_WINDOW_MS) {
      expect(await isSettled()).toBe(true);
    }
  }).toPass({ timeout: 10_000, intervals: [50, 100, 200] });
}

/**
 * Hit-scan the canvas for the real extrude-depth drag handle in CLIENT (page)
 * pixel space, via the engine's OWN `hitExtrudeHandle` raycast exposed at
 * `window.__vpEngine` (see openEditorDebug). The handle is a small 3D gizmo
 * anchored at the region centroid, pointing along the sketch-plane normal —
 * its screen position depends on the camera + plane with no closed-form pixel
 * offset, so scanning with the exact method the real pointer handlers use is
 * the only robust way to find a genuine click target. Wrapped in `toPass` so
 * the first attempt can land before the handle's first post-arm render (the
 * engine is render-on-demand — see ViewportEngine `invalidate()`).
 */
export async function findExtrudeHandle(page: Page): Promise<{ x: number; y: number }> {
  let found: { x: number; y: number } | null = null;
  await expect(async () => {
    found = await page.evaluate(() => {
      const engine = (window as unknown as { __vpEngine?: { hitExtrudeHandle(x: number, y: number): boolean } })
        .__vpEngine;
      const canvas = document.querySelector('[data-testid="viewport-canvas"] canvas') as HTMLCanvasElement | null;
      if (!engine || !canvas) return null;
      const rect = canvas.getBoundingClientRect();
      const step = 6;
      for (let y = rect.top; y <= rect.bottom; y += step) {
        for (let x = rect.left; x <= rect.right; x += step) {
          if (engine.hitExtrudeHandle(x, y)) return { x, y };
        }
      }
      return null;
    });
    expect(found).not.toBeNull();
  }).toPass({ timeout: 10_000, intervals: [100, 200, 400, 800] });
  return found as unknown as { x: number; y: number };
}

// ── Precise plane→screen targeting (select-tool / region-pick clicks) ────────
//
// A canvas-center-relative pixel OFFSET only pins a point exactly for the ONE
// click that produced it (`screenToPlane` is a real perspective raycast onto
// the sketch plane, not an affine map) — you cannot linearly combine two such
// offsets to predict a THIRD point's screen position (e.g. "the other corner",
// an edge midpoint, a shape's centroid). Verified empirically: an initial
// version of these specs guessed those points from pixel-offset arithmetic and
// silently missed every hit-test. The robust fix reads the REAL entity
// coordinates back from the live sketch session (`window.__stores.sketch`,
// always exposed in dev — see main.tsx) and projects any derived plane point to
// a real client pixel through the SAME camera the engine uses
// (`window.__vpEngine`, exposed under `?vpdebug` — see openEditorDebug), i.e.
// the exact inverse of `screenToPlane`.

export interface SketchPlaneSnapshot {
  kind: string;
  origin: [number, number, number];
  xAxis: [number, number, number];
  yAxis: [number, number, number];
  normal: [number, number, number];
}
export interface SketchLineSnapshot {
  p0: [number, number];
  p1: [number, number];
}

/**
 * Read the live sketch session's plane + Line entities. MUST be called while
 * still in sketch mode — SketchController clears `sketchStore.session` once the
 * mode leaves "sketch" (finishing / cancelling), so snapshot BEFORE pressing
 * Enter if a later step (e.g. a region-pick click) needs a plane point computed
 * from this sketch's geometry.
 */
export async function getSketchSnapshot(
  page: Page,
): Promise<{ plane: SketchPlaneSnapshot; lines: SketchLineSnapshot[] }> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: {
        sketch: {
          getState(): {
            session: {
              plane: SketchPlaneSnapshot;
              entities: Array<{ type: string; p0?: [number, number]; p1?: [number, number] }>;
            } | null;
          };
        };
      };
    };
    const session = w.__stores?.sketch.getState().session;
    if (!session) throw new Error("getSketchSnapshot: no live sketch session (call before finishing the sketch)");
    const lines: SketchLineSnapshot[] = [];
    for (const e of session.entities) {
      if (e.type === "Line" && e.p0 && e.p1) lines.push({ p0: e.p0, p1: e.p1 });
    }
    const { kind, origin, xAxis, yAxis, normal } = session.plane;
    return { plane: { kind, origin, xAxis, yAxis, normal }, lines };
  });
}

/**
 * The document projection's feature-timeline labels (`__stores.document`). The
 * inspector's on-screen HISTORY is a capped, selection-dependent slice (a BODY
 * selection shows only `features.slice(0,3)`), so a committed op's timeline row is
 * asserted against the projection, not the DOM.
 */
export async function getFeatureLabels(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { document: { getState(): { features: Array<{ label: string }> } } };
    };
    return (w.__stores?.document.getState().features ?? []).map((f) => f.label);
  });
}

/**
 * Read the live sketch session's first Circle entity (plane-local center +
 * radius). Same live-session requirement as `getSketchSnapshot` — call BEFORE
 * finishing the sketch. Used to toggle a circle region in the multi-select flow
 * (its centroid isn't recoverable from a screen offset once the camera restores).
 */
export async function getSketchCircle(page: Page): Promise<{ center: [number, number]; radius: number }> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: {
        sketch: {
          getState(): {
            session: { entities: Array<{ type: string; center?: [number, number]; radius?: number }> } | null;
          };
        };
      };
    };
    const session = w.__stores?.sketch.getState().session;
    if (!session) throw new Error("getSketchCircle: no live sketch session (call before finishing the sketch)");
    const circle = session.entities.find((e) => e.type === "Circle" && e.center && typeof e.radius === "number");
    if (!circle?.center || typeof circle.radius !== "number") throw new Error("getSketchCircle: no circle in the session");
    return { center: circle.center, radius: circle.radius };
  });
}

/**
 * Read the live sketch session's first Ellipse entity (plane-local centre +
 * semi-axes + rotation). Same live-session requirement as `getSketchSnapshot` —
 * call BEFORE finishing the sketch. The centre is the region-pick target for the
 * ellipse's own fill cell (an offset guess is unreliable: `screenToPlane` is a
 * perspective raycast, and the camera restores on finish).
 */
export async function getSketchEllipse(page: Page): Promise<{
  center: [number, number];
  majorR: number;
  minorR: number;
  rotation: number;
}> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: {
        sketch: {
          getState(): {
            session: {
              entities: Array<{
                type: string;
                center?: [number, number];
                majorR?: number;
                minorR?: number;
                rotation?: number;
              }>;
            } | null;
          };
        };
      };
    };
    const session = w.__stores?.sketch.getState().session;
    if (!session) throw new Error("getSketchEllipse: no live sketch session (call before finishing the sketch)");
    const e = session.entities.find(
      (x) => x.type === "Ellipse" && x.center && typeof x.majorR === "number" && typeof x.minorR === "number",
    );
    if (!e?.center || typeof e.majorR !== "number" || typeof e.minorR !== "number") {
      throw new Error("getSketchEllipse: no ellipse in the session");
    }
    return { center: e.center, majorR: e.majorR, minorR: e.minorR, rotation: e.rotation ?? 0 };
  });
}

/**
 * Total entity count in the live sketch session, ANY type (Line/Rect-lines/
 * Circle/Arc/Point) — unlike `getSketchSnapshot`, which only surfaces Lines.
 * For degenerate-input specs that just need "did anything commit", regardless
 * of which tool was armed. Same live-session requirement as `getSketchSnapshot`
 * (call before leaving sketch mode).
 */
export async function getSketchEntityCount(page: Page): Promise<number> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { sketch: { getState(): { session: { entities: unknown[] } | null } } };
    };
    const session = w.__stores?.sketch.getState().session;
    if (!session) throw new Error("getSketchEntityCount: no live sketch session (call before finishing the sketch)");
    return session.entities.length;
  });
}

/** One live sketch entity, every geometric field any type can carry (Arc's
 *  center/radius/start/end alongside Line's p0/p1) — the union `SketchEntity`
 *  itself uses. Fuller than `getSketchSnapshot` (Lines only) / `getSketchCircle`
 *  / `getSketchEllipse` (one type each): for a spec that needs to read back an
 *  Arc, or several entity types in the same session, at once. */
export interface FullEntitySnap {
  id: string;
  type: string;
  construction?: boolean;
  p0?: [number, number];
  p1?: [number, number];
  center?: [number, number];
  radius?: number;
  start?: [number, number];
  end?: [number, number];
}

/** Every live sketch entity, all geometric fields, id included. Same
 *  live-session requirement as `getSketchSnapshot` (call before finishing). */
export async function getSketchEntitiesFull(page: Page): Promise<FullEntitySnap[]> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { sketch: { getState(): { session: { entities: FullEntitySnap[] } | null } } };
    };
    const session = w.__stores?.sketch.getState().session;
    if (!session) throw new Error("getSketchEntitiesFull: no live sketch session (call before finishing the sketch)");
    return session.entities.map((e) => ({ ...e }));
  });
}

/** One live sketch constraint, straight off the session (id/type/entities/
 *  positions/value) — the wire shape `SketchConstraint` itself uses. */
export interface ConstraintSnap {
  id: string;
  type: string;
  entities: string[];
  positions?: string[];
  value?: number;
}

/** Every live sketch constraint. Same live-session requirement as
 *  `getSketchSnapshot` (call before finishing the sketch). */
export async function getSketchConstraints(page: Page): Promise<ConstraintSnap[]> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { sketch: { getState(): { session: { constraints: ConstraintSnap[] } | null } } };
    };
    const session = w.__stores?.sketch.getState().session;
    if (!session) throw new Error("getSketchConstraints: no live sketch session (call before finishing the sketch)");
    return session.constraints.map((c) => ({ ...c }));
  });
}

/**
 * Raycast a CLIENT (page) pixel onto the current sketch plane, through the
 * engine's OWN `screenToPlane` — the exact forward direction `planePointToClient`
 * below inverts. Requires `window.__vpEngine` (see openEditorDebug). Ground truth
 * for "what plane point will this pixel click actually produce": with snapping
 * disabled (`setSnapPref` for `grid`/`sketchGuideLines`/`polarTracking`) the
 * SNAPPED point a real click resolves to is bit-identical to this raw raycast, so
 * a spec can know a click's exact plane coordinates ahead of the click itself.
 */
export async function screenToPlane(page: Page, x: number, y: number): Promise<{ x: number; y: number } | null> {
  return page.evaluate(
    ({ x, y }) => {
      const engine = (
        window as unknown as { __vpEngine?: { screenToPlane(x: number, y: number): { x: number; y: number } | null } }
      ).__vpEngine;
      return engine ? engine.screenToPlane(x, y) : null;
    },
    { x, y },
  );
}

/**
 * Project a sketch-plane (u,v) coordinate to a CLIENT (page) pixel through the
 * live camera — the exact inverse of the engine's `screenToPlane` raycast.
 * Requires `window.__vpEngine` (see openEditorDebug). Plain mat4 math (world =
 * plane basis * (u,v); clip = projectionMatrix * matrixWorldInverse * world;
 * NDC = clip / w; pixel = NDC via the canvas rect) — `CameraRig.apply()` keeps
 * `matrixWorldInverse` synchronously current on every camera move, so no
 * render-frame wait is needed here (unlike `findExtrudeHandle`'s raycast, which
 * hits a mesh whose OWN world matrix only updates on the next render).
 */
export async function planePointToClient(
  page: Page,
  plane: SketchPlaneSnapshot,
  point: { x: number; y: number },
): Promise<{ x: number; y: number }> {
  return page.evaluate(
    ({ plane, point }) => {
      const w = window as unknown as {
        __vpEngine?: {
          rig: {
            getCamera(): {
              matrixWorldInverse: { elements: number[] };
              projectionMatrix: { elements: number[] };
            };
          };
        };
      };
      const engine = w.__vpEngine;
      if (!engine) throw new Error("planePointToClient: window.__vpEngine missing (use openEditorDebug)");
      const [ox, oy, oz] = plane.origin;
      const [xx, xy, xz] = plane.xAxis;
      const [yx, yy, yz] = plane.yAxis;
      const wx = ox + point.x * xx + point.y * yx;
      const wy = oy + point.x * xy + point.y * yy;
      const wz = oz + point.x * xz + point.y * yz;

      const camera = engine.rig.getCamera();
      const m = camera.matrixWorldInverse.elements; // column-major 4x4
      const pm = camera.projectionMatrix.elements;

      const vx = m[0] * wx + m[4] * wy + m[8] * wz + m[12];
      const vy = m[1] * wx + m[5] * wy + m[9] * wz + m[13];
      const vz = m[2] * wx + m[6] * wy + m[10] * wz + m[14];
      const vw = m[3] * wx + m[7] * wy + m[11] * wz + m[15];

      const cx = pm[0] * vx + pm[4] * vy + pm[8] * vz + pm[12] * vw;
      const cy = pm[1] * vx + pm[5] * vy + pm[9] * vz + pm[13] * vw;
      const cw = pm[3] * vx + pm[7] * vy + pm[11] * vz + pm[15] * vw;

      const ndcX = cx / cw;
      const ndcY = cy / cw;

      const canvas = document.querySelector('[data-testid="viewport-canvas"] canvas') as HTMLCanvasElement | null;
      if (!canvas) throw new Error("planePointToClient: viewport canvas missing");
      const rect = canvas.getBoundingClientRect();
      return {
        x: rect.left + ((ndcX + 1) / 2) * rect.width,
        y: rect.top + ((1 - ndcY) / 2) * rect.height,
      };
    },
    { plane, point },
  );
}

/**
 * A deliberate click (move → down → up, no drag) at ABSOLUTE client coordinates
 * — same tap shape as `clickAt`, for points obtained from `planePointToClient`
 * rather than a canvas-center-relative offset.
 */
export async function clickAtClient(page: Page, x: number, y: number): Promise<void> {
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.up();
}

/** Read the model-tool debug surface (`?vpdebug`) — `phase`, `revolvePhase`, … */
export async function extrudeDebug(page: Page): Promise<Record<string, unknown> | null> {
  return page.evaluate(
    () => (window as unknown as { __extrudePreview?: Record<string, unknown> }).__extrudePreview ?? null,
  );
}

/**
 * Drive the commit gesture on the armed extrude: grab the real depth handle,
 * drag, then RELEASE — which KEEPS the tool armed (no implicit commit). The
 * caller still has to confirm explicitly (Enter / chip-✓ / click-away), because
 * `ModelToolController` commits only on that.
 *
 * Since UNIFY-UX (`7d7c82a`) a genuine grab is REQUIRED: confirming without one
 * leaves the depth at zero and no body is created, which is a silent no-op from
 * a spec's point of view. The grab is retried because the handle's screen
 * position depends on a settled camera, and a miss must not be read as "extrude
 * is broken" — the phase check turns a missed grab into a retry, not a failure.
 */
export async function dragExtrudeDepth(page: Page, dy = -20): Promise<void> {
  await expect(async () => {
    const { x, y } = await findExtrudeHandle(page);
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(x, y + dy, { steps: 4 });
    const phase = (await extrudeDebug(page))?.phase;
    if (phase !== "dragging") {
      await page.mouse.up();
      throw new Error(`extrude grab missed — phase was ${String(phase)}`);
    }
  }).toPass({ timeout: 10_000, intervals: [200, 400, 800] });
  await page.mouse.up();
}

// ── Live dimension chips (SP-1 W4) ───────────────────────────────────────────

/** One live dimension chip's renderable facts, as read straight off the store
 *  (`liveDimStore` — `LiveDimChipField` minus its plane anchor). Reading the
 *  store instead of the DOM avoids depending on the chip's formatted/unit text. */
export interface LiveDimSnapshot {
  field: string;
  value: number;
  locked: boolean;
}

/** The currently open live-dimension chip set (`__stores.liveDim`). Empty when
 *  no gesture has an armed dimension frame (idle tool, or the chips pref is off). */
export async function getLiveDims(page: Page): Promise<LiveDimSnapshot[]> {
  return page.evaluate(() => {
    const w = window as unknown as {
      __stores?: { liveDim: { getState(): { fields: LiveDimSnapshot[] } } };
    };
    return (w.__stores?.liveDim.getState().fields ?? []).map((f) => ({
      field: f.field,
      value: f.value,
      locked: f.locked,
    }));
  });
}

/**
 * Flip a snap/show preference straight through the settings store
 * (`__stores.settings.setSnap`) — the sanctioned escape hatch for a spec that
 * needs e.g. `dimensionRound` off (raw, unrounded coordinates) without driving
 * the snap popover UI. `key` is whichever `SnapSettings` field the spec needs.
 */
export async function setSnapPref(page: Page, key: string, value: boolean): Promise<void> {
  await page.evaluate(
    ({ key, value }) => {
      const w = window as unknown as {
        __stores?: { settings: { getState(): { setSnap(key: string, value: boolean): void } } };
      };
      w.__stores?.settings.getState().setSnap(key, value);
    },
    { key, value },
  );
}

/**
 * A live dimension chip's `<input>`, by field id (`"length"`, `"angle"`, `"width"`,
 * …) — `LiveDimField.tsx`'s `data-testid={"live-dim-"+field}`. Mounted in an
 * overlay layer with `aria-hidden` on an ancestor (same convention as the
 * constraint badges), so this is a plain attribute selector rather than a
 * role/label query — those respect the accessibility tree, this does not.
 */
export function liveDimField(page: Page, field: string): Locator {
  return page.locator(`[data-testid="live-dim-${field}"]`);
}

export async function commitExtrudeAtHandle(page: Page): Promise<void> {
  // The whole gesture is wrapped in a retry: an occasional handle-scan miss (the
  // scan and the real pointerdown are a frame apart — a render-on-demand engine can
  // re-render the handle in between) leaves the tool armed rather than erroring, so
  // the only reliable signal is the durable side effect of a REAL commit:
  // finishExtrude() flips the Extrude toolbar button back to unpressed on success.
  const extrudeBtn = page.getByRole("button", { name: "Extrude", exact: true });
  await expect(async () => {
    const { x, y } = await findExtrudeHandle(page);
    let pointerDown = false;
    try {
      await page.mouse.move(x, y);
      await page.mouse.down();
      pointerDown = true;
      await page.mouse.move(x, y + 12, { steps: 4 });
      // The grab must have landed — assert the phase became "dragging" MID-drag. Without
      // this a missed grab (scan/pointerdown are a frame apart) leaves the tool armed the
      // whole time, so the "release keeps armed" check below passes vacuously. Inside the
      // toPass retry, a real miss re-scans + re-grabs on the next attempt.
      expect((await extrudeDebug(page))?.phase).toBe("dragging");
      await page.mouse.up(); // release → stays armed (no implicit commit)
      pointerDown = false;
      // Debug surface: the tool must remain armed after the release.
      expect((await extrudeDebug(page))?.phase).toBe("armed");
      await page.keyboard.press("Enter"); // explicit confirm → commit
      await expect(extrudeBtn).not.toHaveAttribute("aria-pressed", "true", { timeout: 1_500 });
    } finally {
      if (pointerDown) await page.mouse.up();
    }
  }).toPass({ timeout: 20_000, intervals: [200, 500, 1_000] });
}
