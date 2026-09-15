/**
 * Keyboard verbs. Native key events go to the FRONTMOST application, so every one of these
 * runs the frontmost gate first — a keystroke sent to the wrong window is not merely a
 * failed test, it lands in whatever the user has open.
 *
 * Two app-specific refusals live here because they are the two that silently corrupt a run:
 * typing into something that cannot accept text (the characters become shortcuts), and the
 * cross-mode fallback where a bare model-tool letter finishes an open sketch.
 *
 * Both of those read the PAGE, and both are wrong once a native panel is in front — the keys are
 * then going somewhere `document.activeElement` knows nothing about. `surface:"native"` is the
 * caller saying so: it skips those two page-derived checks and settles on the accessibility tree
 * instead of the DOM. It changes nothing about the keystroke itself, which is a real CGEvent
 * either way.
 */
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { AgentError } from "../../errors.ts";
import type { SessionBridge } from "../../session/types.ts";
import { defineTool, type ToolCtx } from "../defineTool.ts";
import type { ActionResult, DeliveryPhase } from "../envelope.ts";
import { deliveryFor, okResult } from "../envelope.ts";
import { ModsSchema, type ModInput } from "../schemas.ts";
import type { ActionEnv } from "./actionPipeline.ts";
import { beginAction, finishAction, toModKeys } from "./actionPipeline.ts";
import { dispatchKey, dispatchText, laneFor } from "./webviewInput.ts";

/** Model-mode tool letters: pressed inside a sketch they finish it and arm a model tool. */
const CROSS_MODE_KEYS = new Set(["e", "r", "f", "b", "k", "p", "c", "m", "t", "d", "s"]);
export const CROSS_MODE_WARNING =
  "cross-mode fallback: this key may finish the active sketch and arm a model tool";

/**
 * Which surface the keystroke is going to.
 *
 * Keyboard verbs have no target — native key events go wherever focus is — so this is the one
 * thing the caller knows and the harness cannot infer without walking the accessibility tree
 * before every keystroke. It changes two things and nothing else: the WebView focus checks are
 * skipped (with a native Save panel up, `document.activeElement` describes a page that is not
 * receiving the keys, so those checks refuse or warn about the wrong thing), and the settle
 * watches the accessibility subtree instead of the DOM idle tuple, which a native panel never
 * moves.
 */
const SurfaceSchema = z
  .enum(["webview", "native"])
  .optional()
  .describe(
    'Default "webview". Pass "native" when the keys are going to something the WebView does not own — a Save/Open panel, a sheet, a native menu, a permission dialog. Use native_modal or native_focused_window to check which is in front.',
  );

const ModeSchema = z
  .enum(["real_user", "webview"])
  .optional()
  .describe(
    '"real_user" (default in a foreground session) presses the physical key. "webview" dispatches a KeyboardEvent in the page: it reaches the app\'s own keydown handlers but NOT the macOS menu accelerator. An interaction:"background" session defaults to "webview" and refuses an explicit "real_user".',
  );

export const WEBVIEW_KEY_WARNING =
  'dispatched in the page: this exercised the app\'s JavaScript keydown lane, NOT the macOS menu accelerator. A real chord is claimed by the NSMenu item first, so a passing result here does not prove the menu binding works. Use native_menu_invoke to drive the menu item itself, or interaction:"foreground" for a real keystroke.';

/** `laneFor` with the session's policy already supplied. */
function lane(ctx: ToolCtx, requested: "real_user" | "webview" | undefined, tool: string): "real_user" | "webview" {
  return laneFor(ctx.session.interactionPolicy, requested, tool);
}

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

/** The webview lane dispatches into the page; a native surface is by definition not the page. */
function nativeNeedsRealUser(tool: string): AgentError {
  return new AgentError("INVALID_TARGET", `${tool} cannot reach a native surface in webview mode`, {
    remediation:
      'A dispatched DOM event never leaves the page, so it cannot type into a native panel. Drop `mode:"webview"` and keep surface:"native" — the real_user lane posts OS key events, which go wherever focus actually is.',
    details: { tool, mode: "webview", surface: "native" },
  });
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
    surface?: "webview" | "native";
    data?: unknown;
    /** The in-page equivalent; absent means this verb has no webview lane. */
    inPageInput?: (env: ActionEnv) => Promise<void>;
    tool?: string;
    mode?: "real_user" | "webview";
    input(env: ActionEnv): Promise<void>;
  },
): Promise<ActionResult> {
  const inPage =
    opts.inPageInput !== undefined && lane(ctx, opts.mode, opts.tool ?? "keyboard") === "webview";
  if (inPage && opts.surface === "native") throw nativeNeedsRealUser(opts.tool ?? "keyboard");
  const env = await beginAction(ctx, inPage ? { requireFrontmost: false } : {});
  const result = await finishAction(env, {
    actionId,
    mode: inPage ? "webview" : "real_user",
    backend: inPage ? "webdriver" : "cgevent",
    surface: opts.surface ?? "webview",
    stateChanging: true,
    ...(inPage ? { warnings: [...(opts.warnings ?? []), WEBVIEW_KEY_WARNING] } : {}),
    ...(opts.warnings === undefined ? {} : { warnings: opts.warnings }),
    ...(opts.screenshot === undefined ? {} : { screenshot: opts.screenshot }),
    ...(opts.data === undefined ? {} : { data: opts.data }),
    input: inPage && opts.inPageInput !== undefined ? opts.inPageInput : opts.input,
  });
  // keyboardAction backs only keyboard_down/keyboard_up: both physically press/release a key.
  return { ...result, fidelity: "physical_key" };
}

