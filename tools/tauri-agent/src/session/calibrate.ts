/**
 * Coordinate calibration: proves `global = innerPosition/scaleFactor + cssPoint` before
 * any input is posted.
 *
 * Four numeric checks catch a wrong scale factor or a stale window rect; the hover probe
 * is the only one that proves the whole chain end to end, because it physically moves the
 * real cursor and asks the page whether the intended element lit up. A failure here is
 * CALIBRATION_FAILED and no click is ever attempted.
 *
 * Calibration is OBSERVATION, so it must not change the app. The cursor moves (a move
 * changes no document state) but the wheel probe's notch is swallowed in the page before
 * the viewport's own handler can turn it into a camera op — otherwise session_start would
 * silently zoom the model, and a later "scroll two notches" would deliver more than two.
 * Every point a probe posts input at goes through `checkPoint`, the same gate the action
 * pipeline uses, so no probe can step the cursor into another application.
 */
import { checkPoint, rectCenter } from "../geometry/mapping.ts";
import type { Pt, Rect, WindowGeom } from "../geometry/types.ts";
import type { InputDevice, WheelSample } from "../geometry/wheelClass.ts";
import { classifyWheel } from "../geometry/wheelClass.ts";
import { AgentError } from "../errors.ts";
import { log } from "../log.ts";
import type { AxWindowInfo, AxWindowsResult, NativeInput, NativeWindowInfo } from "../platform/adapter.ts";
import type { WindowGeomReading } from "../semantic/webdriver.ts";
import type { SessionBridge, TauriWindowReading, WindowIdentity, WindowTable } from "./types.ts";

/** A reading and a native rect may disagree by this much and still be the same window. */
const TOL = 1;
const HOVER_SETTLE_MS = 120;
const HOVER_STEP_OFF_PT = 40;
/** Fallback step-off for a probe hemmed in on both sides; still enough motion for :hover. */
const HOVER_STEP_OFF_NEAR_PT = 8;
const WHEEL_ATTEMPTS = 4;
const WHEEL_READ_TRIES = 10;
const WHEEL_READ_INTERVAL_MS = 100;
/** A wheel this far (css px) from the probe point is the USER's, not ours: never swallow it. */
const WHEEL_MATCH_TOL_PX = 4;
/**
 * The in-page listener removes itself after this long no matter what. A thrown probe or a
 * dropped bridge must never leave the page eating the user's wheel; comfortably longer than
 * one attempt's read window (WHEEL_READ_TRIES * WHEEL_READ_INTERVAL_MS = 1 s) and re-armed
 * per attempt.
 */
const WHEEL_ARM_TIMEOUT_MS = 5_000;

export interface GeomChecks {
  innerSizeMatchesViewport: boolean;
  visualViewportScaleIsOne: boolean;
  devicePixelRatioMatchesScaleFactor: boolean;
  innerPositionMatchesNativeBounds: boolean;
}

/** The probe as it ran: a real cursor move onto a known element, and what `:hover` said. */
export interface HoverProbeRun {
  ok: boolean;
  preferred: string;
  usedFallback: boolean;
  description: string;
  hit: boolean;
  hover: boolean;
  css: Pt;
  global: Pt;
}

/**
 * The hover probe is the only thing that VERIFIES the CSS→global mapping, so a session that
 * skipped it has a mapping that is computed but unproven. That is sound for a background
 * session — nothing it can do posts a global point, because the webview lane dispatches at
 * `clientX/clientY` and accessibility actuation addresses an `AXUIElement` — but it must be
 * visible rather than absent, which is why the skip is a variant and not a missing field.
 */
export type HoverProbe = { skipped: string } | HoverProbeRun;

/** Narrows to the variant that actually moved the cursor. */
export function hoverRan(p: HoverProbe): p is HoverProbeRun {
  return !("skipped" in p);
}

export type WheelProbe =
  | {
      skipped: string;
      /**
       * True when a real notch was posted before the probe gave up. Two skip reasons imply
       * exactly that — a sample that never arrived means the swallowing listener did NOT match
       * the notch, so it reached whatever was under the cursor — and a caller that reports
       * "it posted no input" for those is lying about an unaccounted CGEvent.
       */
      posted: boolean;
    }
  | {
      /** false: the app never scored the notch as a mouse wheel, so zoom may read as pan. */
      ok: boolean;
      device: InputDevice | "unknown";
      linesPerNotch: number;
      attempts: number;
      sample: WheelSample;
    };

export interface CalibrationReport {
  ok: boolean;
  at: string;
  checks: GeomChecks;
  measurements: {
    scaleFactor: number;
    devicePixelRatio: number;
    visualViewportScale: number;
    innerSizeCss: { width: number; height: number };
    viewport: { width: number; height: number };
    contentOriginPt: Pt;
    nativeBoundsPt: Rect;
  };
  hoverProbe: HoverProbe;
  wheelProbe: WheelProbe;
}

