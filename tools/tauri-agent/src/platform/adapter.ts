/**
 * Platform seam for everything the agent cannot do through the WebView bridge:
 * OS-level pointer/keyboard input, native window enumeration/focus, native screen capture.
 *
 * Every coordinate handed to an adapter is a **global display point** (top-left origin, y down).
 * Phase 1 ships the macOS adapter only; the other two exist so the seam is honest about it.
 */
import { AgentError } from "../errors.ts";
import type { Pt, Rect } from "../geometry/types.ts";
import { MacCapture } from "./macos/capture.ts";
import { MacInput } from "./macos/input.ts";
import { MacWindows } from "./macos/windows.ts";
import { createLinuxAdapter } from "./linux/index.ts";
import { createWindowsAdapter } from "./windows/index.ts";

export type MouseButton = "left" | "right" | "middle";

/**
 * Physical modifier keys. The schema-level `Primary` alias is NOT accepted here — callers
 * translate `Primary` to `Command` on macOS before reaching the adapter.
 */
export type ModKey = "Command" | "Control" | "Option" | "Shift" | "Fn";

export interface NativeWindowInfo {
  windowId: number;
  /** CGWindowLevel: 0 is a normal window, higher levels are panels, menus and tooltips */
  layer: number;
  /** global points */
  bounds: Rect;
  /** absent when Screen Recording is not granted */
  name?: string;
  onscreen: boolean;
}

export interface Permissions {
  accessibility: boolean;
  screenRecording: boolean;
}

export interface NativeInput {
  move(p: Pt, opts?: { durationMs?: number; steps?: number; mods?: ModKey[] }): Promise<void>;
  down(button: MouseButton, p: Pt, opts?: { clickState?: 1 | 2 | 3; mods?: ModKey[] }): Promise<void>;
  up(button: MouseButton, p: Pt, opts?: { clickState?: 1 | 2 | 3; mods?: ModKey[] }): Promise<void>;
  click(
    button: MouseButton,
    p: Pt,
    opts?: { count?: 1 | 2 | 3; intervalMs?: number; mods?: ModKey[] },
  ): Promise<void>;
  path(
    button: MouseButton,
    points: Pt[],
    opts?: { durationMs?: number; holdMs?: number; dwellMs?: number; mods?: ModKey[] },
  ): Promise<void>;
  /** line units; positive dy = wheel up */
  scroll(p: Pt, delta: { dy: number; dx?: number }, opts?: { mods?: ModKey[] }): Promise<void>;
  /** key: "a".."z", "0".."9", "Enter", "Escape", "Tab", "Space", "Backspace", "Delete", "ArrowUp"..., "F1".., or a ModKey */
  keyDown(key: string, opts?: { mods?: ModKey[] }): Promise<void>;
  keyUp(key: string, opts?: { mods?: ModKey[] }): Promise<void>;
  press(key: string, mods?: ModKey[]): Promise<void>;
  type(text: string, opts?: { perCharMs?: number }): Promise<void>;
  /** never throws */
  releaseAll(): Promise<void>;
  cursor(): Promise<Pt>;
  permissions(opts?: { prompt?: boolean }): Promise<Permissions>;
  dispose(): Promise<void>;
}

export interface NativeWindows {
  /**
   * On-screen windows of pid, in CGWindowList order. That order is not identity and changes
   * as windows are raised, so callers pick by area, name and layer — never by index.
   */
  list(pid: number): Promise<NativeWindowInfo[]>;
  /** Read-only frontmost check: activates nothing, so it is safe before every input verb. */
  isFrontmost(pid: number): Promise<boolean>;
  /** Activates the app; true only if it is verified frontmost afterwards. */
  focus(pid: number): Promise<boolean>;
}

export interface NativeCapture {
  window(windowId: number, outPath: string, boundsPt?: Rect): Promise<CaptureResult>;
  /** rect in global points */
  region(rect: Rect, outPath: string): Promise<CaptureResult>;
  screen(outPath: string, boundsPt?: Rect): Promise<CaptureResult>;
  /** sips -Z */
  preview(srcPath: string, outPath: string, maxPx: number): Promise<void>;
}

export interface CaptureResult {
  width: number;
  height: number;
  pixelScale: number;
  /** set when the image does not match the bounds that were asked for */
  warning?: string;
}

export interface PlatformAdapter {
  input: NativeInput;
  windows: NativeWindows;
  capture: NativeCapture;
  name: "macos" | "windows" | "linux";
}

export function createPlatformAdapter(): PlatformAdapter {
  switch (process.platform) {
    case "darwin":
      return createMacAdapter();
    case "win32":
      return createWindowsAdapter();
    default:
      return createLinuxAdapter();
  }
}

function createMacAdapter(): PlatformAdapter {
  return { input: new MacInput(), windows: new MacWindows(), capture: new MacCapture(), name: "macos" };
}

export function unsupported(platform: string): AgentError {
  return new AgentError(
    "UNSUPPORTED_PLATFORM_CAPABILITY",
    `Native input, window and capture adapters are implemented for macOS only; this is ${platform}.`,
    { details: { platform } },
  );
}
