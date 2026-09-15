/**
 * The shared discipline every acting tool runs through.
 *
 *   ready → calibrated → frontmost → resolve → point gate → native input → settle →
 *   effects → screenshot → envelope
 *
 * Three of those steps exist only to refuse. `assertFrontmost` stops events going to
 * whatever app IS in front; `checkPoint` stops a coordinate outside the window or under
 * the traffic lights; `recheckHit` stops the click that a tooltip or popover stole during
 * the cursor's own dwell. None of them can be skipped by a tool author, which is why the
 * pipeline owns them instead of each tool.
 *
 * There are two lanes through the resolve and the settle, and exactly two differences between
 * them. A WebView target resolves to a CSS point gated by `checkPoint` against the calibrated
 * window and settles on the DOM idle tuple; an `{axRef}` target resolves through the
 * accessibility tree to a point already in global display points, gated by `checkGlobalPoint`
 * against every window this application owns, and settles on the AX subtree. The input is
 * identical either way — real CGEvents at a screen point — which is why `mode` stays
 * "real_user" and `backend` stays "cgevent" for both, and `surface` is what tells them apart.
 */
import type { ErrorCode } from "../../errors.ts";
import { AgentError } from "../../errors.ts";
import { log } from "../../log.ts";
import { checkGlobalPoint, checkPoint, globalToCss } from "../../geometry/mapping.ts";
import type { OwnedWindow } from "../../geometry/mapping.ts";
import type { Pt, Rect, WindowGeom } from "../../geometry/types.ts";
import type { AxRefStore, ResolvedAx } from "../../native/resolve.ts";
import { AxResolver, axRefStoreFrom } from "../../native/resolve.ts";
import type { ModKey, PlatformAdapter } from "../../platform/adapter.ts";
import { countSince, devJsonlPath, logCursor } from "../../observe/logs.ts";
import type { Resolved, Target } from "../../semantic/resolve.ts";
import type { AgentWindow } from "../../semantic/snapshotScript.ts";
import type { SnapNode, Snapshot } from "../../semantic/snapshot.ts";
import { readRevision } from "../../semantic/snapshot.ts";
import type { AgentConfig, ResolvedConfig } from "../../session/config.ts";
import type { CaptureCapability, SessionBridge } from "../../session/types.ts";
import type { Journal } from "../../trace/journal.ts";
import type { ActionResult, DeliveryPhase, Interaction, InteractionPolicy, Mode } from "../envelope.ts";
import { deliveryFor, toAgentError } from "../envelope.ts";
import { isAxTargetInput } from "../schemas.ts";
import type { ModInput, TargetInput } from "../schemas.ts";
import type { SettleOutcome } from "./settle.ts";
import { NATIVE_SETTLE_NO_PID_WARNING, settle, settleNative } from "./settle.ts";
import { POOL_REFRESHED_WARNING, resolveWithRefresh } from "./targetPool.ts";

/** A wheel probe verdict, structurally: the tools only read `ok`. */
export type WheelProbeLike =
  | { skipped: string; posted?: boolean }
  | { ok: boolean; device: string; linesPerNotch: number };

/**
 * The session surface the acting tools need. Narrower than `SessionOrchestrator` so the
 * pipeline is testable with a fake; the real orchestrator satisfies it structurally.
 */
export interface PipelineSession {
  readonly refs: Map<string, SnapNode>;
  /**
   * The ACCESSIBILITY ref pool, from the last AX walk this session took — the native sibling of
   * `refs`. Optional because it only exists once a native tool has walked: the resolver treats an
   * absent pool as "no local knowledge" and lets the helper be the sole authority, which refuses
   * later but never wrongly.
   */
  axRefs?: AxRefStore;
  wheelLinesPerNotch: number;
  windowId?: number;
  lastSnapshot?: Snapshot;
  /** Which interaction policy this session runs under; "background" never activates anything. */
  readonly interactionPolicy: InteractionPolicy;
  requireReady(): void;
  ensureCalibrated(): Promise<void>;
  assertFrontmost(): Promise<void>;
  /** Resolves to true when it actually ACTIVATED the app, false when it was already in front. */
  assertFrontmostOrFocus(): Promise<boolean>;
  requireBridge(): SessionBridge;
  requirePlatform(): PlatformAdapter;
  requireGeom(): WindowGeom;
  geometryChanged(): Promise<boolean>;
  reconnectBridge(): Promise<void>;
  ensureWheelProbe(): Promise<WheelProbeLike>;
  status(): { pid?: number; launched: boolean; capture: CaptureCapability };
}

