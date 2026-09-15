/**
 * Native discovery: what the WebView cannot see.
 *
 * `ui_snapshot` reads the DOM, so it is blind to everything the page does not own — and in this
 * app that includes the things a user most often has to get past. Save and Open really do go
 * through `NSSavePanel` / `NSOpenPanel`; the app menu, a native context menu, a sheet, a
 * permission dialog and the three title-bar buttons are all native too. To the WebView every one
 * of those is invisible, so a run that hits one reports "nothing happened" while a panel sits in
 * front of the window.
 *
 * Six of these tools read the accessibility tree, which sees all of it. AX LOCATES, and in a
 * foreground session CGEvent ACTS — a ref from a query goes to `pointer_click {axRef}`, which
 * clicks it with the real mouse exactly as a user would. That is still the strongest evidence
 * available and still the default.
 *
 * Three tools ACT: `native_press`, `native_set_value` and `native_menu_invoke`. They exist for
 * the interaction:"background" policy, which has no native input at all and would otherwise be
 * unable to get past a Save panel. They ask the control to perform itself, which proves the
 * app's accessibility plumbing works and NOT that a user could click the button — so they report
 * `mode:"accessibility"`, `backend:"ax"`, and they never close a real-user acceptance claim.
 *
 * Refs are generation-scoped, `@a<gen>e<n>`, and EVERY walk mints a new generation — `native_find`
 * as much as `native_snapshot`, and the native settle after an action walks too. A ref from an
 * older generation is refused (ELEMENT_STALE) rather than aliased onto whatever now occupies the
 * slot, so the rule is simply: snapshot, act on what it showed, snapshot again.
 *
 * Unlike the `ui_*` queries these do NOT calibrate first. AX coordinates are global display
 * points and owe nothing to the window geometry, and the case where a native panel is in front is
 * exactly the case where the WebView bridge is least trustworthy. Reading the native surface must
 * not depend on the surface it is there to work around.
 */
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { AgentError } from "../../errors.ts";
import type { AxMenuItem, AxNode, AxWindowInfo, NativeAccessibility, PlatformAdapter } from "../../platform/adapter.ts";
import { axWindowIdValue } from "../../platform/adapter.ts";
import { AxResolver, axRefStoreFrom } from "../../native/resolve.ts";
import { defineTool, type ToolCtx } from "../defineTool.ts";
import { deliveryFor, okResult } from "../envelope.ts";
import { AxRefSchema } from "../schemas.ts";
import type { PipelineSession } from "./actionPipeline.ts";

/** Rendered-text budget, the same order as the webview snapshot's. */
const MAX_CHARS = 20_000;
const DEFAULT_MAX_NODES = 400;

interface NativeEnv {
  session: PipelineSession;
  platform: PlatformAdapter;
  ax: NativeAccessibility;
  pid: number;
}

function beginNative(ctx: ToolCtx): NativeEnv {
  const session: PipelineSession = ctx.session;
  session.requireReady();
  const pid = session.status().pid;
  if (pid === undefined) {
    throw new AgentError("APP_NOT_RUNNING", "this session has no app process whose accessibility tree could be read", {
      remediation: "Run session_start (or session_status to see why the session has no pid) before reading the native UI.",
    });
  }
  const platform = session.requirePlatform();
  return { session, platform, ax: platform.ax, pid };
}

/** Every walk mints a generation, so the pool is REPLACED — merging would mix two generations. */
function adoptPool(session: PipelineSession, generation: number, nodes: AxNode[]): void {
  session.axRefs = axRefStoreFrom(generation, nodes);
}

function fmtBounds(n: { bounds: { x: number; y: number; width: number; height: number } | null }): string {
  const b = n.bounds;
  if (b === null) return "(no frame)";
  return `@${Math.round(b.x)},${Math.round(b.y)} ${Math.round(b.width)}x${Math.round(b.height)}`;
}

