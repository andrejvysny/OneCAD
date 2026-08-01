/*
 * Viewport chrome store.
 *
 * Holds the camera / display state the status bar + corner cluster render. The
 * viewport engine (F-WP4) writes `cursor` (pointer raycast onto Z=0) and
 * `cameraViewLabel` (on camera change) through ViewportRoot; `zoomFit`/`homeView`
 * dispatch to the live engine via the bridge. DOF is still a solver mock.
 */
import { createStore, useStore } from "zustand";
import { getViewportEngine } from "@/viewport/engineBridge";
import { selectedBodyIds, selectionStore } from "@/stores/selectionStore";
import type { InputDevice } from "@/viewport/engine/navInput";

export type Projection = "persp" | "ortho";
export type DisplayMode = "shaded" | "shadedEdges" | "wireframe";

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

const DISPLAY_CYCLE: DisplayMode[] = ["shaded", "shadedEdges", "wireframe"];

/** Human label for the display-mode button (its own tooltip + aria-label). */
export const DISPLAY_MODE_LABEL: Record<DisplayMode, string> = {
  shaded: "Shaded",
  shadedEdges: "Shaded + edges",
  wireframe: "Wireframe",
};

/** Sticky status hint shown for the whole time isolation is on. */
export const ISOLATE_HINT = "Isolation on — Esc or ⇧I to exit";

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
  displayMode: DisplayMode;
  gridVisible: boolean;
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
  /** Current DOF count the shell displays (mirrors the active sketch solver). */
  dofBadge: number | null;
  /** Status-bar hint (tool prompt, error, or transient confirmation). */
  statusHint: StatusHint | null;
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
  setPendingExtrude(sketchId: string | null): void;
  setProjection(p: Projection): void;
  cycleDisplayMode(): void;
  toggleGrid(): void;
  setActiveSketch(id: string | null): void;
  /** Engine → store: live pointer read-out (raycast onto Z=0). */
  setCursor(c: CursorCoords): void;
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
}

export const viewportStore = createStore<ViewportState>()((set, get) => ({
  projection: "persp",
  // shadedEdges is what the renderer has always actually drawn (BodyObject adds
  // a face Mesh AND an edge LineSegments), so this is the honest default — and
  // it makes the first click on the display button visibly change something.
  displayMode: "shadedEdges",
  // On by default: the grid renders (GridPlane) so the viewport never looks
  // empty; the grid button shows the pressed (accent) treatment to match.
  gridVisible: true,
  activeSketchId: null,
  cameraViewLabel: "TOP",
  detectedInputDevice: "mouse",
  fov: 76,
  cursor: { x: 273, y: 210, z: 0 },
  dofBadge: null,
  statusHint: null,
  pendingExtrudeSketch: null,
  isolatedBodyIds: null,

  setPendingExtrude(sketchId) {
    set({ pendingExtrudeSketch: sketchId });
  },

  setProjection(p) {
    set({ projection: p });
  },

  cycleDisplayMode() {
    set((s) => {
      const next = DISPLAY_CYCLE[(DISPLAY_CYCLE.indexOf(s.displayMode) + 1) % DISPLAY_CYCLE.length];
      return { displayMode: next };
    });
  },

  toggleGrid() {
    set((s) => ({ gridVisible: !s.gridVisible }));
  },

  setActiveSketch(id) {
    set({ activeSketchId: id });
  },

  setCursor(c) {
    set({ cursor: c });
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
      set({ statusHint: null });
      return;
    }
    const severity = opts?.severity ?? "info";
    const sticky = opts?.sticky ?? false;
    set({ statusHint: { message, severity, sticky } });
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
}));

/** Typed selector hook over the vanilla store. */
export function useViewportStore<T>(selector: (s: ViewportState) => T): T {
  return useStore(viewportStore, selector);
}

/**
 * Format the mono X/Y/Z read-out exactly like prototype 1c (white-space:pre):
 *   "X  273.00   Y  210.00   Z    0.00"
 * (axis + 2 spaces + value right-padded to width 6, columns joined by 3 spaces).
 */
export function formatCursor(c: CursorCoords): string {
  const cols: [string, number][] = [
    ["X", c.x],
    ["Y", c.y],
    ["Z", c.z],
  ];
  return cols.map(([ax, v]) => `${ax}  ${v.toFixed(2).padStart(6)}`).join("   ");
}
