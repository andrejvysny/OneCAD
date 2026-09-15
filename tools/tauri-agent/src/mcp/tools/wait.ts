/**
 * `wait_for` — the only correct way to synchronise with an app that regenerates geometry in
 * a worker process. A fixed sleep either flakes or wastes seconds; these conditions poll a
 * real observable (DOM, revision, log line, window count) and report what they LAST saw when
 * they give up, which is usually the whole diagnosis.
 */
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { AgentError, isAgentError } from "../../errors.ts";
import { tailLogs, devJsonlPath, type LogLevel } from "../../observe/logs.ts";
import { Resolver, refStoreFrom } from "../../semantic/resolve.ts";
import type { Target } from "../../semantic/resolve.ts";
import type { IdleReading } from "../../semantic/idleScript.ts";
import { readIdle, readRevision, takeSnapshot } from "../../semantic/snapshot.ts";
import type { SessionBridge } from "../../session/types.ts";
import { defineTool, type ToolCtx } from "../defineTool.ts";
import type { ActionResult } from "../envelope.ts";
import { errorResult, okResult } from "../envelope.ts";
import { TargetSchema, type TargetInput } from "../schemas.ts";
import { sleep } from "./actionPipeline.ts";

const POLL_MS = 100;
const DEFAULT_TIMEOUT_MS = 10_000;

const LevelSchema = z.enum(["TRACE", "DEBUG", "INFO", "WARN", "ERROR"]);

const ConditionSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("element"), target: TargetSchema }),
  z.object({ kind: z.literal("element_hidden"), target: TargetSchema }),
  z.object({ kind: z.literal("text"), text: z.string().min(1) }),
  z.object({ kind: z.literal("text_gone"), text: z.string().min(1) }),
  z.object({
    kind: z.literal("attribute"),
    target: TargetSchema,
    name: z.string().min(1),
    value: z.string().optional().describe("Omit to wait for the attribute to merely be present."),
  }),
  z.object({ kind: z.literal("revision_stable"), quietMs: z.number().int().positive().max(10_000).optional() }),
  z.object({
    kind: z.literal("worker_idle"),
    // No options: "idle" is regenBusy === 0 AND geometryPending === false, both read from the
    // app's own stores. An UNKNOWN reading (the signal is not published) is never reported as
    // idle — the wait times out reporting exactly that, rather than claiming a finished regen.
  }),
  z.object({ kind: z.literal("render_idle"), quietMs: z.number().int().positive().max(10_000).optional() }),
  z.object({
    kind: z.literal("render_frame_after"),
    frames: z.number().int().nonnegative().describe("Wait until ViewportEngine.frameCount exceeds this."),
  }),
  z.object({
    kind: z.literal("snapshot_at_least"),
    id: z.number().int().nonnegative().describe("Wait until the backend's published snapshot id reaches this."),
  }),
  z.object({ kind: z.literal("camera_stable"), quietMs: z.number().int().positive().max(10_000).optional() }),
  z.object({
    kind: z.literal("log_line"),
    grep: z.string().min(1).describe("Regular expression matched against the raw dev.jsonl line."),
    level: LevelSchema.optional(),
    lane: z.string().optional().describe("Matches the log `target` prefix: fe, worker, onecad_lib, onecad_protocol::frames."),
  }),
  z.object({
    kind: z.literal("window"),
    count: z.number().int().nonnegative().optional().describe("Wait until the app has at least this many on-screen windows."),
    name: z.string().optional().describe("Wait until a window with this title appears."),
  }),
  z.object({ kind: z.literal("delay"), ms: z.number().int().nonnegative().max(60_000) }),
]);

type Condition = z.infer<typeof ConditionSchema>;
type Probe = () => Promise<{ done: boolean; observed: unknown }>;

interface PageProbe {
  present: boolean;
  visible: boolean;
  attr: string | null;
}

/** A selector that can be queried directly, so the common waits cost one tiny script. */
function staticSelector(ctx: ToolCtx, target: TargetInput): string | null {
  if ("css" in target) return target.css;
  if ("testId" in target) return `[data-testid="${target.testId}"]`;
  if ("ref" in target) return ctx.session.refs.get(target.ref)?.css ?? null;
  return null;
}

