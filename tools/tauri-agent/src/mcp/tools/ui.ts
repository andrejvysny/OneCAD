/**
 * Observation tools: the semantic tree, element lookup, element detail, and native capture.
 *
 * These read; they never act. They still run `ensureCalibrated()` first, because a snapshot
 * whose rects are mapped with a stale `geom` would hand the caller coordinates that click
 * in the wrong place later on.
 */
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { AgentError } from "../../errors.ts";
import { cssToGlobal, rectCenter } from "../../geometry/mapping.ts";
import type { Pt, Rect, WindowGeom } from "../../geometry/types.ts";
import type { CaptureResult } from "../../platform/adapter.ts";
import type { SnapNode, Snapshot } from "../../semantic/snapshot.ts";
import { diffSnapshots, takeSnapshot } from "../../semantic/snapshot.ts";
import type { Resolved, Target } from "../../semantic/resolve.ts";
import { POOL_REFRESHED_WARNING, poolMiss, refreshPool, resolveWithRefresh } from "./targetPool.ts";
import type { SessionOrchestrator } from "../../session/orchestrator.ts";
import { defineTool } from "../defineTool.ts";
import { okResult } from "../envelope.ts";
import { RectSchema, TargetSchema, type TargetInput } from "../schemas.ts";

const WINDOW_CONTEXT_PAD_PT = 200;

async function resolveTarget(
  session: SessionOrchestrator,
  target: Target,
  warnings: string[],
): Promise<Resolved> {
  const { resolved, refreshed } = await resolveWithRefresh(session, session.requireBridge(), target, {
    geom: session.requireGeom(),
  });
  if (refreshed) warnings.push(POOL_REFRESHED_WARNING);
  return resolved;
}

function brief(n: SnapNode): Record<string, unknown> {
  return { ref: n.ref, role: n.role, name: n.name, testId: n.testId, rect: n.rect, disabled: n.disabled };
}

function globalOf(session: SessionOrchestrator, css: Pt): Pt {
  return cssToGlobal(css, session.requireGeom());
}

/** A page-side selector for the resolved element; `point` targets have none. */
function selectorOf(target: TargetInput, resolved: Resolved): string {
  if (resolved.node) return resolved.node.css;
  if ("css" in target) return target.css;
  throw new AgentError("INVALID_TARGET", "this tool needs an element target, not a raw point", {
    details: { target },
  });
}

function renderDiff(before: Snapshot, after: Snapshot): string {
  const d = diffSnapshots(before, after);
  const line = (sign: string, n: SnapNode): string =>
    `${sign} ${n.role} "${n.name}" ${n.ref}${n.testId ? ` testid=${n.testId}` : ""}`;
  const out = [
    ...d.added.map((n) => line("+", n)),
    ...d.removed.map((n) => line("-", n)),
    ...d.changed.map((c) => `~ ${c.after.role} "${c.after.name}" ${c.after.ref} ${JSON.stringify(c.after.state)}`),
  ];
  return out.length > 0 ? out.join("\n") : "(no semantic change since the last snapshot)";
}

async function rootSelector(session: SessionOrchestrator, target: TargetInput, warnings: string[]): Promise<string> {
  if ("css" in target) return target.css;
  if ("testId" in target) return `[data-testid="${target.testId}"]`;
  if ("point" in target) {
    throw new AgentError("INVALID_TARGET", "`root` cannot be a point target", { details: { target } });
  }
  // A ref, role/name or text root is resolved to its element first (pool refresh included).
  const resolved = await resolveTarget(session, target as Target, warnings);
  if (!resolved.node) throw new AgentError("ELEMENT_NOT_FOUND", "root target has no snapshot node", { details: { target } });
  return resolved.node.css;
}