export interface PipelineCtx {
  session: PipelineSession;
  journal: Journal;
  config: Pick<ResolvedConfig, "root" | "config">;
}

export interface ActionEnv {
  session: PipelineSession;
  cfg: AgentConfig;
  journal: Journal;
  geom: WindowGeom;
  platform: PlatformAdapter;
  bridge: SessionBridge;
  occlusion: Rect[];
  /** What this session can prove with a picture; decides whether a capture is attempted at all. */
  capture: CaptureCapability;
  logPath: string | null;
  pid?: number;
  /** Collected while resolving (pool refresh etc.); merged into the envelope by finishAction. */
  warnings: string[];
  /**
   * What this call has done to the user's desktop so far. Mutable because the gate that may
   * activate the app runs in `beginAction`, long before the envelope is built — and because a
   * background session's whole promise is that both booleans are still false at the end.
   */
  interaction: Interaction;
}

export interface Hit {
  /** The WebView resolution. Absent for an accessibility target, which has no DOM element. */
  resolved?: Resolved;
  /**
   * CSS pixels in the main window's content box. For a native hit this is the projection of the
   * global point into that space — well defined, and deliberately reported, but it names no DOM
   * element and may fall outside the window: read `surface` before treating it as a page point.
   */
  css: Pt;
  global: Pt;
  /** Present when the point came from the accessibility tree rather than the WebView. */
  ax?: ResolvedAx;
  surface: "webview" | "native";
}

export interface ActionSpec {
  actionId: string;
  mode: Mode;
  backend: ActionResult["backend"];
  /**
   * Which surface this action addressed. It selects the settle instrument — the DOM idle tuple
   * for "webview", the accessibility subtree for "native" — and is reported in the envelope.
   * Omit only for an action that addresses no surface at all.
   */
  surface?: ActionResult["surface"];
  /** Drives the "state_changing" screenshot policy. */
  stateChanging: boolean;
  /**
   * This action's input moves the global cursor. True for every native pointer verb; false for
   * keyboard verbs and for the whole webview lane, which dispatches in the page and never
   * touches the pointer. Only counted once input has actually started.
   */
  movesCursor?: boolean;
  screenshot?: boolean;
  warnings?: string[];
  target?: ActionResult["target"];
  resolvedPoint?: { global: Pt; css: Pt };
  resolveMs?: number;
  data?: unknown;
  input(env: ActionEnv): Promise<void>;
}

/** How long a tooltip or popover gets to open under the cursor before the click re-check. */
export const MOVE_RECHECK_MS = 60;
const FAILURE_SHOT_BUDGET_MS = 5_000;

export const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

export async function beginAction(
  ctx: PipelineCtx,
  opts: { requireFrontmost?: boolean } = {},
): Promise<ActionEnv> {
  const session = ctx.session;
  session.requireReady();
  // Focus BEFORE reading geometry: on a multi-Space desktop the app's window is only returned by
  // the on-screen window query while its Space is active, so bringing the app forward first is what
  // makes calibration see the window at all. (Calibrating first would hit an empty window list.)
  const foregroundChanged =
    opts.requireFrontmost === false ? false : await session.assertFrontmostOrFocus();
  await session.ensureCalibrated();
  const cfg = ctx.config.config;
  const status = session.status();
  return {
    session,
    cfg,
    journal: ctx.journal,
    geom: session.requireGeom(),
    platform: session.requirePlatform(),
    bridge: session.requireBridge(),
    occlusion: cfg.nativeOcclusion.rects,
    capture: status.capture,
    warnings: [],
    interaction: { policy: session.interactionPolicy, foregroundChanged, cursorMoved: false },
    logPath: devJsonlPath({
      root: ctx.config.root,
      devJsonl: cfg.logs.devJsonl,
      journalDir: ctx.journal.dir,
      launched: status.launched,
    }),
    ...(status.pid === undefined ? {} : { pid: status.pid }),
  };
}

