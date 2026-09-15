/**
 * macOS window enumeration and activation, through the same Swift helper as input.
 *
 * `NSRunningApplication.activate` is not reliable on its own when the calling process is a
 * faceless CLI, so a failed focus falls back to System Events via osascript and re-checks
 * with the helper (the helper's own `focus` verb is the only source of truth for frontmost).
 */
import type { Rect } from "../../geometry/types.ts";
import { log } from "../../log.ts";
import type { NativeWindowInfo, NativeWindows } from "../adapter.ts";
import { getHelperClient } from "./input.ts";

interface RawWindow {
  windowId: number;
  layer: number;
  bounds: { x: number; y: number; w: number; h: number };
  name?: string;
  onscreen: boolean;
}

export class MacWindows implements NativeWindows {
  private readonly client = getHelperClient();

  /**
   * Every window owned by `pid`, each with its CGWindowLevel, in CGWindowList order.
   *
   * That order is NOT identity: it is front-to-back at the moment of the call and changes when
   * a panel opens or the window is raised. Callers pick a window by area, name and layer.
   *
   * `all: true` swaps `kCGWindowListOptionOnScreenOnly` for `kCGWindowListOptionAll`, so windows
   * that are minimised or on another Space come back too and `onscreen` becomes a real reading
   * rather than a constant true. The window-identity table asks for that set, because a window
   * that is not on screen this instant is still the window a Tauri label names.
   */
  async list(pid: number, opts?: { all?: boolean }): Promise<NativeWindowInfo[]> {
    const r = await this.client.request("windows", { pid, all: opts?.all ?? false });
    const raw = Array.isArray(r.windows) ? (r.windows as RawWindow[]) : [];
    return raw.map((w) => ({
      windowId: w.windowId,
      layer: Number(w.layer ?? 0),
      bounds: toRect(w.bounds),
      ...(w.name === undefined ? {} : { name: w.name }),
      onscreen: Boolean(w.onscreen),
    }));
  }

  /** Read-only: no activation, so it is safe to call before every native input verb. */
  async isFrontmost(pid: number): Promise<boolean> {
    const r = await this.client.request("frontmost", { pid });
    return r.frontmost === true;
  }

  /** Activates, then reports what is ACTUALLY frontmost — never what was merely requested. */
  async focus(pid: number): Promise<boolean> {
    const first = await this.client.request("focus", { pid });
    if (first.frontmost === true) return true;

    log.debug("activate did not take; falling back to System Events", { pid });
    await systemEventsFocus(pid);
    return this.isFrontmost(pid);
  }
}

function toRect(b: { x: number; y: number; w: number; h: number }): Rect {
  return { x: b.x, y: b.y, width: b.w, height: b.h };
}

/** Best-effort; failures are logged and left for the helper's re-check to report. */
async function systemEventsFocus(pid: number): Promise<void> {
  const script = `tell application "System Events" to set frontmost of (first process whose unix id is ${pid}) to true`;
  try {
    const proc = Bun.spawn(["/usr/bin/osascript", "-e", script], {
      stdin: "ignore",
      stdout: "pipe",
      stderr: "pipe",
    });
    const [code, stderr] = await Promise.all([proc.exited, new Response(proc.stderr).text()]);
    if (code !== 0) log.warn("osascript focus fallback failed", { pid, code, stderr: stderr.trim().slice(0, 400) });
  } catch (e) {
    log.warn("osascript focus fallback could not run", { pid, error: String(e) });
  }
}
