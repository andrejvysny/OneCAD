/**
 * Live acceptance for the acting tools: `bun tools/tauri-agent/scripts/smoke-actions.ts`.
 *
 * This MOVES THE REAL MOUSE AND KEYBOARD. It launches the app, clicks "New project",
 * hovers a toolbar tool, presses a key, turns the wheel over the viewport and orbits it
 * with a right+Shift drag, then stops and proves nothing survived.
 *
 * The tools are driven through their registered MCP handlers (not their internals), so what
 * this exercises is exactly what a client would call. Everything prints to stderr.
 */
import { randomBytes } from "node:crypto";
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import type { ToolCtx } from "../src/mcp/defineTool.ts";
import type { ActionResult } from "../src/mcp/envelope.ts";
import { registerAllTools } from "../src/mcp/tools/index.ts";
import { loadConfig, resolveFromRoot } from "../src/session/config.ts";
import { tailFile } from "../src/session/launch.ts";
import { SessionOrchestrator } from "../src/session/orchestrator.ts";
import { spawnRunner } from "../src/session/procs.ts";
import { Journal } from "../src/trace/journal.ts";

function say(label: string, value?: unknown): void {
  process.stderr.write(value === undefined ? `${label}\n` : `${label} ${JSON.stringify(value, null, 2)}\n`);
}

const resolved = loadConfig();
const sessionId = `smoke-actions-${new Date().toISOString().replace(/[:.]/g, "-")}-${randomBytes(2).toString("hex")}`;
const journal = new Journal(resolveFromRoot(resolved.root, `${resolved.config.artifacts.dir}/${sessionId}`));
const session = new SessionOrchestrator({ sessionId, journal, config: resolved });
const ctx: ToolCtx = { session, journal, config: resolved };

const tools = new Map<string, (args: unknown) => Promise<CallToolResult>>();
registerAllTools(
  {
    registerTool(name: string, _cfg: unknown, cb: (args: unknown) => Promise<CallToolResult>) {
      tools.set(name, cb);
    },
  } as unknown as McpServer,
  ctx,
);

async function callOnce(name: string, args: Record<string, unknown>): Promise<ActionResult> {
  const tool = tools.get(name);
  if (!tool) throw new Error(`tool not registered: ${name}`);
  const raw = await tool(args);
  return JSON.parse((raw.content[0] as { text: string }).text) as ActionResult;
}

/** One retry, then give up loudly — a step that needs three tries is a defect, not a flake. */
async function step(name: string, args: Record<string, unknown>): Promise<ActionResult> {
  let envelope = await callOnce(name, args);
  if (envelope.status === "error") {
    say(`RETRY ${name} after`, envelope.error);
    envelope = await callOnce(name, args);
  }
  say(`${name} =>`, envelope);
  if (envelope.status === "error") throw new Error(`${name} failed: ${envelope.error?.code}`);
  return envelope;
}

async function survivors(): Promise<string> {
  const r = await spawnRunner(["/usr/bin/pgrep", "-fl", resolved.config.app.processPatterns.join("|")]);
  return r.stdout.trim();
}

interface FindData {
  rect?: { x: number; y: number; width: number; height: number };
}

async function run(): Promise<void> {
  await step("session_start", { mode: "launch", launch: "dev", reuseExisting: true });
  await step("ui_snapshot", {});
  await step("pointer_click", { target: { role: "button", name: "New project" }, inline: false });
  await step("wait_for", {
    condition: { kind: "element", target: { testId: "viewport-canvas" } },
    timeoutMs: 60_000,
  });
  await step("wait_for", { condition: { kind: "revision_stable" }, timeoutMs: 30_000 });
  await step("ui_snapshot", {});
  await step("pointer_hover", { target: { role: "button", name: "Extrude" }, screenshot: true, inline: false });
  await step("keyboard_press", { key: "h", inline: false });

  const canvas = await step("ui_find", { target: { testId: "viewport-canvas" } });
  const rect = (canvas.data as FindData).rect;
  if (rect === undefined) throw new Error("viewport-canvas has no rect");
  const cx = rect.x + rect.width / 2;
  const cy = rect.y + rect.height / 2;

  const scroll = await step("pointer_scroll", { target: { testId: "viewport-canvas" }, dy: 2, inline: false });
  say("WHEEL PROBE", (scroll.data as { wheelProbe?: unknown }).wheelProbe);

  await step("pointer_drag_path", {
    points: [
      { x: cx - 60, y: cy },
      { x: cx, y: cy },
      { x: cx + 60, y: cy },
    ],
    space: "webview",
    button: "right",
    mods: ["Shift"],
    durationMs: 500,
    holdMs: 120,
    dwellMs: 120,
    inline: false,
  });
  await step("ui_screenshot", { mode: "window", label: "smoke-actions", inline: false });
  await step("observe_logs", { level: "ERROR", limit: 20 });
}

say(`smoke-actions: artifacts in ${journal.dir}`);
let failure: unknown;
try {
  await run();
} catch (e) {
  failure = e;
  say("FAILED", { error: String(e), details: (e as { details?: unknown }).details });
  say("LAUNCHER LOG TAIL", await tailFile(journal.artifactPath("launcher.log"), 40));
}

try {
  say("session_stop =>", await callOnce("session_stop", {}));
} catch (e) {
  failure ??= e;
  say("STOP FAILED", { error: String(e) });
}

const left = await survivors();
say("SURVIVORS", left);
if (left.length > 0) {
  say("the app must not be left running; these pids are still alive — inspect them, do not kill anything else");
  failure ??= new Error("survivors after session_stop");
}
process.exit(failure === undefined ? 0 : 1);
