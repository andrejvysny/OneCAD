/**
 * "Has the app gone quiet?" — one frame, then a quiet window over the whole idle tuple.
 *
 * A DOM-mutation revision alone is far too weak here. A viewport ORBIT changes camera
 * matrices and repaints WebGL with zero DOM mutations, so a revision-only settle declares
 * the action finished while rendering is still in progress; and a click on Extrude runs
 * frontend → Rust → the OCCT worker → mesh generation → a Three.js upload, none of which
 * DOM quietness reports on. So the quiet window requires `(rev, regenBusy, geometryPending,
 * documentRevision, frames)` all unchanged AND `regenBusy === 0` AND
 * `geometryPending === false`: the orbit settles on `frames`, and an extrude cannot settle
 * while a regen is in flight.
 *
 * A settle timeout is never fatal: the input WAS posted, so refusing the envelope would
 * hide evidence the caller needs. A long regen legitimately keeps mutating for seconds;
 * the caller is told with a warning — one that NAMES the member still moving — and pointed
 * at `wait_for` instead.
 *
 * That whole tuple describes the WebView, so it is the wrong instrument for a native target:
 * a Save panel opening changes nothing in the DOM. `settleNative`, at the foot of this file,
 * watches the accessibility subtree instead, on the same frameMs/quietMs/timeoutMs knobs and
 * with the same two properties — a timeout warns rather than failing, and it names what moved.
 */
import type { AxNode, AxSnapshotResult, NativeAccessibility } from "../../platform/adapter.ts";
import type { IdleReading } from "../../semantic/idleScript.ts";
import { readIdle } from "../../semantic/snapshot.ts";
import type { BridgeLike } from "../../semantic/webdriver.ts";

/**
 * Which members of the idle tuple take part in the quiet window.
 *
 * `rev` is deliberately NOT switchable: it is the baseline signal and the -1 guard below
 * is what stops an uninstrumented page reporting a false quiet.
 */
export interface SettleSignals {
  regenBusy: boolean;
  geometryPending: boolean;
  documentRevision: boolean;
  frames: boolean;
}

export interface SettleConfig {
  frameMs: number;
  quietMs: number;
  timeoutMs: number;
  signals: SettleSignals;
}

export interface SettleOutcome {
  settled: boolean;
  /** The DOM revision the quiet window ended on. ABSENT after a native settle, which reads no page. */
  afterRevision?: number;
  warning?: string;
  /** The last reading the quiet window took. */
  idle?: IdleReading;
  /** Members still moving or still busy when it gave up, by name. */
  moving?: string[];
  /** Enabled members this build does not publish — the settle proved less than it looks. */
  unavailable?: string[];
  /** The accessibility reading a native settle ended on. */
  ax?: AxSettleBrief;
}

export type Sleep = (ms: number) => Promise<void>;

const realSleep: Sleep = (ms) => new Promise((r) => setTimeout(r, ms));

type Member = "rev" | "regenBusy" | "geometryPending" | "documentRevision" | "frames";

const MEMBERS: readonly Member[] = ["rev", "regenBusy", "geometryPending", "documentRevision", "frames"];

function watched(sig: SettleSignals, m: Member): boolean {
  return m === "rev" ? true : sig[m];
}

function valueOf(r: IdleReading, m: Member): number | boolean | null {
  return m === "rev" ? r.rev : r[m];
}

/**
 * Members whose value differs between two readings, formatted `name from→to`.
 *
 * A member this build does not publish reads `null` in BOTH samples, so it compares equal
 * and never blocks a settle — it is reported through `unavailableMembers` instead, because
 * blocking on an absent signal would hang every action on a build that lacks it.
 */
function movingMembers(a: IdleReading, b: IdleReading, sig: SettleSignals): Array<{ member: Member; detail: string }> {
  const out: Array<{ member: Member; detail: string }> = [];
  for (const m of MEMBERS) {
    if (!watched(sig, m)) continue;
    const from = valueOf(a, m);
    const to = valueOf(b, m);
    if (from !== to) out.push({ member: m, detail: `${m} ${String(from)}→${String(to)}` });
  }
  return out;
}

