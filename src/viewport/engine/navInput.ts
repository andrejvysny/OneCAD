/*
 * navInput — pure input reducer for wheel + WebKit-gesture navigation.
 *
 * The browser gives no device identity: a MacBook trackpad's two-finger scroll
 * and a mouse wheel arrive as the same `wheel` event. Everything here exists to
 * recover enough intent from that one event stream to drive three gestures:
 *
 *   two-finger scroll          → pan
 *   shift + two-finger scroll  → orbit
 *   pinch                      → zoom
 *   mouse wheel                → zoom          (unchanged from before)
 *
 * This module is PURE — no DOM, no timers, no mutation of its inputs. Segment
 * boundaries are derived from event timestamps rather than a timer, which is
 * what makes momentum, modifier latching and pinch arbitration unit-testable.
 * `CadOrbitControls` is a thin adapter: DOM event → NavEvent → navReduce → NavOp.
 *
 * Auto-detection is BEST-EFFORT. The persisted user override is the only
 * deterministic answer, because the underlying signal genuinely is ambiguous.
 */

// ---- Public types --------------------------------------------------------

export type InputDevice = "mouse" | "trackpad";
export type DevicePref = "auto" | "mouse" | "trackpad";

/**
 * A camera operation, ready to apply.
 *
 * `pan`/`orbit` deltas are in MOUSE-DRAG-PIXEL EQUIVALENTS: the device gain has
 * already been applied, so the controller feeds them straight into its existing
 * `pan()` / `applyOrbit()`, whose `PAN_SENS` / `ORBIT_SPEED` then apply exactly
 * as they do for a real drag. That keeps one set of camera-feel constants
 * instead of two divergent ones.
 *
 * `zoom.factor` is an absolute distance multiplier (`distance *= factor`), so
 * `factor < 1` moves the camera closer.
 */
export type NavOp =
  | { kind: "pan"; dx: number; dy: number }
  | { kind: "orbit"; dx: number; dy: number }
  | { kind: "zoom"; factor: number; clientX: number; clientY: number };

export type NavEvent =
  | {
      type: "wheel";
      deltaX: number;
      deltaY: number;
      deltaMode: number;
      ctrlKey: boolean;
      shiftKey: boolean;
      clientX: number;
      clientY: number;
      t: number;
    }
  | { type: "gesturestart"; scale: number; clientX: number; clientY: number; t: number }
  | { type: "gesturechange"; scale: number; clientX: number; clientY: number; t: number }
  | { type: "gestureend"; t: number };

export interface NavContext {
  pref: DevicePref;
  /** Viewport height in CSS px — only used to expand DOM_DELTA_PAGE. */
  viewportH: number;
}

export interface NavResult {
  state: NavState;
  op: NavOp | null;
  /** Effective device used to route this event — respects `pref`. */
  device: InputDevice;
  /**
   * What the heuristic believes, IGNORING `pref`. This is what the "Auto"
   * label shows, so a user who has pinned an override can still see what
   * detection would have chosen.
   */
  detected: InputDevice;
}

// ---- Tunables ------------------------------------------------------------
//
// PLACEHOLDERS pending the Phase 0 probe on real hardware. Every value below is
// a reasoned starting point, not a measured one. See docs in the plan.

/** No wheel event for this long ⇒ the next one starts a fresh gesture. */
export const SEGMENT_IDLE_MS = 120;
/** DOM_DELTA_LINE → px. Roughly one text line. */
export const LINE_HEIGHT_PX = 16;
/** Ignore ctrl-wheel for this long after `gestureend` (WebKit tail). */
export const POST_GESTURE_SUPPRESS_MS = 200;

/** Mouse-wheel dolly: factor = exp(deltaY * ZOOM_SENS). */
export const ZOOM_SENS = 0.0015;
/** Ctrl-wheel pinch encoding (Chromium/Firefox/WebView2). Stronger per unit
 *  than a wheel tick, because pinch deltas are small. */
export const PINCH_SENS = 0.01;

/** Dimensionless: wheel CSS px → equivalent mouse-drag px. */
export const TRACKPAD_PAN_GAIN = 1;
/** Trackpad orbit is twitchier than a drag; damp it. */
export const TRACKPAD_ORBIT_GAIN = 0.6;

/** Per-event zoom clamp, mirroring the controller's own guard. */
const MIN_FACTOR = 0.2;
const MAX_FACTOR = 5;

