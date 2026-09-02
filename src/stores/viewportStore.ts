/*
 * Viewport chrome store.
 *
 * Holds the camera / display state the status bar + corner cluster render. The
 * viewport engine (F-WP4) writes `cursor` (pointer raycast onto Z=0) plus its
 * plane-projected sibling `cursorPlaneUV` (design item 11) and
 * `cameraViewLabel` (on camera change) through ViewportRoot; `zoomFit`/`homeView`
 * dispatch to the live engine via the bridge. DOF is still a solver mock.
 */
import { createStore, useStore } from "zustand";
import { getViewportEngine } from "@/viewport/engineBridge";
import { selectedBodyIds, selectionStore } from "@/stores/selectionStore";
import { log } from "@/debug/log";
import { formatCursorAxis, lengthSuffix } from "@/units/format";
import type { LengthUnitId } from "@/units/lengthUnits";
import type { InputDevice } from "@/viewport/engine/navInput";

export type Projection = "persp" | "ortho";

export type StatusSeverity = "info" | "error";

/**
 * A status-bar hint carries its own presentation policy:
 *   - `severity` picks the text treatment (`error` renders red).
 *   - `sticky` hints persist until superseded/cleared (tool prompts + errors);
 *     non-sticky hints auto-dismiss after {@link AUTO_DISMISS_MS} (success/reject
 *     confirmations).
 */
export interface StatusHint {
  message: string;
  severity: StatusSeverity;
  sticky: boolean;
}

export interface CursorCoords {
  x: number;
  y: number;
  z: number;
}

/**
 * The SAME pointer hit as `cursor`, projected onto the active sketch plane
 * (design item 11 / audit A8): on a non-Z-up-XY plane `cursor`'s world Y is
 * not the sketch's own +U, so a typed dimension can't be checked against it.
 * Written by the identical engine/writer that sets `cursor` — never a second
 * per-frame path — via `ViewportEngine.screenToPlane`, the same projection
 * `SketchController`'s own gestures use. `{u:0,v:0}` outside a sketch session
 * (no `sketchPlane` on the engine ⇒ `screenToPlane` returns null there).
 */
export interface CursorPlaneCoords {
  u: number;
  v: number;
}

/** Sticky status hint shown for the whole time isolation is on. */
export const ISOLATE_HINT = "Isolation on — Esc or ⇧I to exit";

/** Sticky status hint shown for the whole time the section view is on. */
export const SECTION_HINT = "Section view on — Esc or ⇧X to exit";

/** Refusal shown when the backend cannot clip at all (experimental WebGPU). */
export const SECTION_UNSUPPORTED_HINT =
  "Section view is unavailable on the experimental WebGPU renderer";

/** The three world planes a section cut can run along (its normal's axis). */
export type SectionPlaneId = "XY" | "XZ" | "YZ";

/**
 * TRANSIENT section (cut-away) view. Never persisted and never a document fact:
 * it clips what the VIEWPORT draws and writes nothing, exactly like
 * {@link ViewportState.isolatedBodyIds}. `plane`/`offsetMm`/`flip` survive a
 * toggle so re-enabling returns to the cut the user set up, and the cut is
 * turned OFF on document close (`resetDocumentScopedUi`) — a plane positioned
 * against the outgoing model means nothing against the incoming one.
 */
export interface SectionState {
  enabled: boolean;
  plane: SectionPlaneId;
  /** Distance along the plane's axis, in MILLIMETRES (the document unit). */
  offsetMm: number;
  /** Keep the other half of the model instead. */
  flip: boolean;
}

export const SECTION_DEFAULT: SectionState = {
  enabled: false,
  plane: "XY",
  offsetMm: 0,
  flip: false,
};

/**
 * Where the cut should start on a given plane: the MIDDLE of the scene along
 * that plane's axis.
 *
 * Not 0. The unflipped cut keeps `axis · p <= offset`, so a plain
 * sketch-on-XY-then-extrude body — which occupies z ∈ [0, depth] — is discarded
 * ENTIRELY by a cut at the origin: an empty viewport, no cross-section, and no
 * cap to explain it, recoverable only by finding the slider. Seeding from the
 * scene means turning the section on always cuts THROUGH something.
 *
 * `null` from the engine (no bodies, or no engine yet) falls back to 0, which is
 * the only defensible guess with nothing to measure.
 */
function seededOffset(plane: SectionPlaneId): number {
  const range = getViewportEngine()?.sectionOffsetRange(plane) ?? null;
  return range ? (range.min + range.max) / 2 : 0;
}

/** Non-sticky hints self-clear after this long. */
export const AUTO_DISMISS_MS = 4000;

