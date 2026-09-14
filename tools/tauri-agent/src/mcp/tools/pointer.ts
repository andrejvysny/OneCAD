/**
 * Pointer verbs. Every one of them posts real CGEvents at a global screen point, which is
 * the whole point of the harness: a click the window server delivered is evidence, a
 * dispatched DOM event is not.
 */
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { AgentError } from "../../errors.ts";
import { globalToCss } from "../../geometry/mapping.ts";
import type { Pt } from "../../geometry/types.ts";
import { SEGMENT_IDLE_MS } from "../../geometry/wheelClass.ts";
import type { MouseButton } from "../../platform/adapter.ts";
import { defineTool, type ToolCtx } from "../defineTool.ts";
import type { ActionResult } from "../envelope.ts";
import { ButtonSchema, ModsSchema, OffsetSchema, SpaceSchema, TargetSchema, type TargetInput } from "../schemas.ts";
import type { ActionEnv, Hit } from "./actionPipeline.ts";
import {
  MOVE_RECHECK_MS,
  beginAction,
  finishAction,
  pointHit,
  recheckHit,
  resolveTarget,
  sleep,
  targetBrief,
  toModKeys,
} from "./actionPipeline.ts";
import { dispatchPointer, selectorOf, webviewUnsupported } from "./webviewInput.ts";

const ModeSchema = z
  .enum(["real_user", "webview"])
  .optional()
  .describe('"real_user" (default) posts OS events; "webview" dispatches DOM events and is not user-grade evidence.');

const DEFAULT_MOVE_MS = 180;
const DEFAULT_DRAG_MS = 400;

function buttonIndex(button: MouseButton): number {
  return button === "left" ? 0 : button === "right" ? 2 : 1;
}

interface SingleSpec {
  target: TargetInput;
  offset?: Pt;
  stateChanging: boolean;
  screenshot?: boolean;
  forDrag?: boolean;
  input(env: ActionEnv, hit: Hit): Promise<void>;
}

/** resolve → gate → native input → settle, for the verbs that act on exactly one element. */
async function singleTarget(ctx: ToolCtx, actionId: string, spec: SingleSpec): Promise<ActionResult> {
  const env = await beginAction(ctx);
  const t0 = Date.now();
  const hit = await resolveTarget(env, spec.target, {
    ...(spec.offset === undefined ? {} : { offset: spec.offset }),
    ...(spec.forDrag === true ? { forDrag: true } : {}),
  });
  const brief = targetBrief(hit);
  return finishAction(env, {
    actionId,
    mode: "real_user",
    backend: "cgevent",
    stateChanging: spec.stateChanging,
    resolveMs: Date.now() - t0,
    ...(spec.screenshot === undefined ? {} : { screenshot: spec.screenshot }),
    ...(brief === undefined ? {} : { target: brief }),
    resolvedPoint: { global: hit.global, css: hit.css },
    input: (e) => spec.input(e, hit),
  });
}

/** The webview lane for move and click; refuses to pretend for anything else. */
async function webviewPointer(
  ctx: ToolCtx,
  actionId: string,
  kind: "move" | "click",
  args: { target: TargetInput; offset?: Pt; button?: MouseButton; mods?: z.infer<typeof ModsSchema>; screenshot?: boolean },
): Promise<ActionResult> {
  const env = await beginAction(ctx, { requireFrontmost: false });
  const t0 = Date.now();
  const hit = await resolveTarget(env, args.target, args.offset === undefined ? {} : { offset: args.offset });
  const brief = targetBrief(hit);
  return finishAction(env, {
    actionId,
    mode: "webview",
    backend: "webdriver",
    stateChanging: kind === "click",
    resolveMs: Date.now() - t0,
    ...(args.screenshot === undefined ? {} : { screenshot: args.screenshot }),
    ...(brief === undefined ? {} : { target: brief }),
    resolvedPoint: { global: hit.global, css: hit.css },
    input: async (e) => {
      const out = await dispatchPointer(e.bridge, {
        ref: hit.resolved.node?.ref ?? null,
        selector: selectorOf(args.target, hit.resolved.node?.css),
        point: hit.css,
        kind,
        button: buttonIndex(args.button ?? "left"),
        mods: args.mods ?? [],
      });
      if (!out.ok) {
        throw new AgentError("ELEMENT_NOT_FOUND", "no element received the dispatched events", {
          details: { css: hit.css },
        });
      }
    },
  });
}

