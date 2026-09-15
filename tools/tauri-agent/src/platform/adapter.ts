/**
 * Platform seam for everything the agent cannot do through the WebView bridge:
 * OS-level pointer/keyboard input, native window enumeration/focus, native screen capture.
 *
 * Every coordinate handed to an adapter is a **global display point** (top-left origin, y down).
 * Phase 1 ships the macOS adapter only; the other two exist so the seam is honest about it.
 */
import { AgentError } from "../errors.ts";
import type { Pt, Rect } from "../geometry/types.ts";
import { MacAccessibility } from "./macos/ax.ts";
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

/**
 * The active input source. The helper's key map is US/ANSI with no layout translation
 * (`main.swift` `Keys.map`), so `keyboard_press("z")` sends whatever physical key sits at the
 * ANSI Z position — `isAnsiUs: false` is the caller's only warning that a bare key name may
 * not produce the character it names.
 */
export interface KeyboardLayoutInfo {
  /** e.g. "com.apple.keylayout.US"; the raw TIS input source id. */
  inputSourceId: string;
  isAnsiUs: boolean;
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
  /**
   * The active keyboard input source. Optional: the Windows/Linux adapters never construct
   * (they throw UNSUPPORTED_PLATFORM_CAPABILITY at `createPlatformAdapter`), so nothing else
   * needs to implement it, but a fake `NativeInput` in a test may also omit it.
   */
  layout?(): Promise<KeyboardLayoutInfo>;
  dispose(): Promise<void>;
}

export interface NativeWindows {
  /**
   * On-screen windows of pid, in CGWindowList order. That order is not identity and changes
   * as windows are raised, so callers pick by area, name and layer — never by index.
   *
   * `all: true` widens the query to EVERY window of the process, on screen or not; each row's
   * `onscreen` then reports which it is. Identity work wants that superset — a window parked on
   * another Space is still the window a label names, and judging a correlated CGWindowID
   * "foreign to this process" because it is not on screen right now would be wrong.
   */
  list(pid: number, opts?: { all?: boolean }): Promise<NativeWindowInfo[]>;
  /** Read-only frontmost check: activates nothing, so it is safe before every input verb. */
  isFrontmost(pid: number): Promise<boolean>;
  /** Activates the app; true only if it is verified frontmost afterwards. */
  focus(pid: number): Promise<boolean>;
}

/**
 * How the helper correlated an AX window element to a CGWindowID. Today it emits
 * `"axPrivate"` (the private `_AXUIElementGetWindow`, the id `windows` and `screencapture -l`
 * already speak), `"boundsMatch"` (exactly one on-screen window of the pid had that rect, or
 * that rect and that title), `"ambiguous"` (several did — two windows of one app routinely
 * share a rect to the pixel) and `"none"` (nothing matched). Reported text, not a contract:
 * it is carried verbatim so a new value from the helper is never silently re-labelled.
 */
export type AxWindowIdSource = string;

/**
 * A CGWindowID the helper could correlate, or an explicit record that it could NOT.
 *
 * Deliberately a discriminated union rather than `number | null`: reading `.id` without first
 * narrowing on `known` does not compile, so "this element is in window 42" and "we could not
 * tell which window this element is in" can never be confused at a call site. A guess there
 * would send a click into the wrong window while reporting the right one.
 */
export type AxWindowId =
  | { known: true; id: number; source: AxWindowIdSource }
  | { known: false; source: AxWindowIdSource };

/** The correlated CGWindowID, or null — for logging and error details only, never for a gate. */
export function axWindowIdValue(w: AxWindowId): number | null {
  return w.known ? w.id : null;
}

export interface AxWindowInfo {
  role: string | null;
  subrole: string | null;
  title: string | null;
  /** global points; null when AX reported no position and size */
  bounds: Rect | null;
  /** AX did not expose the attribute is reported as false: absent means not main/modal. */
  main: boolean;
  modal: boolean;
  /** The application's own `AXFocusedWindow`, not the window's own `AXFocused` attribute. */
  focused: boolean;
  window: AxWindowId;
}

