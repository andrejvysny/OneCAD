import { readFile } from "node:fs/promises";
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import type { CallToolResult } from "@modelcontextprotocol/sdk/types.js";
import { z } from "zod";
import { AgentError } from "../errors.ts";
import { log } from "../log.ts";
import type { Journal } from "../trace/journal.ts";
import type { ResolvedConfig } from "../session/config.ts";
import type { SessionOrchestrator } from "../session/orchestrator.ts";
import type { ActionResult, Mode } from "./envelope.ts";
import { errorResult } from "./envelope.ts";

export interface ToolCtx {
  session: SessionOrchestrator;
  journal: Journal;
  config: ResolvedConfig;
}

// The contract fixes defineTool's signature at (server, def), so the shared context is
// installed once by server.ts instead of being threaded through every registration.
let ctxSingleton: ToolCtx | undefined;

export function setToolCtx(ctx: ToolCtx): void {
  ctxSingleton = ctx;
}

export function getToolCtx(): ToolCtx {
  if (!ctxSingleton) throw new AgentError("INTERNAL", "tool context not installed; call setToolCtx first");
  return ctxSingleton;
}

const DIAGNOSTIC_PREFIX = "DIAGNOSTIC (not a user action): ";

export function defineTool<S extends z.ZodTypeAny>(
  server: McpServer,
  def: {
    name: string;
    description: string;
    input: S;
    kind: "action" | "query" | "diagnostic";
    handler: (args: z.infer<S>, ctx: ToolCtx, actionId: string) => Promise<ActionResult>;
  },
): void {
  const description = def.kind === "diagnostic" ? DIAGNOSTIC_PREFIX + def.description : def.description;
  const fallbackMode: Mode = def.kind === "diagnostic" ? "diagnostic" : "real_user";

  registerRaw(server)(def.name, { description, inputSchema: shapeOf(def.input) }, async (raw) => {
    const args = (raw ?? {}) as Record<string, unknown>;
    const ctx = getToolCtx();
    const actionId = ctx.journal.nextActionId();
    const started = Date.now();
    let result: ActionResult;
    try {
      result = await def.handler(def.input.parse(args) as z.infer<S>, ctx, actionId);
    } catch (cause) {
      log.error("tool failed", { tool: def.name, actionId, error: String(cause) });
      result = errorResult(actionId, fallbackMode, cause);
    }
    // `interaction.policy` is stamped HERE, centrally, and nowhere else.
    //
    // Only `beginAction` knows what a call did to the desktop, but most tools never go through it
    // — the twenty-odd bare `okResult`/`errorResult` sites, the three accessibility actuation
    // tools that deliberately skip the pipeline, and the catch above. Every one of those inherits
    // `quietInteraction()`'s "foreground" default, which in a background session is a plain lie in
    // the very block that exists to prove the session kept its promise. This is the single place
    // every result passes through, and the session is the only authority on its own policy.
    //
    // The two booleans are NOT touched: a call that did not run the frontmost gate genuinely
    // changed nothing, and overwriting what `beginAction` measured would be worse than the default.
    result.interaction = { ...result.interaction, policy: ctx.session.interactionPolicy };
    const durationMs = Date.now() - started;
    await ctx.journal.append({
      ts: new Date().toISOString(),
      sessionId: ctx.session.sessionId,
      actionId,
      tool: def.name,
      input: args,
      result,
      durationMs,
    });
    return toCallToolResult(result, args);
  });
}

function shapeOf(schema: z.ZodTypeAny): Record<string, z.ZodTypeAny> {
  return schema instanceof z.ZodObject ? (schema.shape as Record<string, z.ZodTypeAny>) : {};
}

type RawRegister = (
  name: string,
  config: { description: string; inputSchema: Record<string, z.ZodTypeAny> },
  cb: (args: unknown) => Promise<CallToolResult>,
) => unknown;

// registerTool's generics re-infer every zod shape through ShapeOutput, which blows TS's
// instantiation depth (TS2589) on schemas this size. The runtime call is unchanged.
function registerRaw(server: McpServer): RawRegister {
  return server.registerTool.bind(server) as unknown as RawRegister;
}

async function toCallToolResult(
  result: ActionResult,
  args: Record<string, unknown>,
): Promise<CallToolResult> {
  const content: CallToolResult["content"] = [{ type: "text", text: JSON.stringify(result) }];
  const preview = result.screenshot?.previewPath;
  if (preview !== undefined && args.inline !== false) {
    const image = await readPreview(preview);
    if (image) content.push({ type: "image", data: image, mimeType: "image/png" });
  }
  return { content, isError: result.status === "error" };
}

async function readPreview(path: string): Promise<string | null> {
  try {
    return (await readFile(path)).toString("base64");
  } catch (cause) {
    log.warn("preview unreadable", { path, error: String(cause) });
    return null;
  }
}