export async function resolveTarget(
  env: ActionEnv,
  target: TargetInput,
  opts: { offset?: Pt; forDrag?: boolean } = {},
): Promise<Hit> {
  if (isAxTargetInput(target)) return resolveAxTarget(env, target.axRef, opts);
  const { resolved, refreshed } = await resolveWithRefresh(env.session, env.bridge, target as Target, {
    forInput: true,
    geom: env.geom,
    ...(opts.forDrag === true ? { forDrag: true } : {}),
    ...(opts.offset === undefined ? {} : { offset: opts.offset }),
  });
  if (refreshed) env.warnings.push(POOL_REFRESHED_WARNING);
  if (opts.forDrag === true && dragRegionOf(resolved)) {
    throw new AgentError("ELEMENT_OCCLUDED", "a drag starting here would move the window, not the content", {
      remediation: "Start the drag on a point that is not over a data-tauri-drag-region element.",
      details: { reason: "tauri-drag-region", css: resolved.css },
    });
  }
  return {
    resolved,
    css: resolved.css,
    global: checkPoint(resolved.css, env.geom, env.occlusion),
    surface: "webview",
  };
}

/**
 * The native lane of the same step: accessibility LOCATES, CGEvent ACTS.
 *
 * Two things are different from the webview lane and nothing else is. The point arrives already
 * in global display points, so it is gated by `checkGlobalPoint` against every window this
 * application owns rather than by `checkPoint` against the one calibrated window — an
 * `NSSavePanel`, a sheet or a menu legitimately extends past the main window, and gating those
 * on it would make them unreachable by construction. And no occlusion rects are passed: those
 * describe native controls sitting ABOVE the webview, which is exactly what this lane exists to
 * reach.
 *
 * `offset` is accepted for symmetry and applied to the AX centre in global points. `forDrag` has
 * no native meaning — a native element is not a `data-tauri-drag-region` — so it is ignored here
 * rather than silently mapped onto the webview check.
 */
async function resolveAxTarget(
  env: ActionEnv,
  axRef: string,
  opts: { offset?: Pt } = {},
): Promise<Hit> {
  const pid = env.pid;
  if (pid === undefined) {
    throw new AgentError("APP_NOT_RUNNING", "this session does not know the app's pid, so it cannot read the accessibility tree", {
      remediation: "Start or attach a session first (session_start); an accessibility target is addressed by process id.",
      details: { axRef },
    });
  }
  const platform = env.platform;
  const resolver = new AxResolver({
    ax: platform.ax,
    windows: platform.windows,
    pid,
    ...(env.session.axRefs === undefined ? {} : { refs: env.session.axRefs }),
  });
  const ax = await resolver.resolve({ ref: axRef });
  // A held ref never walks, so the pool cannot change here — written back anyway so this stays
  // correct if the resolver ever mints a generation on this path.
  if (resolver.refs !== undefined) env.session.axRefs = resolver.refs;
  const global = await offsetPoint(env, pid, ax.global, opts.offset);
  return { ax, global, css: globalToCss(global, env.geom), surface: "native" };
}

/**
 * The resolver gated the element's CENTRE. An offset moves the point away from what was gated,
 * so it is gated again against the same windows — otherwise `offset` would be a way to post a
 * click outside every window this application owns, which is the one thing the gate exists to
 * stop. No offset means no second window read.
 */
async function offsetPoint(env: ActionEnv, pid: number, centre: Pt, offset: Pt | undefined): Promise<Pt> {
  if (offset === undefined) return centre;
  const owned: OwnedWindow[] = (await env.platform.windows.list(pid))
    .filter((w) => w.onscreen)
    .map((w) => ({ windowId: w.windowId, bounds: w.bounds }));
  return checkGlobalPoint({ x: centre.x + offset.x, y: centre.y + offset.y }, owned, null);
}

/**
 * `Resolved.dragRegion` is reported for raw-point targets, which have no snapshot node for
 * the resolver's own drag-region refusal to inspect. Read defensively so this file does not
 * break while the semantic layer lands that field.
 */
function dragRegionOf(resolved: Resolved): boolean {
  return (resolved as { dragRegion?: boolean }).dragRegion === true;
}

