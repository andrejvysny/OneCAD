import { AgentError } from "../errors.ts";
import type { BridgeLike } from "./webdriver.ts";
import { SNAPSHOT_BUDGET_MS } from "./webdriver.ts";
import type { InstrumentReport } from "./instrumentScript.ts";
import { installInstrumentation } from "./instrumentScript.ts";
import type { IdleCamera, IdleReading } from "./idleScript.ts";
import { readIdleInPage } from "./idleScript.ts";
import { snapshotInPage } from "./snapshotScript.ts";
import type {
  AgentWindow,
  PageNode,
  PageSnapshot,
  SnapshotScriptOpts,
  SnapshotScriptMode,
} from "./snapshotScript.ts";

/** Pins the script's signature at compile time; `execute` erases it on the way out. */
const SNAPSHOT_SCRIPT: (o: SnapshotScriptOpts) => PageSnapshot = snapshotInPage;
const INSTRUMENT_SCRIPT: () => InstrumentReport = installInstrumentation;
const IDLE_SCRIPT: () => IdleReading = readIdleInPage;

export type SnapshotMode = "interactive" | "accessibility" | "dom" | "diff";

/** Identical shape to the in-page node; kept as one declaration so they cannot drift. */
export type SnapNode = PageNode;

export interface Snapshot {
  /** Namespaces this snapshot's refs; a ref from an older generation is stale. */
  generation: number;
  revision: number;
  title: string;
  viewport: { width: number; height: number };
  nodes: SnapNode[];
  text: string;
  truncated: boolean;
}

export const DEFAULT_MAX_NODES = 300;
export const DEFAULT_MAX_CHARS = 24_000;

export interface TakeSnapshotOpts {
  mode: SnapshotMode;
  rootCss?: string;
  maxNodes?: number;
  maxChars?: number;
}

function fmtValue(v: string | boolean | number): string {
  const s = String(v);
  return /\s/.test(s) ? `"${s}"` : s;
}

function renderLine(n: SnapNode): string {
  const parts = [`${"  ".repeat(n.depth)}${n.role} "${n.name.replace(/"/g, '\\"')}" ${n.ref}`];
  if (n.testId !== undefined && n.testId !== "") parts.push(`testid=${n.testId}`);
  const keys = Object.keys(n.state).sort();
  if (keys.length > 0) {
    parts.push(`[${keys.map((k) => `${k}=${fmtValue(n.state[k] as string)}`).join(" ")}]`);
  }
  if (n.dragRegion) parts.push("drag-region");
  return parts.join(" ");
}

/**
 * One line per node, indented under each landmark ancestor. `maxChars` is applied
 * here rather than in the page because the text is built here; `total` carries the
 * count the in-page `maxNodes` cap already dropped, so one trailer covers both.
 */
function renderTree(nodes: SnapNode[], maxChars: number, total: number): { text: string; rendered: number } {
  const lines: string[] = [];
  let used = 0;
  for (const n of nodes) {
    const line = renderLine(n);
    if (used + line.length + 1 > maxChars) break;
    lines.push(line);
    used += line.length + 1;
  }
  const dropped = total - lines.length;
  if (dropped > 0) lines.push(`…(+${dropped} nodes; refine with ui_find)`);
  return { text: lines.join("\n"), rendered: lines.length - (dropped > 0 ? 1 : 0) };
}

export async function takeSnapshot(bridge: BridgeLike, opts: TakeSnapshotOpts): Promise<Snapshot> {
  const maxNodes = opts.maxNodes ?? DEFAULT_MAX_NODES;
  const maxChars = opts.maxChars ?? DEFAULT_MAX_CHARS;
  // "diff" is a server-side comparison of two interactive snapshots.
  const scriptMode: SnapshotScriptMode = opts.mode === "diff" ? "interactive" : opts.mode;
  const page = await bridge.execute<PageSnapshot>(
    "ui_snapshot",
    SNAPSHOT_SCRIPT as never,
    [{ mode: scriptMode, rootCss: opts.rootCss, maxNodes }],
    // The script WRITES page globals (the ref map, the generation counter, the
    // probes) yet is marked read-only, because "readOnly" here means "safe to
    // re-send": it changes no application state, and a retry simply issues a new
    // generation of refs. The refs from the abandoned attempt are unreachable —
    // no caller ever saw them.
    { readOnly: true, budgetMs: SNAPSHOT_BUDGET_MS },
  );
  if (page === null || typeof page !== "object" || !Array.isArray(page.nodes)) {
    throw new AgentError("INTERNAL", `ui_snapshot returned an unusable payload: ${JSON.stringify(page)}`, {
      details: { page },
    });
  }
  if (!page.rootFound) {
    throw new AgentError("ELEMENT_NOT_FOUND", `snapshot root ${opts.rootCss ?? "body"} does not exist`, {
      remediation: "Re-take the snapshot without `root`, then pick a root from the returned tree.",
      details: { rootCss: opts.rootCss },
    });
  }
  const { text, rendered } = renderTree(page.nodes, maxChars, page.total);
  return {
    generation: page.generation,
    revision: page.revision,
    title: page.title,
    viewport: page.viewport,
    // Every captured node stays available to ui_find / resolve; `maxChars` caps
    // only the rendered text, and the in-page ref map holds all of them anyway.
    nodes: page.nodes,
    text,
    truncated: page.truncated || rendered < page.nodes.length,
  };
}