// Auto-dismiss lives in the module (not React): each `setStatusHint` bumps a token
// and, for a non-sticky hint, arms one timer keyed to that token. The timer clears
// the hint only if its token is still current (latest-wins), so a newer hint that
// re-armed the timer is never clobbered by a stale one. Every set/clear cancels the
// outstanding timer first, so a sticky hint or an explicit clear stops the clock.
let dismissTimer: ReturnType<typeof setTimeout> | null = null;
let hintToken = 0;

function cancelDismiss(): void {
  if (dismissTimer !== null) {
    clearTimeout(dismissTimer);
    dismissTimer = null;
  }
}

export interface ViewportState {
  projection: Projection;
  gridVisible: boolean;
  /**
   * Side of ONE minor grid cell, in MILLIMETRES (the document unit — the
   * readout converts for display).
   *
   * Engine → store, pushed from the camera-change listener in `ViewportRoot`
   * and NOT per frame: `chooseGridStep` is a step function of camera distance,
   * so it changes a handful of times across a whole zoom.
   */
  gridStep: number;
  activeSketchId: string | null;
  cameraViewLabel: string;
  /**
   * What the wheel heuristic currently believes the input device is. RUNTIME
   * ONLY — never persisted (a stale classification is exactly the bug the
   * per-segment classifier avoids). Shown as the "Auto" sublabel so a wrong
   * guess is visible instead of mysterious.
   */
  detectedInputDevice: InputDevice;
  fov: number;
  cursor: CursorCoords;
  /** `cursor`'s plane-local (u, v) counterpart — see {@link CursorPlaneCoords}. */
  cursorPlaneUV: CursorPlaneCoords;
  /** Current DOF count the shell displays (mirrors the active sketch solver). */
  dofBadge: number | null;
  /** Status-bar hint (tool prompt, error, or transient confirmation). */
  statusHint: StatusHint | null;
  /** Bumped every `setStatusHint` — lets a REPEATED message re-key its consumer. */
  statusHintSeq: number;
  /** Finish-sketch → auto-arm extrude handoff: the sketch just finished (F-WP7). */
  pendingExtrudeSketch: string | null;
  /**
   * TRANSIENT body isolation (W3). `null` = off; otherwise exactly the bodies
   * that stay visible.
   *
   * This is a VIEW mask, never a document fact: the tree eye
   * (`SetVisibility`) stays the persisted truth and MeshIngest ANDs the two, so
   * a body hidden by the eye stays hidden inside an isolate set and re-hides
   * when isolation ends. Cleared on document close (`resetDocumentScopedUi`) and
   * whenever a model tool arms.
   */
  isolatedBodyIds: string[] | null;
  /**
   * TRANSIENT section view (see {@link SectionState}). Read by `ViewportRoot`,
   * which drives BOTH body-material owners plus the engine's `SectionLayer` —
   * the same two-owner shape the theme refresh has.
   */
  section: SectionState;
  /**
   * True while the document is READY but at least one visible body has no
   * scene object yet (the open-to-first-mesh window). Owned by `MeshIngest`
   * (`src/viewport/mesh/meshSync.ts`); drives the "Rebuilding geometry…" chip.
   */
  geometryPending: boolean;
  setPendingExtrude(sketchId: string | null): void;
  setProjection(p: Projection): void;
  toggleGrid(): void;
  /** Engine → store: side of one minor grid cell (mm). No-op when unchanged. */
  setGridStep(mm: number): void;
  setActiveSketch(id: string | null): void;
  /**
   * Engine → store: live pointer read-out — `c` is the raycast onto Z=0
   * (world XYZ, unchanged behavior), `planeUV` is the SAME hit projected onto
   * the active sketch plane (null when no sketch plane is live).
   */
  setCursor(c: CursorCoords, planeUV: CursorPlaneCoords | null): void;
  /** Engine → store: canonical view name (TOP/FRONT/…/ISO/—). */
  setCameraViewLabel(label: string): void;
  setDetectedInputDevice(device: InputDevice): void;
  /**
   * Set (or clear, with `null`) the status-bar hint. Plain `setStatusHint(msg)`
   * shows a non-sticky info hint that auto-dismisses; pass `opts` to raise the
   * severity or make it sticky. Every call cancels any pending auto-dismiss.
   */
  setStatusHint(message: string | null, opts?: { severity?: StatusSeverity; sticky?: boolean }): void;
  /**
   * Frame the SELECTION when one names bodies (a body ref, or the owning body of
   * a face/edge pick); otherwise frame the whole scene. Dispatches to the live
   * viewport engine (no-op until it mounts).
   */
  zoomFit(): void;
  homeView(): void;
  /** Isolate the bodies the selection names. No-op when it names none. */
  isolateSelection(): void;
  /** Leave isolation (Esc ladder, model-tool arm, document close). Idempotent. */
  exitIsolate(): void;
  /**
   * ⇧I / the NavPill button: enter or leave isolation.
   *
   * IGNORED — in BOTH directions — while an armed preview is holding committed
   * bodies hidden. The engine saves each body's `visible` flag when the preview
   * hides it and replays that snapshot on restore, so flipping isolation
   * underneath a live preview would either resurrect a body isolation hid or
   * strand one the preview hid. The unconditional {@link exitIsolate} is safe
   * because every path that calls it (Esc with no armed tool, the tool-arm hook,
   * document close) runs after the preview has already been cancelled.
   */
  toggleIsolate(): void;
  /**
   * ⇧X / the NavPill button / the Layers row: turn the section view on or off.
   * REFUSED on the WebGPU backend, which ignores `material.clippingPlanes`.
   */
  toggleSection(): void;
  /** Which world plane the cut runs along. Leaves the offset alone. */
  setSectionPlane(plane: SectionPlaneId): void;
  /** Move the cut along its axis (mm) — called once per slider/drag tick. */
  setSectionOffset(offsetMm: number): void;
  /** Keep the other half. */
  flipSection(): void;
  /** Leave the section view (Esc ladder, document close). Idempotent. */
  exitSection(): void;
  /** Set (no-op-guarded to avoid render churn) whether geometry is still loading. */
  setGeometryPending(pending: boolean): void;
}