export function targetBrief(hit: Hit): ActionResult["target"] | undefined {
  if (hit.ax) {
    // The AX ref, role and title, in the same three fields the webview brief uses: a report
    // reads one shape whichever surface the click landed on.
    const p = hit.ax.point;
    return {
      ref: p.ref,
      ...(p.role === null ? {} : { role: p.role }),
      ...(p.title === null ? {} : { name: p.title }),
    };
  }
  const n = hit.resolved?.node;
  if (!n) return undefined;
  return { ref: n.ref, role: n.role, name: n.name, ...(n.testId === undefined ? {} : { testId: n.testId }) };
}

/** A CSS point mapped and gated exactly like an element target. */
export function pointHit(env: ActionEnv, css: Pt): Hit {
  return {
    resolved: { css, source: "point" },
    css,
    global: checkPoint(css, env.geom, env.occlusion),
    surface: "webview",
  };
}

/**
 * S6: the cursor's own arrival can open a tooltip or popover over the target, so the element
 * that was hit-tested before the move may not be the one that receives the press.
 */
export async function recheckHit(env: ActionEnv, hit: Hit, target: TargetInput): Promise<void> {
  // A native target has no DOM element and no `elementFromPoint` to ask. Its own guard already
  // ran in the helper — `ax_point` re-validates the ref, refuses a role that changed under the
  // handle, and samples the rect twice 50 ms apart — so this check is skipped rather than
  // approximated against a page that does not own the element.
  if (hit.surface === "native") return;
  const node = hit.resolved?.node;
  const selector = node?.css ?? ("css" in target ? target.css : null);
  if (selector === null) return;
  const ok = await env.bridge.execute<boolean>(
    "pointer.recheckHit",
    ((ref: string | null, sel: string, x: number, y: number) => {
      const refs = (window as unknown as AgentWindow).__tauriAgentRefs;
      // A held ref is never re-found by its positional selector: a detached element must
      // read as occluded/stale, not as whichever sibling slid into its slot (review B1).
      let el: Element | null = ref && refs ? refs.get(ref) ?? null : null;
      if (ref === null && sel) el = document.querySelector(sel);
      if (el === null || !el.isConnected) return false;
      const under = document.elementFromPoint(x, y);
      return under !== null && (under === el || el.contains(under));
    }) as never,
    [node?.ref ?? null, selector, hit.css.x, hit.css.y],
    { readOnly: true },
  );
  if (!ok) {
    throw new AgentError("ELEMENT_OCCLUDED", "something opened over the target while the cursor moved onto it", {
      remediation:
        "A hover-triggered tooltip or popover now covers the point. Dismiss it, target the overlay itself, or click a point outside its bounds.",
      details: { reason: "post-move-occlusion", css: hit.css, selector },
    });
  }
}

export function toModKeys(mods: ModInput[] | undefined): ModKey[] {
  const primary: ModKey = process.platform === "darwin" ? "Command" : "Control";
  return (mods ?? []).map((m) => (m === "Primary" ? primary : m));
}

interface BeforeState {
  revision: number;
  consoleErrors: number;
  logCursor: string;
  windows: number;
}

/**
 * Carried through the stages so `recover` can tell an action that was REFUSED from one that
 * was DELIVERED. Mutable on purpose: the phase is the only thing a failure handler can use to
 * decide whether re-sending this call would double-apply it.
 */
interface DeliveryTracker {
  phase: DeliveryPhase;
}

export async function finishAction(env: ActionEnv, spec: ActionSpec): Promise<ActionResult> {
  const warnings = [...env.warnings, ...(spec.warnings ?? [])];
  const timings = { resolve: spec.resolveMs ?? 0, input: 0, settle: 0, capture: 0 };
  const tracker: DeliveryTracker = { phase: "not_started" };
  try {
    // An explicitly requested picture this session cannot take is refused BEFORE any input is
    // posted, so the caller is told plainly and the call is still retry-safe. (A policy-driven
    // picture is best-effort evidence and is handled after the input, as a warning.)
    refuseUnavailableExplicitShot(env, spec);
    const before = await captureBefore(env);
    const started = Date.now();
    tracker.phase = "input_started";
    if (spec.movesCursor === true) env.interaction.cursorMoved = true;
    try {
      await spec.input(env);
      tracker.phase = "input_completed";
    } finally {
      timings.input = Date.now() - started;
    }
    return await afterInput(env, spec, before, warnings, timings, tracker);
  } catch (cause) {
    return await recover(env, spec, cause, warnings, timings, tracker.phase);
  }
}