export interface ProbeWindow extends Window {
  __tauriAgentProbe?: Element;
  __tauriAgentWheel?: WheelSample | null;
  __tauriAgentWheelLast?: number;
  /** Set while the wheel probe is armed; removes the capture listener and its guard timer. */
  __tauriAgentWheelDisarm?: (() => void) | undefined;
}

/** What `armWheelInPage` reports: where the viewport is, and whether it armed. */
export interface WheelArm {
  present: boolean;
  rect: Rect | null;
  armed?: boolean;
}

interface ProbePick {
  found: boolean;
  usedFallback: boolean;
  description: string;
  rect: Rect | null;
}

interface HoverReading {
  pageFocus?: boolean;
  visibility?: string;
  connected: boolean;
  hit: boolean;
  hover: boolean;
  rect: Rect | null;
}

/**
 * BOOTSTRAP ONLY — the largest layer-0 window of the app, on the assumption that its other
 * windows are tooltips and panels.
 *
 * This is a heuristic, not identity. It exists for the one moment at which nothing better is
 * available: the first window discovery of a session, before the bridge and accessibility have
 * produced a label ↔ window table. Once `WindowTable` exists, `selectTargetWindow` answers
 * "which window does this session mean" from that table and this function is not reached. Do
 * not reintroduce it as a fallback for an unresolved label: an unresolved label means the
 * harness cannot prove which window an event would reach, and the largest window is a guess
 * that looks exactly like an answer.
 */
export function pickLargestWindow(windows: NativeWindowInfo[], pid: number): NativeWindowInfo {
  const area = (w: NativeWindowInfo): number => w.bounds.width * w.bounds.height;
  // Layer 0 is the app's normal window level; panels and overlays live higher and may be
  // larger, so they only count when no layer-0 window exists.
  const normal = windows.filter((w) => w.layer === 0);
  const pool = normal.length > 0 ? normal : windows;
  const best = pool.reduce<NativeWindowInfo | undefined>(
    (acc, w) => (acc === undefined || area(w) > area(acc) ? w : acc),
    undefined,
  );
  if (best === undefined) {
    throw new AgentError("WINDOW_NOT_FOUND", `pid ${pid} has no on-screen window`, { details: { pid } });
  }
  return best;
}

// --- window identity ------------------------------------------------------

/** The `withGlobalTauri` surface the table reads. Present only in an agent/e2e build. */
interface TauriWebviewWindowsGlobal {
  __TAURI__: {
    webviewWindow: {
      getAllWebviewWindows(): Promise<
        Array<{
          label: string;
          title(): Promise<string>;
          outerPosition(): Promise<{ x: number; y: number }>;
          innerSize(): Promise<{ width: number; height: number }>;
          scaleFactor(): Promise<number>;
          setFocus(): Promise<void>;
        }>
      >;
    };
  };
}

/**
 * In-page half of the window table. Serialized with `Function.prototype.toString`, so it MUST
 * NOT close over anything in this module.
 *
 * `getAllWebviewWindows()` is asked rather than `getAllWindows()` because the set that matters
 * is the windows that actually host a webview — those are the ones a label can address and the
 * ones native input is posted into.
 */
export async function readTauriWindowsInPage(): Promise<TauriWindowReading[]> {
  const api = (window as unknown as TauriWebviewWindowsGlobal).__TAURI__.webviewWindow;
  const all = await api.getAllWebviewWindows();
  const rows: TauriWindowReading[] = [];
  for (const w of all) {
    const [title, pos, size, sf] = await Promise.all([
      w.title(),
      w.outerPosition(),
      w.innerSize(),
      w.scaleFactor(),
    ]);
    rows.push({
      label: w.label,
      // An empty title is reported as null: "this window has no title" must not match an AX
      // window that also has none, which would make two untitled windows look like one.
      title: typeof title === "string" && title.length > 0 ? title : null,
      outerPositionPx: { x: pos.x, y: pos.y },
      innerSizePx: { width: size.width, height: size.height },
      scaleFactor: sf,
    });
  }
  return rows;
}

export function readTauriWindows(bridge: SessionBridge): Promise<TauriWindowReading[]> {
  return bridge.execute<TauriWindowReading[]>("window.table", readTauriWindowsInPage as never, [], {
    readOnly: true,
  });
}

/**
 * In-page half of `window_focus`. Activating the APPLICATION raises whichever of its windows is
 * already key, which is the wrong one exactly when the caller asked for a different label — so
 * the named window is raised through Tauri, from the webview, by label.
 */