function registerTypeText(server: McpServer): void {
  defineTool(server, {
    name: "keyboard_type_text",
    description:
      "Type text with the real keyboard into whatever is focused. Refuses when the focused element cannot accept text, because the app would then interpret every character as a shortcut — click the field first, or pass allowShortcuts:true if firing shortcuts is what you want. Pass surface:\"native\" to type into a native panel (the filename field of a Save panel, say): the WebView focus check does not apply there and would refuse the very thing you want.",
    input: z.object({
      text: z.string().min(1),
      perCharMs: z.number().int().nonnegative().max(1000).optional(),
      allowShortcuts: z.boolean().optional().describe("Type even when nothing editable is focused."),
      surface: SurfaceSchema,
      mode: z.enum(["real_user", "webview"]).optional(),
      screenshot: z.boolean().optional(),
      inline: z.boolean().optional(),
    }),
    kind: "action",
    handler: async (args, ctx, actionId) => {
      const inPage = lane(ctx, args.mode, "keyboard_type_text") === "webview";
      if (args.surface === "native" && inPage) throw nativeNeedsRealUser("keyboard_type_text");
      const env = await beginAction(ctx, inPage ? { requireFrontmost: false } : {});
      // The focus check reads `document.activeElement`, which describes the PAGE. With a native
      // panel in front the page is not receiving these keys at all, so the check would refuse a
      // perfectly good keystroke on evidence about the wrong surface.
      const focus = args.surface === "native" ? null : await activeElementInfo(env.bridge);
      if (focus !== null && !focus.editable && args.allowShortcuts !== true) {
        throw new AgentError(
          "INVALID_TARGET",
          `the focused element (${focus.tag === "" ? "none" : `<${focus.tag}>`}) does not accept typed text`,
          {
            remediation:
              "Click the text field first (pointer_click), then type. Pass allowShortcuts:true only when you deliberately want the characters to act as keyboard shortcuts. If a native panel or sheet is in front, pass surface:\"native\" — this check reads the WebView's focus, which that panel does not have.",
            details: { ...focus },
          },
        );
      }
      const result = await finishAction(env, {
        actionId,
        mode: inPage ? "webview" : "real_user",
        backend: inPage ? "webdriver" : "cgevent",
        surface: args.surface ?? "webview",
        stateChanging: true,
        ...(args.screenshot === undefined ? {} : { screenshot: args.screenshot }),
        data: focus === null ? { surface: "native" } : { focus },
        input: async (e) => {
          if (inPage) {
            await dispatchText(e.bridge, args.text);
            return;
          }
          const typeOpts = args.perCharMs === undefined ? undefined : { perCharMs: args.perCharMs };
          await e.platform.input.type(args.text, typeOpts);
        },
      });
      // Text entry posts a synthesized Unicode string, not per-character physical key events:
      // a `code`-sensitive handler on the other end sees the wrong key.
      return { ...result, fidelity: "text_entry" };
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
      surface: SurfaceSchema,
      mode: ModeSchema,
      screenshot: z.boolean().optional(),
      inline: z.boolean().optional(),
    }),
    kind: "action",
    handler: async (args, ctx, actionId) =>
      pressThrough(ctx, actionId, args.key, args.mods, args.repeat ?? 1, args.screenshot, args.surface, args.mode),
  });

  defineTool(server, {
    name: "keyboard_shortcut",
    description:
      'Press a key combination written as one string, e.g. "Primary+S" (Command+S on macOS), "Primary+Shift+Z", "Escape". Same warnings as keyboard_press. In OneCAD "Primary+S" opens a NATIVE Save panel, which the WebView cannot see: use native_modal / native_snapshot to read it, and surface:"native" for the keys you then send to it.',
    input: z.object({
      combo: z.string().min(1),
      surface: SurfaceSchema,
      mode: ModeSchema,
      screenshot: z.boolean().optional(),
      inline: z.boolean().optional(),
    }),
    kind: "action",
    handler: async (args, ctx, actionId) => {
      const parsed = parseCombo(args.combo);
      return pressThrough(ctx, actionId, parsed.key, parsed.mods, 1, args.screenshot, args.surface, args.mode);
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
  surface: "webview" | "native" | undefined,
  mode?: "real_user" | "webview",
): Promise<ActionResult> {
  const tool = repeat === 1 && mods !== undefined ? "keyboard_shortcut" : "keyboard_press";
  const inPage = lane(ctx, mode, tool) === "webview";
  // A native panel is not the page: there is nothing in the DOM to dispatch to, and a result
  // saying the keys arrived would be about a surface that never received them.
  if (inPage && surface === "native") throw nativeNeedsRealUser(tool);
  const env = await beginAction(ctx, inPage ? { requireFrontmost: false } : {});
  const warnings: string[] = inPage ? [WEBVIEW_KEY_WARNING] : [];
  // Only a BARE letter takes the cross-mode path; the same letter under a modifier is a chord.
  // A key going to a native panel never reaches the sketch tool at all, so the warning would be
  // about a surface this keystroke is not touching.
  if (
    surface !== "native" &&
    (mods ?? []).length === 0 &&
    CROSS_MODE_KEYS.has(key.toLowerCase()) &&
    (await sketchActive(env.bridge))
  ) {
    warnings.push(CROSS_MODE_WARNING);
  }
  const native = toModKeys(mods);
  const result = await finishAction(env, {
    actionId,
    mode: inPage ? "webview" : "real_user",
    backend: inPage ? "webdriver" : "cgevent",
    surface: surface ?? "webview",
    stateChanging: true,
    warnings,
    ...(screenshot === undefined ? {} : { screenshot }),
    data: { key, mods: native, repeat },
    input: async (e) => {
      if (inPage) {
        await dispatchKey(e.bridge, { key, mods: mods ?? [], phase: "press", repeat });
        return;
      }
      for (let i = 0; i < repeat; i += 1) await e.platform.input.press(key, native);
    },
  });
  // pressThrough backs both keyboard_press and keyboard_shortcut. The native lane physically
  // presses and releases; the page lane synthesizes a keydown/keyup pair with the same `code`,
  // which is a physical-key SHAPE without being a physical key — `mode` is what separates them.
  return { ...result, fidelity: "physical_key" };
}

const HoldInput = z.object({
  key: z.string().min(1).describe("A key name, or a modifier (Command, Control, Option, Shift, Fn)."),
  mods: ModsSchema.optional(),
  surface: SurfaceSchema,
  mode: ModeSchema,
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
        tool: "keyboard_down",
        ...(args.mode === undefined ? {} : { mode: args.mode }),
        ...(args.surface === undefined ? {} : { surface: args.surface }),
        input: (e) => e.platform.input.keyDown(args.key, { mods: toModKeys(args.mods) }),
        inPageInput: (e) => dispatchKey(e.bridge, { key: args.key, mods: args.mods ?? [], phase: "down", repeat: 1 }).then(() => undefined),
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
        tool: "keyboard_up",
        ...(args.mode === undefined ? {} : { mode: args.mode }),
        ...(args.surface === undefined ? {} : { surface: args.surface }),
        input: (e) => e.platform.input.keyUp(args.key, { mods: toModKeys(args.mods) }),
        inPageInput: (e) => dispatchKey(e.bridge, { key: args.key, mods: args.mods ?? [], phase: "up", repeat: 1 }).then(() => undefined),
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
      // Tracks what actually happened so the envelope's `delivery` is honest: this tool DOES
      // post input (a release), unlike a query, so it must not report `inputStarted: false`.
      let phase: DeliveryPhase = "not_started";
      // The single documented exception to a background session's "no OS event" promise, and it
      // is stated here rather than left for a reader to notice. `releaseAll` is passed through the
      // refusing seam on purpose — it releases only what the HELPER pressed, so it can only ever
      // hand a key back, never take one — but the helper process is shared across sessions, so an
      // earlier FOREGROUND session in this agent process can have left something latched, and
      // clearing it posts a real key-up.
      if (ctx.session.interactionPolicy === "background") {
        warnings.push(
          'this is the one verb an interaction:"background" session lets through to the OS: it posts a key-up ONLY for something the harness itself is still holding, and can never press anything',
        );
      }
      try {
        await ctx.session.requirePlatform().input.releaseAll();
        phase = "input_completed";
      } catch (e) {
        warnings.push(`nothing was released: ${e instanceof Error ? e.message : String(e)}`);
      }
      return okResult(actionId, "real_user", {
        backend: "cgevent",
        status: warnings.length > 0 ? "warning" : "ok",
        warnings,
        // No `surface`: this verb addresses neither one. It releases what the AGENT is holding,
        // targets nothing, and settles on nothing, so claiming either surface would be a lie.
        // Releasing is not itself a document mutation, so stateChanging:false — but the phase
        // still marks input as posted, which is what makes `inputStarted` true below.
        delivery: deliveryFor(phase, false),
      });
    },
  });
}

export function registerKeyboardTools(server: McpServer): void {
  registerTypeText(server);
  registerPress(server);
  registerHoldRelease(server);
}