function registerSnapshot(server: McpServer): void {
  defineTool(server, {
    name: "ui_snapshot",
    description:
      "Semantic tree of the app window: one line per interactive element with role, accessible name, ref (@s<gen>e<n>, e.g. @s3e12), data-testid and state. Refs are only valid until the next snapshot. mode \"diff\" reports what changed since the previous snapshot; \"dom\" and \"accessibility\" widen the filter.",
    input: z.object({
      mode: z.enum(["interactive", "accessibility", "dom", "diff"]).optional(),
      sinceRevision: z.number().int().nonnegative().optional().describe("Report unchanged when the DOM revision still matches."),
      root: TargetSchema.optional().describe("Limit the walk to this subtree (css, testId or ref)."),
      maxNodes: z.number().int().positive().max(1000).optional(),
    }),
    kind: "query",
    handler: async (args, ctx, actionId) => {
      const session = ctx.session;
      await session.ensureCalibrated();
      const mode = args.mode ?? "interactive";
      const warnings: string[] = [];
      const rootCss = args.root === undefined ? undefined : await rootSelector(session, args.root, warnings);
      const snap = await takeSnapshot(session.requireBridge(), {
        mode,
        ...(rootCss === undefined ? {} : { rootCss }),
        ...(args.maxNodes === undefined ? {} : { maxNodes: args.maxNodes }),
      });
      const previous = session.lastSnapshot;
      // A scoped snapshot must not shrink the pool to its subtree: keep the earlier nodes and
      // let the new ones (newer generation) take precedence, so role/name targets outside the
      // root keep resolving and stale entries are still refreshed on a miss.
      if (rootCss === undefined) session.refs.clear();
      for (const n of snap.nodes) session.refs.set(n.ref, n);
      session.lastSnapshot = snap;

      let text = snap.text;
      if (mode === "diff") {
        if (previous === undefined) warnings.push("no previous snapshot to diff against; returning the full tree");
        else text = renderDiff(previous, snap);
      }
      const unchanged = args.sinceRevision !== undefined && snap.revision === args.sinceRevision;
      return okResult(actionId, "real_user", {
        backend: "none",
        warnings,
        ...(session.windowId === undefined ? {} : { windowId: session.windowId }),
        data: {
          revision: snap.revision,
          generation: snap.generation,
          title: snap.title,
          viewport: snap.viewport,
          count: snap.nodes.length,
          truncated: snap.truncated,
          unchanged,
          text,
        },
      });
    },
  });
}

function matchAll(target: TargetInput, nodes: SnapNode[]): SnapNode[] {
  if ("ref" in target) return nodes.filter((n) => n.ref === target.ref);
  if ("testId" in target) return nodes.filter((n) => n.testId === target.testId);
  if ("role" in target) {
    const name = target.name?.toLowerCase();
    return nodes.filter((n) => n.role === target.role && (name === undefined || n.name.toLowerCase().includes(name)));
  }
  if ("text" in target) {
    const needle = target.text.toLowerCase();
    return nodes.filter((n) => n.name.toLowerCase().includes(needle));
  }
  throw new AgentError("INVALID_TARGET", "`all` supports ref, testId, role or text targets", { details: { target } });
}

function registerFind(server: McpServer): void {
  defineTool(server, {
    name: "ui_find",
    description:
      "Resolve a target (ref, testId, role+name, text, css or point) against the last snapshot and report where it is, in CSS pixels and in global screen points. Use it to check a selector before acting on it, or with all:true to see every candidate behind an ambiguous name.",
    input: z.object({
      target: TargetSchema,
      all: z.boolean().optional().describe("Return every match instead of failing on ambiguity."),
    }),
    kind: "query",
    handler: async (args, ctx, actionId) => {
      const session = ctx.session;
      await session.ensureCalibrated();
      const target = args.target;
      if (args.all === true) {
        let matches = matchAll(target, [...session.refs.values()]);
        const warnings: string[] = [];
        if (matches.length === 0 && poolMiss(target as Target)) {
          await refreshPool(session, session.requireBridge());
          matches = matchAll(target, [...session.refs.values()]);
          warnings.push(POOL_REFRESHED_WARNING);
        }
        return okResult(actionId, "real_user", {
          warnings,
          backend: "none",
          data: {
            count: matches.length,
            matches: matches.map((n) => ({ ...brief(n), global: globalOf(session, rectCenter(n.rect)) })),
          },
        });
      }
      const warnings: string[] = [];
      const resolved = await resolveTarget(session, target as Target, warnings);
      const global = globalOf(session, resolved.css);
      return okResult(actionId, "real_user", {
        backend: "none",
        warnings,
        ...(resolved.node === undefined
          ? {}
          : { target: { ref: resolved.node.ref, role: resolved.node.role, name: resolved.node.name, testId: resolved.node.testId } }),
        resolvedPoint: { global, css: resolved.css },
        data: {
          source: resolved.source,
          rect: resolved.rect,
          ...(resolved.node === undefined ? {} : { node: brief(resolved.node) }),
        },
      });
    },
  });
}