export async function focusTauriWindowInPage(label: string): Promise<boolean> {
  const api = (window as unknown as TauriWebviewWindowsGlobal).__TAURI__.webviewWindow;
  const all = await api.getAllWebviewWindows();
  const found = all.find((w) => w.label === label);
  if (found === undefined) return false;
  await found.setFocus();
  return true;
}

export function focusTauriWindow(bridge: SessionBridge, label: string): Promise<boolean> {
  return bridge.execute<boolean>("window.focusLabel", focusTauriWindowInPage as never, [label], {
    readOnly: false,
  });
}

/** AX reports a process's `AXWindows`, which is not only `AXWindow`s (Finder's desktop is not). */
function windowPool(ax: AxWindowsResult): AxWindowInfo[] {
  return ax.windows.filter((w) => w.role === null || w.role === "AXWindow");
}

function near(a: number, b: number): boolean {
  return Math.abs(a - b) <= TOL;
}

function describeAx(w: AxWindowInfo): string {
  const title = w.title === null ? "untitled" : JSON.stringify(w.title);
  const id = w.window.known ? String(w.window.id) : `unknown (${w.window.source})`;
  return `${title}, CGWindowID ${id}`;
}

/**
 * True when this AX window is the window the table entry names.
 *
 * The id is the answer whenever both sides have one. The bounds+title fallback is reached only
 * when the helper could not name THIS window's id — and it compares against the very evidence
 * that resolved the entry, so it cannot be looser than the correlation was. It is deliberately
 * strict rather than permissive: every caller treats "not the target" as a reason to refuse, so
 * a false negative costs a refusal and a false positive costs a click in the wrong window.
 */
export function sameWindow(w: AxWindowInfo, entry: WindowIdentity): boolean {
  if (entry.windowId === undefined) return false;
  if (w.window.known) return w.window.id === entry.windowId;
  if (entry.bounds === undefined || w.bounds === null) return false;
  return (
    near(w.bounds.x, entry.bounds.x) &&
    near(w.bounds.y, entry.bounds.y) &&
    near(w.bounds.width, entry.bounds.width) &&
    near(w.bounds.height, entry.bounds.height) &&
    w.title === entry.title
  );
}

function unresolved(r: TauriWindowReading, source: string, why: string): WindowIdentity {
  return { label: r.label, title: r.title, source, key: false, modal: false, unresolved: why };
}

/**
 * Correlates each Tauri label to one native window, or records why it could not be.
 *
 * Three sources of evidence, in order of strength:
 *
 *  - **sole** — one labelled window and one AX window of this process. Counting is proof: there
 *    is no other window for the label to be, whatever the bounds and title say.
 *  - **bounds** — exactly one AX window whose frame origin is the label's `outerPosition` in
 *    global points. Positions collide far less often than sizes do.
 *  - **bounds+title** (and then size) — the tie-breakers when several windows share an origin.
 *
 * The CGWindowID is then cross-checked against `natives`, which is every window of this process:
 * an id that names no window of this pid is refused rather than carried, because that id is what
 * `screencapture -l` and the occlusion gate would go on to trust.
 */