/**
 * macOS folds shift+wheel onto the horizontal axis. For a MOUSE that is handled
 * where it matters (see `opFor`) by taking whichever axis carries the delta.
 *
 * For a TRACKPAD it is not that simple and remains UNVERIFIED: a two-finger
 * scroll carries genuine 2D motion, so if the platform also folds it under
 * shift, orbit loses its yaw axis. Un-swapping unconditionally would be worse —
 * it would turn a real horizontal orbit into a pitch. Flip this only if the
 * Phase 0 probe shows a trackpad's shift+scroll really does collapse to one axis.
 */
export const SHIFT_AXIS_SWAP = false;

// Classification thresholds.
const BIG_TICK_PX = 100; // a conventional wheel notch, premultiplied
const SMALL_DELTA_PX = 40;
const FAST_GAP_MS = 50;
const SLOW_GAP_MS = 100;
/** Score margin required to flip the carried prior. */
const PRIOR_FLIP_MARGIN = 2;

// ---- State ---------------------------------------------------------------

type PinchSource = "none" | "gesture" | "wheel";

interface Segment {
  device: InputDevice;
  /** Definitive evidence seen ⇒ `device` is no longer revisable this segment. */
  decided: boolean;
  /** shiftKey as it was at segment start — NOT re-read per event. */
  shiftLatched: boolean;
  lastT: number;
  trackpadScore: number;
  mouseScore: number;
}

export interface NavState {
  /** Carried between segments; seeds an ambiguous segment. */
  readonly prior: InputDevice;
  readonly segment: Segment | null;
  readonly pinchSource: PinchSource;
  /** Cumulative scale of the in-flight WebKit gesture, or 0 when idle. */
  readonly gestureScale: number;
  readonly gestureEndedAt: number;
}

/**
 * Initial state. Prior is `mouse` so an existing mouse user's very first scroll
 * still zooms — the pre-existing behaviour never regresses on event one.
 */
export function navInit(): NavState {
  return {
    prior: "mouse",
    segment: null,
    pinchSource: "none",
    gestureScale: 0,
    gestureEndedAt: -Infinity,
  };
}

// ---- Pure helpers (exported for direct testing) --------------------------

/** deltaMode → CSS pixels, with the shift axis-swap undone if it applies. */
export function normalizeWheel(
  ev: { deltaX: number; deltaY: number; deltaMode: number; shiftKey: boolean },
  viewportH: number,
): { dx: number; dy: number } {
  const unit =
    ev.deltaMode === 1 ? LINE_HEIGHT_PX : ev.deltaMode === 2 ? Math.max(viewportH, 1) : 1;
  let dx = ev.deltaX * unit;
  let dy = ev.deltaY * unit;
  // The platform folded a vertical scroll onto the horizontal axis; put it back
  // so shift+scroll still carries the user's real 2D finger motion.
  if (SHIFT_AXIS_SWAP && ev.shiftKey && dy === 0 && dx !== 0) {
    const t = dx;
    dx = dy;
    dy = t;
  }
  return { dx, dy };
}

export function clampFactor(f: number): number {
  if (!Number.isFinite(f) || f <= 0) return 1;
  return Math.max(MIN_FACTOR, Math.min(MAX_FACTOR, f));
}

/**
 * Definitive device evidence — the only kind allowed to override a decision
 * mid-gesture.
 *
 * A large integer ctrl-wheel delta is deliberately NOT trackpad evidence: that
 * is a mouse user holding Ctrl to zoom, and misreading it as a trackpad would
 * make their next plain scroll pan.
 */
export function definitiveDevice(ev: {
  deltaMode: number;
  ctrlKey: boolean;
  deltaY: number;
}): InputDevice | null {
  if (ev.deltaMode !== 0) return "mouse";
  if (ev.ctrlKey && Math.abs(ev.deltaY) < SMALL_DELTA_PX) return "trackpad";
  return null;
}