/** Members that are stable but say the app is NOT idle. A null is unknown, so it is not busy. */
function busyMembers(r: IdleReading, sig: SettleSignals): Array<{ member: Member; detail: string }> {
  const out: Array<{ member: Member; detail: string }> = [];
  if (sig.regenBusy && r.regenBusy !== null && r.regenBusy > 0) {
    out.push({ member: "regenBusy", detail: `regenBusy=${r.regenBusy}` });
  }
  if (sig.geometryPending && r.geometryPending === true) {
    out.push({ member: "geometryPending", detail: "geometryPending=true" });
  }
  return out;
}

/** Enabled members this build does not publish. Never silently ignored — always reported. */
function unavailableMembers(r: IdleReading, sig: SettleSignals): Member[] {
  return MEMBERS.filter((m) => m !== "rev" && watched(sig, m) && valueOf(r, m) === null);
}

/**
 * `readRevision` answers -1 when the page carries no mutation observer. Two of those compare
 * equal, so an uninstrumented page would "settle" instantly having watched nothing at all.
 * The session installs the probes at every bridge construction, so reaching this means the
 * install failed — degraded, but the caller has to be told rather than handed a false quiet.
 */
export const UNINSTRUMENTED_WARNING =
  "the page carries no mutation observer (revision -1), so the quiet-window check proved NOTHING; " +
  "the action WAS sent — confirm the outcome with ui_snapshot, and see session_status.instrumentWarning";

/**
 * A settle that watched fewer signals than it was asked to is reported, never swallowed:
 * silently degrading to DOM-only settling is the defect this module exists to fix.
 */
export function signalsUnavailableWarning(missing: readonly string[]): string {
  const them = missing.length === 1 ? "that signal" : "those signals";
  return (
    `the quiet window could not watch ${missing.join(", ")} — this build does not publish ${them} ` +
    "(frames needs the app launched with ?vpdebug; the store signals need a dev build), so a " +
    "WebGL-only repaint or an in-flight regen could still have been running; the action WAS sent — " +
    "confirm the outcome with ui_snapshot"
  );
}

function timeoutWarning(timeoutMs: number, rev: number, parts: readonly string[]): string {
  return (
    `the UI was still changing ${timeoutMs}ms after the input — ${parts.join("; ")} (DOM revision ${rev}); ` +
    'the action WAS sent — confirm the outcome with ui_snapshot, or wait_for {kind:"revision_stable"} / ' +
    '{kind:"worker_idle"} / {kind:"render_idle"}'
  );
}

export async function settle(
  bridge: BridgeLike,
  cfg: SettleConfig,
  sleep: Sleep = realSleep,
): Promise<SettleOutcome> {
  const deadline = Date.now() + cfg.timeoutMs;
  await sleep(cfg.frameMs);
  let last = await readIdle(bridge);
  for (;;) {
    await sleep(cfg.quietMs);
    const cur = await readIdle(bridge);
    if (cur.rev === -1) {
      return { settled: false, afterRevision: cur.rev, idle: cur, warning: UNINSTRUMENTED_WARNING };
    }
    const moving = movingMembers(last, cur, cfg.signals);
    const busy = busyMembers(cur, cfg.signals);
    if (moving.length === 0 && busy.length === 0) {
      const missing = unavailableMembers(cur, cfg.signals);
      return {
        settled: true,
        afterRevision: cur.rev,
        idle: cur,
        ...(missing.length === 0
          ? {}
          : { unavailable: missing, warning: signalsUnavailableWarning(missing) }),
      };
    }
    last = cur;
    if (Date.now() >= deadline) {
      const parts: string[] = [];
      if (moving.length > 0) parts.push(`still moving: ${moving.map((m) => m.detail).join(", ")}`);
      if (busy.length > 0) parts.push(`still busy: ${busy.map((m) => m.detail).join(", ")}`);
      const missing = unavailableMembers(cur, cfg.signals);
      if (missing.length > 0) parts.push(`not watched (unavailable): ${missing.join(", ")}`);
      return {
        settled: false,
        afterRevision: cur.rev,
        idle: cur,
        moving: [...new Set([...moving, ...busy].map((m) => m.member))],
        ...(missing.length === 0 ? {} : { unavailable: missing }),
        warning: timeoutWarning(cfg.timeoutMs, cur.rev, parts),
      };
    }
  }
}

