/**
 * Keyboard verbs. Native key events go to the FRONTMOST application, so every one of these
 * runs the frontmost gate first — a keystroke sent to the wrong window is not merely a
 * failed test, it lands in whatever the user has open.
 *
 * Two app-specific refusals live here because they are the two that silently corrupt a run:
 * typing into something that cannot accept text (the characters become shortcuts), and the
 * cross-mode fallback where a bare model-tool letter finishes an open sketch.
 */
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { AgentError } from "../../errors.ts";
import type { SessionBridge } from "../../session/types.ts";
import { defineTool, type ToolCtx } from "../defineTool.ts";
import type { ActionResult } from "../envelope.ts";
import { okResult } from "../envelope.ts";
import { ModsSchema, type ModInput } from "../schemas.ts";
import type { ActionEnv } from "./actionPipeline.ts";
import { beginAction, finishAction, toModKeys } from "./actionPipeline.ts";
import { dispatchText } from "./webviewInput.ts";

/** Model-mode tool letters: pressed inside a sketch they finish it and arm a model tool. */
const CROSS_MODE_KEYS = new Set(["e", "r", "f", "b", "k", "p", "c", "m", "t", "d", "s"]);
export const CROSS_MODE_WARNING =
  "cross-mode fallback: this key may finish the active sketch and arm a model tool";

const MOD_ALIASES: Record<string, ModInput> = {
  primary: "Primary",
  cmd: "Command",
  command: "Command",
  meta: "Command",
  super: "Command",
  ctrl: "Control",
  control: "Control",
  alt: "Option",
  opt: "Option",
  option: "Option",
  shift: "Shift",
  fn: "Fn",
};

export function parseCombo(combo: string): { key: string; mods: ModInput[] } {
  const parts = combo.split("+").map((p) => p.trim()).filter((p) => p.length > 0);
  const key = parts.pop();
  if (key === undefined) {
    throw new AgentError("INVALID_TARGET", `"${combo}" is not a key combination`, {
      remediation: 'Write it as modifiers then key, e.g. "Primary+Shift+S" or "Escape".',
    });
  }
  const mods = parts.map((p) => {
    const mod = MOD_ALIASES[p.toLowerCase()];
    if (mod === undefined) {
      throw new AgentError("INVALID_TARGET", `"${p}" is not a modifier`, {
        remediation: "Use Primary, Command, Control, Option, Shift or Fn (Cmd/Ctrl/Alt/Meta are accepted aliases).",
        details: { combo, unknown: p },
      });
    }
    return mod;
  });
  return { key, mods };
}

interface FocusInfo {
  editable: boolean;
  tag: string;
  type: string | null;
}

function activeElementInfo(bridge: SessionBridge): Promise<FocusInfo> {
  return bridge.execute<FocusInfo>(
    "keyboard.activeElement",
    (() => {
      const el = document.activeElement as HTMLElement | null;
      if (el === null) return { editable: false, tag: "", type: null };
      const tag = el.tagName.toLowerCase();
      const type = el.getAttribute("type");
      const textInput = tag === "input" && !["button", "checkbox", "radio", "submit", "reset", "file", "range", "color"].includes((type ?? "text").toLowerCase());
      return { editable: textInput || tag === "textarea" || el.isContentEditable, tag, type };
    }) as never,
    [],
    { readOnly: true },
  );
}

function sketchActive(bridge: SessionBridge): Promise<boolean> {
  return bridge.execute<boolean>(
    "keyboard.sketchActive",
    (() => document.querySelector('[data-testid="sketch-dof"]') !== null) as never,
    [],
    { readOnly: true },
  );
}

/** Every keyboard verb shares the pipeline; only the posted events differ. */
async function keyboardAction(
  ctx: ToolCtx,
  actionId: string,
  opts: {
    warnings?: string[];
    screenshot?: boolean;
    data?: unknown;
    input(env: ActionEnv): Promise<void>;
  },
): Promise<ActionResult> {
  const env = await beginAction(ctx);
  return finishAction(env, {
    actionId,
    mode: "real_user",
    backend: "cgevent",
    stateChanging: true,
    ...(opts.warnings === undefined ? {} : { warnings: opts.warnings }),
    ...(opts.screenshot === undefined ? {} : { screenshot: opts.screenshot }),
    ...(opts.data === undefined ? {} : { data: opts.data }),
    input: opts.input,
  });
}

function registerTypeText(server: McpServer): void {
  defineTool(server, {
    name: "keyboard_type_text",
    description:
      "Type text with the real keyboard into whatever is focused. Refuses when the focused element cannot accept text, because the app would then interpret every character as a shortcut — click the field first, or pass allowShortcuts:true if firing shortcuts is what you want.",
    input: z.object({
      text: z.string().min(1),
      perCharMs: z.number().int().nonnegative().max(1000).optional(),
      allowShortcuts: z.boolean().optional().describe("Type even when nothing editable is focused."),
      mode: z.enum(["real_user", "webview"]).optional(),
      screenshot: z.boolean().optional(),
      inline: z.boolean().optional(),
    }),
    kind: "action",
    handler: async (args, ctx, actionId) => {
      const env = await beginAction(ctx, { requireFrontmost: args.mode === "webview" ? false : undefined });
      const focus = await activeElementInfo(env.bridge);
      if (!focus.editable && args.allowShortcuts !== true) {
        throw new AgentError(
          "INVALID_TARGET",
          `the focused element (${focus.tag === "" ? "none" : `<${focus.tag}>`}) does not accept typed text`,
          {
            remediation:
              "Click the text field first (pointer_click), then type. Pass allowShortcuts:true only when you deliberately want the characters to act as keyboard shortcuts.",
            details: { ...focus },
          },
        );
      }
      return finishAction(env, {
        actionId,
        mode: args.mode === "webview" ? "webview" : "real_user",
        backend: args.mode === "webview" ? "webdriver" : "cgevent",
        stateChanging: true,
        ...(args.screenshot === undefined ? {} : { screenshot: args.screenshot }),
        data: { focus },
        input: async (e) => {
          if (args.mode === "webview") {
            await dispatchText(e.bridge, args.text);
            return;
          }
          const typeOpts = args.perCharMs === undefined ? undefined : { perCharMs: args.perCharMs };
          await e.platform.input.type(args.text, typeOpts);
        },
      });
    },
  });
}