export const viewportStore = createStore<ViewportState>()((set, get) => ({
  projection: "persp",
  // On by default: the grid renders (GridPlane) so the viewport never looks
  // empty; the grid button shows the pressed (accent) treatment to match.
  gridVisible: true,
  // Placeholder only. The engine pushes the real step as soon as it mounts, and
  // the readout is gated on the engine being ready, so this is never displayed.
  // NOT seeded via `chooseGridStep`: that would drag three.js into every module
  // that touches this store, including the startup chrome.
  gridStep: 10,
  activeSketchId: null,
  cameraViewLabel: "TOP",
  detectedInputDevice: "mouse",
  fov: 76,
  cursor: { x: 273, y: 210, z: 0 },
  cursorPlaneUV: { u: 0, v: 0 },
  dofBadge: null,
  statusHint: null,
  statusHintSeq: 0,
  pendingExtrudeSketch: null,
  isolatedBodyIds: null,
  section: SECTION_DEFAULT,
  geometryPending: false,

  setPendingExtrude(sketchId) {
    set({ pendingExtrudeSketch: sketchId });
  },

  setProjection(p) {
    set({ projection: p });
  },

  toggleGrid() {
    set((s) => ({ gridVisible: !s.gridVisible }));
  },

  setGridStep(mm) {
    if (!(mm > 0) || get().gridStep === mm) return;
    set({ gridStep: mm });
  },

  setActiveSketch(id) {
    set({ activeSketchId: id });
  },

  setCursor(c, planeUV) {
    set({ cursor: c, cursorPlaneUV: planeUV ?? { u: 0, v: 0 } });
  },

  setDetectedInputDevice(device) {
    set({ detectedInputDevice: device });
  },
  setCameraViewLabel(label) {
    set({ cameraViewLabel: label });
  },

  setStatusHint(message, opts) {
    const token = ++hintToken;
    cancelDismiss();
    if (message === null) {
      set({ statusHint: null, statusHintSeq: token });
      return;
    }
    const severity = opts?.severity ?? "info";
    const sticky = opts?.sticky ?? false;
    // Every hint the user actually SAW, in the one place all 90+ callers pass
    // through — grep tag `hint` to reconstruct what the UI told them, and when.
    // A CLEAR (message === null) is silent: it carries no information.
    log(severity === "error" ? "warn" : "debug", "hint", message, { severity, sticky });
    set({ statusHint: { message, severity, sticky }, statusHintSeq: token });
    if (!sticky) {
      dismissTimer = setTimeout(() => {
        dismissTimer = null;
        if (hintToken !== token) return; // superseded by a newer hint — leave it
        set({ statusHint: null });
      }, AUTO_DISMISS_MS);
    }
  },

  // Dispatch to the live engine via the bridge; no-op before it mounts.
  zoomFit() {
    const engine = getViewportEngine();
    if (!engine) return;
    const ids = selectedBodyIds(selectionStore.getState().selected);
    // A selection of only sketches/regions/datums yields no ids — that is a
    // fit-all, not a fit-nothing.
    if (ids.length > 0) engine.fitToBodies(ids);
    else engine.fitView();
  },
  homeView() {
    getViewportEngine()?.homeView();
  },

  isolateSelection() {
    const ids = selectedBodyIds(selectionStore.getState().selected);
    if (ids.length === 0) return;
    set({ isolatedBodyIds: ids });
    get().setStatusHint(ISOLATE_HINT, { sticky: true });
  },

  exitIsolate() {
    if (get().isolatedBodyIds === null) return;
    set({ isolatedBodyIds: null });
    // Clear only OUR hint: a tool that armed while isolation was on has already
    // published its own prompt, and stomping it would lose the live instruction.
    if (get().statusHint?.message === ISOLATE_HINT) get().setStatusHint(null);
  },

  toggleIsolate() {
    if (getViewportEngine()?.hasPreviewHiddenBodies()) return;
    if (get().isolatedBodyIds !== null) get().exitIsolate();
    else get().isolateSelection();
  },

  toggleSection() {
    // The NavPill button is disabled on WebGPU, but ⇧X is not — and turning on a
    // cut that cannot cut (three's WebGPU materials ignore clippingPlanes) would
    // announce a section that never appears. Say so instead.
    if (getViewportEngine()?.isWebGPU) {
      get().setStatusHint(SECTION_UNSUPPORTED_HINT, { severity: "error" });
      return;
    }
    if (get().section.enabled) get().exitSection();
    else {
      // Re-seeded on every enable, which also stops a stale offset from the
      // PREVIOUS document surviving into this one (the plane/offset deliberately
      // outlive a toggle, so nothing else would reset them).
      set((s) => ({
        section: { ...s.section, enabled: true, offsetMm: seededOffset(s.section.plane) },
      }));
      get().setStatusHint(SECTION_HINT, { sticky: true });
    }
  },

  setSectionPlane(plane) {
    if (get().section.plane === plane) return;
    // The offset was measured along the OLD axis, so carrying it over would put
    // the cut somewhere the user never asked for — and 0 can be entirely off the
    // model. Re-seed to the middle of the scene along the NEW axis.
    set((s) => ({ section: { ...s.section, plane, offsetMm: seededOffset(plane) } }));
  },

  setSectionOffset(offsetMm) {
    if (!Number.isFinite(offsetMm) || get().section.offsetMm === offsetMm) return;
    set((s) => ({ section: { ...s.section, offsetMm } }));
  },

  flipSection() {
    set((s) => ({ section: { ...s.section, flip: !s.section.flip } }));
  },

  exitSection() {
    if (!get().section.enabled) return;
    set((s) => ({ section: { ...s.section, enabled: false } }));
    // Clear only OUR hint — same rule as exitIsolate: a tool that armed while
    // the section was on has already published its own prompt.
    if (get().statusHint?.message === SECTION_HINT) get().setStatusHint(null);
  },

  setGeometryPending(pending) {
    if (get().geometryPending !== pending) set({ geometryPending: pending });
  },
}));

