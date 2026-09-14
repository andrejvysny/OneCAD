import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { AgentError } from "../../errors.ts";
import { log } from "../../log.ts";
import type { SessionBridge } from "../../session/types.ts";
import { defineTool } from "../defineTool.ts";
import { okResult } from "../envelope.ts";

interface TauriWindowsGlobal {
  __TAURI__: { window: { getAllWindows(): Promise<Array<{ label: string }>> | Array<{ label: string }> } };
}

/** Tauri labels come from the webview; the native list comes from CGWindowList. They do not correlate 1:1. */
async function tauriLabels(bridge: SessionBridge): Promise<string[] | undefined> {
  try {
    return await bridge.execute<string[]>(
      "window.labels",
      (async () => {
        const all = await (window as unknown as TauriWindowsGlobal).__TAURI__.window.getAllWindows();
        return all.map((w) => w.label);
      }) as never,
      [],
      { readOnly: true },
    );
  } catch (e) {
    log.debug("window labels unavailable", { error: String(e) });
    return undefined;
  }
}

export function registerWindowTools(server: McpServer): void {
  defineTool(server, {
    name: "window_list",
    description:
      "List the app's on-screen native windows (CGWindowID, global-point bounds, title) plus the Tauri window labels the webview reports. Use it to confirm which window input and screenshots will target.",
    input: z.object({}),
    kind: "query",
    handler: async (_args, ctx, actionId) => {
      await ctx.session.ensureCalibrated();
      const pid = ctx.session.status().pid as number;
      const windows = await ctx.session.requirePlatform().windows.list(pid);
      const labels = await tauriLabels(ctx.session.requireBridge());
      return okResult(actionId, "real_user", {
        backend: "none",
        ...(ctx.session.windowId === undefined ? {} : { windowId: ctx.session.windowId }),
        data: { pid, active: ctx.session.windowId, windows, ...(labels === undefined ? {} : { labels }) },
      });
    },
  });

  defineTool(server, {
    name: "window_focus",
    description:
      "Bring the app window to the front. Keyboard input goes to the frontmost application, so call this after clicking away to another app, or when a keyboard step did not land.",
    input: z.object({
      label: z.string().optional().describe("Tauri window label; only the configured main window is supported."),
    }),
    kind: "action",
    handler: async (args, ctx, actionId) => {
      ctx.session.requireReady();
      const warnings: string[] = [];
      const expected = ctx.config.config.app.windowLabel;
      if (args.label !== undefined && args.label !== expected) {
        warnings.push(`only the "${expected}" window is supported; focusing it instead of "${args.label}"`);
      }
      const pid = ctx.session.status().pid as number;
      if (!(await ctx.session.requirePlatform().windows.focus(pid))) {
        throw new AgentError("WINDOW_NOT_FOREGROUND", `the app (pid ${pid}) did not become frontmost`, {
          details: { pid, label: expected },
        });
      }
      // Focusing can raise and reposition the window; the mapping must follow it.
      if (await ctx.session.geometryChanged()) await ctx.session.calibrate();
      return okResult(actionId, "real_user", {
        backend: "none",
        ...(ctx.session.windowId === undefined ? {} : { windowId: ctx.session.windowId }),
        warnings,
        data: { pid, focused: true, label: expected },
      });
    },
  });
}