/** Non-definitive evidence, as a (trackpad, mouse) score pair. */
export function weighEvidence(
  n: { dx: number; dy: number },
  gapMs: number,
  shiftKey: boolean,
): { trackpad: number; mouse: number } {
  let trackpad = 0;
  let mouse = 0;
  // A wheel has one axis, so a horizontal component normally means two fingers.
  // NOT under shift: macOS folds a mouse wheel onto deltaX there, and scoring
  // that as trackpad would flip a mouse user into panning on every shift+wheel.
  if (n.dx !== 0 && !shiftKey) trackpad += 2;
  // Wheel notches are whole numbers; trackpad pixels are continuous.
  if (!Number.isInteger(n.dy) || !Number.isInteger(n.dx)) trackpad += 2;
  // Compare against the dominant axis so a folded shift+wheel is still measured.
  const mag = Math.max(Math.abs(n.dy), Math.abs(n.dx));
  if (mag >= BIG_TICK_PX && Number.isInteger(n.dy) && Number.isInteger(n.dx) && gapMs > SLOW_GAP_MS) {
    mouse += 2;
  }
  if (mag < SMALL_DELTA_PX && gapMs < FAST_GAP_MS) trackpad += 1;
  return { trackpad, mouse };
}

function resolve(pref: DevicePref, detected: InputDevice): InputDevice {
  return pref === "auto" ? detected : pref;
}

// ---- Reducer -------------------------------------------------------------

/**
 * The single entry point. Same (state, event, ctx) always yields the same
 * result — no clocks, no randomness, no DOM reads.
 */
export function navReduce(state: NavState, ev: NavEvent, ctx: NavContext): NavResult {
  switch (ev.type) {
    case "gesturestart":
      return reduceGestureStart(state, ev, ctx);
    case "gesturechange":
      return reduceGestureChange(state, ev, ctx);
    case "gestureend":
      return reduceGestureEnd(state, ev, ctx);
    case "wheel":
      return reduceWheel(state, ev, ctx);
  }
}

function reduceGestureStart(
  state: NavState,
  ev: Extract<NavEvent, { type: "gesturestart" }>,
  ctx: NavContext,
): NavResult {
  // First source seen owns pinch for the rest of the session. A gesture stream
  // is strictly better than the ctrl-wheel encoding, and claiming it here means
  // a stray ctrl-wheel can never double-apply.
  const next: NavState = {
    ...state,
    pinchSource: state.pinchSource === "none" ? "gesture" : state.pinchSource,
    // A start without a matching end (or a duplicate start) resets cleanly
    // rather than leaking the previous gesture's scale.
    gestureScale: ev.scale > 0 ? ev.scale : 1,
    prior: "trackpad",
  };
  return { state: next, op: null, device: resolve(ctx.pref, "trackpad"), detected: "trackpad" };
}

function reduceGestureChange(
  state: NavState,
  ev: Extract<NavEvent, { type: "gesturechange" }>,
  ctx: NavContext,
): NavResult {
  const device = resolve(ctx.pref, "trackpad");
  if (state.pinchSource !== "gesture" || state.gestureScale <= 0 || ev.scale <= 0) {
    return { state, op: null, device, detected: "trackpad" };
  }
  // WebKit `scale` is CUMULATIVE from gesture start, not per-event. Fingers
  // apart ⇒ scale grows ⇒ factor < 1 ⇒ camera moves closer.
  const factor = clampFactor(state.gestureScale / ev.scale);
  return {
    state: { ...state, gestureScale: ev.scale },
    op: { kind: "zoom", factor, clientX: ev.clientX, clientY: ev.clientY },
    device,
    detected: "trackpad",
  };
}

function reduceGestureEnd(
  state: NavState,
  ev: Extract<NavEvent, { type: "gestureend" }>,
  ctx: NavContext,
): NavResult {
  const device = resolve(ctx.pref, state.prior);
  // WebKit is known to fire `gestureend` twice; a second one must be inert.
  if (state.gestureScale === 0) return { state, op: null, device, detected: state.prior };
  return {
    state: { ...state, gestureScale: 0, gestureEndedAt: ev.t },
    op: null,
    device,
    detected: state.prior,
  };
}

