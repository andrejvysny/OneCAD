/**
 * Diagnostic escape hatches. They run JavaScript and Tauri commands directly, which no user
 * can do — `defineTool` prefixes their descriptions and the envelope carries
 * `mode:"diagnostic"` so a result from here can never be mistaken for user-grade evidence.
 * They are still journalled like everything else.
 */
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { AgentError } from "../../errors.ts";
import { defineTool } from "../defineTool.ts";
import { okResult } from "../envelope.ts";

interface EvalOutcome {
  ok: boolean;
  value?: unknown;
  error?: string;
}

function registerEvalJs(server: McpServer): void {
  defineTool(server, {
    name: "debug_eval_js",
    description:
      "Evaluate a JavaScript expression or function body inside the app's webview and return its (JSON-serialisable) result. Dev builds expose window.__stores, window.__client and window.__logsDump() here. Use it to understand state, never to perform an action a user would perform.",
    input: z.object({
      script: z.string().min(1).describe('Function body; `args` holds the values passed. e.g. "return document.title"'),
      args: z.array(z.unknown()).optional(),
      inline: z.boolean().optional(),
    }),
    kind: "diagnostic",
    handler: async (args, ctx, actionId) => {
      ctx.session.requireReady();
      const out = await ctx.session.requireBridge().execute<EvalOutcome>(
        "debug_eval_js",
        ((src: string, a: unknown[]) => {
          try {
            const fn = new Function("...args", src) as (...x: unknown[]) => unknown;
            return { ok: true, value: JSON.parse(JSON.stringify(fn(...a) ?? null)) as unknown };
          } catch (e) {
            return { ok: false, error: e instanceof Error ? `${e.name}: ${e.message}` : String(e) };
          }
        }) as never,
        [args.script, args.args ?? []],
        { readOnly: false },
      );
      if (!out.ok) {
        throw new AgentError("INTERNAL", `the script threw: ${out.error ?? "unknown error"}`, {
          remediation: "Fix the script, or check that the value it returns is JSON-serialisable.",
          details: { script: args.script },
        });
      }
      return okResult(actionId, "diagnostic", { backend: "js", data: { value: out.value } });
    },
  });
}

function registerInvoke(server: McpServer): void {
  defineTool(server, {
    name: "debug_invoke_tauri_command",
    description:
      "Call a Tauri #[tauri::command] directly over IPC from the webview and return its result — the backend's own view of the document, with no UI in the way. Read-only queries (get_projection, composition_status, query_*) are the intended use; invoking a mutating command changes the document without any user action having occurred.",
    input: z.object({
      command: z.string().min(1).describe("The Rust function name, snake_case."),
      args: z.record(z.unknown()).optional(),
      inline: z.boolean().optional(),
    }),
    kind: "diagnostic",
    handler: async (args, ctx, actionId) => {
      ctx.session.requireReady();
      const value = await ctx.session.requireBridge().invoke<unknown>(args.command, args.args ?? {});
      return okResult(actionId, "diagnostic", { backend: "js", data: { command: args.command, value } });
    },
  });
}

export function registerDebugTools(server: McpServer): void {
  registerEvalJs(server);
  registerInvoke(server);
}
