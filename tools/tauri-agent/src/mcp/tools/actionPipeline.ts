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
 */
import { AgentError } from "../../errors.ts";
import { log } from "../../log.ts";
import { checkPoint } from "../../geometry/mapping.ts";
import type { Pt, Rect, WindowGeom } from "../../geometry/types.ts";
import type { ModKey, PlatformAdapter } from "../../platform/adapter.ts";
import { countSince, devJsonlPath, logCursor } from "../../observe/logs.ts";
import type { Resolved, Target } from "../../semantic/resolve.ts";
import type { AgentWindow } from "../../semantic/snapshotScript.ts";
import type { SnapNode, Snapshot } from "../../semantic/snapshot.ts";
import { readRevision } from "../../semantic/snapshot.ts";
import type { AgentConfig, ResolvedConfig } from "../../session/config.ts";
import type { SessionBridge } from "../../session/types.ts";
import type { Journal } from "../../trace/journal.ts";
import type { ActionResult, Mode } from "../envelope.ts";
import { toAgentError } from "../envelope.ts";
import type { ModInput, TargetInput } from "../schemas.ts";
import { settle } from "./settle.ts";
import { POOL_REFRESHED_WARNING, resolveWithRefresh } from "./targetPool.ts";

/** A wheel probe verdict, structurally: the tools only read `ok`. */
export type WheelProbeLike = { skipped: string } | { ok: boolean; device: string; linesPerNotch: number };

/**
 * The session surface the acting tools need. Narrower than `SessionOrchestrator` so the
 * pipeline is testable with a fake; the real orchestrator satisfies it structurally.
 */
export interface PipelineSession {
  readonly refs: Map<string, SnapNode>;
  wheelLinesPerNotch: number;
  windowId?: number;
  lastSnapshot?: Snapshot;
  requireReady(): void;
  ensureCalibrated(): Promise<void>;
  assertFrontmost(): Promise<void>;
  assertFrontmostOrFocus(): Promise<void>;
  requireBridge(): SessionBridge;
  requirePlatform(): PlatformAdapter;
  requireGeom(): WindowGeom;
  geometryChanged(): Promise<boolean>;
  reconnectBridge(): Promise<void>;
  ensureWheelProbe(): Promise<WheelProbeLike>;
  status(): { pid?: number; launched: boolean };
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
  logPath: string | null;
  pid?: number;
  /** Collected while resolving (pool refresh etc.); merged into the envelope by finishAction. */
  warnings: string[];
}

export interface Hit {
  resolved: Resolved;
  css: Pt;
  global: Pt;
}

export interface ActionSpec {
  actionId: string;
  mode: Mode;
  backend: ActionResult["backend"];
  /** Drives the "state_changing" screenshot policy. */
  stateChanging: boolean;
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
  if (opts.requireFrontmost !== false) await session.assertFrontmostOrFocus();
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
    warnings: [],
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
  return { resolved, css: resolved.css, global: checkPoint(resolved.css, env.geom, env.occlusion) };
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
  const n = hit.resolved.node;
  if (!n) return undefined;
  return { ref: n.ref, role: n.role, name: n.name, ...(n.testId === undefined ? {} : { testId: n.testId }) };
}

/** A CSS point mapped and gated exactly like an element target. */
export function pointHit(env: ActionEnv, css: Pt): Hit {
  return { resolved: { css, source: "point" }, css, global: checkPoint(css, env.geom, env.occlusion) };
}

/**
 * S6: the cursor's own arrival can open a tooltip or popover over the target, so the element
 * that was hit-tested before the move may not be the one that receives the press.
 */