function registerMove(server: McpServer): void {
  defineTool(server, {
    name: "pointer_move",
    description:
      "Move the real mouse cursor onto an element, without pressing anything. Produces the same hover state a user would see; the envelope records the global screen point the cursor reached.",
    input: z.object({
      target: TargetSchema,
      offset: OffsetSchema.optional().describe("CSS-pixel offset from the element centre."),
      durationMs: z.number().int().nonnegative().max(10_000).optional(),
      mode: ModeSchema,
      screenshot: z.boolean().optional(),
      inline: z.boolean().optional(),
    }),
    kind: "action",
    handler: async (args, ctx, actionId) => {
      if (args.mode === "webview") {
        return webviewPointer(ctx, actionId, "move", args as Parameters<typeof webviewPointer>[3]);
      }
      return singleTarget(ctx, actionId, {
        target: args.target,
        ...(args.offset === undefined ? {} : { offset: args.offset }),
        stateChanging: false,
        ...(args.screenshot === undefined ? {} : { screenshot: args.screenshot }),
        input: (e, hit) => e.platform.input.move(hit.global, { durationMs: args.durationMs ?? DEFAULT_MOVE_MS }),
      });
    },
  });
}

function registerHover(server: McpServer): void {
  defineTool(server, {
    name: "pointer_hover",
    description:
      "Move the real cursor onto an element and dwell there so hover-delayed UI (tooltips, menus, hover cards) has time to appear. Pair it with a screenshot to prove what the user would see.",
    input: z.object({
      target: TargetSchema,
      offset: OffsetSchema.optional(),
      durationMs: z.number().int().nonnegative().max(10_000).optional(),
      dwellMs: z.number().int().nonnegative().max(30_000).optional().describe("Time held still on the element; defaults to input.dwellMs (600ms)."),
      mode: ModeSchema,
      screenshot: z.boolean().optional().describe("true captures the window after the dwell — how tooltip evidence is produced."),
      inline: z.boolean().optional(),
    }),
    kind: "action",
    handler: async (args, ctx, actionId) => {
      if (args.mode === "webview") throw webviewUnsupported("pointer_hover");
      return singleTarget(ctx, actionId, {
        target: args.target,
        ...(args.offset === undefined ? {} : { offset: args.offset }),
        stateChanging: false,
        ...(args.screenshot === undefined ? {} : { screenshot: args.screenshot }),
        input: async (e, hit) => {
          await e.platform.input.move(hit.global, { durationMs: args.durationMs ?? DEFAULT_MOVE_MS });
          await sleep(args.dwellMs ?? e.cfg.input.dwellMs);
        },
      });
    },
  });
}