/** Fields whose change is semantically interesting; rect is excluded so a scroll or resize does not report the whole tree as changed. */
function signature(n: SnapNode): string {
  return [n.role, n.name, n.testId ?? "", n.disabled, n.dragRegion, JSON.stringify(n.state)].join("|");
}

export function diffSnapshots(
  a: Snapshot,
  b: Snapshot,
): { added: SnapNode[]; removed: SnapNode[]; changed: Array<{ before: SnapNode; after: SnapNode }> } {
  const byFp = (s: Snapshot): Map<string, SnapNode> => {
    const m = new Map<string, SnapNode>();
    for (const n of s.nodes) if (!m.has(n.fp)) m.set(n.fp, n);
    return m;
  };
  const am = byFp(a);
  const bm = byFp(b);
  const added = b.nodes.filter((n) => !am.has(n.fp));
  const removed = a.nodes.filter((n) => !bm.has(n.fp));
  const changed: Array<{ before: SnapNode; after: SnapNode }> = [];
  for (const [fp, before] of am) {
    const after = bm.get(fp);
    if (after && signature(before) !== signature(after)) changed.push({ before, after });
  }
  return { added, removed, changed };
}

/**
 * Installs the mutation-revision and console probes the action pipeline depends on.
 *
 * Called once per bridge (session start, bridge rebuild, app reconnect) so an action taken
 * before the first `ui_snapshot` still settles against a real counter. Idempotent in the page,
 * so calling it again after a snapshot already installed them is a no-op.
 */
export function ensureInstrumentation(bridge: BridgeLike): Promise<InstrumentReport> {
  return bridge.execute<InstrumentReport>(
    "instrument.install",
    INSTRUMENT_SCRIPT as never,
    [],
    // Writes page globals but touches no application state, and a retry simply re-reports
    // the live state — "read-only" here means "safe to re-send", as for the snapshot script.
    { readOnly: true },
  );
}

function finiteOr(v: unknown, fallback: number | null): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : fallback;
}

function idleCamera(v: unknown): IdleCamera | null {
  if (v === null || typeof v !== "object") return null;
  const c = v as Record<string, unknown>;
  const keys = ["x", "y", "z", "tx", "ty", "tz", "distance"] as const;
  const out = {} as Record<string, number>;
  for (const k of keys) {
    const n = finiteOr(c[k], null);
    if (n === null) return null;
    out[k] = n;
  }
  return out as unknown as IdleCamera;
}

/**
 * The webview is a system boundary, so an unusable field is turned into `null` here rather
 * than compared as `undefined` later: `null` is the ONE encoding of "this signal is
 * unavailable", and the settle path warns on it instead of quietly treating it as idle.
 */
function normalizeIdle(raw: unknown): IdleReading {
  const r = (raw === null || typeof raw !== "object" ? {} : raw) as Record<string, unknown>;
  return {
    rev: finiteOr(r.rev, -1) ?? -1,
    regenBusy: finiteOr(r.regenBusy, null),
    geometryPending: typeof r.geometryPending === "boolean" ? r.geometryPending : null,
    documentRevision: finiteOr(r.documentRevision, null),
    frames: finiteOr(r.frames, null),
    camera: idleCamera(r.camera ?? null),
  };
}

/**
 * One reading of the CAD/WebGL idle signals — what `settle()` and the `worker_idle` /
 * `render_idle` / `camera_stable` waits compare. Also removes the `?vpdebug` origin pill
 * from the viewport the first time it finds it; see `idleScript.ts`.
 */
export async function readIdle(bridge: BridgeLike): Promise<IdleReading> {
  const raw = await bridge.execute<IdleReading>(
    "ui_idle",
    IDLE_SCRIPT as never,
    [],
    // Writes one page global (the chrome-removed latch) and unregisters a debug-only
    // overlay item; touches no application state, so it is safe to re-send.
    { readOnly: true },
  );
  return normalizeIdle(raw);
}

/** `rev === -1` means the probes are not installed — the page was never instrumented. */
export function readRevision(bridge: BridgeLike): Promise<{ rev: number; lastMutationAt: number; now: number }> {
  return bridge.execute<{ rev: number; lastMutationAt: number; now: number }>(
    "ui_revision",
    (() => {
      const r = (window as unknown as AgentWindow).__tauriAgentRev;
      return {
        rev: r ? r.rev : -1,
        lastMutationAt: r ? r.lastMutationAt : 0,
        now: performance.now(),
      };
    }) as never,
    [],
    { readOnly: true },
  );
}
