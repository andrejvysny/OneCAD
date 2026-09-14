import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { defineTool } from "../defineTool.ts";
import { deliveryFor, okResult } from "../envelope.ts";

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
  interaction: z
    .enum(["foreground", "background"])
    .optional()
    .describe(
      'How much of the user\'s desktop this session may touch. "foreground" (default) activates the app and posts real CGEvents — full fidelity, and it TAKES the pointer and the frontmost application. "background" never moves the cursor, never activates and never posts an OS event: it dispatches events inside the page, actuates native chrome through accessibility, and still screenshots the window while it sits behind whatever you are working in. Launching still activates the app once (a Tauri/tao behaviour no flag avoids) — use mode:"attach" to avoid even that.',
    ),
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
        // Starting a session launches a process and runs calibration, which posts real cursor
        // moves. Re-sending it is never a no-op, so it is not retry-safe.
        delivery: deliveryFor("input_completed", true),
        // The one honest place to report the launch activation: it happened during THIS call,
        // and `foregroundStolenAtLaunch` on the session says it will not happen again.
        interaction: {
          policy: status.interaction,
          foregroundChanged: status.interaction === "foreground" || status.foregroundStolenAtLaunch === true,
          cursorMoved: status.interaction === "foreground",
        },
        data: status,
      });
    },
  });

  defineTool(server, {
    name: "session_stop",
    description:
      "Stop the session: release input, close the bridge, kill the process tree this session LAUNCHED, sweep for survivors of this checkout and check the ports. An attached session never signals the app — it detaches and its sweep is report-only. The returned `teardown` {survivors, portsFree, launched, detached} is the measured evidence a report should cite for 'nothing left running'.",
    input: z.object({
      killApp: z
        .boolean()
        .optional()
        .describe(
          "Launched sessions only. Default true; false detaches instead. Ignored when the session attached.",
        ),
      forceKillAttached: z
        .boolean()
        .optional()
        .describe(
          "Attached sessions only. Default false. true deliberately terminates an app this session did NOT launch.",
        ),
    }),
    kind: "action",
    handler: async (args, ctx, actionId) =>
      okResult(actionId, "real_user", {
        backend: "none",
        // Stopping signals processes and releases held input: side effects by definition.
        delivery: deliveryFor("input_completed", true),
        data: await ctx.session.stop(args),
      }),
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
