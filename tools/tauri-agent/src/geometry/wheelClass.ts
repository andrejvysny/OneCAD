/**
 * Mirror of the app's wheel device heuristic (`src/viewport/engine/navInput.ts`).
 *
 * The calibration probe sends ONE synthetic notch and needs to know what the app
 * would conclude from it — if the app scores it `trackpad`, the same notch pans
 * instead of zooming and every later "zoom the view" step is a lie. The app code
 * is deliberately NOT imported (the agent is a standalone bun package outside the
 * frontend's module graph); `tests/wheelClass.test.ts` runs both against the same
 * grid so a drift in either copy fails the gate.
 */

export type InputDevice = "mouse" | "trackpad";

/** navInput.ts: a conventional wheel notch, premultiplied. */
export const BIG_TICK_PX = 100;
/** navInput.ts */
export const SMALL_DELTA_PX = 40;
/** navInput.ts */
export const FAST_GAP_MS = 50;
/** navInput.ts */
export const SLOW_GAP_MS = 100;
/** navInput.ts: DOM_DELTA_LINE → px. */
export const LINE_HEIGHT_PX = 16;
/** navInput.ts: no wheel event for this long ⇒ a fresh segment. */
export const SEGMENT_IDLE_MS = 120;

export interface WheelSample {
  deltaMode: number;
  deltaX: number;
  deltaY: number;
  /**
   * Recorded by the page probe and IGNORED here. A probe notch always opens a
   * fresh navInput segment, and navInput weighs an opening event with an infinite
   * gap whatever the real spacing was; scoring a measured gap made the mirror
   * disagree with the app on exactly the notch the probe is trying to certify.
   */
  gapMs?: number;
}

/** navInput.definitiveDevice, minus the ctrl-wheel branch the probe never sends. */
function definitive(sample: WheelSample): InputDevice | null {
  return sample.deltaMode !== 0 ? "mouse" : null;
}

/** navInput.normalizeWheel with shiftKey false (the probe sends a plain notch). */
function normalize(sample: WheelSample): { dx: number; dy: number } {
  const unit = sample.deltaMode === 1 ? LINE_HEIGHT_PX : 1;
  return { dx: sample.deltaX * unit, dy: sample.deltaY * unit };
}

/** navInput.weighEvidence with shiftKey false. */
function weigh(n: { dx: number; dy: number }, gapMs: number): { trackpad: number; mouse: number } {
  let trackpad = 0;
  let mouse = 0;
  if (n.dx !== 0) trackpad += 2;
  if (!Number.isInteger(n.dy) || !Number.isInteger(n.dx)) trackpad += 2;
  const mag = Math.max(Math.abs(n.dy), Math.abs(n.dx));
  if (mag >= BIG_TICK_PX && Number.isInteger(n.dy) && Number.isInteger(n.dx) && gapMs > SLOW_GAP_MS) {
    mouse += 2;
  }
  if (mag < SMALL_DELTA_PX && gapMs < FAST_GAP_MS) trackpad += 1;
  return { trackpad, mouse };
}

/**
 * What the app would decide for this single event opening a fresh segment.
 * `unknown` means the sample carries no motion at all, so the app would produce
 * no camera op and the probe proved nothing — never treat it as agreement.
 */
export function classifyWheel(sample: WheelSample, prior: InputDevice): InputDevice | "unknown" {
  if (!Number.isFinite(sample.deltaX) || !Number.isFinite(sample.deltaY)) return "unknown";
  const forced = definitive(sample);
  if (forced) return forced;
  const n = normalize(sample);
  if (n.dx === 0 && n.dy === 0) return "unknown";
  // navInput.reduceWheel passes Infinity for the first event of a segment.
  const w = weigh(n, Number.POSITIVE_INFINITY);
  if (w.trackpad > w.mouse) return "trackpad";
  if (w.mouse > w.trackpad) return "mouse";
  // navInput seeds an ambiguous opening event from the carried prior.
  return prior;
}
