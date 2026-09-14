/**
 * Phase-0 exit run (spec §37) through the REAL MCP boundary: spawn the server exactly as
 * Claude Code does (`bun src/mcp/server.ts`, cwd = repo root), speak JSON-RPC over stdio,
 * and drive the app with the public tools. Envelopes go to stderr; exit 1 on any error
 * envelope. This moves the real mouse and keyboard.
 */
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { resolve } from "node:path";

const root = resolve(import.meta.dir, "../../..");
const proc = spawn("bun", ["tools/tauri-agent/src/mcp/server.ts"], {
  cwd: root,
  stdio: ["pipe", "pipe", "pipe"],
  env: { ...process.env, TAURI_AGENT_LOG: process.env.TAURI_AGENT_LOG ?? "warn" },
});
proc.stderr.on("data", (d) => process.stderr.write(`[server] ${d}`));
const lines = createInterface({ input: proc.stdout });
const pending = new Map<number, (v: unknown) => void>();
lines.on("line", (line) => {
  const msg = JSON.parse(line) as { id?: number; result?: unknown; error?: unknown };
  if (msg.id !== undefined) pending.get(msg.id)?.(msg.error ?? msg.result);
});
let nextId = 1;
function rpc(method: string, params: unknown): Promise<unknown> {
  const id = nextId++;
  proc.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
  return new Promise((r) => pending.set(id, r));
}
interface Envelope { actionId: string; status: string; backend?: string; target?: unknown; resolvedPoint?: unknown; state?: unknown; warnings?: string[]; error?: unknown; screenshot?: { path: string }; data?: unknown }
let failed = false;
async function call(name: string, args: Record<string, unknown>): Promise<Envelope> {
  const res = (await rpc("tools/call", { name, arguments: args })) as { content: Array<{ type: string; text?: string }>; isError?: boolean };
  const text = res.content.find((c) => c.type === "text")?.text ?? "{}";
  let env: Envelope;
  try {
    env = JSON.parse(text) as Envelope;
  } catch {
    process.stderr.write(`NON-ENVELOPE ${name}: ${text.slice(0, 300)}\n`);
    failed = true;
    return { actionId: "?", status: "error" };
  }
  const brief = { tool: name, actionId: env.actionId, status: env.status, backend: env.backend, target: env.target, point: env.resolvedPoint, state: env.state, warnings: env.warnings, error: env.error, shot: env.screenshot?.path, image: res.content.some((c) => c.type === "image") };
  process.stderr.write(`${JSON.stringify(brief)}\n`);
  if (env.status === "error") failed = true;
  return env;
}

await rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "phase0", version: "0" } });
proc.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" })}\n`);
const tools = (await rpc("tools/list", {})) as { tools: Array<{ name: string }> };
process.stderr.write(`TOOLS ${tools.tools.length}: ${tools.tools.map((t) => t.name).join(" ")}\n`);

await call("session_start", { mode: "launch", launch: "dev" });
await call("ui_screenshot", { mode: "window", label: "start", inline: false });
const snap = await call("ui_snapshot", {});
process.stderr.write(`SNAPSHOT\n${String((snap.data as { text?: string })?.text ?? "").split("\n").slice(0, 25).join("\n")}\n`);
await call("pointer_click", { target: { role: "button", name: "New project" } });
await call("wait_for", { condition: { kind: "element", target: { testId: "viewport-canvas" } }, timeoutMs: 20000 });
await call("wait_for", { condition: { kind: "revision_stable" }, timeoutMs: 5000 });
await call("ui_snapshot", {});
await call("pointer_hover", { target: { role: "button", name: "New sketch" }, dwellMs: 700, screenshot: true });
await call("pointer_click", { target: { role: "button", name: "New sketch" } });
await call("wait_for", { condition: { kind: "text", text: "Select a sketch plane" }, timeoutMs: 5000 });
await call("keyboard_press", { key: "Escape" });
const vp = await call("ui_inspect", { target: { testId: "viewport-canvas" } });
const rect = (vp.data as { rect?: { x: number; y: number; width: number; height: number } | null })?.rect ?? undefined;
if (rect) {
  const cx = rect.x + rect.width / 2, cy = rect.y + rect.height / 2;
  await call("pointer_drag_path", { points: [{ x: cx, y: cy }, { x: cx + 120, y: cy + 40 }], space: "webview", button: "right", mods: ["Shift"], durationMs: 600 });
  await call("pointer_scroll", { target: { testId: "viewport-canvas" }, dy: -2 });
} else {
  process.stderr.write(`NO VIEWPORT RECT: ${JSON.stringify(vp.data)}\n`);
  failed = true;
}
await call("keyboard_shortcut", { combo: "Primary+S" });
await call("wait_for", { condition: { kind: "delay", ms: 1500 } });
await call("ui_screenshot", { mode: "window_context", label: "after-save-shortcut", inline: false });
await call("keyboard_press", { key: "Escape" });
await call("observe_logs", { grep: "save_document|CLOSE_REQUESTED|regen:", limit: 20 });
await call("observe_logs", { level: "error", limit: 20 });
await call("session_stop", {});
proc.stdin.end();
process.stderr.write(`PHASE0 ${failed ? "FAILED" : "OK"}\n`);
process.exit(failed ? 1 : 0);