async function afterInput(
  env: ActionEnv,
  spec: ActionSpec,
  before: BeforeState,
  warnings: string[],
  timings: ActionResult["timingsMs"],
  tracker: DeliveryTracker,
): Promise<ActionResult> {
  const s0 = Date.now();
  const outcome = await settleFor(env, spec);
  timings.settle = Date.now() - s0;
  if (outcome.warning !== undefined) warnings.push(outcome.warning);
  const effects = await effectsSince(env, before, warnings);
  const c0 = Date.now();
  // The picture is evidence, never a verdict: a capture that fails here must not turn a
  // delivered action into an error the caller would then re-send.
  let shot: ActionResult["screenshot"] | undefined;
  let evidenceIncomplete = false;
  if (wantsShot(env.cfg.screenshots.policy, spec)) {
    if (!env.capture.available) {
      // Skipping silently would be worse than the failed attempt this replaces: the caller
      // would read a picture-less "ok" as a picture-less success rather than as blind driving.
      evidenceIncomplete = true;
      warnings.push(NO_CAPTURE_WARNING);
    } else {
      try {
        shot = await captureShot(env, spec.actionId, "after", warnings);
      } catch (cause) {
        const err = toAgentError(cause);
        evidenceIncomplete = true;
        warnings.push(
          `the input WAS delivered; only the after-action screenshot failed (${err.code}: ${err.message}), so this result carries no picture — do not re-send this call`,
        );
      }
    }
  }
  timings.capture = Date.now() - c0;
  tracker.phase = "postconditions_completed";
  return {
    actionId: spec.actionId,
    status: warnings.length > 0 ? "warning" : "ok",
    mode: spec.mode,
    backend: spec.backend,
    ...(env.geom.windowId === undefined ? {} : { windowId: env.geom.windowId }),
    ...(spec.target === undefined ? {} : { target: spec.target }),
    ...(spec.resolvedPoint === undefined ? {} : { resolvedPoint: spec.resolvedPoint }),
    ...(spec.surface === undefined ? {} : { surface: spec.surface }),
    state: {
      beforeRevision: before.revision,
      // A native settle never read the page, so there IS no after-revision: reporting the
      // before-revision here would claim the DOM was observed unchanged when it was not looked at.
      ...(outcome.afterRevision === undefined ? {} : { afterRevision: outcome.afterRevision }),
      settled: outcome.settled,
    },
    effects,
    ...(shot === undefined ? {} : { screenshot: shot }),
    warnings,
    delivery: deliveryFor(tracker.phase, spec.stateChanging, evidenceIncomplete),
    interaction: env.interaction,
    timingsMs: timings,
    ...dataPatch(spec.data, outcome),
  };
}

/**
 * Which instrument the quiet window uses. The DOM idle tuple is the wrong one for a native
 * target — a Save panel changes nothing in the WebView, so every member is stable from the
 * first read and the settle would report a quiet it never observed.
 */
async function settleFor(env: ActionEnv, spec: ActionSpec): Promise<SettleOutcome> {
  if (spec.surface !== "native") return settle(env.bridge, env.cfg.settle);
  const pid = env.pid;
  if (pid === undefined) return { settled: false, warning: NATIVE_SETTLE_NO_PID_WARNING };
  return settleNative(
    {
      ax: env.platform.ax,
      pid,
      // Every walk bumps the helper's ref generation, so the session's AX pool has to follow it
      // or the refs it holds are a generation behind and refuse while perfectly live.
      onWalk: (walk) => {
        env.session.axRefs = axRefStoreFrom(walk.generation, walk.nodes);
      },
    },
    env.cfg.settle,
  );
}

/**
 * The tool's own payload, plus what a native settle saw. `generation` is the load-bearing part:
 * the settle re-walked the tree, so every AX ref minted before this action is now stale and the
 * caller has to know that without guessing.
 */
function dataPatch(data: unknown, outcome: SettleOutcome): { data?: unknown } {
  if (outcome.ax === undefined) return data === undefined ? {} : { data };
  const base = typeof data === "object" && data !== null && !Array.isArray(data) ? (data as Record<string, unknown>) : {};
  return { data: { ...base, nativeSettle: outcome.ax } };
}

