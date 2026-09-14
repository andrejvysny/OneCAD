import { AgentError } from "../errors.ts";
import type { Pt, Rect, WindowGeom } from "./types.ts";

/**
 * The whole native-input story rests on one identity:
 *
 *   global = innerPosition / scaleFactor + cssPoint
 *
 * `innerPosition()` is PHYSICAL px while `getBoundingClientRect()` is CSS px, so
 * the division is not cosmetic — on a 2x display, dropping it puts the cursor in
 * a different application.
 */
function contentOrigin(g: WindowGeom): Pt {
  const sf = g.scaleFactor;
  if (!Number.isFinite(sf) || sf <= 0) {
    throw new AgentError("CALIBRATION_FAILED", `scaleFactor ${String(sf)} is not usable`, {
      remediation: "Re-read the window geometry; a non-positive scale factor means the bridge returned a stale or unresolved value.",
      details: { scaleFactor: sf },
    });
  }
  return { x: g.innerPositionPx.x / sf, y: g.innerPositionPx.y / sf };
}

function logicalSize(g: WindowGeom): { width: number; height: number } {
  const sf = g.scaleFactor;
  return { width: g.innerSizePx.width / sf, height: g.innerSizePx.height / sf };
}

export function cssToGlobal(css: Pt, g: WindowGeom): Pt {
  const o = contentOrigin(g);
  return { x: o.x + css.x, y: o.y + css.y };
}

export function globalToCss(global: Pt, g: WindowGeom): Pt {
  const o = contentOrigin(g);
  return { x: global.x - o.x, y: global.y - o.y };
}

/** Identical to `cssToGlobal`: the Overlay title bar makes content == window. */
export function windowToGlobal(win: Pt, g: WindowGeom): Pt {
  return cssToGlobal(win, g);
}

/**
 * Half-open on the far edges: a point at exactly `origin + size` addresses the
 * first pixel OUTSIDE the window, which is somebody else's app.
 */
export function pointInWindow(global: Pt, g: WindowGeom): boolean {
  if (!Number.isFinite(global.x) || !Number.isFinite(global.y)) return false;
  const o = contentOrigin(g);
  const s = logicalSize(g);
  return global.x >= o.x && global.x < o.x + s.width && global.y >= o.y && global.y < o.y + s.height;
}

/** `rects` are css px, content-relative (the macOS traffic lights, typically). */
export function pointInNativeOcclusion(css: Pt, rects: Rect[]): boolean {
  return rects.some(
    (r) => css.x >= r.x && css.x < r.x + r.width && css.y >= r.y && css.y < r.y + r.height,
  );
}

/**
 * JSON carries no NaN or Infinity: the embedded driver serializes both as `null`,
 * and `null` arithmetics as 0. Every rect that crosses the bridge is screened here
 * before any comparison can quietly succeed on blanks.
 */
export function rectIsFinite(r: Rect | null | undefined): r is Rect {
  return (
    r !== null &&
    r !== undefined &&
    Number.isFinite(r.x) &&
    Number.isFinite(r.y) &&
    Number.isFinite(r.width) &&
    Number.isFinite(r.height)
  );
}

export function rectStable(a: Rect, b: Rect, tolPx = 0.5): boolean {
  if (!rectIsFinite(a) || !rectIsFinite(b)) return false;
  return (
    Math.abs(a.x - b.x) <= tolPx &&
    Math.abs(a.y - b.y) <= tolPx &&
    Math.abs(a.width - b.width) <= tolPx &&
    Math.abs(a.height - b.height) <= tolPx
  );
}

export function rectCenter(r: Rect, offset?: Pt): Pt {
  return {
    x: r.x + r.width / 2 + (offset?.x ?? 0),
    y: r.y + r.height / 2 + (offset?.y ?? 0),
  };
}

/**
 * The last gate before a native event is posted. Refuses rather than guesses:
 * a point outside the window, or under a native control the webview cannot see,
 * would land on another application.
 */
export function checkPoint(css: Pt, geom: WindowGeom, occlusionRects: Rect[]): Pt {
  const global = cssToGlobal(css, geom);
  if (!pointInWindow(global, geom)) {
    throw new AgentError(
      "POINT_OUTSIDE_WINDOW",
      `point css(${css.x}, ${css.y}) maps to global(${global.x}, ${global.y}), outside the window content box`,
      {
        details: {
          css,
          global,
          contentOrigin: contentOrigin(geom),
          logicalSize: logicalSize(geom),
          windowId: geom.windowId,
        },
      },
    );
  }
  if (pointInNativeOcclusion(css, occlusionRects)) {
    throw new AgentError(
      "ELEMENT_OCCLUDED",
      `point css(${css.x}, ${css.y}) is under a native window control`,
      {
        remediation:
          "The macOS traffic lights sit above the webview. Target an element outside nativeOcclusion, or scroll it into a clear area.",
        details: { reason: "native-titlebar-controls", css, global },
      },
    );
  }
  return global;
}