export function correlateWindows(
  readings: TauriWindowReading[],
  ax: AxWindowsResult,
  natives: NativeWindowInfo[],
): WindowIdentity[] {
  const pool = windowPool(ax);
  const nativeIds = new Set(natives.map((w) => w.windowId));
  const soleNative = natives.filter((w) => w.layer === 0);
  const entries = readings.map((r): WindowIdentity => {
    const sf = r.scaleFactor;
    if (!Number.isFinite(sf) || sf <= 0) {
      return unresolved(r, "none", `the window reported a scale factor of ${String(sf)}`);
    }
    const origin = { x: r.outerPositionPx.x / sf, y: r.outerPositionPx.y / sf };

    let source: string;
    let candidates: AxWindowInfo[];
    if (readings.length === 1 && pool.length === 1) {
      source = "sole";
      candidates = pool;
    } else {
      source = "bounds";
      candidates = pool.filter((w) => w.bounds !== null && near(w.bounds.x, origin.x) && near(w.bounds.y, origin.y));
      if (candidates.length > 1 && r.title !== null) {
        const byTitle = candidates.filter((w) => w.title === r.title);
        if (byTitle.length > 0) {
          candidates = byTitle;
          source = "bounds+title";
        }
      }
      if (candidates.length > 1) {
        const w = r.innerSizePx.width / sf;
        const h = r.innerSizePx.height / sf;
        const bySize = candidates.filter((c) => c.bounds !== null && near(c.bounds.width, w) && near(c.bounds.height, h));
        if (bySize.length > 0) {
          candidates = bySize;
          source = `${source}+size`;
        }
      }
    }

    if (candidates.length === 0) {
      return unresolved(r, source, `no accessibility window of this app is at (${origin.x}, ${origin.y})`);
    }
    if (candidates.length > 1) {
      return unresolved(
        r,
        source,
        `${candidates.length} accessibility windows of this app match this label (${candidates.map(describeAx).join("; ")})`,
      );
    }

    const match = candidates[0] as AxWindowInfo;
    // `{known:false}` carries no `id` member at all, so this is the only place an id can enter
    // the table — and it enters only when the helper actually named one, or when counting did.
    let id: number | undefined;
    if (match.window.known) id = match.window.id;
    else if (source === "sole" && soleNative.length === 1) id = (soleNative[0] as NativeWindowInfo).windowId;
    if (id === undefined) {
      return unresolved(
        r,
        source,
        `the helper could not name a CGWindowID for the matched window (${match.window.source})`,
      );
    }
    if (!nativeIds.has(id)) {
      return unresolved(r, source, `CGWindowID ${id} is not a window of pid ${ax.pid}`);
    }
    return {
      label: r.label,
      title: r.title,
      windowId: id,
      ...(match.bounds === null ? {} : { bounds: match.bounds }),
      source,
      key: match.focused,
      modal: match.modal,
    };
  });

  // Two labels that resolved to one window means the correlation was wrong for at least one of
  // them, and nothing here can say which. Both become unresolved: a refusal on both is a bug
  // report, while keeping one would be an even bet on whose input goes astray.
  const claimants = new Map<number, number>();
  for (const e of entries) {
    if (e.windowId !== undefined) claimants.set(e.windowId, (claimants.get(e.windowId) ?? 0) + 1);
  }
  return entries.map((e) => {
    if (e.windowId === undefined || (claimants.get(e.windowId) ?? 0) < 2) return e;
    return {
      label: e.label,
      title: e.title,
      source: e.source,
      key: e.key,
      modal: e.modal,
      unresolved: `CGWindowID ${e.windowId} was claimed by more than one Tauri label`,
    };
  });
}

export function buildWindowTable(
  pid: number,
  readings: TauriWindowReading[],
  ax: AxWindowsResult,
  natives: NativeWindowInfo[],
): WindowTable {
  return {
    at: new Date().toISOString(),
    pid,
    privateWindowIdApi: ax.privateWindowIdApi,
    entries: correlateWindows(readings, ax, natives),
  };
}

/** The table entry for `label`, or WINDOW_NOT_FOUND naming exactly what went wrong. */
export function requireIdentity(table: WindowTable, label: string): WindowIdentity & { windowId: number } {
  const entry = table.entries.find((e) => e.label === label);
  if (entry === undefined) {
    throw new AgentError("WINDOW_NOT_FOUND", `no Tauri window is labelled "${label}"`, {
      details: { label, known: table.entries.map((e) => e.label) },
    });
  }
  if (entry.windowId === undefined) {
    throw new AgentError(
      "WINDOW_NOT_FOUND",
      `the window labelled "${label}" could not be correlated to a native window: ${entry.unresolved ?? "unknown reason"}`,
      { details: { label, source: entry.source, reason: entry.unresolved } },
    );
  }
  return entry as WindowIdentity & { windowId: number };
}

/**
 * Which native window this session means — from the table when one exists, and from the
 * largest-window heuristic ONLY while it does not.
 *
 * The ordering is the point: `pickLargestWindow` is bootstrap, the table is identity, and an
 * unresolved label falls through to neither. It refuses.
 */
export function selectTargetWindow(
  windows: NativeWindowInfo[],
  table: WindowTable | undefined,
  label: string,
  pid: number,
): NativeWindowInfo {
  if (table === undefined) return pickLargestWindow(windows, pid);
  const entry = requireIdentity(table, label);
  const win = windows.find((w) => w.windowId === entry.windowId);
  if (win === undefined) {
    throw new AgentError(
      "WINDOW_NOT_FOUND",
      `the window labelled "${label}" (CGWindowID ${entry.windowId}) is not among the app's on-screen windows`,
      { details: { label, windowId: entry.windowId, onscreen: windows.map((w) => w.windowId) } },
    );
  }
  return win;
}

export function buildGeom(reading: WindowGeomReading, win: NativeWindowInfo): WindowGeom {
  return {
    innerPositionPx: reading.innerPositionPx,
    innerSizePx: reading.innerSizePx,
    scaleFactor: reading.scaleFactor,
    nativeBoundsPt: win.bounds,
    windowId: win.windowId,
  };
}