function pageProbe(bridge: SessionBridge, selector: string, attr: string | null): Promise<PageProbe> {
  return bridge.execute<PageProbe>(
    "wait.element",
    ((sel: string, name: string | null) => {
      const el = document.querySelector(sel);
      if (el === null) return { present: false, visible: false, attr: null };
      const r = el.getBoundingClientRect();
      return {
        present: true,
        visible: r.width > 0 && r.height > 0,
        attr: name === null ? null : el.getAttribute(name),
      };
    }) as never,
    [selector, attr],
    { readOnly: true },
  );
}

/**
 * role/name and text targets need the semantic pool, and the pool is only as fresh as the
 * last snapshot — so waiting on one re-snapshots each poll and republishes the refs, which
 * also leaves the caller able to act on what the wait found.
 */
async function viaSnapshot(ctx: ToolCtx, target: TargetInput): Promise<{ present: boolean; observed: unknown }> {
  const bridge = ctx.session.requireBridge();
  const snap = await takeSnapshot(bridge, { mode: "interactive" });
  ctx.session.refs.clear();
  for (const n of snap.nodes) ctx.session.refs.set(n.ref, n);
  ctx.session.lastSnapshot = snap;
  try {
    const resolved = await new Resolver(bridge, refStoreFrom(snap.nodes)).resolve(target as Target);
    return { present: true, observed: { ref: resolved.node?.ref, rect: resolved.rect, source: resolved.source } };
  } catch (e) {
    if (isAgentError(e) && e.code === "ELEMENT_NOT_FOUND") return { present: false, observed: { nodes: snap.nodes.length } };
    throw e;
  }
}

async function elementState(
  ctx: ToolCtx,
  target: TargetInput,
  attr: string | null,
): Promise<{ present: boolean; observed: unknown; attr: string | null }> {
  const selector = staticSelector(ctx, target);
  if (selector === null) {
    const r = await viaSnapshot(ctx, target);
    return { present: r.present, observed: r.observed, attr: null };
  }
  const probe = await pageProbe(ctx.session.requireBridge(), selector, attr);
  return { present: probe.present && probe.visible, observed: { selector, ...probe }, attr: probe.attr };
}

function textProbe(bridge: SessionBridge, needle: string): Promise<boolean> {
  return bridge.execute<boolean>(
    "wait.text",
    ((t: string) => (document.body?.innerText ?? "").includes(t)) as never,
    [needle],
    { readOnly: true },
  );
}

function elementProbes(ctx: ToolCtx, cond: Condition): Probe | null {
  if (cond.kind === "element" || cond.kind === "element_hidden") {
    const want = cond.kind === "element";
    return async () => {
      const s = await elementState(ctx, cond.target, null);
      return { done: s.present === want, observed: s.observed };
    };
  }
  if (cond.kind === "attribute") {
    return async () => {
      const s = await elementState(ctx, cond.target, cond.name);
      const done = s.present && s.attr !== null && (cond.value === undefined || s.attr === cond.value);
      return { done, observed: { [cond.name]: s.attr, ...(s.observed as object) } };
    };
  }
  if (cond.kind === "text" || cond.kind === "text_gone") {
    const want = cond.kind === "text";
    return async () => {
      const seen = await textProbe(ctx.session.requireBridge(), cond.text);
      return { done: seen === want, observed: { text: cond.text, present: seen } };
    };
  }
  return null;
}