/* ------------------------------------------------------------------------------------------ *
 * The native lane.
 *
 * A DOM-mutation revision is meaningless for a native target: clicking Save in an NSSavePanel
 * changes nothing in the WebView, so every member of the idle tuple is stable from the first
 * read and the quiet window reports a settle having watched the wrong surface entirely. The
 * native settle watches what actually moves — the accessibility subtree of the window that
 * currently has focus — and goes quiet when two consecutive walks hash the same.
 *
 * The two properties the DOM lane has are kept exactly: a timeout is a WARNING and never fatal
 * (the input was already posted), and the warning NAMES what was still moving.
 * ------------------------------------------------------------------------------------------ */

/** What a native settle observed, carried in the envelope so a report can cite it. */
export interface AxSettleBrief {
  /** The AX ref generation the last walk minted. Every ref older than this is stale. */
  generation: number;
  nodeCount: number;
  hash: string;
  /** "root" | "requested" | "focused" | "main" | "only" — how the walked window was chosen. */
  windowSource: string;
  windowTitle: string | null;
  truncated: boolean;
  stopReason: string;
}

export interface AxSettleDeps {
  ax: Pick<NativeAccessibility, "snapshot">;
  pid: number;
  /** Walk cap. An AX read costs ~0.26 ms per node and this runs once per quiet window. */
  maxNodes?: number;
  /**
   * Called with every walk. `ax_snapshot` BUMPS the helper's ref generation, so a caller that
   * keeps an AX ref pool must be handed each walk or its pool falls a generation behind and
   * refuses refs that are perfectly live.
   */
  onWalk?: (walk: AxSnapshotResult) => void;
}

export const AX_SETTLE_MAX_NODES = 300;

/** Naming is capped so one slow-animating panel cannot produce a thousand-line warning. */
const NAMED_CHANGES = 3;

interface AxReading extends AxSettleBrief {
  /** One string per node, in walk order; the diff that names what moved is taken over these. */
  fingerprints: string[];
}

/**
 * Everything about a node that a user could see change, rounded to whole points so sub-pixel
 * jitter is not reported as movement. `ref` is deliberately EXCLUDED: every walk mints a new
 * generation, so including it would make two identical trees hash differently and nothing would
 * ever settle.
 */
function axFingerprint(n: AxNode): string {
  const b = n.bounds;
  const where = b === null ? "no-frame" : `@${Math.round(b.x)},${Math.round(b.y)} ${Math.round(b.width)}x${Math.round(b.height)}`;
  const role = n.subrole === null ? String(n.role) : `${String(n.role)}/${n.subrole}`;
  const value = n.value === null ? "" : ` =${String(n.value).slice(0, 60)}`;
  return `${role} ${JSON.stringify(n.title ?? "")}${value} ${where} enabled=${String(n.enabled)} focused=${String(n.focused)} d${n.depth}`;
}