function reduceWheel(
  state: NavState,
  ev: Extract<NavEvent, { type: "wheel" }>,
  ctx: NavContext,
): NavResult {
  const n = normalizeWheel(ev, ctx.viewportH);

  // --- pinch encoded as ctrl-wheel -----------------------------------------
  if (ev.ctrlKey) {
    const owned = state.pinchSource === "gesture";
    const inTail = ev.t - state.gestureEndedAt < POST_GESTURE_SUPPRESS_MS;
    const definitive = definitiveDevice(ev);
    // Even when the zoom is dropped, the device evidence is still worth keeping.
    const prior = definitive ?? state.prior;
    if (owned || inTail) {
      return {
        state: { ...state, prior },
        op: null,
        device: resolve(ctx.pref, prior),
        detected: prior,
      };
    }
    const factor = clampFactor(Math.exp(n.dy * PINCH_SENS));
    return {
      state: {
        ...state,
        prior,
        pinchSource: state.pinchSource === "none" ? "wheel" : state.pinchSource,
        // A pinch is not a pan/orbit gesture; end any segment it interrupts.
        segment: null,
      },
      op: { kind: "zoom", factor, clientX: ev.clientX, clientY: ev.clientY },
      device: resolve(ctx.pref, prior),
      detected: prior,
    };
  }

  // --- pan / orbit / dolly --------------------------------------------------
  const prev = state.segment;
  const gap = prev ? ev.t - prev.lastT : Infinity;
  const definitive = definitiveDevice(ev);

  let seg: Segment;
  let prior = state.prior;

  if (!prev || gap > SEGMENT_IDLE_MS) {
    // A finished segment folds its accumulated evidence into the prior, so the
    // NEXT gesture starts better-informed without ever flipping mid-gesture.
    if (prev && !prev.decided) prior = settlePrior(prior, prev);
    else if (prev) prior = prev.device;

    // The first event of a segment may act on non-definitive evidence — that is
    // what lets a diagonal or fractional first flick pan instead of zooming.
    const first = weighEvidence(n, Infinity, ev.shiftKey);
    const opening: InputDevice =
      definitive ??
      (first.trackpad > first.mouse ? "trackpad" : first.mouse > first.trackpad ? "mouse" : prior);

    seg = {
      device: opening,
      decided: definitive !== null,
      shiftLatched: ev.shiftKey,
      lastT: ev.t,
      trackpadScore: first.trackpad,
      mouseScore: first.mouse,
    };
  } else {
    const w = weighEvidence(n, gap, ev.shiftKey);
    seg = {
      ...prev,
      // Only definitive evidence may change the device mid-gesture; anything
      // weaker would let a momentum tail flip pan into zoom halfway through.
      device: definitive ?? prev.device,
      decided: prev.decided || definitive !== null,
      lastT: ev.t,
      trackpadScore: prev.trackpadScore + w.trackpad,
      mouseScore: prev.mouseScore + w.mouse,
    };
  }
  if (definitive) prior = definitive;

  const device = resolve(ctx.pref, seg.device);
  const op = opFor(device, seg.shiftLatched, n, ev.clientX, ev.clientY);
  // `detected` reports the live segment verdict, not the between-segment prior,
  // so a continuous scroll surfaces detection to the UI immediately.
  return { state: { ...state, prior, segment: seg }, op, device, detected: seg.device };
}

/** Fold a finished segment's evidence into the prior for the next one. */
function settlePrior(prior: InputDevice, seg: Segment): InputDevice {
  if (seg.trackpadScore >= seg.mouseScore + PRIOR_FLIP_MARGIN) return "trackpad";
  if (seg.mouseScore >= seg.trackpadScore + PRIOR_FLIP_MARGIN) return "mouse";
  return prior;
}

function opFor(
  device: InputDevice,
  shiftLatched: boolean,
  n: { dx: number; dy: number },
  clientX: number,
  clientY: number,
): NavOp | null {
  if (device === "mouse") {
    // macOS folds a shift+wheel onto the HORIZONTAL axis (verified in Chromium:
    // shift + wheel arrives as deltaX=N, deltaY=0). A wheel has only one real
    // axis, so whichever one carries the delta is the scroll — take that and
    // shift+wheel keeps zooming instead of doing nothing.
    const d = n.dy !== 0 ? n.dy : n.dx;
    if (d === 0) return null;
    return {
      kind: "zoom",
      factor: clampFactor(Math.exp(d * ZOOM_SENS)),
      clientX,
      clientY,
    };
  }
  if (n.dx === 0 && n.dy === 0) return null;
  if (shiftLatched) {
    return {
      kind: "orbit",
      dx: n.dx * TRACKPAD_ORBIT_GAIN,
      dy: n.dy * TRACKPAD_ORBIT_GAIN,
    };
  }
  return { kind: "pan", dx: n.dx * TRACKPAD_PAN_GAIN, dy: n.dy * TRACKPAD_PAN_GAIN };
}