export function geomChecks(reading: WindowGeomReading, geom: WindowGeom): GeomChecks {
  const sf = reading.scaleFactor;
  return {
    innerSizeMatchesViewport:
      Math.abs(reading.innerSizePx.width / sf - reading.innerWidth) <= TOL &&
      Math.abs(reading.innerSizePx.height / sf - reading.innerHeight) <= TOL,
    visualViewportScaleIsOne: reading.vvScale === 1,
    devicePixelRatioMatchesScaleFactor: reading.dpr === sf,
    innerPositionMatchesNativeBounds:
      Math.abs(reading.innerPositionPx.x / sf - geom.nativeBoundsPt.x) <= TOL &&
      Math.abs(reading.innerPositionPx.y / sf - geom.nativeBoundsPt.y) <= TOL,
  };
}

/** True when the window moved, resized or changed display since `geom` was built. */
export function geomDiffers(reading: WindowGeomReading, win: NativeWindowInfo, geom: WindowGeom): boolean {
  return (
    Math.abs(reading.innerPositionPx.x - geom.innerPositionPx.x) > TOL ||
    Math.abs(reading.innerPositionPx.y - geom.innerPositionPx.y) > TOL ||
    Math.abs(reading.innerSizePx.width - geom.innerSizePx.width) > TOL ||
    Math.abs(reading.innerSizePx.height - geom.innerSizePx.height) > TOL ||
    reading.scaleFactor !== geom.scaleFactor ||
    win.windowId !== geom.windowId ||
    Math.abs(win.bounds.x - geom.nativeBoundsPt.x) > TOL ||
    Math.abs(win.bounds.y - geom.nativeBoundsPt.y) > TOL ||
    Math.abs(win.bounds.width - geom.nativeBoundsPt.width) > TOL ||
    Math.abs(win.bounds.height - geom.nativeBoundsPt.height) > TOL
  );
}

export function buildReport(
  reading: WindowGeomReading,
  win: NativeWindowInfo,
  checks: GeomChecks,
  hover: HoverProbe,
  wheel: WheelProbe,
): CalibrationReport {
  const sf = reading.scaleFactor;
  return {
    ok: true,
    at: new Date().toISOString(),
    checks,
    measurements: {
      scaleFactor: sf,
      devicePixelRatio: reading.dpr,
      visualViewportScale: reading.vvScale,
      innerSizeCss: { width: reading.innerSizePx.width / sf, height: reading.innerSizePx.height / sf },
      viewport: { width: reading.innerWidth, height: reading.innerHeight },
      contentOriginPt: { x: reading.innerPositionPx.x / sf, y: reading.innerPositionPx.y / sf },
      nativeBoundsPt: win.bounds,
    },
    hoverProbe: hover,
    wheelProbe: wheel,
  };
}

function failed(message: string, details: Record<string, unknown>): AgentError {
  return new AgentError("CALIBRATION_FAILED", message, { details });
}

/**
 * Picks the element the hover probe will aim at and parks it on `window.__tauriAgentProbe`.
 *
 * The configured probe is preferred, but it is screen-specific (`document-title` only
 * exists once a document is open), so any small, hit-testable, unoccluded element is an
 * equally valid instrument — the probe measures the mapping, not that element.
 */
const PICK_PROBE_TIMEOUT_MS = 10_000;
const PICK_PROBE_INTERVAL_MS = 250;

/**
 * The WebDriver endpoint answers before React has painted the start screen, so the first
 * pick can legitimately find nothing; keep looking for a bounded time before failing.
 */
async function pickProbeUntil(deps: ProbeDeps, preferred: string): Promise<ProbePick> {
  const attempts = Math.ceil(PICK_PROBE_TIMEOUT_MS / PICK_PROBE_INTERVAL_MS);
  let pick = await pickProbe(deps.bridge, preferred, deps.occlusion);
  for (let i = 1; i < attempts && (!pick.found || pick.rect === null); i += 1) {
    await deps.sleep(PICK_PROBE_INTERVAL_MS);
    pick = await pickProbe(deps.bridge, preferred, deps.occlusion);
  }
  return pick;
}

function pickProbe(bridge: SessionBridge, preferred: string, avoid: Rect[]): Promise<ProbePick> {
  return bridge.execute<ProbePick>(
    "calibrate.pickProbe",
    ((sel: string, avoidRects: Rect[]) => {
      const w = window as unknown as ProbeWindow;
      const pre = sel ? document.querySelector(sel) : null;
      const pool: Element[] = pre ? [pre] : [];
      const found = document.querySelectorAll('[data-testid], button, [role="button"], a[href], h1, h2');
      for (let i = 0; i < found.length; i += 1) pool.push(found[i] as Element);
      for (const el of pool) {
        const r = el.getBoundingClientRect();
        if (r.width < 24 || r.height < 16 || r.width > 400 || r.height > 200) continue;
        const cx = r.left + r.width / 2;
        const cy = r.top + r.height / 2;
        if (cx < 8 || cy < 8 || cx > innerWidth - 8 || cy > innerHeight - 8) continue;
        if (avoidRects.some((a) => cx >= a.x && cx < a.x + a.width && cy >= a.y && cy < a.y + a.height)) continue;
        const hit = document.elementFromPoint(cx, cy);
        if (!hit || !(hit === el || el.contains(hit))) continue;
        w.__tauriAgentProbe = el;
        const testId = el.getAttribute("data-testid");
        return {
          found: true,
          usedFallback: el !== pre,
          description: `${el.tagName.toLowerCase()}${testId ? `[data-testid="${testId}"]` : ""}`,
          rect: { x: r.left, y: r.top, width: r.width, height: r.height },
        };
      }
      return { found: false, usedFallback: pre === null, description: "", rect: null };
    }) as never,
    [preferred, avoid],
    { readOnly: false },
  );
}

