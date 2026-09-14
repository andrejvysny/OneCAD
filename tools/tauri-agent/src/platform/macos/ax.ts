/**
 * macOS accessibility reads, through the same Swift helper as input and window enumeration.
 *
 * AX LOCATES; CGEvent IS THE ONLY ACCEPTANCE-GRADE ACTUATOR. Accessibility finds what the
 * WebView does not own — a native open/save panel, the app menu, a sheet, a permission dialog,
 * the title-bar buttons — and in a foreground session the native input verbs act on it as a user
 * would.
 *
 * Three verbs here DO act: `press`, `setValue` and `menuPress`. They exist because an
 * interaction:"background" session has no native input at all, and native chrome would otherwise
 * be unreachable from it. They are not physical input — nothing moves, nothing is pressed, the
 * app need not be in front — so every result they produce is labelled `mode:"accessibility"` and
 * none of them closes a real-user acceptance claim.
 *
 * COORDINATES. Every bounds, centre and display rect that comes back is ALREADY a global display
 * point, top-left origin, y down — the space `click` and `move` consume. That was measured, not
 * assumed: an `AXWindow`'s `AXPosition`/`AXSize` read byte-identical to the same window's
 * `kCGWindowBounds`. Nothing here converts a coordinate, and nothing downstream should either.
 *
 * This file only COERCES the helper's replies into typed rows, exactly as `MacWindows` does for
 * `windows`. The refusal ladder lives in the helper (`ax_point`) and in `src/native/resolve.ts`;
 * a value AX did not expose is carried through as `null` rather than defaulted, because "the
 * application did not say" and "the application said false" are different facts.
 */
import { AgentError } from "../../errors.ts";
import type { Pt, Rect } from "../../geometry/types.ts";
import type {
  AxBlocker,
  AxFindOpts,
  AxMenuItem,
  AxMenuKey,
  AxMenuResult,
  AxModalResult,
  AxNode,
  AxPointInfo,
  AxSnapshotOpts,
  AxSnapshotResult,
  AxWalkResult,
  AxWindowId,
  AxWindowInfo,
  AxWindowsResult,
  AxMenuPressResult,
  AxPressResult,
  AxSetValueResult,
  ModKey,
  NativeAccessibility,
} from "../adapter.ts";
import type { HelperClient } from "./input.ts";
import { getHelperClient } from "./input.ts";

type Raw = Record<string, unknown>;

/** The helper's rect shape. Its `w`/`h` spelling is why every rect crosses `toRect`. */
interface RawRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export class MacAccessibility implements NativeAccessibility {
  private readonly client: HelperClient;

  constructor(client: HelperClient = getHelperClient()) {
    this.client = client;
  }

  async windows(pid: number): Promise<AxWindowsResult> {
    const r = await this.client.request("ax_windows", { pid });
    return {
      pid: num(r.pid, pid),
      privateWindowIdApi: r.privateWindowIdApi === true,
      windows: rows(r.windows).map(toWindowInfo),
    };
  }

  async snapshot(pid: number, opts: AxSnapshotOpts = {}): Promise<AxSnapshotResult> {
    const r = await this.client.request("ax_snapshot", {
      pid,
      ...(opts.window === undefined ? {} : { window: opts.window }),
      ...(opts.maxNodes === undefined ? {} : { maxNodes: opts.maxNodes }),
      ...(opts.root === undefined ? {} : { root: opts.root }),
    });
    const window = r.window;
    return {
      ...toWalk(r, pid),
      windowSource: str(r.windowSource) ?? "",
      window: isRaw(window) ? toWindowInfo(window) : null,
    };
  }

  /**
   * A find is a SNAPSHOT: it mints a fresh generation, so every ref handed out before it —
   * including refs from the last `snapshot` — is stale afterwards. Refs that silently survive
   * a re-walk are the bug generation scoping exists to prevent.
   */
  async find(pid: number, opts: AxFindOpts): Promise<AxWalkResult> {
    const r = await this.client.request("ax_find", {
      pid,
      ...(opts.role === undefined ? {} : { role: opts.role }),
      ...(opts.title === undefined ? {} : { title: opts.title }),
      ...(opts.value === undefined ? {} : { value: opts.value }),
    });
    return toWalk(r, pid);
  }