function registerClick(server: McpServer): void {
  defineTool(server, {
    name: "pointer_click",
    description:
      "Move the real cursor onto an element and click it with the OS mouse. Re-checks the hit test after the move, so a tooltip or popover that opened under the cursor is refused (ELEMENT_OCCLUDED) instead of being clicked by mistake.",
    input: z.object({
      target: TargetSchema,
      offset: OffsetSchema.optional(),
      button: ButtonSchema.optional().describe("Default \"left\"."),
      count: z.union([z.literal(1), z.literal(2), z.literal(3)]).optional().describe("1 single, 2 double, 3 triple click."),
      mods: ModsSchema.optional().describe("Held for the whole click; \"Primary\" is Command on macOS."),
      durationMs: z.number().int().nonnegative().max(10_000).optional(),
      mode: ModeSchema,
      screenshot: z.boolean().optional(),
      inline: z.boolean().optional(),
    }),
    kind: "action",
    handler: async (args, ctx, actionId) => {
      if (args.mode === "webview") {
        return webviewPointer(ctx, actionId, "click", args as Parameters<typeof webviewPointer>[3]);
      }
      const mods = toModKeys(args.mods);
      return singleTarget(ctx, actionId, {
        target: args.target,
        ...(args.offset === undefined ? {} : { offset: args.offset }),
        stateChanging: true,
        ...(args.screenshot === undefined ? {} : { screenshot: args.screenshot }),
        input: async (e, hit) => {
          await e.platform.input.move(hit.global, { durationMs: args.durationMs ?? DEFAULT_MOVE_MS, mods });
          await sleep(MOVE_RECHECK_MS);
          await recheckHit(e, hit, args.target);
          await e.platform.input.click(args.button ?? "left", hit.global, {
            count: args.count ?? 1,
            intervalMs: e.cfg.input.clickIntervalMs,
            mods,
          });
        },
      });
    },
  });
}

function registerDownUp(server: McpServer): void {
  const input = z.object({
    target: TargetSchema.optional().describe("Omit to act at the cursor's current position."),
    offset: OffsetSchema.optional(),
    button: ButtonSchema.optional(),
    mods: ModsSchema.optional(),
    mode: ModeSchema,
    inline: z.boolean().optional(),
  });
  const half = (name: "pointer_down" | "pointer_up", description: string): void => {
    defineTool(server, {
      name,
      description,
      input,
      kind: "action",
      handler: async (args, ctx, actionId) => {
        if (args.mode === "webview") throw webviewUnsupported(name);
        const button = args.button ?? "left";
        const mods = toModKeys(args.mods);
        const act = async (e: ActionEnv, hit: Hit): Promise<void> => {
          const fn = name === "pointer_down" ? e.platform.input.down : e.platform.input.up;
          await fn.call(e.platform.input, button, hit.global, { mods });
        };
        if (args.target !== undefined) {
          return singleTarget(ctx, actionId, {
            target: args.target,
            ...(args.offset === undefined ? {} : { offset: args.offset }),
            ...(name === "pointer_down" ? { forDrag: true } : {}),
            stateChanging: true,
            input: act,
          });
        }
        return atCursor(ctx, actionId, act);
      },
    });
  };
  half(
    "pointer_down",
    "Press and HOLD a mouse button at an element (or at the cursor's current position). Use it with pointer_move and pointer_up to build a gesture the drag verbs do not cover; a held button is released automatically if anything later fails.",
  );
  half(
    "pointer_up",
    "Release a held mouse button at an element (or at the cursor's current position). Pairs with pointer_down.",
  );
}

/** For the verbs that may act wherever the cursor already is. */
async function atCursor(
  ctx: ToolCtx,
  actionId: string,
  act: (env: ActionEnv, hit: Hit) => Promise<void>,
): Promise<ActionResult> {
  const env = await beginAction(ctx);
  const t0 = Date.now();
  const hit = pointHit(env, globalToCss(await env.platform.input.cursor(), env.geom));
  return finishAction(env, {
    actionId,
    mode: "real_user",
    backend: "cgevent",
    stateChanging: true,
    resolveMs: Date.now() - t0,
    resolvedPoint: { global: hit.global, css: hit.css },
    input: (e) => act(e, hit),
  });
}

export function registerPointerTools(server: McpServer): void {
  registerMove(server);
  registerHover(server);
  registerClick(server);
  registerDownUp(server);
  registerDrag(server);
  registerDragPath(server);
  registerScroll(server);
}

/** Straight-line interpolation; the adapter eases between whatever points it is given. */
function interpolate(from: Pt, to: Pt, steps: number | undefined): Pt[] {
  const n = Math.max(1, Math.min(Math.floor(steps ?? 1), 200));
  if (n <= 1) return [from, to];
  const out: Pt[] = [from];
  for (let i = 1; i <= n; i += 1) {
    out.push({ x: from.x + ((to.x - from.x) * i) / n, y: from.y + ((to.y - from.y) * i) / n });
  }
  return out;
}