function readHover(bridge: SessionBridge): Promise<HoverReading> {
  return bridge.execute<HoverReading>(
    "calibrate.hover",
    (() => {
      const el = (window as unknown as ProbeWindow).__tauriAgentProbe;
      if (!el || !el.isConnected) return { connected: false, hit: false, hover: false, rect: null, pageFocus: document.hasFocus(), visibility: document.visibilityState };
      const r = el.getBoundingClientRect();
      const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      return {
        connected: true,
        hit: hit !== null && (hit === el || el.contains(hit)),
        hover: el.matches(":hover"),
        rect: { x: r.left, y: r.top, width: r.width, height: r.height },
        // Diagnostics for a false `hover`: WebKit applies :hover only in the key window.
        pageFocus: document.hasFocus(),
        visibility: document.visibilityState,
      };
    }) as never,
    [],
    { readOnly: true },
  );
}

export interface ProbeDeps {
  bridge: SessionBridge;
  input: NativeInput;
  geom: WindowGeom;
  occlusion: Rect[];
  sleep: (ms: number) => Promise<void>;
  /** Brings the app to the front; WebKit only applies :hover while the app is the key window. */
  refocus?: () => Promise<void>;
}

export async function hoverProbe(deps: ProbeDeps, preferredTestId: string): Promise<HoverProbe> {
  const preferred = preferredTestId ? `[data-testid="${preferredTestId}"]` : "";
  const pick = await pickProbeUntil(deps, preferred);
  if (!pick.found || pick.rect === null) {
    throw failed("no element in the page is usable as a hover probe", { preferred, waitedMs: PICK_PROBE_TIMEOUT_MS });
  }
  const css = rectCenter(pick.rect);
  // The same gate the action pipeline uses: outside the window OR under a native control
  // (the traffic lights) both mean the cursor would land somewhere it must not.
  let global: Pt;
  try {
    global = checkPoint(css, deps.geom, deps.occlusion);
  } catch (e) {
    throw failed(`the probe element is not a legal point to post input at: ${reasonOf(e)}`, {
      css,
      geom: deps.geom,
      occlusion: deps.occlusion,
    });
  }
  // Retry the move+read: a background window reports `hit` (elementFromPoint) but no `:hover`,
  // because WebKit only tracks hover for the key window. Re-focus between attempts.
  let reading = await moveAndRead(deps, css, global);
  for (let attempt = 1; attempt < HOVER_ATTEMPTS && !(reading.hit && reading.hover); attempt += 1) {
    if (deps.refocus) await deps.refocus();
    await deps.sleep(HOVER_SETTLE_MS);
    reading = await moveAndRead(deps, css, global);
  }
  const probe: HoverProbe = {
    ok: reading.connected && reading.hit && reading.hover,
    preferred,
    usedFallback: pick.usedFallback,
    description: pick.description,
    hit: reading.hit,
    hover: reading.hover,
    css,
    global,
  };
  if (!probe.ok) {
    throw failed(
      reading.connected
        ? `the cursor was moved to global(${global.x}, ${global.y}) but the page does not report ${pick.description} as hovered (hit=${reading.hit}, pageFocus=${reading.pageFocus ?? "?"})`
        : "the probe element left the document during calibration",
      { probe, reading },
    );
  }
  return probe;
}

const HOVER_ATTEMPTS = 3;

/**
 * Step-off candidates in CSS space, tried in order. A fixed `+40,+40` walks the cursor OUT
 * of the window for any probe element near the right or bottom edge — into whatever
 * application owns those pixels. Searching means a probe anywhere in the content box has a
 * legal neighbour; the 8 px pair is the last resort for a probe hemmed in on both sides.
 */