interface Inspection {
  found: boolean;
  tag: string;
  role: string | null;
  name: string | null;
  attrs: Record<string, string>;
  rect: Rect | null;
  dragRegion: boolean;
  hitIsSelf: boolean;
  occluder: { tag: string; testId: string | null; role: string | null } | null;
}

function registerInspect(server: McpServer): void {
  defineTool(server, {
    name: "ui_inspect",
    description:
      "Full detail for one element: every attribute, its computed role and accessible name, its rect in CSS pixels and global screen points, whether anything occludes its centre, and whether it sits on a window drag region (where a press would move the window instead of clicking).",
    input: z.object({ target: TargetSchema }),
    kind: "query",
    handler: async (args, ctx, actionId) => {
      const session = ctx.session;
      await session.ensureCalibrated();
      const warnings: string[] = [];
      const resolved = await resolveTarget(session, args.target as Target, warnings);
      const selector = selectorOf(args.target, resolved);
      const info = await session.requireBridge().execute<Inspection>(
        "ui_inspect",
        ((sel: string) => {
          const el = document.querySelector(sel);
          const empty = {
            found: false, tag: "", role: null, name: null, attrs: {},
            rect: null, dragRegion: false, hitIsSelf: false, occluder: null,
          };
          if (!el) return empty;
          const attrs: Record<string, string> = {};
          for (let i = 0; i < el.attributes.length; i += 1) {
            const a = el.attributes[i] as Attr;
            attrs[a.name] = a.value.slice(0, 200);
          }
          const r = el.getBoundingClientRect();
          const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
          const hitIsSelf = hit !== null && (hit === el || el.contains(hit));
          return {
            found: true,
            tag: el.tagName.toLowerCase(),
            role: el.getAttribute("role"),
            name: (el.getAttribute("aria-label") ?? el.textContent ?? "").trim().slice(0, 120),
            attrs,
            rect: { x: r.left, y: r.top, width: r.width, height: r.height },
            dragRegion: el.closest("[data-tauri-drag-region]") !== null,
            hitIsSelf,
            occluder:
              hit && !hitIsSelf
                ? { tag: hit.tagName.toLowerCase(), testId: hit.getAttribute("data-testid"), role: hit.getAttribute("role") }
                : null,
          };
        }) as never,
        [selector],
        { readOnly: true },
      );
      if (!info.found || info.rect === null) {
        throw new AgentError("ELEMENT_NOT_FOUND", `nothing matches ${selector} any more`, { details: { selector } });
      }
      const css = rectCenter(info.rect);
      return okResult(actionId, "real_user", {
        backend: "none",
        warnings,
        resolvedPoint: { global: globalOf(session, css), css },
        data: { selector, ...info, occluded: !info.hitIsSelf, rectGlobal: rectToGlobal(info.rect, session.requireGeom()) },
      });
    },
  });
}

type CaptureMode = "window" | "region" | "screen" | "window_context";

function safeLabel(label: string | undefined, mode: CaptureMode): string {
  const raw = label ?? mode;
  const clean = raw.replace(/[^A-Za-z0-9._-]/g, "-").slice(0, 60);
  return clean.length > 0 ? clean : mode;
}

function padded(rect: Rect, pad: number): Rect {
  return {
    x: Math.max(0, rect.x - pad),
    y: Math.max(0, rect.y - pad),
    width: rect.width + pad * 2,
    height: rect.height + pad * 2,
  };
}

