import { AgentError } from "../errors.ts";
import { globalToCss, rectCenter, rectIsFinite, rectStable } from "../geometry/mapping.ts";
import type { Pt, Rect, Space, WindowGeom } from "../geometry/types.ts";
import { checkInPage, dragRegionAtPoint } from "./checkScript.ts";
import type { InputCheck } from "./checkScript.ts";
import type { SnapNode } from "./snapshot.ts";
import type { BridgeLike } from "./webdriver.ts";

export type Target =
  | { ref: string }
  | { testId: string }
  | { role: string; name?: string }
  | { text: string }
  | { css: string }
  | { point: Pt & { space: Space } };

export type ResolveSource = "ref" | "role" | "testId" | "text" | "css" | "point";

export interface Resolved {
  node?: SnapNode;
  css: Pt;
  rect?: Rect;
  source: ResolveSource;
  /** The point sits on a drag region: a click is fine, a drag would move the window. */
  dragRegion?: boolean;
}

export interface ResolveOpts {
  offset?: Pt;
  /** Re-verify identity, stability and hit-testing in the page before input. */
  forInput?: boolean;
  /** A drag must not start on a `data-tauri-drag-region` element — it would move the window. */
  forDrag?: boolean;
  /** Required only for `{point:{space:"global"}}`; the Resolver holds no geometry of its own. */
  geom?: WindowGeom;
}

export interface RefStore {
  get(ref: string): SnapNode | undefined;
  all(): SnapNode[];
}

export function refStoreFrom(nodes: SnapNode[]): RefStore {
  const byRef = new Map(nodes.map((n) => [n.ref, n]));
  return { get: (ref) => byRef.get(ref), all: () => nodes };
}

/** `selector` is internal: the page-side handle used to re-find the element. */
interface Located extends Resolved {
  selector?: string;
}

function brief(n: SnapNode): Record<string, unknown> {
  return { ref: n.ref, role: n.role, name: n.name, testId: n.testId };
}

function fromNode(node: SnapNode, source: ResolveSource, off: Pt | undefined): Located {
  return {
    node,
    css: pointOf(node.rect, off, brief(node)),
    rect: node.rect,
    source,
    selector: node.css,
  };
}

function ambiguous(kind: string, needle: string, matches: SnapNode[]): AgentError {
  return new AgentError("INVALID_TARGET", `${kind} "${needle}" matches ${matches.length} elements`, {
    remediation: "Pass the `ref` of the intended element, or narrow with role + testId.",
    details: { kind, needle, candidates: matches.slice(0, 12).map(brief) },
  });
}

function notFound(kind: string, needle: string, pool: SnapNode[]): AgentError {
  return new AgentError("ELEMENT_NOT_FOUND", `no element for ${kind} "${needle}"`, {
    remediation: "Take a fresh ui_snapshot; refs and names are only valid for the snapshot that produced them.",
    details: { kind, needle, snapshotNodes: pool.length },
  });
}

/**
 * The page-side handle for RE-FINDING the element, or `null` when re-finding would
 * be a guess.
 *
 * `node.css` is POSITIONAL (`tag:nth-of-type(n)`). Once a row is removed it matches
 * the sibling that slid into that slot, and since the fingerprint is positional
 * too, the impostor passes the identity check — the agent clicks the wrong row and
 * reports success. So a held `ref` never falls back (detached is staleness), and a
 * name-based target falls back only through an identity a path cannot alias.
 */
function fallbackSelector(located: Located): string | null {
  if (located.source === "ref") return null;
  if (located.source === "css") return located.selector ?? null;
  const node = located.node;
  if (!node) return null;
  if (node.testId !== undefined && node.testId !== "") {
    return `[data-testid=${JSON.stringify(node.testId)}]`;
  }
  const last = node.css.split(" > ").pop() ?? "";
  return last.includes("#") ? last : null;
}

/** Refuses before a non-finite rect can become a plausible-looking coordinate. */
function pointOf(rect: Rect, off: Pt | undefined, where: Record<string, unknown>): Pt {
  if (!rectIsFinite(rect)) {
    throw new AgentError("INVALID_TARGET", "the element's rect has non-finite components", {
      remediation: "Re-take the snapshot; the page reported a rect the bridge could not carry.",
      details: { ...where, rect },
    });
  }
  return rectCenter(rect, off);
}

