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

/** Same half-open rule as `pointInWindow`, against a rect in whatever space both share. */
export function pointInRect(p: Pt, r: Rect): boolean {
  if (!Number.isFinite(p.x) || !Number.isFinite(p.y) || !rectIsFinite(r)) return false;
  return p.x >= r.x && p.x < r.x + r.width && p.y >= r.y && p.y < r.y + r.height;
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

/** A window this application owns, in global points. `NativeWindowInfo` satisfies it. */
export interface OwnedWindow {
  windowId: number;
  /** global points */
  bounds: Rect;
}

/**
 * The webview's native-occlusion rects TOGETHER WITH the window they were measured against.
 *
 * `nativeOcclusion.rects` are css px relative to ONE window's content box — the calibrated main
 * window's. Handing those numbers to a gate that only sees a global point would silently
 * reinterpret `{x:0,y:0,w:80,h:28}` as the top-left corner of the primary DISPLAY, refusing
 * arbitrary native points near the menu bar and admitting real traffic-light hits on a window
 * that does not sit at the origin. Carrying the anchor makes the space explicit and lets the
 * gate apply the rects only where they mean something.
 */
export interface GlobalOcclusion {
  /** The window the rects were measured against; its `windowId` is what anchors them. */
  geom: WindowGeom;
  /** css px, relative to THAT window's content box */
  rects: Rect[];
}

/**
 * The native sibling of `checkPoint`, for a point that is ALREADY in global points.
 *
 * `checkPoint` maps a css point through the calibrated main window and refuses anything outside
 * it. That is right for a webview target and wrong for a native one: an `NSSavePanel`, a sheet
 * that overhangs its parent, a menu or a popover legitimately extends beyond the main window's
 * bounds, and gating those on the main window makes them unreachable by construction. This gate
 * admits a point that falls inside ANY window this application owns, and still refuses one that
 * falls in another application's window.
 *
 * `allowedWindows` must be sourced from the app's OWN native windows — `NativeWindows.list(pid)`,
 * which asks CGWindowList by pid, so ownership is established by the OS rather than inferred.
 * Its front-to-back order is used here as ORDER, never as identity: the first window containing
 * the point is the one that would actually receive the click.
 *
 * Occlusion is applied only to the anchor window. A point that lands in some OTHER window of the
 * app is not tested against rects that were never measured there — see `GlobalOcclusion`. Pass
 * `null` when the target IS the native control (the AX layer exists to reach the title-bar
 * buttons, the menu bar and panel chrome; refusing them as "occluders" would defeat it).
 */
export function checkGlobalPoint(
  global: Pt,
  allowedWindows: OwnedWindow[],
  occlusion: GlobalOcclusion | null,
): Pt {
  if (!Number.isFinite(global.x) || !Number.isFinite(global.y)) {
    throw new AgentError("INVALID_TARGET", `global point (${global.x}, ${global.y}) is not finite`, {
      remediation: "The upstream measurement produced NaN or Infinity. Re-take it rather than posting input at it.",
      details: { global },
    });
  }
  if (allowedWindows.length === 0) {
    throw new AgentError("WINDOW_NOT_FOUND", "the application has no on-screen window to contain this point", {
      details: { global },
    });
  }
  const hit = allowedWindows.find((w) => pointInRect(global, w.bounds));
  if (!hit) {
    throw new AgentError(
      "POINT_OUTSIDE_WINDOW",
      `global(${global.x}, ${global.y}) is in no window of this application`,
      {
        remediation:
          "The point belongs to another application's window, or the window that held it closed. Re-read the app's windows and re-target; never post input at a point this gate refused.",
        details: {
          global,
          windows: allowedWindows.slice(0, 12).map((w) => ({ windowId: w.windowId, bounds: w.bounds })),
        },
      },
    );
  }
  // Only the anchor window's rects mean anything, and only for that window.
  if (occlusion && occlusion.rects.length > 0 && hit.windowId === occlusion.geom.windowId) {
    const css = globalToCss(global, occlusion.geom);
    if (pointInNativeOcclusion(css, occlusion.rects)) {
      throw new AgentError("ELEMENT_OCCLUDED", `global(${global.x}, ${global.y}) is under a native window control`, {
        remediation:
          "The macOS traffic lights sit above the webview. Target an element outside nativeOcclusion, or aim at the native control itself through the accessibility layer.",
        details: { reason: "native-titlebar-controls", global, css, windowId: hit.windowId },
      });
    }
  }
  return global;
}