  async point(ref: string): Promise<AxPointInfo> {
    const r = await this.client.request("ax_point", { ref });
    return {
      ref: str(r.ref) ?? ref,
      generation: num(r.generation, 0),
      pid: num(r.pid, 0),
      role: str(r.role),
      subrole: str(r.subrole),
      title: str(r.title),
      enabled: flag(r.enabled),
      focused: flag(r.focused),
      // The helper refuses an empty frame and a centre on no display, so a successful reply
      // always carries all three. A missing one is a broken helper, not a point at the origin:
      // defaulting would hand back (0, 0), which is a real and clickable screen location.
      bounds: required(toRect(r.bounds), "bounds", ref),
      center: required(toPt(r.center), "center", ref),
      display: required(toRect(r.display), "display", ref),
      frontmost: r.frontmost === true,
      window: toWindowId(r.windowId, r.windowIdSource),
      windowTitle: str(r.windowTitle),
    };
  }

  async focusedWindow(pid: number): Promise<AxWindowInfo | null> {
    const r = await this.client.request("ax_focused_window", { pid });
    return isRaw(r.window) ? toWindowInfo(r.window) : null;
  }

  async modal(pid: number): Promise<AxModalResult> {
    const r = await this.client.request("ax_modal", { pid });
    const blockers: AxBlocker[] = rows(r.blockers).map((b) => ({
      ...toWindowInfo(b),
      reason: str(b.reason) ?? "",
    }));
    return { pid: num(r.pid, pid), modal: r.modal === true, blockers };
  }

  async press(ref: string, action?: string, acceptDisruption?: "yes"): Promise<AxPressResult> {
    const r = await this.client.request("ax_press", {
      ref,
      ...(action === undefined ? {} : { action }),
      ...(acceptDisruption === undefined ? {} : { acceptDisruption }),
    });
    return { ref: str(r.ref) ?? ref, action: str(r.action) ?? "AXPress", role: str(r.role) ?? null };
  }

  async setValue(ref: string, value: string): Promise<AxSetValueResult> {
    const r = await this.client.request("ax_set_value", { ref, value });
    return {
      ref: str(r.ref) ?? ref,
      role: str(r.role) ?? null,
      requested: str(r.requested) ?? value,
      value: str(r.value) ?? null,
      matched: r.matched === true,
    };
  }

  async menuPress(pid: number, path: string[]): Promise<AxMenuPressResult> {
    const r = await this.client.request("ax_menu_press", { pid, path });
    return { pid: num(r.pid, pid), path: rowsOfStrings(r.path, path) };
  }

  async menu(pid: number): Promise<AxMenuResult> {
    const r = await this.client.request("ax_menu", { pid });
    return {
      pid: num(r.pid, pid),
      hasMenuBar: r.hasMenuBar === true,
      items: rows(r.items).map(toMenuItem),
      total: num(r.total, 0),
      truncated: r.truncated === true,
      stopReason: str(r.stopReason) ?? "",
    };
  }
}

/**
 * A geometry field `ax_point` promises on every successful reply. Absent means the helper and
 * this file disagree about the protocol, which is an agent fault — and the one thing it must
 * not become is a plausible coordinate.
 */
function required<T>(value: T | null, field: string, ref: string): T {
  if (value === null) {
    throw new AgentError("INTERNAL", `ax_point reply for '${ref}' has no usable '${field}'`, {
      details: { verb: "ax_point", ref, field },
    });
  }
  return value;
}

/** The physical modifiers `AXMenuItemCmdModifiers` can name; `Fn` has no Carbon bit. */
const MENU_MODS = new Set<string>(["Command", "Control", "Option", "Shift"]);