export async function recheckHit(env: ActionEnv, hit: Hit, target: TargetInput): Promise<void> {
  const node = hit.resolved.node;
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

export async function finishAction(env: ActionEnv, spec: ActionSpec): Promise<ActionResult> {
  const warnings = [...env.warnings, ...(spec.warnings ?? [])];
  const timings = { resolve: spec.resolveMs ?? 0, input: 0, settle: 0, capture: 0 };
  try {
    const before = await captureBefore(env);
    const started = Date.now();
    try {
      await spec.input(env);
    } finally {
      timings.input = Date.now() - started;
    }
    return await afterInput(env, spec, before, warnings, timings);
  } catch (cause) {
    return await recover(env, spec, cause, warnings, timings);
  }
}

async function afterInput(
  env: ActionEnv,
  spec: ActionSpec,
  before: BeforeState,
  warnings: string[],
  timings: ActionResult["timingsMs"],
): Promise<ActionResult> {
  const s0 = Date.now();
  const outcome = await settle(env.bridge, env.cfg.settle);
  timings.settle = Date.now() - s0;
  if (outcome.warning !== undefined) warnings.push(outcome.warning);
  const effects = await effectsSince(env, before, warnings);
  const c0 = Date.now();
  const shot = wantsShot(env.cfg.screenshots.policy, spec) ? await captureShot(env, spec.actionId, "after") : undefined;
  timings.capture = Date.now() - c0;
  return {
    actionId: spec.actionId,
    status: warnings.length > 0 ? "warning" : "ok",
    mode: spec.mode,
    backend: spec.backend,
    ...(env.geom.windowId === undefined ? {} : { windowId: env.geom.windowId }),
    ...(spec.target === undefined ? {} : { target: spec.target }),
    ...(spec.resolvedPoint === undefined ? {} : { resolvedPoint: spec.resolvedPoint }),
    state: { beforeRevision: before.revision, afterRevision: outcome.afterRevision, settled: outcome.settled },
    effects,
    ...(shot === undefined ? {} : { screenshot: shot }),
    warnings,
    timingsMs: timings,
    ...(spec.data === undefined ? {} : { data: spec.data }),
  };
}

/**
 * The action threw after (or while) posting native input. Nothing is retried — a re-sent
 * click double-applies — but the machine must not be left with a button or modifier held,
 * and the failure is worth a picture.
 */
async function recover(
  env: ActionEnv,
  spec: ActionSpec,
  cause: unknown,
  warnings: string[],
  timings: ActionResult["timingsMs"],
): Promise<ActionResult> {
  await env.platform.input.releaseAll().catch((e: unknown) => log.warn("releaseAll failed", { error: String(e) }));
  const err = toAgentError(cause);
  if (err.code === "BRIDGE_WEDGED") await reconnectOnce(env, warnings);
  const c0 = Date.now();
  const shot = env.cfg.screenshots.policy === "never" ? undefined : await failureShot(env, spec.actionId);
  timings.capture = Date.now() - c0;
  return {
    actionId: spec.actionId,
    status: "error",
    mode: spec.mode,
    backend: spec.backend,
    ...(env.geom.windowId === undefined ? {} : { windowId: env.geom.windowId }),
    ...(spec.target === undefined ? {} : { target: spec.target }),
    ...(spec.resolvedPoint === undefined ? {} : { resolvedPoint: spec.resolvedPoint }),
    ...(shot === undefined ? {} : { screenshot: shot }),
    warnings,
    error: {
      code: err.code,
      message: err.message,
      remediation: err.remediation,
      ...(err.details === undefined ? {} : { details: err.details }),
    },
    timingsMs: timings,
  };
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

async function captureShot(env: ActionEnv, actionId: string, label: string): Promise<ActionResult["screenshot"]> {
  const path = env.journal.artifactPath(`${actionId}-${label}.png`);
  const previewPath = path.replace(/\.png$/, "-preview.png");
  const shot = await env.platform.capture.window(env.geom.windowId, path, env.geom.nativeBoundsPt);
  await env.platform.capture.preview(path, previewPath, env.cfg.screenshots.previewMaxPx);
  return { path, previewPath, ...shot, captureMode: "window" };
}

/** Best effort under a hard budget: a failing action must not also hang on its own picture. */
async function failureShot(env: ActionEnv, actionId: string): Promise<ActionResult["screenshot"] | undefined> {
  try {
    return await Promise.race([
      captureShot(env, actionId, "failure"),
      sleep(FAILURE_SHOT_BUDGET_MS).then(() => {
        throw new Error(`capture exceeded ${FAILURE_SHOT_BUDGET_MS}ms`);
      }),
    ]);
  } catch (e) {
    log.warn("failure screenshot skipped", { actionId, error: String(e) });
    return undefined;
  }
}
