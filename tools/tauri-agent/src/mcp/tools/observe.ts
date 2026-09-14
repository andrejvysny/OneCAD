/**
 * `observe_logs` — the Rust/C++ half of the evidence. The webview's console only sees the
 * frontend; regen failures, worker faults and IPC errors exist solely in `dev.jsonl`.
 */
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { DEFAULT_LIMIT, devJsonlPath, tailLogs } from "../../observe/logs.ts";
import { defineTool } from "../defineTool.ts";
import { okResult } from "../envelope.ts";

export function registerObserveTools(server: McpServer): void {
  defineTool(server, {
    name: "observe_logs",
    description:
      "Tail the app's structured log (dev.jsonl) from a byte cursor. Covers all four lanes — fe (forwarded webview events), worker (the C++ sidecar's stderr), onecad_lib (Rust), onecad_protocol::frames — so it is the only way to see a regen failure or a worker fault. Pass the returned cursor as `since` next time to get just the new lines.",
    input: z.object({
      since: z
        .string()
        .optional()
        .describe("Cursor from a previous call; omit to read the whole file (capped at the most recent 4 MB)."),
      level: z
        .preprocess((v) => (typeof v === "string" ? v.toUpperCase() : v), z.enum(["TRACE", "DEBUG", "INFO", "WARN", "ERROR"]))
        .optional()
        .describe("Minimum severity; ERROR alone is usually what you want."),
      lane: z
        .string()
        .optional()
        .describe('Prefix of the log `target`: "fe", "worker", "onecad_lib", "onecad_protocol::frames".'),
      grep: z.string().optional().describe("Regular expression matched against the raw JSON line."),
      limit: z.number().int().positive().max(DEFAULT_LIMIT).optional().describe(`Most recent N matching lines (default ${DEFAULT_LIMIT}).`),
      inline: z.boolean().optional(),
    }),
    kind: "query",
    handler: async (args, ctx, actionId) => {
      const path = devJsonlPath({
        root: ctx.config.root,
        devJsonl: ctx.config.config.logs.devJsonl,
        journalDir: ctx.journal.dir,
        launched: ctx.session.status().launched,
      });
      const tail = await tailLogs(path, args);
      const warnings = tail.missing
        ? [`no log file at ${tail.path === "" ? "(logs.devJsonl is null)" : tail.path}; the app writes it only in debug builds or with ONECAD_LOG_DIR set`]
        : [];
      return okResult(actionId, "diagnostic", {
        status: warnings.length > 0 ? "warning" : "ok",
        warnings,
        data: {
          path: tail.path,
          cursor: tail.cursor,
          count: tail.lines.length,
          matched: tail.matched,
          dropped: tail.dropped,
          lines: tail.lines.map((l) => l.fields ?? { raw: l.raw }),
        },
      });
    },
  });
}
