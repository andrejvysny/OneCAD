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
  | "Polygon"
  | "Slot"
  | "Point"
  | "Select";

async function bootEditor(page: Page, path: string): Promise<void> {
  await page.goto(path);
  await page.getByRole("button", { name: "New project" }).click();
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
export async function openEditorDebug(page: Page): Promise<void> {
  await bootEditor(page, "/?vpdebug");
}

/** The viewport container's bounding box (the canvas fills the same rect). */
async function canvasBox(page: Page): Promise<{ x: number; y: number; width: number; height: number }> {
  const box = await page.locator(CANVAS).boundingBox();
  if (!box) throw new Error("viewport canvas has no bounding box");
  return box;
}

/** Canvas-center-relative offset (px) → absolute page coordinates. */
async function toPage(page: Page, dx: number, dy: number): Promise<{ x: number; y: number }> {
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

/** Select a sketch drawing tool from the floating toolbar and confirm it armed. */
export async function selectSketchTool(page: Page, label: SketchToolLabel): Promise<void> {
  const btn = page.getByRole("button", { name: label, exact: true });
  await btn.click();
  await expect(btn).toHaveAttribute("aria-pressed", "true");
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
 * engine's own `datumHitTest`. Returns the pixel plus the datum id found there.
 * Same rationale as `findPlaneQuad`: a datum sits at an arbitrary offset with no
 * closed-form screen position.
 */
export async function findDatumQuad(page: Page): Promise<{ x: number; y: number; id: string }> {
  let found: { x: number; y: number; id: string } | null = null;
  await expect(async () => {
    found = await page.evaluate(() => {
      const engine = (
        window as unknown as { __vpEngine?: { datumHitTest(x: number, y: number): string | null } }
      ).__vpEngine;
      const canvas = document.querySelector('[data-testid="viewport-canvas"] canvas') as HTMLCanvasElement | null;
      if (!engine || !canvas) return null;
      const rect = canvas.getBoundingClientRect();
      const step = 8;
      for (let y = rect.top + step; y <= rect.bottom - step; y += step) {
        for (let x = rect.left + step; x <= rect.right - step; x += step) {
          const id = engine.datumHitTest(x, y);
          if (id) return { x, y, id };
        }
      }
      return null;
    });
    expect(found).not.toBeNull();
  }).toPass({ timeout: 15_000, intervals: [150, 300, 600, 1_000] });
  return found as unknown as { x: number; y: number; id: string };
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
 * The persistent sketch constraint toolbar (SketchConstraintToolbar.tsx —
 * role="toolbar" aria-label="Constraints"). ConstraintContextChips renders a
 * SECOND floating row with the exact same button labels near the selection
 * centroid whenever the applicable set is non-empty, so any geometric/
 * dimensional constraint button lookup MUST be scoped to this toolbar —
 * an unscoped `getByRole("button", { name: "Horizontal" })` resolves to two
 * elements (strict-mode violation) as soon as something is selected.
 */
export function constraintToolbar(page: Page): Locator {
  return page.getByRole("toolbar", { name: "Constraints" });
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
  const isSettled = (): Promise<boolean> =>
    page.evaluate(() => {
      const engine = (window as unknown as { __vpEngine?: { controls?: { tween: unknown } | null } }).__vpEngine;
      return !engine?.controls?.tween;
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
 * Drive the MODEL-HARDEN Wave 1 commit gesture on the armed extrude: grab the real
 * depth handle, drag, RELEASE (which now KEEPS the tool armed — no implicit
 * commit), assert it stayed armed via the debug surface, then press Enter to
 * confirm. `ModelToolController` commits only on this explicit confirm (Enter /
 * chip-✓ / click-away); a release alone just sets the depth and stays armed.
 */
export async function commitExtrudeAtHandle(page: Page): Promise<void> {
  // The whole gesture is wrapped in a retry: an occasional handle-scan miss (the
  // scan and the real pointerdown are a frame apart — a render-on-demand engine can
  // re-render the handle in between) leaves the tool armed rather than erroring, so
  // the only reliable signal is the durable side effect of a REAL commit:
  // finishExtrude() flips the Extrude toolbar button back to unpressed on success.
  const extrudeBtn = page.getByRole("button", { name: "Extrude", exact: true });
  await expect(async () => {
    const { x, y } = await findExtrudeHandle(page);
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(x, y + 12, { steps: 4 });
    // The grab must have landed — assert the phase became "dragging" MID-drag. Without
    // this a missed grab (scan/pointerdown are a frame apart) leaves the tool armed the
    // whole time, so the "release keeps armed" check below passes vacuously. Inside the
    // toPass retry, a real miss re-scans + re-grabs on the next attempt.
    expect((await extrudeDebug(page))?.phase).toBe("dragging");
    await page.mouse.up(); // release → stays armed (no implicit commit)
    // Debug surface: the tool must remain armed after the release.
    expect((await extrudeDebug(page))?.phase).toBe("armed");
    await page.keyboard.press("Enter"); // explicit confirm → commit
    await expect(extrudeBtn).not.toHaveAttribute("aria-pressed", "true", { timeout: 1_500 });
  }).toPass({ timeout: 20_000, intervals: [200, 500, 1_000] });
}
