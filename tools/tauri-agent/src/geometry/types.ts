/**
 * Coordinate spaces used by the agent.
 *
 *   css     — CSS pixels, origin at the webview content's top-left.
 *   window  — window-relative points. With `titleBarStyle: Overlay` + `hiddenTitle`
 *             the content view spans the whole frame, so this equals `css`.
 *   global  — display points, top-left origin, y down. What CGEvent consumes.
 */
export interface Pt {
  x: number;
  y: number;
}

/** css px, viewport-relative. */
export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface WindowGeom {
  /** tauri innerPosition() — PHYSICAL px, so it must be divided by scaleFactor. */
  innerPositionPx: Pt;
  innerSizePx: { width: number; height: number };
  scaleFactor: number;
  /** from CGWindowList (global points, top-left origin) */
  nativeBoundsPt: Rect;
  /** CGWindowID */
  windowId: number;
}

export type Space = "webview" | "window" | "global";