function axLine(n: AxNode): string {
  const indent = "  ".repeat(Math.min(n.depth, 10));
  const role = n.subrole === null ? String(n.role) : `${String(n.role)}/${n.subrole}`;
  const parts = [`${indent}${n.ref} ${role}`];
  if (n.title !== null && n.title !== "") parts.push(JSON.stringify(n.title));
  if (n.value !== null && n.value !== "") parts.push(`value=${JSON.stringify(String(n.value).slice(0, 80))}`);
  parts.push(fmtBounds(n));
  if (n.enabled === false) parts.push("disabled");
  if (n.focused === true) parts.push("focused");
  if (n.actions.length > 0) parts.push(`actions=${n.actions.join(",")}`);
  return parts.join(" ");
}

function renderNodes(nodes: AxNode[]): { text: string; rendered: number } {
  const lines: string[] = [];
  let used = 0;
  for (const n of nodes) {
    const line = axLine(n);
    if (used + line.length + 1 > MAX_CHARS) break;
    lines.push(line);
    used += line.length + 1;
  }
  const dropped = nodes.length - lines.length;
  if (dropped > 0) lines.push(`…(+${dropped} not shown; narrow with native_find or native_snapshot {root})`);
  return { text: lines.join("\n"), rendered: lines.length - (dropped > 0 ? 1 : 0) };
}

function windowBrief(w: AxWindowInfo | null): Record<string, unknown> | null {
  if (w === null) return null;
  return {
    role: w.role,
    subrole: w.subrole,
    title: w.title,
    bounds: w.bounds,
    main: w.main,
    modal: w.modal,
    focused: w.focused,
    // The correlated CGWindowID, or null when the helper could not tell — never a guess.
    windowId: axWindowIdValue(w.window),
    windowIdSource: w.window.source,
  };
}

function registerSnapshot(server: McpServer): void {
  defineTool(server, {
    name: "native_snapshot",
    description:
      "Accessibility tree of a NATIVE window of the app — everything ui_snapshot cannot see, because the WebView does not own it: a macOS Save/Open panel (OneCAD's Save and Open really do go through one), a sheet, a permission dialog, a native context menu, the title-bar buttons. One line per element with its ref (@a<gen>e<n>), AX role, title, value, screen rect and available actions. Feed a ref to pointer_click {axRef} to click it with the real mouse. Every walk invalidates the refs of the previous one, so re-snapshot after acting. Read-only: this never presses anything.",
    input: z.object({
      window: z.number().int().nonnegative().optional().describe("CGWindowID (from window_list or native_focused_window). Omit to walk the focused window, then the main one; ambiguity is refused rather than guessed."),
      maxNodes: z.number().int().positive().max(2000).optional().describe("Walk cap; default 400. An AX read costs about 0.26 ms per node."),
      root: AxRefSchema.optional().describe("Drill into this element of the CURRENT generation instead of walking the whole window."),
    }),
    kind: "query",
    handler: async (args, ctx, actionId) => {
      const env = beginNative(ctx);
      const walk = await env.ax.snapshot(env.pid, {
        maxNodes: args.maxNodes ?? DEFAULT_MAX_NODES,
        ...(args.window === undefined ? {} : { window: args.window }),
        ...(args.root === undefined ? {} : { root: args.root }),
      });
      adoptPool(env.session, walk.generation, walk.nodes);
      const { text, rendered } = renderNodes(walk.nodes);
      return okResult(actionId, "real_user", {
        backend: "none",
        surface: "native",
        data: {
          generation: walk.generation,
          windowSource: walk.windowSource,
          window: windowBrief(walk.window),
          count: walk.nodes.length,
          rendered,
          visited: walk.total,
          truncated: walk.truncated || rendered < walk.nodes.length,
          stopReason: walk.stopReason,
          text,
        },
      });
    },
  });
}