export interface AxWindowsResult {
  pid: number;
  /** false when `_AXUIElementGetWindow` could not be resolved: every id then comes from bounds+title. */
  privateWindowIdApi: boolean;
  windows: AxWindowInfo[];
}

export interface AxNode {
  /** `@a<generation>e<n>`. A ref from an older generation is refused, never aliased onto a new node. */
  ref: string;
  role: string | null;
  subrole: string | null;
  title: string | null;
  value: string | number | null;
  /** null means AX did not expose the attribute, which is not the same as false. */
  enabled: boolean | null;
  focused: boolean | null;
  /** global points */
  bounds: Rect | null;
  depth: number;
  /** Action names, so a caller can SEE that something is pressable. Pressing it is still a click. */
  actions: string[];
}

export interface AxWalkResult {
  pid: number;
  generation: number;
  nodes: AxNode[];
  /** Elements VISITED, not elements that exist; equal to the number found only when not truncated. */
  total: number;
  truncated: boolean;
  /** "complete" | "maxNodes" | "maxVisit" | "depth" | "deadline" */
  stopReason: string;
}

export interface AxSnapshotResult extends AxWalkResult {
  /** "root" | "requested" | "focused" | "main" | "only" — how the walked window was chosen. */
  windowSource: string;
  window: AxWindowInfo | null;
}

export interface AxPointInfo {
  ref: string;
  generation: number;
  pid: number;
  role: string | null;
  subrole: string | null;
  title: string | null;
  /** The application's own claim. Reported, never enforced — a mislabelled control is still clickable. */
  enabled: boolean | null;
  focused: boolean | null;
  /** global points, the second of two samples 50 ms apart */
  bounds: Rect;
  /** global points, already the space `click` consumes — no conversion */
  center: Pt;
  /** The display the centre falls on, global points. */
  display: Rect;
  frontmost: boolean;
  window: AxWindowId;
  windowTitle: string | null;
}

/** An AX window that is covering the application, with why it counts as one. */
export interface AxBlocker extends AxWindowInfo {
  /** "modal" | "sheet" | "dialog" */
  reason: string;
}

export interface AxModalResult {
  pid: number;
  modal: boolean;
  blockers: AxBlocker[];
}

export interface AxMenuKey {
  char: string | null;
  virtualKey: number | null;
  glyph: number | null;
  /** The raw Carbon mask, as AX reports it in `AXMenuItemCmdModifiers`. */
  modifiersRaw: number | null;
  mods: ModKey[];
}

export interface AxMenuItem {
  title: string | null;
  role: string | null;
  enabled: boolean | null;
  /** global points */
  bounds: Rect | null;
  depth: number;
  /** Titles of titled ancestors, menu bar first; untitled `AXMenu` wrappers contribute none. */
  path: string[];
  key: AxMenuKey | null;
}

export interface AxMenuResult {
  pid: number;
  hasMenuBar: boolean;
  items: AxMenuItem[];
  total: number;
  truncated: boolean;
  stopReason: string;
}

export interface AxSnapshotOpts {
  /** CGWindowID. Omit for the focused → main → sole ladder, which REFUSES rather than guessing. */
  window?: number;
  maxNodes?: number;
  /** Drill into a node of the CURRENT generation. Resolved before the generation is bumped. */
  root?: string;
}

export interface AxFindOpts {
  /** Exact match (AX roles are a fixed vocabulary). */
  role?: string;
  /** Case-insensitive substring. */
  title?: string;
  value?: string;
}

/**
 * Accessibility. Seven verbs READ; three ACT.
 *
 * **CGEvent is the only acceptance-grade actuator.** That is the architecture, and it is narrower
 * than the absolute this comment used to state ("nothing here performs an AXAction"). Accessibility
 * LOCATES what the WebView does not own — a native open/save panel, the app menu, a sheet, a
 * permission dialog, the title-bar buttons — and in a foreground session the native input verbs act
 * on it as a user would, which remains the strongest evidence available.
 *
 * `press`, `setValue` and `menuPress` ask the control to perform itself instead. Nothing moves, no
 * key is pressed, and the application need not be in front — which is why they are the only
 * actuation an `interaction:"background"` session has, and why every result they produce is
 * labelled `mode:"accessibility"` and never closes a real-user acceptance claim.
 *
 * Every coordinate is a global display point, top-left origin, y down: the same space
 * `NativeInput` consumes, measured rather than assumed (an `AXWindow`'s `AXPosition`/`AXSize`
 * read byte-identical to the same window's `kCGWindowBounds`). Never insert a conversion.
 */