const STEP_OFFSETS: readonly Pt[] = [
  { x: -HOVER_STEP_OFF_PT, y: 0 },
  { x: HOVER_STEP_OFF_PT, y: 0 },
  { x: 0, y: -HOVER_STEP_OFF_PT },
  { x: 0, y: HOVER_STEP_OFF_PT },
  { x: -HOVER_STEP_OFF_PT, y: -HOVER_STEP_OFF_PT },
  { x: HOVER_STEP_OFF_PT, y: HOVER_STEP_OFF_PT },
  { x: -HOVER_STEP_OFF_NEAR_PT, y: 0 },
  { x: HOVER_STEP_OFF_NEAR_PT, y: 0 },
];

function reasonOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** First candidate `checkPoint` accepts, as a global point. Never guesses. */
export function stepOffGlobal(deps: Pick<ProbeDeps, "geom" | "occlusion">, css: Pt): Pt {
  const tried: Pt[] = [];
  for (const d of STEP_OFFSETS) {
    const candidate = { x: css.x + d.x, y: css.y + d.y };
    tried.push(candidate);
    try {
      return checkPoint(candidate, deps.geom, deps.occlusion);
    } catch {
      // Outside the window, or under a native control: try the next side.
    }
  }
  throw failed("no legal step-off point exists next to the hover probe", {
    css,
    tried,
    geom: deps.geom,
    occlusion: deps.occlusion,
  });
}

/** One step-off-then-onto move (WebKit needs movement to update :hover) and a hover read. */
async function moveAndRead(deps: ProbeDeps, css: Pt, global: Pt): Promise<HoverReading> {
  await deps.input.move(stepOffGlobal(deps, css));
  await deps.sleep(HOVER_SETTLE_MS / 2);
  await deps.input.move(global);
  await deps.sleep(HOVER_SETTLE_MS);
  return readHover(deps.bridge);
}

/**
 * In-page half of the wheel probe. Serialized with `Function.prototype.toString`, so it
 * MUST NOT close over anything in this module: every helper lives inside its own body and
 * only erased types cross the boundary.
 *
 * Two calls per probe. `probe === null` is the LOCATE pass: report the viewport rect and
 * register nothing, so the caller can gate the point through `checkPoint` before anything
 * is posted. A non-null `probe` (the css point the notch will be posted at) ARMS.
 *
 * Arming is what makes calibration read-only. The listener is capture-phase on `window`,
 * which runs strictly before the app's own bubble-phase listener on the canvas element
 * (`src/viewport/engine/CadOrbitControls.ts`), and `passive:false` is what makes
 * `preventDefault` real; `stopImmediatePropagation` then means the notch never reaches the
 * camera at all. It cannot be `once`, because a wheel that is not ours must pass through
 * and leave the probe armed — so it disarms itself on the matching notch, and a guard timer
 * disarms it regardless if the probe never comes back.
 */
export function armWheelInPage(probe: Pt | null, tolPx: number, timeoutMs: number): WheelArm {
  const w = window as unknown as ProbeWindow;
  // A previous arm (a retried script, an abandoned probe) must not outlive this one.
  const prev = w.__tauriAgentWheelDisarm;
  if (prev) prev();
  const el = document.querySelector('[data-testid="viewport-canvas"]');
  if (!el) return { present: false, rect: null, armed: false };
  const r = el.getBoundingClientRect();
  const rect = { x: r.left, y: r.top, width: r.width, height: r.height };
  w.__tauriAgentWheel = null;
  if (probe === null) return { present: true, rect, armed: false };

  let timer = 0;
  const onWheel = (e: Event): void => {
    const ev = e as WheelEvent;
    // The user may spin their own wheel during the probe window. Only the notch posted at
    // the probe point is ours; anything else passes through untouched and leaves us armed.
    if (Math.abs(ev.clientX - probe.x) > tolPx || Math.abs(ev.clientY - probe.y) > tolPx) return;
    const now = performance.now();
    const last = w.__tauriAgentWheelLast;
    w.__tauriAgentWheelLast = now;
    w.__tauriAgentWheel = {
      deltaMode: ev.deltaMode,
      deltaX: ev.deltaX,
      deltaY: ev.deltaY,
      gapMs: last === undefined ? 1e6 : Math.min(1e6, now - last),
    };
    ev.preventDefault();
    ev.stopImmediatePropagation();
    disarm();
  };
  const disarm = (): void => {
    if (timer !== 0) window.clearTimeout(timer);
    timer = 0;
    window.removeEventListener("wheel", onWheel, true);
    w.__tauriAgentWheelDisarm = undefined;
  };
  w.__tauriAgentWheelDisarm = disarm;
  window.addEventListener("wheel", onWheel, { capture: true, passive: false });
  timer = window.setTimeout(disarm, timeoutMs);
  return { present: true, rect, armed: true };
}

