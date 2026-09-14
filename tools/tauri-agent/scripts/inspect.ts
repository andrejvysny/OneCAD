/** Thorough manual inspection in one process: launch, drive a real modelling flow, save every
 *  envelope and screenshot. Mirrors how a real MCP session drives the app (no focus-fighting). */
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { resolve } from "node:path";
import { mkdirSync, copyFileSync, writeFileSync } from "node:fs";

const root = resolve(import.meta.dir, "../../..");
const outDir = resolve(root, ".tauri-agent/inspect");
mkdirSync(outDir, { recursive: true });
const proc = spawn("bun", ["tools/tauri-agent/src/mcp/server.ts"], {
  cwd: root, stdio: ["pipe", "pipe", "pipe"], env: { ...process.env, TAURI_AGENT_LOG: "info" },
});
proc.stderr.on("data", (d) => process.stderr.write(`[server] ${d}`));
const pending = new Map<number, (v: unknown) => void>();
createInterface({ input: proc.stdout }).on("line", (l) => {
  const m = JSON.parse(l) as { id?: number; result?: unknown; error?: unknown };
  if (m.id !== undefined) pending.get(m.id)?.(m.error ?? m.result);
});
let id = 1;
const rpc = (method: string, params: unknown): Promise<unknown> => {
  const n = id++;
  proc.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id: n, method, params })}\n`);
  return new Promise((r) => pending.set(n, r));
};
await rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "inspect", version: "0" } });
proc.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" })}\n`);

const manifest: unknown[] = [];
async function call(step: string, name: string, args: Record<string, unknown>): Promise<Record<string, unknown>> {
  const res = (await rpc("tools/call", { name, arguments: args })) as { content: Array<{ type: string; text?: string }> };
  const env = JSON.parse(res.content.find((c) => c.type === "text")?.text ?? "{}") as Record<string, unknown>;
  const shot = (env.screenshot as { path?: string } | undefined)?.path;
  let saved: string | undefined;
  if (shot) { saved = resolve(outDir, `${step}.png`); try { copyFileSync(shot, saved); } catch { saved = shot; } }
  const row = { step, tool: name, status: env.status, backend: env.backend, target: env.target, point: env.resolvedPoint, state: env.state, warnings: env.warnings, error: env.error, data: env.data, shot: saved };
  manifest.push(row);
  const brief = { step, tool: name, status: env.status, backend: env.backend, warn: (env.warnings as unknown[])?.length, err: (env.error as { code?: string } | undefined)?.code, shot: saved };
  process.stderr.write(`${JSON.stringify(brief)}\n`);
  return env;
}
function findText(env: Record<string, unknown>): string { return String((env.data as { text?: string } | undefined)?.text ?? ""); }