function registerFind(server: McpServer): void {
  defineTool(server, {
    name: "native_find",
    description:
      "Search the app's native accessibility tree for elements matching an AX role, title or value — the native counterpart of ui_find, for a Save/Open panel, a sheet, a menu or the title bar. Roles are exact (AXButton, AXTextField, AXStaticText); title and value are case-insensitive substrings. Like native_snapshot this is a walk, so it mints a new ref generation and invalidates earlier refs. Read-only.",
    input: z
      .object({
        role: z.string().min(1).optional().describe('Exact AX role, e.g. "AXButton".'),
        title: z.string().min(1).optional().describe("Case-insensitive substring of the element's title."),
        value: z.string().min(1).optional().describe("Case-insensitive substring of the element's value."),
      })
      .describe("At least one of role, title or value."),
    kind: "query",
    handler: async (args, ctx, actionId) => {
      const env = beginNative(ctx);
      if (args.role === undefined && args.title === undefined && args.value === undefined) {
        throw new AgentError("INVALID_TARGET", "a native_find needs at least one of role, title or value", {
          remediation: "Narrow the search, or take a native_snapshot to see what the window exposes.",
        });
      }
      const walk = await env.ax.find(env.pid, {
        ...(args.role === undefined ? {} : { role: args.role }),
        ...(args.title === undefined ? {} : { title: args.title }),
        ...(args.value === undefined ? {} : { value: args.value }),
      });
      adoptPool(env.session, walk.generation, walk.nodes);
      const { text, rendered } = renderNodes(walk.nodes);
      return okResult(actionId, "real_user", {
        backend: "none",
        surface: "native",
        data: {
          generation: walk.generation,
          count: walk.nodes.length,
          rendered,
          visited: walk.total,
          truncated: walk.truncated,
          stopReason: walk.stopReason,
          text,
        },
      });
    },
  });
}

function registerInspect(server: McpServer): void {
  defineTool(server, {
    name: "native_inspect",
    description:
      "Full detail for one native element held by ref: its AX role, title, value, enabled and focused state, its rect and centre in global screen points, which window of this app contains it, and whether that centre is a point this harness would actually click. Runs the same refusal ladder a click does (ELEMENT_STALE, ELEMENT_MOVING, POINT_OUTSIDE_WINDOW), so use it to check a target before acting on it. Read-only.",
    input: z.object({ axRef: AxRefSchema }),
    kind: "query",
    handler: async (args, ctx, actionId) => {
      const env = beginNative(ctx);
      const resolver = new AxResolver({
        ax: env.ax,
        windows: env.platform.windows,
        pid: env.pid,
        ...(env.session.axRefs === undefined ? {} : { refs: env.session.axRefs }),
      });
      const r = await resolver.resolve({ ref: args.axRef });
      const p = r.point;
      return okResult(actionId, "real_user", {
        backend: "none",
        surface: "native",
        target: {
          ref: p.ref,
          ...(p.role === null ? {} : { role: p.role }),
          ...(p.title === null ? {} : { name: p.title }),
        },
        data: {
          source: r.source,
          generation: r.generation,
          role: p.role,
          subrole: p.subrole,
          title: p.title,
          enabled: p.enabled,
          focused: p.focused,
          bounds: p.bounds,
          centre: p.center,
          display: p.display,
          frontmost: p.frontmost,
          windowId: axWindowIdValue(p.window),
          windowIdSource: p.window.source,
          windowTitle: p.windowTitle,
          // The point gate admitted this centre: it is inside a window this application owns, so
          // a click here would land on this element. NOT a claim that the element does anything —
          // read `enabled` for that; a disabled button is still a clickable point.
          gateAdmitsPoint: true,
          ...(r.node === undefined ? {} : { node: r.node }),
        },
      });
    },
  });
}

function registerFocusedWindow(server: McpServer): void {
  defineTool(server, {
    name: "native_focused_window",
    description:
      "Which window of the app currently has keyboard focus, as the accessibility API reports it: role, subrole, title, screen rect, whether it is the main window, whether it is modal, and its CGWindowID when that could be correlated. Use it to tell 'the app is busy' from 'a native panel took focus and the keys are going there'. Read-only.",
    input: z.object({}),
    kind: "query",
    handler: async (_args, ctx, actionId) => {
      const env = beginNative(ctx);
      const w = await env.ax.focusedWindow(env.pid);
      return okResult(actionId, "real_user", {
        backend: "none",
        surface: "native",
        data: { pid: env.pid, focusedWindow: windowBrief(w), hasFocusedWindow: w !== null },
      });
    },
  });
}

