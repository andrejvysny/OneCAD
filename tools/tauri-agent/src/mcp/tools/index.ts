import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { defineTool, setToolCtx, type ToolCtx } from "../defineTool.ts";
import { okResult } from "../envelope.ts";
import { registerDebugTools } from "./debug.ts";
import { registerKeyboardTools } from "./keyboard.ts";
import { registerObserveTools } from "./observe.ts";
import { registerPointerTools } from "./pointer.ts";
import { registerSessionTools } from "./session.ts";
import { registerUiTools } from "./ui.ts";
import { registerWaitTools } from "./wait.ts";
import { registerWindowTools } from "./window.ts";

export function registerAllTools(server: McpServer, ctx: ToolCtx): void {
  setToolCtx(ctx);
  registerSessionTools(server);
  registerWindowTools(server);
  registerUiTools(server);
  registerPointerTools(server);
  registerKeyboardTools(server);
  registerWaitTools(server);
  registerObserveTools(server);
  registerObserveTrace(server);
  registerDebugTools(server);
}

function registerObserveTrace(server: McpServer): void {
  defineTool(server, {
    name: "observe_trace",
    description:
      "Read this session's action journal (one entry per tool call: actionId, tool, input, envelope, durationMs). Use it to cite evidence for what was actually done.",
    input: z.object({
      sinceActionId: z.string().min(1).optional().describe("Return entries after this actionId, e.g. \"A-004\"."),
      limit: z.number().int().positive().max(1000).optional().describe("Most recent N matching entries (default 100)."),
    }),
    kind: "query",
    handler: async (args, ctx, actionId) => {
      const entries = await ctx.journal.since(args.sinceActionId, args.limit);
      return okResult(actionId, "diagnostic", { data: { entries, count: entries.length } });
    },
  });
}