try {
  await call("01-start", "session_start", { mode: "launch", launch: "dev" });
  await call("02-start-shot", "ui_screenshot", { mode: "window", label: "start", inline: false });
  const s1 = await call("03-start-snap", "ui_snapshot", {});
  writeFileSync(resolve(outDir, "03-start-snap.txt"), findText(s1));
  await call("04-newproject", "pointer_click", { target: { role: "button", name: "New project" } });
  await call("05-wait-canvas", "wait_for", { condition: { kind: "element", target: { testId: "viewport-canvas" } }, timeoutMs: 20000 });
  await call("06-wait-stable", "wait_for", { condition: { kind: "revision_stable" }, timeoutMs: 6000 });
  await call("07-editor-shot", "ui_screenshot", { mode: "window", label: "editor", inline: false });
  const s2 = await call("08-editor-snap", "ui_snapshot", {});
  writeFileSync(resolve(outDir, "08-editor-snap.txt"), findText(s2));
  // Enter sketch on a base plane.
  await call("09-newsketch", "pointer_click", { target: { role: "button", name: "New sketch" } });
  await call("10-wait-planeprompt", "wait_for", { condition: { kind: "text", text: "Select a sketch plane" }, timeoutMs: 6000 });
  const vp = await call("11-vp-inspect", "ui_inspect", { target: { testId: "viewport-canvas" } });
  const rect = (vp.data as { rect?: { x: number; y: number; width: number; height: number } }).rect;
  const cx = rect ? rect.x + rect.width / 2 : 650, cy = rect ? rect.y + rect.height / 2 : 450;
  // Nudge the pointer so the plane picker highlights, then pick.
  await call("12-move-vp", "pointer_move", { target: { point: { x: cx, y: cy - 1, space: "webview" } } });
  await call("13-move-vp2", "pointer_move", { target: { point: { x: cx, y: cy, space: "webview" } } });
  await call("14-plane-shot", "ui_screenshot", { mode: "window", label: "planepick", inline: false });
  await call("15-plane-click", "pointer_click", { target: { point: { x: cx, y: cy, space: "webview" } } });
  await call("16-wait-editing", "wait_for", { condition: { kind: "text", text: "Editing" }, timeoutMs: 6000 });
  await call("17-editing-shot", "ui_screenshot", { mode: "window", label: "editing", inline: false });
  // Rectangle.
  await call("18-rect-tool", "pointer_click", { target: { role: "button", name: "Rectangle" } });
  await call("19-rect-c1", "pointer_click", { target: { point: { x: cx - 150, y: cy - 100, space: "webview" } } });
  await call("20-rect-c2", "pointer_click", { target: { point: { x: cx + 150, y: cy + 100, space: "webview" } } });
  await call("21-dof", "ui_find", { target: { testId: "sketch-dof" } });
  await call("22-rect-shot", "ui_screenshot", { mode: "window", label: "rectangle", inline: false });
  await call("23-finish", "keyboard_press", { key: "Enter" });
  await call("24-wait-stable", "wait_for", { condition: { kind: "revision_stable" }, timeoutMs: 6000 });
  await call("25-sketchdone-shot", "ui_screenshot", { mode: "window", label: "sketchdone", inline: false });
  // Extrude.
  await call("26-region-click", "pointer_click", { target: { point: { x: cx, y: cy, space: "webview" } } });
  await call("27-extrude-tool", "pointer_click", { target: { role: "button", name: "Extrude" } });
  await call("28-extrude-shot", "ui_screenshot", { mode: "window", label: "extrude-preview", inline: false });
  await call("29-extrude-commit", "keyboard_press", { key: "Enter" });
  await call("30-wait-regen", "wait_for", { condition: { kind: "element_hidden", target: { testId: "regen-busy" } }, timeoutMs: 8000 });
  await call("31-wait-stable", "wait_for", { condition: { kind: "revision_stable" }, timeoutMs: 6000 });
  await call("32-body-shot", "ui_screenshot", { mode: "window", label: "body", inline: false });
  const s3 = await call("33-history-snap", "ui_snapshot", {});
  writeFileSync(resolve(outDir, "33-history-snap.txt"), findText(s3));
  await call("34-find-extrude", "ui_find", { target: { text: "Extrude" } });
  // Orbit + zoom.
  await call("35-orbit", "pointer_drag_path", { points: [{ x: cx, y: cy }, { x: cx + 140, y: cy + 40 }], space: "webview", button: "right", mods: ["Shift"], durationMs: 600 });
  await call("36-zoom", "pointer_scroll", { target: { testId: "viewport-canvas" }, dy: -3 });
  await call("37-orbit-shot", "ui_screenshot", { mode: "window", label: "orbited", inline: false });
  // Logs.
  const logs = await call("38-logs-regen", "observe_logs", { grep: "regen:|apply_operation|save_document", limit: 15 });
  writeFileSync(resolve(outDir, "38-logs.json"), JSON.stringify((logs.data as { lines?: unknown[] })?.lines ?? [], null, 1));
  const errs = await call("39-logs-err", "observe_logs", { level: "ERROR", limit: 15 });
  writeFileSync(resolve(outDir, "39-errors.json"), JSON.stringify((errs.data as { lines?: unknown[] })?.lines ?? [], null, 1));
} catch (e) {
  process.stderr.write(`INSPECT THREW: ${String(e)}\n`);
} finally {
  const stop = await call("99-stop", "session_stop", {});
  writeFileSync(resolve(outDir, "manifest.json"), JSON.stringify(manifest, null, 1));
  writeFileSync(resolve(outDir, "teardown.json"), JSON.stringify((stop.data as object) ?? {}, null, 1));
  proc.stdin.end();
  process.stderr.write(`OUT ${outDir}\n`);
  process.exit(0);
}