function isRaw(v: unknown): v is Raw {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function rows(v: unknown): Raw[] {
  return Array.isArray(v) ? v.filter(isRaw) : [];
}

function str(v: unknown): string | null {
  return typeof v === "string" ? v : null;
}

/** `null` is the helper's "AX did not expose this attribute", which is not `false`. */
function flag(v: unknown): boolean | null {
  return typeof v === "boolean" ? v : null;
}

function num(v: unknown, fallback: number): number {
  return typeof v === "number" && Number.isFinite(v) ? v : fallback;
}

function toRect(v: unknown): Rect | null {
  if (!isRaw(v)) return null;
  const r = v as unknown as RawRect;
  if (![r.x, r.y, r.w, r.h].every((n) => typeof n === "number" && Number.isFinite(n))) return null;
  return { x: r.x, y: r.y, width: r.w, height: r.h };
}

function toPt(v: unknown): Pt | null {
  if (!isRaw(v)) return null;
  const { x, y } = v as { x: unknown; y: unknown };
  if (typeof x !== "number" || typeof y !== "number" || !Number.isFinite(x) || !Number.isFinite(y)) return null;
  return { x, y };
}

/**
 * The one place a helper `windowId` becomes a typed id.
 *
 * A null id means the helper could NOT tell which window this is — the private symbol was
 * unavailable and two windows of the app shared a rect and a title, which happens routinely.
 * It becomes `{ known: false }`, never a number, so no caller can read "we could not tell"
 * as an id and gate on it.
 */
function toWindowId(id: unknown, source: unknown): AxWindowId {
  const text = str(source) ?? "none";
  if (typeof id === "number" && Number.isFinite(id)) return { known: true, id, source: text };
  return { known: false, source: text };
}

function toWindowInfo(w: Raw): AxWindowInfo {
  return {
    role: str(w.role),
    subrole: str(w.subrole),
    title: str(w.title),
    bounds: toRect(w.bounds),
    main: w.main === true,
    modal: w.modal === true,
    focused: w.focused === true,
    window: toWindowId(w.windowId, w.windowIdSource),
  };
}

function toNode(n: Raw): AxNode {
  const value = n.value;
  return {
    ref: str(n.ref) ?? "",
    role: str(n.role),
    subrole: str(n.subrole),
    title: str(n.title),
    value: typeof value === "string" || typeof value === "number" ? value : null,
    enabled: flag(n.enabled),
    focused: flag(n.focused),
    bounds: toRect(n.bounds),
    depth: num(n.depth, 0),
    actions: Array.isArray(n.actions) ? n.actions.filter((a): a is string => typeof a === "string") : [],
  };
}

function toWalk(r: Raw, pid: number): AxWalkResult {
  return {
    pid: num(r.pid, pid),
    generation: num(r.generation, 0),
    nodes: rows(r.nodes).map(toNode),
    total: num(r.total, 0),
    truncated: r.truncated === true,
    stopReason: str(r.stopReason) ?? "",
  };
}

function toMenuKey(v: unknown): AxMenuKey | null {
  if (!isRaw(v)) return null;
  const mods = Array.isArray(v.mods) ? v.mods : [];
  return {
    char: str(v.char),
    virtualKey: typeof v.virtualKey === "number" ? v.virtualKey : null,
    glyph: typeof v.glyph === "number" ? v.glyph : null,
    modifiersRaw: typeof v.modifiersRaw === "number" ? v.modifiersRaw : null,
    // Validated against the physical modifier names rather than cast: a name the input verbs
    // could not send must not reach a caller typed as one they can.
    mods: mods.filter((m): m is ModKey => typeof m === "string" && MENU_MODS.has(m)),
  };
}

function toMenuItem(i: Raw): AxMenuItem {
  return {
    title: str(i.title),
    role: str(i.role),
    enabled: flag(i.enabled),
    bounds: toRect(i.bounds),
    depth: num(i.depth, 0),
    path: Array.isArray(i.path) ? i.path.filter((p): p is string => typeof p === "string") : [],
    key: toMenuKey(i.key),
  };
}

/** Echoed menu path; falls back to what was asked for rather than inventing an empty path. */
function rowsOfStrings(v: unknown, fallback: string[]): string[] {
  if (!Array.isArray(v)) return fallback;
  const out = v.filter((x): x is string => typeof x === "string");
  return out.length === v.length ? out : fallback;
}
