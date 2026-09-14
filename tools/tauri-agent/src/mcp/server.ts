// Entry point: `bun tools/tauri-agent/src/mcp/server.ts`.
import { randomBytes } from "node:crypto";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { log } from "../log.ts";
import { loadConfig, resolveFromRoot } from "../session/config.ts";
import { SessionOrchestrator } from "../session/orchestrator.ts";
import { Journal } from "../trace/journal.ts";
import type { ToolCtx } from "./defineTool.ts";
import { registerAllTools } from "./tools/index.ts";

// The stdio transport owns process.stdout: a stray console.log would corrupt the JSON-RPC stream.
function installStdoutGuard(): void {
  const toStderr = (...parts: unknown[]): void => {
    process.stderr.write(`${parts.map(stringify).join(" ")}\n`);
  };
  console.log = toStderr;
  console.info = toStderr;
  console.debug = toStderr;
}

function stringify(value: unknown): string {
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value) ?? String(value);
  } catch {
    return String(value);
  }
}

export function newSessionId(now = new Date()): string {
  const p = (n: number): string => String(n).padStart(2, "0");
  const stamp =
    `${now.getFullYear()}${p(now.getMonth() + 1)}${p(now.getDate())}` +
    `-${p(now.getHours())}${p(now.getMinutes())}${p(now.getSeconds())}`;
  return `s-${stamp}-${randomBytes(2).toString("hex")}`;
}

function installSignalHandlers(session: SessionOrchestrator, server: McpServer): void {
  let shuttingDown = false;
  const shutdown = (signal: string): void => {
    if (shuttingDown) return;
    shuttingDown = true;
    log.info("shutting down", { signal, state: session.state });
    void session
      .stop({ killApp: false })
      .catch((e: unknown) => log.warn("stop during shutdown failed", { error: String(e) }))
      .then(() => server.close().catch(() => undefined))
      .finally(() => process.exit(0));
  };
  process.on("SIGTERM", () => shutdown("SIGTERM"));
  process.on("SIGINT", () => shutdown("SIGINT"));
}

async function main(): Promise<void> {
  installStdoutGuard();
  const resolved = loadConfig();
  const sessionId = newSessionId();
  const journal = new Journal(
    resolveFromRoot(resolved.root, `${resolved.config.artifacts.dir}/${sessionId}`),
  );
  const session = new SessionOrchestrator({ sessionId, journal, config: resolved });
  const ctx: ToolCtx = { session, journal, config: resolved };

  const server = new McpServer({ name: "tauri-agent", version: "0.1.0" });
  registerAllTools(server, ctx);
  installSignalHandlers(session, server);

  await server.connect(new StdioServerTransport());
  log.info("tauri-agent ready", { sessionId, root: resolved.root, artifacts: journal.dir });
}

await main().catch((e: unknown) => {
  log.error("tauri-agent failed to start", { error: String(e) });
  process.exit(1);
});
