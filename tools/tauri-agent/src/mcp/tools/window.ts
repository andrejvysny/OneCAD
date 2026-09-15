import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { isAgentError } from "../../errors.ts";
import { log } from "../../log.ts";
import type { SessionBridge } from "../../session/types.ts";
import { defineTool } from "../defineTool.ts";
import { deliveryFor, errorResult, okResult } from "../envelope.ts";

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
      "List the app's on-screen native windows (CGWindowID, global-point bounds, title), the Tauri window labels the webview reports, and the correlated label-to-window table this session gates input on. Use it to confirm which window input and screenshots will target, and to see any label that could NOT be correlated.",
    input: z.object({}),
    kind: "query",
    handler: async (_args, ctx, actionId) => {
      await ctx.session.ensureCalibrated();
      const status = ctx.session.status();
      const pid = status.pid as number;
      const windows = await ctx.session.requirePlatform().windows.list(pid);
      const labels = await tauriLabels(ctx.session.requireBridge());
      return okResult(actionId, "real_user", {
        backend: "none",
        ...(ctx.session.windowId === undefined ? {} : { windowId: ctx.session.windowId }),
        ...(status.windowTableWarning === undefined ? {} : { warnings: [status.windowTableWarning] }),
        data: {
          pid,
          active: ctx.session.windowId,
          // What the session MEANS by "the window", and — when the table exists — the evidence
          // that names it. A label with `unresolved` set is one that every acting tool refuses on.
          target: { label: status.targetLabel, ...(status.targetIsModal ? { isModal: true } : {}) },
          ...(status.windowTable === undefined ? {} : { table: status.windowTable }),
          windows,
          ...(labels === undefined ? {} : { labels }),
        },
      });
    },
  });

  defineTool(server, {
    name: "window_focus",
    description:
      "Bring one of the app's windows to the front and re-point this session at it. Keyboard input goes to the frontmost application's key window, so call this after clicking away to another app, when a keyboard step did not land, or to switch to a second app window by label. A label that cannot be correlated to a native window is refused, never approximated.",
    input: z.object({
      label: z
        .string()
        .min(1)
        .optional()
        .describe("Tauri window label to focus and calibrate against; defaults to the session's current target window."),
    }),
    kind: "action",
    handler: async (args, ctx, actionId) => {
      ctx.session.requireReady();
      const label = args.label ?? ctx.session.status().targetLabel;
      // Focusing raises a window, and the recalibration inside `focusWindow` posts real cursor
      // moves and may post real wheel notches. `okResult`'s default would claim
      // `inputStarted:false, retrySafe:true`, which is the one thing the delivery block must
      // never get wrong — and that stays true when the step FAILS after the raise, so the error
      // envelope is built here with the same delivery instead of being thrown into the generic
      // `not_started` one.
      const delivery = deliveryFor("input_completed", true);
      try {
        const outcome = await ctx.session.focusWindow(label);
        return okResult(actionId, "real_user", {
          backend: "none",
          ...(ctx.session.windowId === undefined ? {} : { windowId: ctx.session.windowId }),
          warnings: outcome.warnings,
          delivery,
          data: {
            pid: ctx.session.status().pid,
            focused: true,
            label,
            windowId: outcome.identity.windowId,
            source: outcome.identity.source,
            recalibrated: outcome.recalibrated,
          },
        });
      } catch (cause) {
        // `raised` is stamped by the session on every failure that happened AFTER the app was
        // activated. Without it the failure came out of label resolution, which touches nothing.
        const raised = isAgentError(cause) && (cause.details as { raised?: boolean } | undefined)?.raised === true;
        return errorResult(actionId, "real_user", cause, raised ? delivery : undefined);
      }
    },
  });
}
