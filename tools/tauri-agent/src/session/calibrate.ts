/**
 * Coordinate calibration: proves `global = innerPosition/scaleFactor + cssPoint` before
 * any input is posted.
 *
 * Four numeric checks catch a wrong scale factor or a stale window rect; the hover probe
 * is the only one that proves the whole chain end to end, because it physically moves the
 * real cursor and asks the page whether the intended element lit up. A failure here is
 * CALIBRATION_FAILED and no click is ever attempted.
 */
import { cssToGlobal, pointInWindow, rectCenter } from "../geometry/mapping.ts";
import type { Pt, Rect, WindowGeom } from "../geometry/types.ts";
import type { InputDevice, WheelSample } from "../geometry/wheelClass.ts";
import { classifyWheel } from "../geometry/wheelClass.ts";
import { AgentError } from "../errors.ts";
import { log } from "../log.ts";
import type { NativeInput, NativeWindowInfo } from "../platform/adapter.ts";
import type { WindowGeomReading } from "../semantic/webdriver.ts";
import type { SessionBridge } from "./types.ts";

/** A reading and a native rect may disagree by this much and still be the same window. */
const TOL = 1;
const HOVER_SETTLE_MS = 120;
const HOVER_STEP_OFF_PT = 40;
const WHEEL_ATTEMPTS = 4;
const WHEEL_READ_TRIES = 10;
const WHEEL_READ_INTERVAL_MS = 100;

export interface GeomChecks {
  innerSizeMatchesViewport: boolean;
  visualViewportScaleIsOne: boolean;
  devicePixelRatioMatchesScaleFactor: boolean;
  innerPositionMatchesNativeBounds: boolean;
}

export interface HoverProbe {
  ok: boolean;
  preferred: string;
  usedFallback: boolean;
  description: string;
  hit: boolean;
  hover: boolean;
  css: Pt;
  global: Pt;
}

export type WheelProbe =
  | { skipped: string }
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

interface ProbeWindow extends Window {
  __tauriAgentProbe?: Element;
  __tauriAgentWheel?: WheelSample | null;
  __tauriAgentWheelLast?: number;
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

/** Largest layer-0 window of the app: its other windows are tooltips and panels. */
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
  const global = cssToGlobal(css, deps.geom);
  if (!pointInWindow(global, deps.geom)) {
    throw failed("the probe element maps outside the window content box", { css, global, geom: deps.geom });
  }
  // Retry the move+read: a background window reports `hit` (elementFromPoint) but no `:hover`,
  // because WebKit only tracks hover for the key window. Re-focus between attempts.
  let reading = await moveAndRead(deps, global);
  for (let attempt = 1; attempt < HOVER_ATTEMPTS && !(reading.hit && reading.hover); attempt += 1) {
    if (deps.refocus) await deps.refocus();
    await deps.sleep(HOVER_SETTLE_MS);
    reading = await moveAndRead(deps, global);
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

/** One step-off-then-onto move (WebKit needs movement to update :hover) and a hover read. */
async function moveAndRead(deps: ProbeDeps, global: { x: number; y: number }): Promise<HoverReading> {
  await deps.input.move({ x: global.x + HOVER_STEP_OFF_PT, y: global.y + HOVER_STEP_OFF_PT });
  await deps.sleep(HOVER_SETTLE_MS / 2);
  await deps.input.move(global);
  await deps.sleep(HOVER_SETTLE_MS);
  return readHover(deps.bridge);
}

function armWheel(bridge: SessionBridge): Promise<{ present: boolean; rect: Rect | null }> {
  return bridge.execute<{ present: boolean; rect: Rect | null }>(
    "calibrate.wheelArm",
    (() => {
      const w = window as unknown as ProbeWindow;
      const el = document.querySelector('[data-testid="viewport-canvas"]');
      if (!el) return { present: false, rect: null };
      w.__tauriAgentWheel = null;
      el.addEventListener(
        "wheel",
        (e) => {
          const ev = e as WheelEvent;
          const now = performance.now();
          const last = w.__tauriAgentWheelLast;
          w.__tauriAgentWheelLast = now;
          w.__tauriAgentWheel = {
            deltaMode: ev.deltaMode,
            deltaX: ev.deltaX,
            deltaY: ev.deltaY,
            gapMs: last === undefined ? 1e6 : Math.min(1e6, now - last),
          };
        },
        { once: true, passive: true, capture: true },
      );
      const r = el.getBoundingClientRect();
      return { present: true, rect: { x: r.left, y: r.top, width: r.width, height: r.height } };
    }) as never,
    [],
    { readOnly: false },
  );
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
 */
export async function wheelProbe(deps: ProbeDeps, startLines: number): Promise<WheelProbe> {
  const armed = await armWheel(deps.bridge);
  if (!armed.present || armed.rect === null) return { skipped: "no viewport" };
  const css = rectCenter(armed.rect);
  const global = cssToGlobal(css, deps.geom);
  if (!pointInWindow(global, deps.geom)) return { skipped: "viewport centre is outside the window" };

  let lines = Math.max(1, Math.round(startLines));
  let last: { device: InputDevice | "unknown"; sample: WheelSample } | null = null;
  for (let attempt = 1; attempt <= WHEEL_ATTEMPTS; attempt += 1) {
    if (attempt > 1) await armWheel(deps.bridge);
    await deps.input.scroll(global, { dy: lines });
    const sample = await pollWheel(deps, WHEEL_READ_TRIES);
    if (sample === null) return { skipped: "the viewport received no wheel event" };
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
}

async function pollWheel(deps: ProbeDeps, tries: number): Promise<WheelSample | null> {
  for (let i = 0; i < tries; i += 1) {
    await deps.sleep(WHEEL_READ_INTERVAL_MS);
    const sample = await readWheel(deps.bridge);
    if (sample !== null) return sample;
  }
  return null;
}