function registerPress(server: McpServer): void {
  defineTool(server, {
    name: "keyboard_press",
    description:
      "Press and release one key with the real keyboard. Warns when a sketch is open and the key is a model-tool letter, because the app's cross-mode fallback would finish the sketch and arm that tool instead of doing nothing.",
    input: z.object({
      key: z.string().min(1).describe('"a".."z", "Enter", "Escape", "Tab", "Space", "ArrowUp", "F1"...'),
      mods: ModsSchema.optional(),
      repeat: z.number().int().positive().max(50).optional().describe("Press this many times (default 1)."),
      mode: z.enum(["real_user"]).optional(),
      screenshot: z.boolean().optional(),
      inline: z.boolean().optional(),
    }),
    kind: "action",
    handler: async (args, ctx, actionId) =>
      pressThrough(ctx, actionId, args.key, args.mods, args.repeat ?? 1, args.screenshot),
  });

  defineTool(server, {
    name: "keyboard_shortcut",
    description:
      'Press a key combination written as one string, e.g. "Primary+S" (Command+S on macOS), "Primary+Shift+Z", "Escape". Same warnings as keyboard_press.',
    input: z.object({
      combo: z.string().min(1),
      mode: z.enum(["real_user"]).optional(),
      screenshot: z.boolean().optional(),
      inline: z.boolean().optional(),
    }),
    kind: "action",
    handler: async (args, ctx, actionId) => {
      const parsed = parseCombo(args.combo);
      return pressThrough(ctx, actionId, parsed.key, parsed.mods, 1, args.screenshot);
    },
  });
}

async function pressThrough(
  ctx: ToolCtx,
  actionId: string,
  key: string,
  mods: ModInput[] | undefined,
  repeat: number,
  screenshot: boolean | undefined,
): Promise<ActionResult> {
  const env = await beginAction(ctx);
  const warnings: string[] = [];
  // Only a BARE letter takes the cross-mode path; the same letter under a modifier is a chord.
  if ((mods ?? []).length === 0 && CROSS_MODE_KEYS.has(key.toLowerCase()) && (await sketchActive(env.bridge))) {
    warnings.push(CROSS_MODE_WARNING);
  }
  const native = toModKeys(mods);
  return finishAction(env, {
    actionId,
    mode: "real_user",
    backend: "cgevent",
    stateChanging: true,
    warnings,
    ...(screenshot === undefined ? {} : { screenshot }),
    data: { key, mods: native, repeat },
    input: async (e) => {
      for (let i = 0; i < repeat; i += 1) await e.platform.input.press(key, native);
    },
  });
}

const HoldInput = z.object({
  key: z.string().min(1).describe("A key name, or a modifier (Command, Control, Option, Shift, Fn)."),
  mods: ModsSchema.optional(),
  inline: z.boolean().optional(),
});

function registerHoldRelease(server: McpServer): void {
  defineTool(server, {
    name: "keyboard_down",
    description:
      "Press and HOLD a key without releasing it — for chorded gestures such as holding Shift while dragging. Always pair it with keyboard_up or keyboard_release_all.",
    input: HoldInput,
    kind: "action",
    handler: async (args, ctx, actionId) =>
      keyboardAction(ctx, actionId, {
        data: { key: args.key },
        input: (e) => e.platform.input.keyDown(args.key, { mods: toModKeys(args.mods) }),
      }),
  });

  defineTool(server, {
    name: "keyboard_up",
    description: "Release a key held by keyboard_down.",
    input: HoldInput,
    kind: "action",
    handler: async (args, ctx, actionId) =>
      keyboardAction(ctx, actionId, {
        data: { key: args.key },
        input: (e) => e.platform.input.keyUp(args.key, { mods: toModKeys(args.mods) }),
      }),
  });

  defineTool(server, {
    name: "keyboard_release_all",
    description:
      "Release every key, modifier and mouse button the agent may still be holding. Never fails; call it if a run was interrupted mid-gesture and the machine feels stuck.",
    input: z.object({}),
    kind: "action",
    handler: async (_args, ctx, actionId) => {
      const warnings: string[] = [];
      try {
        await ctx.session.requirePlatform().input.releaseAll();
      } catch (e) {
        warnings.push(`nothing was released: ${e instanceof Error ? e.message : String(e)}`);
      }
      return okResult(actionId, "real_user", {
        backend: "cgevent",
        status: warnings.length > 0 ? "warning" : "ok",
        warnings,
      });
    },
  });
}

export function registerKeyboardTools(server: McpServer): void {
  registerTypeText(server);
  registerPress(server);
  registerHoldRelease(server);
}
