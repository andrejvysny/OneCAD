import type { ErrorCode } from "../errors.ts";
import { AgentError, REMEDIATION, isAgentError } from "../errors.ts";
import type { Pt } from "../geometry/types.ts";

/**
 * The fidelity class of an action, from strongest to weakest.
 *
 * "real_user" is a real CGEvent posted into the system event stream: the only class that proves
 * a human could have done it. "webview" dispatches the event sequence inside the page, so the DOM
 * handlers run but nothing native does. "accessibility" asks the control itself to act
 * (`AXPress`, a settable `AXValue`), which reaches native chrome the WebView does not own but is
 * not physical input either. "diagnostic" reads or writes internal state and says nothing about
 * the UI at all.
 *
 * Only "real_user" closes an acceptance claim. The other three are labelled so a report cannot
 * quietly present them as one.
 */
export type Mode = "real_user" | "webview" | "accessibility" | "diagnostic";

/** Which interaction policy the session runs under; see `InteractionPolicy`. */
export type InteractionPolicy = "foreground" | "background";

/**
 * What this call did to the user's desktop.
 *
 * A background session promises three things, and this block is where each call reports whether
 * it kept them: the policy it ran under, whether it activated or raised anything, and whether it
 * moved the global cursor. Both booleans must be false on every background result — the single
 * activation a background LAUNCH causes is reported on the session
 * (`SessionStatus.foregroundStolenAtLaunch`), never on an action.
 */
export interface Interaction {
  policy: InteractionPolicy;
  /** This call activated the app or raised a window. */
  foregroundChanged: boolean;
  /** This call moved the global cursor. */
  cursorMoved: boolean;
}

/** The default: a call that touched neither the frontmost app nor the cursor. */
export function quietInteraction(policy: InteractionPolicy = "foreground"): Interaction {
  return { policy, foregroundChanged: false, cursorMoved: false };
}

/**
 * How far the action got before it finished or failed. The caller's retry decision reads
 * this and nothing else: `status` alone cannot tell "the click was never sent" from "the
 * click landed and the screenshot failed", and re-sending the second one double-applies it.
 */
export type DeliveryPhase =
  | "not_started"
  | "input_started"
  | "input_completed"
  | "postconditions_completed";

export interface Delivery {
  phase: DeliveryPhase;
  inputStarted: boolean;
  inputCompleted: boolean;
  /** The app's state may already have changed, whether or not this result is an error. */
  mayHaveSideEffects: boolean;
  /** Safe to re-send this exact tool call. True ONLY when no input was posted. */
  retrySafe: boolean;
  /** The input landed but the evidence (screenshot / settle / effects) is incomplete. */
  evidenceIncomplete?: boolean;
}

/**
 * The single place the retry invariant lives: `retrySafe` is true exactly at phase
 * `not_started`, and never otherwise.
 *
 * Two things it does NOT claim. A tool that reaches this helper with a truthful phase is
 * covered; one that returns a bare `okResult` gets the `not_started` default, so any tool
 * which posts input or changes process state must pass its own `delivery` (`session_start`,
 * `session_stop` and `window_focus` do). And a failure thrown before `finishAction` is turned
 * into `errorResult` with the same default — reachable via `ensureCalibrated`, whose hover
 * probe posts real cursor moves. That stays retry-safe deliberately: calibration is
 * state-neutral by construction (the wheel probe swallows its own notch, and a bare cursor
 * move changes no application state), so re-sending the call cannot double-apply anything.
 */
export function deliveryFor(
  phase: DeliveryPhase,
  stateChanging: boolean,
  evidenceIncomplete = false,
): Delivery {
  const inputCompleted = phase === "input_completed" || phase === "postconditions_completed";
  return {
    phase,
    inputStarted: phase !== "not_started",
    inputCompleted,
    // An `input_started` failure is the dangerous one: the gesture may have partly landed,
    // so it counts regardless of what the tool declared about state.
    mayHaveSideEffects: phase === "input_started" || (inputCompleted && stateChanging),
    retrySafe: phase === "not_started",
    ...(evidenceIncomplete ? { evidenceIncomplete: true } : {}),
  };
}