function otherProbes(ctx: ToolCtx, cond: Condition, startedAt: number): Probe {
  switch (cond.kind) {
    case "revision_stable":
      return revisionProbe(ctx.session.requireBridge(), cond.quietMs ?? ctx.config.config.settle.quietMs);
    case "worker_idle":
      return workerIdleProbe(ctx.session.requireBridge());
    case "render_idle":
      return idleStableProbe(
        ctx.session.requireBridge(),
        cond.quietMs ?? ctx.config.config.settle.quietMs,
        (r) => (r.frames === null ? null : String(r.frames)),
        (r) => ({ frames: r.frames }),
      );
    case "camera_stable":
      return idleStableProbe(
        ctx.session.requireBridge(),
        cond.quietMs ?? ctx.config.config.settle.quietMs,
        (r) =>
          r.camera === null
            ? null
            : `${r.camera.x},${r.camera.y},${r.camera.z},${r.camera.tx},${r.camera.ty},${r.camera.tz},${r.camera.distance}`,
        (r) => ({ camera: r.camera }),
      );
    case "render_frame_after":
      return renderFrameAfterProbe(ctx.session.requireBridge(), cond.frames);
    case "snapshot_at_least":
      return snapshotProbe(ctx.session.requireBridge(), cond.id);
    case "log_line":
      return logProbe(ctx, cond);
    case "window":
      return windowProbe(ctx, cond);
    case "delay":
      return async () => ({ done: Date.now() - startedAt >= cond.ms, observed: { elapsedMs: Date.now() - startedAt } });
    default:
      throw new AgentError("INVALID_TARGET", `unsupported wait condition ${JSON.stringify(cond)}`);
  }
}

function revisionProbe(bridge: SessionBridge, quietMs: number): Probe {
  let last: number | undefined;
  let since = Date.now();
  return async () => {
    const { rev } = await readRevision(bridge);
    if (last === undefined || rev !== last) {
      last = rev;
      since = Date.now();
    }
    const quietFor = Date.now() - since;
    return { done: quietFor >= quietMs, observed: { revision: rev, quietForMs: quietFor } };
  };
}

/**
 * The CAD transaction, not the DOM: `regenBusy` counts in-flight regen jobs and
 * `geometryPending` is "document READY but a visible body still has no scene object".
 *
 * A null is UNKNOWN, and unknown is never reported as idle — unlike `settle`, which must
 * not block every action on a signal a build does not publish, an explicit `wait_for` was
 * asked for this specific fact and claiming it from an absent reading would be a lie. The
 * timeout's `lastObserved` then says precisely which signal was missing.
 */
function workerIdleProbe(bridge: SessionBridge): Probe {
  return async () => {
    const r = await readIdle(bridge);
    return {
      done: r.regenBusy === 0 && r.geometryPending === false,
      observed: {
        regenBusy: r.regenBusy,
        geometryPending: r.geometryPending,
        documentRevision: r.documentRevision,
      },
    };
  };
}

/**
 * Quiet window over one derived key of the idle reading — `frames` for `render_idle`,
 * the camera pose for `camera_stable`.
 *
 * Rendering is on-demand (`invalidate()` schedules exactly one rAF; an idle viewport
 * renders nothing), so "frameCount stopped moving" really is render-idle. A null key means
 * the signal is unavailable, which restarts the window rather than satisfying it.
 */
function idleStableProbe(
  bridge: SessionBridge,
  quietMs: number,
  key: (r: IdleReading) => string | null,
  observe: (r: IdleReading) => Record<string, unknown>,
): Probe {
  let last: string | null | undefined;
  let since = Date.now();
  return async () => {
    const r = await readIdle(bridge);
    const k = key(r);
    if (last === undefined || k !== last) {
      last = k;
      since = Date.now();
    }
    const quietFor = Date.now() - since;
    return {
      done: k !== null && quietFor >= quietMs,
      observed: { ...observe(r), quietForMs: quietFor, available: k !== null },
    };
  };
}

/** A frame COMMITTED after the given count — the positive counterpart of `render_idle`. */
function renderFrameAfterProbe(bridge: SessionBridge, after: number): Probe {
  return async () => {
    const r = await readIdle(bridge);
    return { done: r.frames !== null && r.frames > after, observed: { frames: r.frames, after } };
  };
}

/**
 * The backend's published snapshot id, via the feature-gated `agent_status` command.
 *
 * `snapshotId` is null when the document runtime is busy — it is read with `try_lock` so the
 * probe can never stall the regen it is waiting for — so null means UNKNOWN and the wait
 * keeps polling. It is never treated as "reached".
 */