/** Typed selector hook over the vanilla store. */
export function useViewportStore<T>(selector: (s: ViewportState) => T): T {
  return useStore(viewportStore, selector);
}

/**
 * Shared column formatter for both the X/Y/Z and U/V read-outs (white-space:pre):
 *   "X  273.00   Y  210.00   Z    0.00  mm"
 * (axis + 2 spaces + value right-padded to width 6, columns joined by 3 spaces),
 * with the display unit named once at the end rather than on every column.
 *
 * Values are millimetres, like everything else the store holds; `unit` is
 * display only. Passing it explicitly (rather than reading the setting inside
 * `formatCursorAxis`) is what makes StatusBar's subscription the thing that
 * drives the re-render — a hidden read would leave the row stale until some
 * unrelated state changed.
 */
function formatAxes(cols: [string, number][], unit?: LengthUnitId): string {
  const row = cols.map(([ax, v]) => `${ax}  ${formatCursorAxis(v, unit)}`).join("   ");
  return `${row}  ${lengthSuffix(unit)}`;
}

/** World X/Y/Z cursor read-out (model mode). */
export function formatCursor(c: CursorCoords, unit?: LengthUnitId): string {
  return formatAxes(
    [
      ["X", c.x],
      ["Y", c.y],
      ["Z", c.z],
    ],
    unit,
  );
}

/**
 * Sketch-plane U/V cursor read-out (design item 11 / audit A8) — same column
 * shape as {@link formatCursor}, shown instead of world X/Y/Z while a sketch
 * session is active, so the numbers on screen match the plane a typed
 * dimension is measured against.
 */
export function formatCursorPlane(c: CursorPlaneCoords, unit?: LengthUnitId): string {
  return formatAxes(
    [
      ["U", c.u],
      ["V", c.v],
    ],
    unit,
  );
}