/** FNV-1a over the fingerprints; only equality is ever asked of it. */
function hashOf(fingerprints: readonly string[]): string {
  let h = 0x811c9dc5;
  for (const fp of fingerprints) {
    for (let i = 0; i < fp.length; i += 1) {
      h ^= fp.charCodeAt(i);
      h = Math.imul(h, 0x01000193) >>> 0;
    }
    h ^= 0x0a;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h.toString(16).padStart(8, "0");
}

async function readAx(deps: AxSettleDeps): Promise<AxReading> {
  const walk = await deps.ax.snapshot(deps.pid, { maxNodes: deps.maxNodes ?? AX_SETTLE_MAX_NODES });
  deps.onWalk?.(walk);
  const fingerprints = walk.nodes.map(axFingerprint);
  return {
    generation: walk.generation,
    nodeCount: walk.nodes.length,
    hash: hashOf(fingerprints),
    windowSource: walk.windowSource,
    windowTitle: walk.window === null ? null : walk.window.title,
    truncated: walk.truncated,
    stopReason: walk.stopReason,
    fingerprints,
  };
}

function briefOf(r: AxReading): AxSettleBrief {
  const { fingerprints: _fingerprints, ...brief } = r;
  return brief;
}

/** Multiset difference: the entries of `a` that `b` does not also have, counting duplicates. */
function missingFrom(a: readonly string[], b: readonly string[]): string[] {
  const left = new Map<string, number>();
  for (const fp of b) left.set(fp, (left.get(fp) ?? 0) + 1);
  const out: string[] = [];
  for (const fp of a) {
    const n = left.get(fp) ?? 0;
    if (n > 0) left.set(fp, n - 1);
    else out.push(fp);
  }
  return out;
}

function sample(items: readonly string[]): string {
  const head = items.slice(0, NAMED_CHANGES).join("; ");
  return items.length > NAMED_CHANGES ? `${head} (+${items.length - NAMED_CHANGES} more)` : head;
}

/**
 * What changed between two walks, in words. An element that MOVED shows up as both a
 * disappearance at the old rect and an appearance at the new one, which is exactly the
 * diagnosis a caller needs for a sheet that is still sliding in.
 */
function axMoving(a: AxReading, b: AxReading): string[] {
  if (a.hash === b.hash) return [];
  const parts: string[] = [];
  if (a.windowSource !== b.windowSource) parts.push(`the walked window changed (${a.windowSource}→${b.windowSource})`);
  if (a.windowTitle !== b.windowTitle) {
    parts.push(`window title ${JSON.stringify(a.windowTitle)}→${JSON.stringify(b.windowTitle)}`);
  }
  if (a.nodeCount !== b.nodeCount) parts.push(`node count ${a.nodeCount}→${b.nodeCount}`);
  const gone = missingFrom(a.fingerprints, b.fingerprints);
  const born = missingFrom(b.fingerprints, a.fingerprints);
  if (born.length > 0) parts.push(`now present: ${sample(born)}`);
  if (gone.length > 0) parts.push(`no longer present: ${sample(gone)}`);
  if (parts.length === 0) parts.push("the accessibility subtree hash changed");
  return parts;
}

function nativeTimeoutWarning(timeoutMs: number, r: AxReading, parts: readonly string[]): string {
  const where = r.windowTitle === null ? `the ${r.windowSource} window` : `the ${r.windowSource} window "${r.windowTitle}"`;
  return (
    `the native UI was still changing ${timeoutMs}ms after the input — ${parts.join("; ")} ` +
    `(${r.nodeCount} accessibility nodes in ${where}); the input WAS sent — confirm the outcome with ` +
    "native_snapshot, native_modal or ui_screenshot"
  );
}

/**
 * The accessibility tree could not be read at all. Not fatal, for the same reason a timeout is
 * not: the input was already posted, and refusing the envelope would hide the evidence.
 */
export function axUnavailableWarning(message: string): string {
  return (
    `the native settle could not read the accessibility tree (${message}), so it proved NOTHING about ` +
    "whether the native surface went quiet; the input WAS sent — confirm the outcome with native_snapshot or ui_screenshot"
  );
}

/** A walk that hit its cap hashed only part of the tree, so the quiet it found is partial. */
export function axTruncatedWarning(r: AxSettleBrief): string {
  return (
    `the native settle hashed only the first ${r.nodeCount} accessibility nodes (stopReason ${r.stopReason}), ` +
    "so anything deeper in the window could still have been changing; the input WAS sent — confirm the outcome with native_snapshot"
  );
}

/** Said when the session cannot name the app process, which is what an AX walk is addressed by. */
export const NATIVE_SETTLE_NO_PID_WARNING =
  "the native settle did not run: this session does not know the app's pid, so the accessibility tree " +
  "could not be walked; the input WAS sent — confirm the outcome with ui_screenshot";

export async function settleNative(
  deps: AxSettleDeps,
  cfg: Pick<SettleConfig, "frameMs" | "quietMs" | "timeoutMs">,
  sleep: Sleep = realSleep,
): Promise<SettleOutcome> {
  const deadline = Date.now() + cfg.timeoutMs;
  let last: AxReading;
  try {
    await sleep(cfg.frameMs);
    last = await readAx(deps);
  } catch (e) {
    return { settled: false, warning: axUnavailableWarning(messageOf(e)) };
  }
  for (;;) {
    await sleep(cfg.quietMs);
    let cur: AxReading;
    try {
      cur = await readAx(deps);
    } catch (e) {
      return { settled: false, ax: briefOf(last), warning: axUnavailableWarning(messageOf(e)) };
    }
    const moving = axMoving(last, cur);
    if (moving.length === 0) {
      const brief = briefOf(cur);
      return {
        settled: true,
        ax: brief,
        ...(brief.truncated ? { warning: axTruncatedWarning(brief) } : {}),
      };
    }
    last = cur;
    if (Date.now() >= deadline) {
      return {
        settled: false,
        ax: briefOf(cur),
        moving,
        warning: nativeTimeoutWarning(cfg.timeoutMs, cur, moving),
      };
    }
  }
}

function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