/** exact → case-insensitive → case-insensitive prefix; the first non-empty tier wins. */
function byNameTiers(pool: SnapNode[], name: string, includeSubstring: boolean): SnapNode[] {
  const lower = name.toLowerCase();
  const tiers = [
    pool.filter((n) => n.name === name),
    pool.filter((n) => n.name.toLowerCase() === lower),
    pool.filter((n) => n.name.toLowerCase().startsWith(lower)),
  ];
  if (includeSubstring) tiers.push(pool.filter((n) => n.name.toLowerCase().includes(lower)));
  return tiers.find((t) => t.length > 0) ?? [];
}

function one(matches: SnapNode[], kind: string, needle: string, pool: SnapNode[]): SnapNode {
  if (matches.length === 0) throw notFound(kind, needle, pool);
  if (matches.length > 1) throw ambiguous(kind, needle, matches);
  return matches[0] as SnapNode;
}

/**
 * Turns a `Target` into one concrete point, or refuses.
 *
 * The refusal path is the point of this class. A stale ref, an element that is
 * still animating, or an element hidden behind an overlay all produce a plausible
 * looking coordinate, and the native backend will happily click it — in the wrong
 * place, in the user's real session. Every one of those is a named error instead.
 */
export class Resolver {
  readonly #bridge: BridgeLike;
  readonly #refs: RefStore;

  constructor(bridge: BridgeLike, refs: RefStore) {
    this.#bridge = bridge;
    this.#refs = refs;
  }

  async resolve(target: Target, opts: ResolveOpts = {}): Promise<Resolved> {
    const located = await this.#locate(target, opts);
    if (!opts.forInput && !opts.forDrag) {
      return { node: located.node, css: located.css, rect: located.rect, source: located.source };
    }
    // A raw point has no element, but it still lands somewhere: the drag-region
    // gate is the one check it cannot skip, or `pointer_drag` moves the window.
    if (located.source === "point") return this.#pointVerdict(located, opts);
    const check = await this.#check(located, opts);
    return this.#verdict(located, check, opts);
  }

  /** Priority: ref → role+name → testId → text → css → point. */
  async #locate(target: Target, opts: ResolveOpts): Promise<Located> {
    const pool = this.#refs.all();
    const off = opts.offset;
    if ("ref" in target) {
      const node = this.#refs.get(target.ref);
      if (!node) throw notFound("ref", target.ref, pool);
      return fromNode(node, "ref", off);
    }
    if ("role" in target) {
      const sameRole = pool.filter((n) => n.role === target.role);
      const matches = target.name === undefined ? sameRole : byNameTiers(sameRole, target.name, false);
      const needle = target.name === undefined ? target.role : `${target.role}:${target.name}`;
      return fromNode(one(matches, "role", needle, pool), "role", off);
    }
    if ("testId" in target) {
      const node = one(pool.filter((n) => n.testId === target.testId), "testId", target.testId, pool);
      return fromNode(node, "testId", off);
    }
    if ("text" in target) {
      return fromNode(one(byNameTiers(pool, target.text, true), "text", target.text, pool), "text", off);
    }
    if ("css" in target) return this.#locateCss(target.css, off);
    // `{axRef}` is a NATIVE target. It shares `TargetSchema` with the webview verbs, so a query
    // tool like `ui_find` will accept one and it arrives here — where falling through to the
    // point branch dereferences a `point` that does not exist. Refuse it by name instead: the
    // webview resolver cannot see a native element at all, and saying so is the whole answer.
    if (!("point" in target)) {
      throw new AgentError("INVALID_TARGET", "this tool addresses the WebView and cannot resolve a native target", {
        remediation:
          "An {axRef} names an element in the accessibility tree, which the WebView does not own. Use native_inspect / native_find to read it, and the pointer/keyboard verbs to act on it.",
        details: { target },
      });
    }
    return { css: this.#locatePoint(target.point, opts), source: "point" };
  }