/**
 * The action threw. Nothing is retried here — a re-sent click double-applies — but the
 * machine must not be left with a button or modifier held, and the failure is worth a
 * picture.
 *
 * The phase decides the verdict. Before or during the input the action did NOT happen, so
 * this is an `error`. Once the input completed, only the evidence failed: reporting that as
 * an `error` is exactly what makes a caller re-send a Save or a Delete, so it comes back as
 * a `warning` with the cause in `data.postconditionError` and no `error` field at all.
 */
async function recover(
  env: ActionEnv,
  spec: ActionSpec,
  cause: unknown,
  warnings: string[],
  timings: ActionResult["timingsMs"],
  phase: DeliveryPhase,
): Promise<ActionResult> {
  await env.platform.input.releaseAll().catch((e: unknown) => log.warn("releaseAll failed", { error: String(e) }));
  const err = toAgentError(cause);
  if (err.code === "BRIDGE_WEDGED") await reconnectOnce(env, warnings);
  const delivered = phase === "input_completed" || phase === "postconditions_completed";
  if (delivered) {
    warnings.push(
      `the input WAS delivered; only the post-action evidence failed (${err.code}: ${err.message}) — the app may already have changed, so do not re-send this call`,
    );
  }
  const c0 = Date.now();
  // No grant means no picture is possible; attempting one here would only add a second,
  // misleading failure on top of the one being reported.
  const skipShot = env.cfg.screenshots.policy === "never" || !env.capture.available;
  const shot = skipShot ? undefined : await failureShot(env, spec.actionId, warnings);
  timings.capture = Date.now() - c0;
  const common = {
    actionId: spec.actionId,
    mode: spec.mode,
    backend: spec.backend,
    ...(env.geom.windowId === undefined ? {} : { windowId: env.geom.windowId }),
    ...(spec.target === undefined ? {} : { target: spec.target }),
    ...(spec.resolvedPoint === undefined ? {} : { resolvedPoint: spec.resolvedPoint }),
    ...(spec.surface === undefined ? {} : { surface: spec.surface }),
    ...(shot === undefined ? {} : { screenshot: shot }),
    warnings,
    interaction: env.interaction,
    timingsMs: timings,
  };
  if (delivered) {
    return {
      ...common,
      status: "warning",
      delivery: deliveryFor(phase, spec.stateChanging, true),
      data: withPostconditionError(spec.data, { code: err.code, message: err.message }),
    };
  }
  return {
    ...common,
    status: "error",
    delivery: deliveryFor(phase, spec.stateChanging),
    error: {
      code: err.code,
      message: err.message,
      remediation: err.remediation,
      ...(err.details === undefined ? {} : { details: err.details }),
    },
  };
}

/** Keeps the tool's own payload (drag points, key, repeat) alongside the postcondition cause. */
function withPostconditionError(
  data: unknown,
  postconditionError: { code: ErrorCode; message: string },
): Record<string, unknown> {
  const base =
    typeof data === "object" && data !== null && !Array.isArray(data) ? (data as Record<string, unknown>) : {};
  return { ...base, postconditionError };
}

async function reconnectOnce(env: ActionEnv, warnings: string[]): Promise<void> {
  try {
    await env.session.reconnectBridge();
    warnings.push("the WebView bridge wedged and was rebuilt; every ref from an earlier ui_snapshot is now invalid");
  } catch (e) {
    warnings.push(`the WebView bridge wedged and could not be rebuilt: ${String(e)}`);
  }
}

async function captureBefore(env: ActionEnv): Promise<BeforeState> {
  return {
    revision: (await readRevision(env.bridge)).rev,
    consoleErrors: await consoleErrorCount(env.bridge),
    logCursor: await logCursor(env.logPath),
    windows: await windowCount(env),
  };
}

async function effectsSince(
  env: ActionEnv,
  before: BeforeState,
  warnings: string[],
): Promise<NonNullable<ActionResult["effects"]>> {
  const consoleErrors = Math.max(0, (await consoleErrorCount(env.bridge)) - before.consoleErrors);
  const logErrors = await countSince(env.logPath, before.logCursor, "ERROR");
  const newWindows = Math.max(0, (await windowCount(env)) - before.windows);
  const windowMoved = await env.session.geometryChanged();
  if (consoleErrors > 0) warnings.push(`the webview logged ${consoleErrors} console error(s) during this action`);
  if (logErrors > 0) warnings.push(`the app logged ${logErrors} ERROR line(s) during this action (see observe_logs)`);
  if (windowMoved) warnings.push("the window moved or resized during this action; coordinates are recalibrated before the next one");
  return { consoleErrors, logErrors, newWindows, windowMoved };
}