function snapshotProbe(bridge: SessionBridge, atLeast: number): Probe {
  return async () => {
    const s = await bridge.invoke<{
      snapshotId: number | null;
      pendingRenderExpectations: number | null;
      pendingBodies: string[] | null;
    }>("agent_status", {}, { readOnly: true });
    const raw = s === null || typeof s !== "object" ? null : s.snapshotId;
    const id = typeof raw === "number" && Number.isFinite(raw) ? raw : null;
    return {
      done: typeof id === "number" && id >= atLeast,
      observed: {
        snapshotId: id,
        atLeast,
        runtimeBusy: id === null,
        pendingRenderExpectations: s?.pendingRenderExpectations ?? null,
        pendingBodies: s?.pendingBodies ?? null,
      },
    };
  };
}

function logProbe(ctx: ToolCtx, cond: Extract<Condition, { kind: "log_line" }>): Probe {
  const path = devJsonlPath({
    root: ctx.config.root,
    devJsonl: ctx.config.config.logs.devJsonl,
    journalDir: ctx.journal.dir,
    launched: ctx.session.status().launched,
  });
  let cursor: string | undefined;
  return async () => {
    const tail = await tailLogs(path, {
      ...(cursor === undefined ? {} : { since: cursor }),
      grep: cond.grep,
      ...(cond.level === undefined ? {} : { level: cond.level as LogLevel }),
      ...(cond.lane === undefined ? {} : { lane: cond.lane }),
      limit: 5,
    });
    cursor = tail.cursor;
    return {
      done: tail.matched > 0,
      observed: { path: tail.path, missing: tail.missing, matched: tail.matched, lines: tail.lines.map((l) => l.raw) },
    };
  };
}

function windowProbe(ctx: ToolCtx, cond: Extract<Condition, { kind: "window" }>): Probe {
  return async () => {
    const pid = ctx.session.status().pid;
    if (pid === undefined) return { done: false, observed: { pid: null } };
    const windows = await ctx.session.requirePlatform().windows.list(pid);
    const named = cond.name === undefined ? true : windows.some((w) => (w.name ?? "").includes(cond.name as string));
    const enough = cond.count === undefined ? windows.length > 0 : windows.length >= cond.count;
    return {
      done: named && enough,
      observed: { count: windows.length, names: windows.map((w) => w.name ?? null) },
    };
  };
}

export function registerWaitTools(server: McpServer): void {
  defineTool(server, {
    name: "wait_for",
    description:
      "Poll until a condition holds, or fail with ACTION_TIMEOUT reporting what was last observed. Conditions: element / element_hidden / attribute (by target), text / text_gone (visible page text), revision_stable (the DOM stopped mutating), worker_idle (no regen in flight and no body waiting for geometry), render_idle (the WebGL frame count stopped moving), render_frame_after (a frame committed past a given count), snapshot_at_least (the backend published a snapshot id), camera_stable (the viewport camera stopped moving), log_line (a regex over dev.jsonl), window (the app's on-screen windows), delay. Use this instead of guessing at sleeps around a regen.",
    input: z.object({
      condition: ConditionSchema,
      timeoutMs: z.number().int().positive().max(600_000).optional().describe("Default 10000."),
      inline: z.boolean().optional(),
    }),
    kind: "query",
    handler: async (args, ctx, actionId) => runWait(ctx, actionId, args.condition, args.timeoutMs ?? DEFAULT_TIMEOUT_MS),
  });
}

async function runWait(
  ctx: ToolCtx,
  actionId: string,
  cond: Condition,
  timeoutMs: number,
): Promise<ActionResult> {
  ctx.session.requireReady();
  const startedAt = Date.now();
  const probe = elementProbes(ctx, cond) ?? otherProbes(ctx, cond, startedAt);
  const deadline = startedAt + timeoutMs;
  let observed: unknown;
  for (;;) {
    const r = await probe();
    observed = r.observed;
    if (r.done) {
      return okResult(actionId, "real_user", {
        data: { kind: cond.kind, elapsedMs: Date.now() - startedAt, observed },
      });
    }
    if (Date.now() >= deadline) {
      const err = new AgentError("ACTION_TIMEOUT", `wait_for ${cond.kind} gave up after ${timeoutMs}ms`, {
        details: { condition: cond, lastObserved: observed },
      });
      return { ...errorResult(actionId, "real_user", err), data: { kind: cond.kind, elapsedMs: Date.now() - startedAt, observed } };
    }
    await sleep(POLL_MS);
  }
}
