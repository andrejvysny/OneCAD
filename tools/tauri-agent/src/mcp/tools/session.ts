import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { defineTool } from "../defineTool.ts";
import { okResult } from "../envelope.ts";

const StartInput = z.object({
  mode: z
    .enum(["launch", "attach"])
    .default("launch")
    .describe('"launch" starts the app; "attach" binds to one already serving WebDriver.'),
  launch: z
    .enum(["dev", "bundled"])
    .optional()
    .describe('"dev" runs `bun run tauri:agent` (first build is slow); "bundled" runs the built .app.'),
  port: z.number().int().min(1).max(65535).optional().describe("WebDriver port; defaults to webdriver.port."),
  reuseExisting: z
    .boolean()
    .optional()
    .describe("Attach instead of failing when the port is already served by this project's app."),
  env: z.record(z.string()).optional().describe("Extra environment variables for the launched app."),
  allowDegradedCapture: z
    .boolean()
    .optional()
    .describe("Start without the Screen Recording grant; screenshots then show wallpaper only."),
});

export function registerSessionTools(server: McpServer): void {
  defineTool(server, {
    name: "session_start",
    description:
      "Launch (or attach to) the real desktop app and make it driveable: permission preflight, port checks, WebDriver bridge, native window discovery, and coordinate calibration. Must be the first tool called; every pointer/keyboard/screenshot tool fails with APP_NOT_RUNNING until it succeeds.",
    input: StartInput,
    kind: "action",
    handler: async (args, ctx, actionId) => {
      const status = await ctx.session.start(args);
      return okResult(actionId, "real_user", {
        backend: "none",
        ...(status.windowId === undefined ? {} : { windowId: status.windowId }),
        data: status,
      });
    },
  });

  defineTool(server, {
    name: "session_stop",
    description:
      "Stop the session: release input, close the bridge, kill the launched process tree, sweep for survivors of this checkout and check the ports. The returned `teardown` {survivors, portsFree, launched} is the measured evidence a report should cite for 'nothing left running'.",
    input: z.object({
      killApp: z
        .boolean()
        .optional()
        .describe("Default true. false leaves an attached app running and only detaches from it."),
    }),
    kind: "action",
    handler: async (args, ctx, actionId) =>
      okResult(actionId, "real_user", { backend: "none", data: await ctx.session.stop(args) }),
  });

  defineTool(server, {
    name: "session_status",
    description:
      "Report the agent session state (Idle/Launching/WaitingForBridge/WaitingForWindow/Ready/Reconnecting/Stopping/Failed), app pid, native window id, ports, TCC permissions, and the coordinate calibration with its hover and wheel probes.",
    input: z.object({}),
    kind: "query",
    handler: async (_args, ctx, actionId) => okResult(actionId, "diagnostic", { data: ctx.session.status() }),
  });
}