function registerDrag(server: McpServer): void {
  defineTool(server, {
    name: "pointer_drag",
    description:
      "Press at one element, move to another with the button held, and release — a real OS drag. Refuses to start on a window drag region, where the press would move the whole window instead. This is how the 3D viewport is orbited (right button + Shift) and panned (middle button).",
    input: z.object({
      from: TargetSchema,
      to: TargetSchema,
      button: ButtonSchema.optional(),
      mods: ModsSchema.optional().describe('Held for the whole gesture; ["Shift"] with the right button orbits the viewport.'),
      holdMs: z.number().int().nonnegative().max(10_000).optional().describe("Pause after the press, before moving."),
      durationMs: z.number().int().nonnegative().max(30_000).optional(),
      steps: z.number().int().positive().max(200).optional().describe("Intermediate points between from and to."),
      dwellMs: z.number().int().nonnegative().max(10_000).optional().describe("Pause at the end, before the release."),
      mode: ModeSchema,
      screenshot: z.boolean().optional(),
      inline: z.boolean().optional(),
    }),
    kind: "action",
    handler: async (args, ctx, actionId) => {
      if (args.mode === "webview") throw webviewUnsupported("pointer_drag");
      const env = await beginAction(ctx);
      const t0 = Date.now();
      const from = await resolveTarget(env, args.from, { forDrag: true });
      const to = await resolveTarget(env, args.to);
      const brief = targetBrief(from);
      return finishAction(env, {
        actionId,
        mode: "real_user",
        backend: "cgevent",
        stateChanging: true,
        resolveMs: Date.now() - t0,
        ...(args.screenshot === undefined ? {} : { screenshot: args.screenshot }),
        ...(brief === undefined ? {} : { target: brief }),
        resolvedPoint: { global: from.global, css: from.css },
        data: { from: from.global, to: to.global },
        input: (e) =>
          e.platform.input.path(args.button ?? "left", interpolate(from.global, to.global, args.steps), {
            durationMs: args.durationMs ?? DEFAULT_DRAG_MS,
            holdMs: args.holdMs ?? 0,
            dwellMs: args.dwellMs ?? 0,
            mods: toModKeys(args.mods),
          }),
      });
    },
  });
}

const DragPathInput = z.object({
  points: z.array(z.object({ x: z.number(), y: z.number() })).min(2).max(200),
  space: SpaceSchema.describe('"webview" = CSS pixels in the page, "window" = window-relative points, "global" = screen points.'),
  button: ButtonSchema.optional(),
  mods: ModsSchema.optional(),
  durationMs: z.number().int().nonnegative().max(30_000).optional(),
  holdMs: z.number().int().nonnegative().max(10_000).optional(),
  dwellMs: z.number().int().nonnegative().max(10_000).optional(),
  mode: ModeSchema,
  screenshot: z.boolean().optional(),
  inline: z.boolean().optional(),
});

function registerDragPath(server: McpServer): void {
  defineTool(server, {
    name: "pointer_drag_path",
    description:
      "Press, follow an explicit list of points with the button held, then release. Every point is mapped to the screen and gated the same way a click target is, so a path that leaves the window is refused rather than dragged across another application.",
    input: DragPathInput,
    kind: "action",
    handler: async (args, ctx, actionId) => {
      if (args.mode === "webview") throw webviewUnsupported("pointer_drag_path");
      const env = await beginAction(ctx);
      const t0 = Date.now();
      // "window" and "webview" coincide: the Overlay title bar makes content span the frame.
      const hits = args.points.map((p) =>
        pointHit(env, args.space === "global" ? globalToCss(p, env.geom) : { x: p.x, y: p.y }),
      );
      const first = hits[0] as Hit;
      return finishAction(env, {
        actionId,
        mode: "real_user",
        backend: "cgevent",
        stateChanging: true,
        resolveMs: Date.now() - t0,
        ...(args.screenshot === undefined ? {} : { screenshot: args.screenshot }),
        resolvedPoint: { global: first.global, css: first.css },
        data: { points: hits.map((h) => h.global), space: args.space },
        input: (e) =>
          e.platform.input.path(
            args.button ?? "left",
            hits.map((h) => h.global),
            {
              durationMs: args.durationMs ?? DEFAULT_DRAG_MS,
              holdMs: args.holdMs ?? 0,
              dwellMs: args.dwellMs ?? 0,
              mods: toModKeys(args.mods),
            },
          ),
      });
    },
  });
}