function consoleErrorCount(bridge: SessionBridge): Promise<number> {
  return bridge.execute<number>(
    "effects.consoleErrors",
    (() => {
      const ring = (window as unknown as AgentWindow).__tauriAgentConsole;
      return ring ? ring.errors : 0;
    }) as never,
    [],
    { readOnly: true },
  );
}

async function windowCount(env: ActionEnv): Promise<number> {
  if (env.pid === undefined) return 0;
  try {
    return (await env.platform.windows.list(env.pid)).length;
  } catch (e) {
    log.debug("window count unavailable", { error: String(e) });
    return 0;
  }
}

export function wantsShot(policy: AgentConfig["screenshots"]["policy"], spec: Pick<ActionSpec, "stateChanging" | "screenshot">): boolean {
  if (policy === "never" || spec.screenshot === false) return false;
  if (spec.screenshot === true) return true;
  if (policy === "always") return true;
  if (policy === "on_failure") return false;
  return spec.stateChanging;
}

/** Said once, in the result of every action whose policy picture this session cannot take. */
export const NO_CAPTURE_WARNING =
  "no screenshot was taken: this session has no Screen Recording grant (it was started with " +
  "allowDegradedCapture), so the window cannot be photographed at all — this action was driven " +
  "blind. Grant Screen Recording in System Settings › Privacy & Security and start a new session " +
  "for picture evidence.";

/**
 * A caller who passed `screenshot:true` asked for a picture, and must be TOLD it cannot have
 * one rather than handed a result that quietly lacks it. Thrown before any input is posted, so
 * the refusal is retry-safe; the policy-driven shot is best-effort and only warns.
 */
function refuseUnavailableExplicitShot(env: ActionEnv, spec: ActionSpec): void {
  if (spec.screenshot !== true || env.capture.available) return;
  if (!wantsShot(env.cfg.screenshots.policy, spec)) return;
  throw new AgentError(
    "SCREEN_CAPTURE_PERMISSION_DENIED",
    "this session cannot take a screenshot: it was started without the Screen Recording grant",
    {
      remediation:
        "Grant Screen Recording in System Settings › Privacy & Security and start a new session, or re-send this call without `screenshot: true`.",
      details: { capture: env.capture },
    },
  );
}

async function captureShot(
  env: ActionEnv,
  actionId: string,
  label: string,
  warnings: string[],
): Promise<ActionResult["screenshot"]> {
  const path = env.journal.artifactPath(`${actionId}-${label}.png`);
  const previewPath = path.replace(/\.png$/, "-preview.png");
  const { warning, authoritative, reason, ...px } = await env.platform.capture.window(
    env.geom.windowId,
    path,
    env.geom.nativeBoundsPt,
  );
  await env.platform.capture.preview(path, previewPath, env.cfg.screenshots.previewMaxPx);
  if (warning !== undefined) warnings.push(warning);
  return {
    path,
    previewPath,
    ...px,
    captureMode: "window",
    // Both halves must hold: the session may photograph, AND this particular image is of the
    // thing that was asked for. `reason` is read only to keep it out of the envelope.
    authoritative: env.capture.authoritative && authoritative !== false && warning === undefined && reason === undefined,
  };
}

/** Best effort under a hard budget: a failing action must not also hang on its own picture. */
async function failureShot(
  env: ActionEnv,
  actionId: string,
  warnings: string[],
): Promise<ActionResult["screenshot"] | undefined> {
  try {
    return await Promise.race([
      captureShot(env, actionId, "failure", warnings),
      sleep(FAILURE_SHOT_BUDGET_MS).then(() => {
        throw new Error(`capture exceeded ${FAILURE_SHOT_BUDGET_MS}ms`);
      }),
    ]);
  } catch (e) {
    log.warn("failure screenshot skipped", { actionId, error: String(e) });
    return undefined;
  }
}