function registerScreenshot(server: McpServer): void {
  defineTool(server, {
    name: "ui_screenshot",
    description:
      "Native screenshot of the real window server output — the only evidence that survives a wedged WebView bridge. mode \"window\" captures the app window, \"window_context\" adds 200pt of surrounding desktop (use it to prove a native menu or tooltip drawn outside the window), \"region\" takes global-point coordinates, \"screen\" captures the whole display. A downscaled preview is returned inline; the full-resolution PNG path is in the envelope.",
    input: z.object({
      mode: z.enum(["window", "region", "screen", "window_context"]).optional(),
      region: RectSchema.optional().describe("Global points, required for mode \"region\"."),
      label: z.string().optional().describe("Filename suffix, so the artifact is recognisable later."),
      inline: z.boolean().optional().describe("false returns only the file paths, without the inline image."),
    }),
    kind: "query",
    handler: async (args, ctx, actionId) => {
      const session = ctx.session;
      // An explicit request for a picture this session cannot take is refused, not attempted:
      // the caller asked for evidence and must be told plainly that none is available.
      const capability = session.status().capture;
      if (!capability.available) {
        throw new AgentError(
          "SCREEN_CAPTURE_PERMISSION_DENIED",
          "this session cannot take a screenshot: it was started without the Screen Recording grant",
          {
            remediation:
              "Grant Screen Recording in System Settings › Privacy & Security and start a new session.",
            details: { capture: capability },
          },
        );
      }
      await session.ensureCalibrated();
      const capture = session.requirePlatform().capture;
      const geom = session.requireGeom();
      const mode: CaptureMode = args.mode ?? "window";
      const path = ctx.journal.artifactPath(`${actionId}-${safeLabel(args.label, mode)}.png`);
      const previewPath = path.replace(/\.png$/, "-preview.png");
      const started = Date.now();
      const { warning, authoritative, reason, ...shot } = await runCapture(
        capture,
        mode,
        path,
        geom.windowId,
        geom.nativeBoundsPt,
        args.region,
      );
      await capture.preview(path, previewPath, ctx.config.config.screenshots.previewMaxPx);
      return okResult(actionId, "real_user", {
        backend: "none",
        windowId: geom.windowId,
        warnings: warning === undefined ? [] : [warning],
        screenshot: {
          path,
          previewPath,
          ...shot,
          captureMode: mode,
          // `mode:"screen"` photographs the whole display with NO bounds to check against, so
          // the width/height agreement that backs `authoritative` for every other mode simply
          // does not run — there is nothing it could have disagreed with. An unverified image
          // must not inherit the flag; a bare `pixelScale: 1` is not evidence of anything.
          authoritative:
            args.mode !== "screen" &&
            capability.authoritative &&
            authoritative !== false &&
            warning === undefined &&
            reason === undefined,
        },
        timingsMs: { resolve: 0, input: 0, settle: 0, capture: Date.now() - started },
        data: { path, previewPath, mode },
      });
    },
  });
}

async function runCapture(
  capture: ReturnType<SessionOrchestrator["requirePlatform"]>["capture"],
  mode: CaptureMode,
  path: string,
  windowId: number,
  bounds: Rect,
  region?: Rect,
): Promise<CaptureResult> {
  switch (mode) {
    case "window":
      return capture.window(windowId, path, bounds);
    case "window_context":
      return capture.region(padded(bounds, WINDOW_CONTEXT_PAD_PT), path);
    case "region":
      if (region === undefined) {
        throw new AgentError("INVALID_TARGET", 'mode "region" requires a `region` rect in global points');
      }
      return capture.region(region, path);
    case "screen":
      // The bounds parameter means the DISPLAY size; the window rect would yield a false scale.
      return capture.screen(path);
  }
}

export function registerUiTools(server: McpServer): void {
  registerSnapshot(server);
  registerFind(server);
  registerInspect(server);
  registerScreenshot(server);
}

/** The element's CSS rect expressed in global display points, for canvas-space planning. */
function rectToGlobal(rect: Rect | null | undefined, geom: WindowGeom): Rect | null {
  if (!rect) return null;
  const o = cssToGlobal({ x: rect.x, y: rect.y }, geom);
  return { x: o.x, y: o.y, width: rect.width, height: rect.height };
}
