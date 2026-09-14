/**
 * Manual driver: run the MCP server once and expose it on a unix socket so a shell can issue one
 * tool call per `mcp-call.ts` invocation while the session (app, bridge, refs) stays alive.
 */
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { resolve } from "node:path";
import { unlinkSync, existsSync } from "node:fs";

const root = resolve(import.meta.dir, "../../..");
const sock = process.env.TAURI_AGENT_SHELL_SOCK ?? "/tmp/tauri-agent-shell.sock";
if (existsSync(sock)) unlinkSync(sock);
const proc = spawn("bun", ["tools/tauri-agent/src/mcp/server.ts"], {
  cwd: root, stdio: ["pipe", "pipe", "pipe"], env: { ...process.env, TAURI_AGENT_LOG: process.env.TAURI_AGENT_LOG ?? "info" },
});
proc.stderr.on("data", (d) => process.stderr.write(`[server] ${d}`));
const pending = new Map<number, (v: unknown) => void>();
createInterface({ input: proc.stdout }).on("line", (line) => {
  const msg = JSON.parse(line) as { id?: number; result?: unknown; error?: unknown };
  if (msg.id !== undefined) pending.get(msg.id)?.(msg.error ? { mcpError: msg.error } : msg.result);
});
let nextId = 1;
function rpc(method: string, params: unknown): Promise<unknown> {
  const id = nextId++;
  proc.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
  return new Promise((r) => pending.set(id, r));
}
await rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "mcp-shell", version: "0" } });
proc.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" })}\n`);
process.stderr.write(`mcp-shell ready on ${sock}\n`);
Bun.listen({
  unix: sock,
  socket: {
    data(socket, data) {
      const req = JSON.parse(data.toString()) as { name: string; arguments: Record<string, unknown> };
      rpc("tools/call", { name: req.name, arguments: req.arguments ?? {} }).then(async (res) => {
        // Large replies (inline PNGs) exceed one socket write; hand over a file path instead.
        const path = `/tmp/tauri-agent-shell-reply-${Date.now()}.json`;
        await Bun.write(path, JSON.stringify(res));
        socket.write(`${path}\n`);
        socket.end();
      });
    },
  },
});