function registerModal(server: McpServer): void {
  defineTool(server, {
    name: "native_modal",
    description:
      "Is a native modal blocking the app — a Save/Open panel, a sheet, an alert, a permission dialog? Reports every blocker with its role, title, rect and why it counts as one. This is the first thing to check when a click or a keystroke seems to have done nothing: the WebView looks unchanged because the app is not listening to it. Read-only.",
    input: z.object({}),
    kind: "query",
    handler: async (_args, ctx, actionId) => {
      const env = beginNative(ctx);
      const m = await env.ax.modal(env.pid);
      return okResult(actionId, "real_user", {
        backend: "none",
        surface: "native",
        data: {
          pid: m.pid,
          modal: m.modal,
          blockers: m.blockers.map((b) => ({ ...windowBrief(b), reason: b.reason })),
        },
      });
    },
  });
}

/** The separator `ax_menu_press` itself uses, so a rendered path can be typed straight back. */
export const MENU_PATH_SEPARATOR = " > ";

function menuLine(item: AxMenuItem): string {
  // `item.path` ALREADY ends with this item's own title — the helper pushes onto `ancestors`
  // before it reads the path (`main.swift`, the ax_menu walk). Appending `item.title` again
  // produced "File > Save > Save", which is exactly the string this tool tells a caller to hand
  // to native_menu_invoke, and which that verb then cannot resolve.
  const path = item.path.filter((p) => p !== "").join(MENU_PATH_SEPARATOR);
  const parts = [path === "" ? "(untitled)" : path];
  const combo = menuCombo(item);
  if (combo !== null) parts.push(combo);
  if (item.enabled === false) parts.push("disabled");
  if (item.bounds !== null) parts.push(fmtBounds(item));
  return parts.join("  ");
}

/**
 * The item's key equivalent, written the way `keyboard_shortcut` takes it, so a caller can copy
 * it straight across instead of translating glyphs. A key with no character (an arrow, a function
 * key) reports its virtual key code rather than pretending to a name.
 */
function menuCombo(item: AxMenuItem): string | null {
  const key = item.key;
  if (key === null) return null;
  const named = key.char !== null && key.char !== "" ? key.char.toUpperCase() : key.virtualKey === null ? null : `vk${key.virtualKey}`;
  if (named === null) return null;
  return [...key.mods, named].join("+");
}

function registerMenuSnapshot(server: McpServer): void {
  defineTool(server, {
    name: "native_menu_snapshot",
    description:
      "The app's menu bar, flattened: every item with its full path (File › Export › STEP…), its keyboard shortcut written the way keyboard_shortcut takes it (e.g. Command+Shift+S), and whether it is enabled. The menu bar is native, so ui_snapshot cannot see a single item of it. Cheapest route to a menu command is usually its shortcut; clicking one needs the menu opened first. Read-only.",
    input: z.object({}),
    kind: "query",
    handler: async (_args, ctx, actionId) => {
      const env = beginNative(ctx);
      const m = await env.ax.menu(env.pid);
      const lines: string[] = [];
      let used = 0;
      for (const item of m.items) {
        const line = menuLine(item);
        if (used + line.length + 1 > MAX_CHARS) break;
        lines.push(line);
        used += line.length + 1;
      }
      const dropped = m.items.length - lines.length;
      if (dropped > 0) lines.push(`…(+${dropped} items not shown)`);
      return okResult(actionId, "real_user", {
        backend: "none",
        surface: "native",
        data: {
          pid: m.pid,
          hasMenuBar: m.hasMenuBar,
          count: m.items.length,
          visited: m.total,
          truncated: m.truncated || dropped > 0,
          stopReason: m.stopReason,
          text: lines.join("\n"),
        },
      });
    },
  });
}

/**
 * The three actuation tools.
 *
 * They deliberately do NOT run through `beginAction`/`finishAction`. That pipeline's first step
 * is the frontmost gate, and needing the window in front is precisely what these avoid; its
 * settle reads a DOM that a native panel never touches. What they do instead is what the other
 * native tools do — resolve against the live accessibility tree and let the helper refuse — plus
 * one honest envelope stating that input WAS delivered and the call is not retry-safe.
 */