function registerScroll(server: McpServer): void {
  defineTool(server, {
    name: "pointer_scroll",
    description:
      "Turn the real mouse wheel over an element (or wherever the cursor is). Notches are spaced far enough apart that the app scores them as a wheel rather than a trackpad — the difference between zooming and panning the 3D viewport. Runs the wheel probe first and warns when the app would not classify the notch as a mouse.",
    input: z.object({
      target: TargetSchema.optional().describe("Omit to scroll at the cursor's current position."),
      offset: OffsetSchema.optional(),
      dy: z.number().describe("Wheel notches (positive = wheel up / zoom in) unless notches:false, where it is raw line units."),
      dx: z.number().optional(),
      notches: z.boolean().optional().describe("Default true: dy/dx count wheel detents, each worth the calibrated lines-per-notch."),
      mods: ModsSchema.optional(),
      mode: ModeSchema,
      screenshot: z.boolean().optional(),
      inline: z.boolean().optional(),
    }),
    kind: "action",
    handler: async (args, ctx, actionId) => {
      if (args.mode === "webview") throw webviewUnsupported("pointer_scroll");
      const env = await beginAction(ctx);
      const warnings: string[] = [];
      const probe = await env.session.ensureWheelProbe();
      if ("ok" in probe && !probe.ok) {
        warnings.push(
          "the app did not score a synthetic notch as a mouse wheel; scrolling may pan instead of zoom (force it in Settings -> Navigation -> Mouse)",
        );
      }
      const t0 = Date.now();
      const hit =
        args.target === undefined
          ? pointHit(env, globalToCss(await env.platform.input.cursor(), env.geom))
          : await resolveTarget(env, args.target, args.offset === undefined ? {} : { offset: args.offset });
      const brief = targetBrief(hit);
      return finishAction(env, {
        actionId,
        mode: "real_user",
        backend: "cgevent",
        stateChanging: true,
        resolveMs: Date.now() - t0,
        warnings,
        ...(args.screenshot === undefined ? {} : { screenshot: args.screenshot }),
        ...(brief === undefined ? {} : { target: brief }),
        resolvedPoint: { global: hit.global, css: hit.css },
        data: { wheelProbe: probe, linesPerNotch: env.session.wheelLinesPerNotch },
        input: (e) => scrollNotches(e, hit.global, args, e.session.wheelLinesPerNotch),
      });
    },
  });
}

interface ScrollArgs {
  dy: number;
  dx?: number;
  notches?: boolean;
  mods?: z.infer<typeof ModsSchema>;
}

async function scrollNotches(env: ActionEnv, p: Pt, args: ScrollArgs, linesPerNotch: number): Promise<void> {
  const mods = toModKeys(args.mods);
  if (args.notches === false) {
    await env.platform.input.scroll(p, { dy: args.dy, dx: args.dx ?? 0 }, { mods });
    return;
  }
  const count = Math.max(1, Math.round(Math.max(Math.abs(args.dy), Math.abs(args.dx ?? 0))));
  const dy = Math.sign(args.dy) * linesPerNotch;
  const dx = Math.sign(args.dx ?? 0) * linesPerNotch;
  for (let i = 0; i < count; i += 1) {
    // navInput closes a wheel segment after SEGMENT_IDLE_MS; notches closer than that are
    // weighed as one trackpad-looking burst and pan instead of zooming.
    await sleep(SEGMENT_IDLE_MS);
    await env.platform.input.scroll(p, { dy, dx }, { mods });
  }
}