export interface NativeAccessibility {
  windows(pid: number): Promise<AxWindowsResult>;
  /** Bumps the ref generation: every ref handed out before this call is stale afterwards. */
  snapshot(pid: number, opts?: AxSnapshotOpts): Promise<AxSnapshotResult>;
  /** A filtered walk, and a SNAPSHOT: it bumps the generation exactly as `snapshot` does. */
  find(pid: number, opts: AxFindOpts): Promise<AxWalkResult>;
  /**
   * Re-validates a ref and reports where the element is NOW. Refuses rather than guesses:
   * ELEMENT_STALE, ELEMENT_NOT_FOUND, ELEMENT_MOVING, POINT_OUTSIDE_WINDOW, INVALID_TARGET.
   */
  point(ref: string): Promise<AxPointInfo>;
  focusedWindow(pid: number): Promise<AxWindowInfo | null>;
  modal(pid: number): Promise<AxModalResult>;
  menu(pid: number): Promise<AxMenuResult>;

  // --- the three verbs that ACT ---------------------------------------------
  //
  // CGEvent remains the only acceptance-grade actuator. These ask the control to perform
  // itself: no cursor moves, no key is pressed, and the application need not be in front —
  // which is exactly why they are the only actuation an interaction:"background" session has,
  // and why every result they produce is labelled `mode:"accessibility"`.

  /**
   * Performs an action the element advertises (default `AXPress`). Refuses an unadvertised action,
   * a DISABLED control (which still advertises `AXPress` and still answers "success"), and window
   * chrome unless `acceptDisruption` is passed — pressing the full-screen or zoom button moves the
   * user's Space or resizes the window, which is not something a background session does silently.
   */
  press(ref: string, action?: string, acceptDisruption?: "yes"): Promise<AxPressResult>;
  /** Sets `AXValue`; refuses when the attribute is not settable, and reports the read-back. */
  setValue(ref: string, value: string): Promise<AxSetValueResult>;
  /** Presses a menu-bar item by its title path, e.g. ["File", "Save"]. */
  menuPress(pid: number, path: string[]): Promise<AxMenuPressResult>;
}

export interface AxPressResult {
  ref: string;
  action: string;
  role: string | null;
}

export interface AxSetValueResult {
  ref: string;
  role: string | null;
  requested: string;
  /** What the element reports AFTER the write; an app may normalise or reject what it stored. */
  value: string | null;
  /** False when the read-back differs from what was asked for. */
  matched: boolean;
}

export interface AxMenuPressResult {
  pid: number;
  path: string[];
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
  /**
   * `false` when the image EXISTS but is not trustworthy evidence of what was asked for —
   * absent means it is. A caller must never promote a non-authoritative image to evidence.
   */
  authoritative?: boolean;
  /** Machine-readable reason; set whenever `authoritative` is false. */
  reason?: string;
}

export interface PlatformAdapter {
  input: NativeInput;
  windows: NativeWindows;
  capture: NativeCapture;
  /**
   * Required, not optional: the Windows and Linux factories THROW rather than returning an
   * adapter, so a required member costs them nothing and a caller never has to ask whether
   * a live adapter happens to have accessibility.
   */
  ax: NativeAccessibility;
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
  return {
    input: new MacInput(),
    windows: new MacWindows(),
    capture: new MacCapture(),
    ax: new MacAccessibility(),
    name: "macos",
  };
}

export function unsupported(platform: string): AgentError {
  return new AgentError(
    "UNSUPPORTED_PLATFORM_CAPABILITY",
    `Native input, window and capture adapters are implemented for macOS only; this is ${platform}.`,
    { details: { platform } },
  );
}