function registerActuation(server: McpServer): void {
  defineTool(server, {
    name: "native_press",
    description:
      'Perform a native control\'s own action (AXPress by default) on an {axRef} from native_snapshot or native_find — the Save button of an NSSavePanel, a sheet button, a title-bar button. Works with the app in the BACKGROUND and moves nothing on screen, which is why it is the interaction:"background" way past native chrome. It is not a real click: it reports mode:"accessibility" and proves the accessibility plumbing, not that a user could reach the control. In a foreground session prefer pointer_click {axRef}, which uses the real mouse.',
    input: z.object({
      ref: AxRefSchema,
      action: z
        .string()
        .optional()
        .describe('Default "AXPress". Must be one the element advertises (native_inspect lists them); an unadvertised action is refused rather than silently doing nothing. A DISABLED control is refused too — a disabled element still advertises AXPress and still answers "success" while doing nothing.'),
      acceptDisruption: z
        .literal("yes")
        .optional()
        .describe('Required to press window chrome — the full-screen, zoom, minimise or close button. Those visibly change the user\'s desktop (a Space switch, a resize, a vanished window), so they are refused unless you mean it.'),
    }),
    kind: "action",
    handler: async (args, ctx, actionId) => {
      const env = beginNative(ctx);
      const out = await env.ax.press(args.ref, args.action, args.acceptDisruption);
      return okResult(actionId, "accessibility", {
        backend: "ax",
        surface: "native",
        target: { ref: args.ref },
        // The action was performed: the app may already have saved, closed or navigated.
        delivery: deliveryFor("input_completed", true),
        data: out,
      });
    },
  });

  defineTool(server, {
    name: "native_set_value",
    description:
      'Set the text value of a native control by {axRef} — the filename field of a Save panel, say. Refuses when the attribute is not settable instead of reporting a write the app discarded, and reports the value read BACK, which can differ when the app normalises it. Not typing: no key is pressed, so a field that reacts to keystrokes rather than to its value will not see it. mode:"accessibility".',
    input: z.object({ ref: AxRefSchema, value: z.string() }),
    kind: "action",
    handler: async (args, ctx, actionId) => {
      const env = beginNative(ctx);
      const out = await env.ax.setValue(args.ref, args.value);
      const warnings = out.matched
        ? []
        : [`the field reports ${JSON.stringify(out.value)} after the write, not ${JSON.stringify(args.value)} — the app normalised or rejected it`];
      return okResult(actionId, "accessibility", {
        backend: "ax",
        surface: "native",
        target: { ref: args.ref },
        status: warnings.length > 0 ? "warning" : "ok",
        warnings,
        delivery: deliveryFor("input_completed", true),
        data: out,
      });
    },
  });

  defineTool(server, {
    name: "native_menu_invoke",
    description:
      'Press an app menu item by its title path, e.g. {path:["File","Save"]} — read the paths from native_menu_snapshot. Works with the app in the background and needs no keyboard, which makes it the reliable way to trigger a menu command: it is immune to keyboard-layout differences, where a positional key map can send the wrong character for a chord like Cmd+Z. A path that matches no item, or more than one, is refused with the paths that do exist. mode:"accessibility" — it proves the menu command runs, not that the keyboard shortcut reaches it.',
    input: z.object({
      path: z
        .array(z.string().min(1))
        .min(2)
        .describe('Titles from the menu bar down, e.g. ["Edit", "Undo"]. Exact, including any trailing ellipsis character. At least two: a lone menu-bar title is not a command, and pressing it would open the menu on the user\'s screen.'),
    }),
    kind: "action",
    handler: async (args, ctx, actionId) => {
      const env = beginNative(ctx);
      const out = await env.ax.menuPress(env.pid, args.path);
      return okResult(actionId, "accessibility", {
        backend: "ax",
        surface: "native",
        delivery: deliveryFor("input_completed", true),
        data: out,
      });
    },
  });
}

export function registerNativeTools(server: McpServer): void {
  registerSnapshot(server);
  registerFind(server);
  registerInspect(server);
  registerFocusedWindow(server);
  registerModal(server);
  registerMenuSnapshot(server);
  registerActuation(server);
}
