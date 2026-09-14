/**
 * The CAD- and WebGL-aware idle reading.
 *
 * A DOM-mutation revision on its own is far too weak a settle predicate for this app.
 * An orbit changes camera matrices and repaints WebGL with ZERO DOM mutations, so the
 * action is declared settled while rendering is still in progress; and a click on
 * Extrude runs frontend → Rust → the OCCT worker → mesh generation → a Three.js upload,
 * which DOM quietness says nothing about. So this reads the signals the app already
 * publishes: the regen counter, the geometry-pending flag, the document revision, and
 * the renderer's monotonic frame count.
 *
 * Rendering is on-demand (`ViewportEngine.invalidate()` schedules exactly one rAF and an
 * idle viewport renders nothing), which is what makes "frameCount stopped moving" a TRUE
 * render-idle predicate rather than a heuristic.
 *
 * EVERY field is nullable and a null means "this signal is unavailable in this build" —
 * never "idle". The caller must report it, because silently degrading to DOM-only
 * settling is the defect this module exists to fix, not an acceptable fallback.
 *
 * Like `checkScript.ts` and `instrumentScript.ts`, the exported function is serialized
 * with `Function.prototype.toString` and evaluated in the webview, so it MUST NOT close
 * over anything in this module: every helper and constant lives inside its own body.
 * Only types cross the boundary, and types are erased.
 */

/** Camera pose, flattened so two readings compare by value. */
export interface IdleCamera {
  x: number;
  y: number;
  z: number;
  tx: number;
  ty: number;
  tz: number;
  distance: number;
}

export interface IdleReading {
  /** `__tauriAgentRev.rev`; -1 when the page carries no mutation observer. */
  rev: number;
  /** `documentStore.regenBusy` — how many regen jobs are in flight. */
  regenBusy: number | null;
  /** `viewportStore.geometryPending` — document READY, a visible body has no scene object. */
  geometryPending: boolean | null;
  /** `documentStore.revision`. */
  documentRevision: number | null;
  /** `__vpEngine.frameCount` — frames ACTUALLY rendered. */
  frames: number | null;
  camera: IdleCamera | null;
}

/** Zustand vanilla store surface; only `getState` is needed here. */
interface StoreLike {
  getState(): Record<string, unknown>;
}

interface IdleWindow extends Window {
  __tauriAgentRev?: { rev: number; lastMutationAt: number };
  /** One-shot latch: the debug chrome was found and removed, never attempt it again. */
  __tauriAgentChromeRemoved?: boolean;
  __stores?: Record<string, StoreLike | undefined>;
  __vpEngine?: {
    frameCount?: number;
    overlay?: { unregister?: (id: string) => void };
    debugSnapshot?: () => Record<string, unknown>;
  };
}

/**
 * One reading of every idle signal, plus a best-effort removal of the `?vpdebug` chrome.
 *
 * Defensive throughout: `__stores` is installed ASYNCHRONOUSLY (a `Promise.all` of dynamic
 * modules in `src/main.tsx`) so it is briefly absent after load, a store may lack a field,
 * and `debugSnapshot()` walks the scene and can throw. A throw in here must never fail the
 * action that is settling, so every signal is read under its own try/catch and reports null.
 */
export function readIdleInPage(): IdleReading {
  const w = window as unknown as IdleWindow;

  // ---- remove the ?vpdebug "origin" pill ----
  // The harness runs with `?vpdebug` so `__vpEngine` exists, but that flag also registers a
  // visible black "origin" chip anchored at the world origin — which in sketch mode is
  // exactly where the user draws — so it would appear in every screenshot this harness
  // captures as evidence. UNREGISTERING is the correct mechanism, not a style change:
  // `HtmlOverlayDriver.update` rewrites `display` on every registered visible item on every
  // frame. Detaching the node is REQUIRED as well, not merely tidy: unregistering stops the
  // driver writing to the element but leaves the last frame's `transform`/`display` in place,
  // so an attached node would simply freeze on screen. It also neutralises the engine's own
  // re-register when a sketch session closes, since a detached element shows nothing.
  //
  // Attempted on each reading until a pill is actually removed (the viewport mounts after the
  // bridge is built, so the engine does not exist yet at instrumentation time), then latched
  // off. Latching on the unregister alone would give up while a visible node was still there.
  if (w.__tauriAgentChromeRemoved !== true) {
    try {
      const engine = w.__vpEngine;
      const overlay = engine ? engine.overlay : undefined;
      if (overlay && typeof overlay.unregister === "function") {
        overlay.unregister("__debug_origin");
        const pills = document.querySelectorAll("[data-vp-debug-label]");
        pills.forEach((el) => {
          el.remove();
        });
        if (pills.length > 0) w.__tauriAgentChromeRemoved = true;
      }
    } catch {
      // The pill is cosmetic; never lose a reading over it.
    }
  }

  let rev = -1;
  try {
    const r = w.__tauriAgentRev;
    if (r && typeof r.rev === "number") rev = r.rev;
  } catch {
    rev = -1;
  }

  let regenBusy: number | null = null;
  let documentRevision: number | null = null;
  try {
    const stores = w.__stores;
    const doc = stores && stores.document ? stores.document.getState() : null;
    if (doc) {
      if (typeof doc.regenBusy === "number") regenBusy = doc.regenBusy;
      if (typeof doc.revision === "number") documentRevision = doc.revision;
    }
  } catch {
    regenBusy = null;
    documentRevision = null;
  }

  let geometryPending: boolean | null = null;
  try {
    const stores = w.__stores;
    const vp = stores && stores.viewport ? stores.viewport.getState() : null;
    if (vp && typeof vp.geometryPending === "boolean") geometryPending = vp.geometryPending;
  } catch {
    geometryPending = null;
  }

  let frames: number | null = null;
  try {
    const engine = w.__vpEngine;
    if (engine && typeof engine.frameCount === "number") frames = engine.frameCount;
  } catch {
    frames = null;
  }

  let camera: IdleCamera | null = null;
  try {
    const engine = w.__vpEngine;
    const snap = engine && typeof engine.debugSnapshot === "function" ? engine.debugSnapshot() : null;
    const pos = snap ? snap.camPos : null;
    const target = snap ? snap.target : null;
    const distance = snap ? snap.distance : null;
    if (
      Array.isArray(pos) &&
      Array.isArray(target) &&
      typeof distance === "number" &&
      typeof pos[0] === "number" &&
      typeof pos[1] === "number" &&
      typeof pos[2] === "number" &&
      typeof target[0] === "number" &&
      typeof target[1] === "number" &&
      typeof target[2] === "number"
    ) {
      camera = {
        x: pos[0],
        y: pos[1],
        z: pos[2],
        tx: target[0],
        ty: target[1],
        tz: target[2],
        distance,
      };
    }
  } catch {
    camera = null;
  }

  return { rev, regenBusy, geometryPending, documentRevision, frames, camera };
}