/** No input posted — correct for every query tool, which is always safe to repeat. */
export function notStartedDelivery(): Delivery {
  return deliveryFor("not_started", false);
}

export interface ActionResult {
  actionId: string;
  status: "ok" | "warning" | "error";
  mode: Mode;
  backend: "cgevent" | "webdriver" | "ax" | "js" | "none";
  windowId?: number;
  target?: { ref?: string; role?: string; name?: string; testId?: string };
  resolvedPoint?: { global: Pt; css: Pt };
  /**
   * Which surface this action addressed, and therefore what `state` and the settle observed.
   *
   * "webview" is the default lane: the point came from the DOM and the quiet window watched the
   * DOM idle tuple. "native" means the point came from the accessibility tree (an `{axRef}`
   * target, or a keystroke declared to be going to a native surface) and the quiet window
   * watched the AX subtree instead — a native Save panel changes nothing in the WebView, so the
   * DOM revision would report a false quiet. Absent for an action that addresses no surface at
   * all, such as `keyboard_release_all`.
   */
  surface?: "webview" | "native";
  /**
   * `afterRevision` is the DOM mutation revision the settle ended on. It is ABSENT after a
   * native settle, which never reads the page: reporting the before-revision there would claim
   * the DOM was observed to be unchanged when it was never looked at.
   */
  state?: { beforeRevision: number; afterRevision?: number; settled: boolean };
  effects?: { consoleErrors: number; logErrors: number; newWindows: number; windowMoved: boolean };
  screenshot?: {
    path: string;
    previewPath: string;
    width: number;
    height: number;
    pixelScale: number;
    captureMode: string;
    /**
     * Whether this image is evidence. True ONLY when the session holds the Screen Recording
     * grant AND the capture itself reported no degradation: a picture taken without the grant
     * shows the wallpaper, and one whose aspect disagrees with the bounds shows another window.
     */
    authoritative: boolean;
  };
  warnings: string[];
  error?: { code: ErrorCode; message: string; remediation: string; details?: Record<string, unknown> };
  /** Whether native input was posted. Read this, not `status`, before re-sending a call. */
  delivery: Delivery;
  /**
   * What this call did to the user's desktop.
   *
   * `policy` is stamped by `defineTool` on every result, so it is correct even for the tools that
   * never reach `beginAction`. The two booleans are measured by the action pipeline and default to
   * false, which is the truth for a call that never ran the frontmost gate.
   */
  interaction: Interaction;
  timingsMs: { resolve: number; input: number; settle: number; capture: number };
  /** How faithful the synthesized input is to a physical keystroke. */
  fidelity?: "text_entry" | "physical_key";
  data?: unknown;
}

export function emptyTimings(): ActionResult["timingsMs"] {
  return { resolve: 0, input: 0, settle: 0, capture: 0 };
}

export function okResult(actionId: string, mode: Mode, patch: Partial<ActionResult> = {}): ActionResult {
  return {
    actionId,
    status: "ok",
    mode,
    backend: "none",
    warnings: [],
    delivery: notStartedDelivery(),
    interaction: quietInteraction(),
    timingsMs: emptyTimings(),
    ...patch,
  };
}

export function errorResult(
  actionId: string,
  mode: Mode,
  cause: unknown,
  delivery: Delivery = notStartedDelivery(),
): ActionResult {
  const err = toAgentError(cause);
  return {
    actionId,
    status: "error",
    mode,
    backend: "none",
    warnings: [],
    delivery,
    interaction: quietInteraction(),
    timingsMs: emptyTimings(),
    error: {
      code: err.code,
      message: err.message,
      remediation: err.remediation,
      ...(err.details === undefined ? {} : { details: err.details }),
    },
  };
}

export function toAgentError(cause: unknown): AgentError {
  if (isAgentError(cause)) return cause;
  const message = cause instanceof Error ? cause.message : String(cause);
  return new AgentError("INTERNAL", message, { remediation: REMEDIATION.INTERNAL, cause });
}