/** In-page half of the disarm. Idempotent, and safe to call on a page that never armed. */
export function disarmWheelInPage(): { disarmed: boolean } {
  const w = window as unknown as ProbeWindow;
  const fn = w.__tauriAgentWheelDisarm;
  if (!fn) return { disarmed: false };
  fn();
  return { disarmed: true };
}

function armWheel(bridge: SessionBridge, probe: Pt | null): Promise<WheelArm> {
  return bridge.execute<WheelArm>(
    "calibrate.wheelArm",
    armWheelInPage as never,
    [probe, WHEEL_MATCH_TOL_PX, WHEEL_ARM_TIMEOUT_MS],
    { readOnly: false },
  );
}

/**
 * Marked read-only so the bridge retries it: this is the call that must land. The in-page
 * guard timer is the backstop for the case where it cannot.
 */
function disarmWheel(bridge: SessionBridge): Promise<{ disarmed: boolean }> {
  return bridge.execute<{ disarmed: boolean }>("calibrate.wheelDisarm", disarmWheelInPage as never, [], {
    readOnly: true,
  });
}

function readWheel(bridge: SessionBridge): Promise<WheelSample | null> {
  return bridge.execute<WheelSample | null>(
    "calibrate.wheelRead",
    (() => (window as unknown as ProbeWindow).__tauriAgentWheel ?? null) as never,
    [],
    { readOnly: true },
  );
}

/**
 * Sends real notches at the viewport until the app's own heuristic scores them `mouse`.
 *
 * A notch the app reads as a trackpad gesture PANS instead of zooming, so a later
 * "zoom the view" step would silently do the wrong thing. The working line count is
 * kept and reused by pointer_scroll.
 *
 * Every notch it posts is swallowed in the page before the viewport sees it, so the probe
 * measures the app's classification without moving the camera. The `finally` is load
 * bearing: success, skip, exhaustion and throw all leave the page disarmed.
 */
export async function wheelProbe(deps: ProbeDeps, startLines: number): Promise<WheelProbe> {
  // Locate first, arm second: the point has to clear `checkPoint` before anything is posted.
  const target = await armWheel(deps.bridge, null);
  if (!target.present || target.rect === null) return { skipped: "no viewport", posted: false };
  const css = rectCenter(target.rect);
  let global: Pt;
  try {
    global = checkPoint(css, deps.geom, deps.occlusion);
  } catch (e) {
    return { skipped: `the viewport centre is not a legal point to post input at: ${reasonOf(e)}`, posted: false };
  }

  let armedOnce = false;
  try {
    let lines = Math.max(1, Math.round(startLines));
    let last: { device: InputDevice | "unknown"; sample: WheelSample } | null = null;
    for (let attempt = 1; attempt <= WHEEL_ATTEMPTS; attempt += 1) {
      const armed = await armWheel(deps.bridge, css);
      if (armed.armed !== true) return { skipped: "the viewport left the page before the probe was armed", posted: armedOnce };
      armedOnce = true;
      await deps.input.scroll(global, { dy: lines });
      const sample = await pollWheel(deps, WHEEL_READ_TRIES);
      // The sample is written by the same listener that swallows the notch, so its absence means
      // the notch was NOT swallowed: it was posted and reached whatever was under the cursor.
      if (sample === null) return { skipped: "the viewport received no wheel event", posted: true };
      const device = classifyWheel(sample, "mouse");
      last = { device, sample };
      if (device === "mouse") return { ok: true, device, linesPerNotch: lines, attempts: attempt, sample };
      log.debug("wheel probe scored non-mouse; doubling", { lines, device, sample });
      lines *= 2;
      await deps.sleep(300);
    }
    // No line count reached "mouse". Reporting the last one tried would install a value that
    // is KNOWN to classify wrongly, so the configured default is kept and ok:false says why.
    return {
      ok: false,
      device: last?.device ?? "unknown",
      linesPerNotch: Math.max(1, Math.round(startLines)),
      attempts: WHEEL_ATTEMPTS,
      sample: last?.sample ?? { deltaMode: 0, deltaX: 0, deltaY: 0, gapMs: 0 },
    };
  } finally {
    if (armedOnce) {
      // A disarm that cannot be delivered must not mask the probe's own outcome; the
      // in-page guard timer removes the listener anyway.
      await disarmWheel(deps.bridge).catch((e: unknown) => {
        log.warn("wheel probe could not disarm the page listener", { reason: reasonOf(e) });
        return { disarmed: false };
      });
    }
  }
}

async function pollWheel(deps: ProbeDeps, tries: number): Promise<WheelSample | null> {
  for (let i = 0; i < tries; i += 1) {
    await deps.sleep(WHEEL_READ_INTERVAL_MS);
    const sample = await readWheel(deps.bridge);
    if (sample !== null) return sample;
  }
  return null;
}