  async #locateCss(selector: string, off: Pt | undefined): Promise<Located> {
    const rect = await this.#bridge.execute<Rect | null>(
      "resolve.css",
      ((sel: string) => {
        const el = document.querySelector(sel);
        if (!el) return null;
        const r = el.getBoundingClientRect();
        return { x: r.left, y: r.top, width: r.width, height: r.height };
      }) as never,
      [selector],
      { readOnly: true },
    );
    if (!rect) throw notFound("css", selector, this.#refs.all());
    return { css: pointOf(rect, off, { source: "css", selector }), rect, source: "css", selector };
  }

  #locatePoint(point: Pt & { space: Space }, opts: ResolveOpts): Pt {
    const off = opts.offset;
    const raw =
      point.space === "global"
        ? this.#globalPoint(point, opts)
        : { x: point.x, y: point.y }; // "window" == "webview": the Overlay title bar makes content span the frame
    return { x: raw.x + (off?.x ?? 0), y: raw.y + (off?.y ?? 0) };
  }

  #globalPoint(point: Pt, opts: ResolveOpts): Pt {
    if (!opts.geom) {
      throw new AgentError("INVALID_TARGET", "a global-space point needs the window geometry", {
        remediation: "Pass `geom` from the session's calibrated WindowGeom, or use space \"webview\".",
        details: { point },
      });
    }
    return globalToCss(point, opts.geom);
  }

  /** One bridged script; see `checkScript.ts` for what it measures and why. */
  #check(located: Located, opts: ResolveOpts): Promise<InputCheck> {
    const off = opts.offset;
    return this.#bridge.execute<InputCheck>(
      "resolve.forInput",
      checkInPage as never,
      [located.node?.ref ?? null, fallbackSelector(located), off?.x ?? 0, off?.y ?? 0],
      { readOnly: true },
    );
  }

  async #pointVerdict(located: Located, opts: ResolveOpts): Promise<Resolved> {
    const dragRegion = await this.#bridge.execute<boolean>(
      "resolve.pointDragRegion",
      dragRegionAtPoint as never,
      [located.css.x, located.css.y],
      { readOnly: true },
    );
    if (dragRegion && opts.forDrag) {
      throw new AgentError("ELEMENT_OCCLUDED", "a drag starting here would move the window, not the content", {
        remediation: "Start the drag on a non-drag-region element, or aim at content below the title bar.",
        details: { source: "point", point: located.css, reason: "tauri-drag-region" },
      });
    }
    return { css: located.css, source: "point", dragRegion };
  }

  #verdict(located: Located, check: InputCheck, opts: ResolveOpts): Resolved {
    const node = located.node;
    const where = node ? brief(node) : { source: located.source };
    if (!check.found || !check.rect1 || !check.rect0) {
      throw new AgentError("ELEMENT_STALE", "the element is no longer in the document", {
        remediation: "Take a fresh ui_snapshot and re-target.",
        details: where,
      });
    }
    if (node && (!check.fpAvailable || check.fp !== node.fp)) {
      throw new AgentError("ELEMENT_STALE", `element identity changed (${node.fp} → ${check.fp ?? "none"})`, {
        remediation: "The page re-rendered since the snapshot. Take a fresh ui_snapshot and re-target.",
        details: { ...where, expectedFp: node.fp, actualFp: check.fp },
      });
    }
    if (!rectIsFinite(check.rect0) || !rectIsFinite(check.rect1)) {
      // JSON has no NaN: the bridge delivers one as `null`, which subtracts to 0
      // and would read as a perfectly still element sitting at the origin.
      throw new AgentError("ELEMENT_MOVING", "the element's rect did not survive the bridge", {
        remediation: "Re-take the snapshot and retry; the page reported a rect with non-finite components.",
        details: { ...where, rect0: check.rect0, rect1: check.rect1 },
      });
    }
    if (!rectStable(check.rect0, check.rect1)) {
      throw new AgentError("ELEMENT_MOVING", "the element moved between two samples 50 ms apart", {
        remediation: "Wait for the transition to finish (wait_for revision_stable), then retry.",
        details: { ...where, rect0: check.rect0, rect1: check.rect1 },
      });
    }
    if (opts.forDrag && check.dragRegion) {
      throw new AgentError("ELEMENT_OCCLUDED", "a drag starting here would move the window, not the content", {
        remediation: "Start the drag on a non-drag-region element.",
        details: { ...where, reason: "tauri-drag-region" },
      });
    }
    if (!check.hitIsSelf) {
      const covered = check.occluder !== null;
      throw new AgentError(
        "ELEMENT_OCCLUDED",
        covered
          ? `another element receives events at this point (${check.occluder?.tag})`
          : "no element is hit-testable at this point; it is outside the viewport or clipped away",
        {
          remediation: covered
            ? "Dismiss the overlay, scroll the target into the clear, or target the occluder."
            : "Scroll the target into view, or reduce the offset so the point falls inside the element.",
          details: { ...where, occluder: check.occluder, point: check.rect1 },
        },
      );
    }
    return {
      node: located.node,
      css: rectCenter(check.rect1, opts.offset),
      rect: check.rect1,
      source: located.source,
      dragRegion: check.dragRegion,
    };
  }
}
