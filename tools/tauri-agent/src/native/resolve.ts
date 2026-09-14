/**
 * Resolving an ACCESSIBILITY target to a point it is safe to click.
 *
 * The mirror of `src/semantic/resolve.ts` for what the WebView does not own. The refusal path is
 * the point of this file, exactly as it is there: a stale ref, an element that is still animating,
 * or an element whose centre is not in a window this application owns all produce a
 * plausible-looking coordinate, and the native backend will happily click it — in the wrong place,
 * in the user's real session. Every one of those is a named error instead.
 *
 * Most of the ladder already lives in the helper: `ax_point` re-validates the ref, refuses a role
 * that changed under the handle, samples the rect twice 50 ms apart at the same 0.5 pt tolerance
 * `rectStable` uses, and refuses a centre that is on no display. This file adds the two things the
 * helper cannot know — better remediation for each refusal, and whether the point lands inside a
 * window this application owns (`checkGlobalPoint`).
 *
 * A held ref is NEVER re-found by its role and title. That is the same rule the webview resolver
 * states for a held `ref`, for the same reason: re-finding would address whichever element now
 * occupies that slot and report success. A ref that no longer resolves is a refusal, full stop.
 */
import { AgentError } from "../errors.ts";
import { checkGlobalPoint } from "../geometry/mapping.ts";
import type { GlobalOcclusion, OwnedWindow } from "../geometry/mapping.ts";
import type { Pt } from "../geometry/types.ts";
import type {
  AxFindOpts,
  AxNode,
  AxPointInfo,
  NativeAccessibility,
  NativeWindows,
} from "../platform/adapter.ts";
import { axWindowIdValue } from "../platform/adapter.ts";

/**
 * A held ref, or a query the helper's `ax_find` can run.
 *
 * The query arm spells `ref?: undefined` so the two are discriminated by a property both arms
 * declare. Without it `{ref}` would also satisfy the all-optional query arm, and an `in` test
 * would not narrow — the exact ambiguity that lets a held ref slide into a re-find.
 */
export type AxQuery = AxFindOpts & { ref?: undefined };
export type AxTarget = { ref: string } | AxQuery;

export interface ResolvedAx {
  /** "ref" resolved a handle the caller already had; "find" walked for it and minted a new one. */
  source: "ref" | "find";
  /** The helper ref generation this result belongs to. A `find` bumps it. */
  generation: number;
  /** The matched node. Absent for a held ref, which is resolved without a walk. */
  node?: AxNode;
  point: AxPointInfo;
  /** global display points, ready for `click` with no conversion */
  global: Pt;
}

export interface ResolveAxOpts {
  /**
   * The webview's native-occlusion rects, anchored to the window they were measured against.
   *
   * ABSENT by default, and deliberately: those rects describe native controls that sit ABOVE the
   * webview, and this layer exists to reach exactly such controls — the title-bar buttons, the
   * menu bar, panel chrome. Refusing them as "occluders" would defeat the layer. Pass it when the
   * point came from the webview's own idea of where something is, where a native control on top
   * of it really is a mis-click.
   */
  occlusion?: GlobalOcclusion;
}

/**
 * The nodes of one AX walk, scoped to the generation that produced them.
 *
 * `generation` is carried so a ref from an EARLIER walk is refused rather than looked up and
 * missed: the two are different facts, and the helper reports them as different codes.
 */
export interface AxRefStore {
  readonly generation: number;
  get(ref: string): AxNode | undefined;
  all(): AxNode[];
}

export function axRefStoreFrom(generation: number, nodes: AxNode[]): AxRefStore {
  const byRef = new Map(nodes.map((n) => [n.ref, n]));
  return { generation, get: (ref) => byRef.get(ref), all: () => nodes };
}

/** `@a<generation>e<index>`, the helper's own shape. Anything else never reaches the wire. */
const REF_PATTERN = /^@a(\d+)e(\d+)$/;

/** AX-flavoured remediation for the refusals `ax_point` can answer with. */
const AX_REMEDIATION: Partial<Record<string, string>> = {
  ELEMENT_STALE:
    "The ref is from an older AX snapshot generation, or the element behind it now reports a different role. Take a fresh AX snapshot and re-target. A held ref is never re-found by role and title, because that would address whichever element now occupies the slot.",
  ELEMENT_NOT_FOUND:
    "No live accessibility element is behind that ref. Take a fresh AX snapshot (or find) and use a ref from it.",
  ELEMENT_MOVING:
    "The element is still animating — a sheet sliding in, a panel resizing — so two rect samples 50 ms apart disagreed. Wait for it to settle and retry.",
  POINT_OUTSIDE_WINDOW:
    "The element has an empty frame, or its centre is on no display. Move the window fully onto one display, or target an element that is actually visible.",
};

function brief(n: AxNode): Record<string, unknown> {
  return { ref: n.ref, role: n.role, subrole: n.subrole, title: n.title };
}

function queryOf(target: AxFindOpts): Record<string, string> {
  const out: Record<string, string> = {};
  if (target.role !== undefined) out.role = target.role;
  if (target.title !== undefined) out.title = target.title;
  if (target.value !== undefined) out.value = target.value;
  return out;
}

export class AxResolver {
  readonly #ax: NativeAccessibility;
  readonly #windows: NativeWindows;
  readonly #pid: number;
  #refs: AxRefStore | undefined;

  constructor(deps: { ax: NativeAccessibility; windows: NativeWindows; pid: number; refs?: AxRefStore }) {
    this.#ax = deps.ax;
    this.#windows = deps.windows;
    this.#pid = deps.pid;
    this.#refs = deps.refs;
  }

  /** The pool this resolver last saw. A `find` replaces it, because a find bumps the generation. */
  get refs(): AxRefStore | undefined {
    return this.#refs;
  }

  async resolve(target: AxTarget, opts: ResolveAxOpts = {}): Promise<ResolvedAx> {
    const located = target.ref === undefined ? await this.#locateFind(target) : this.#locateRef(target.ref);
    const point = await this.#point(located.ref);
    const global = await this.#gate(point, opts);
    return {
      source: located.source,
      generation: point.generation,
      ...(located.node === undefined ? {} : { node: located.node }),
      point,
      global,
    };
  }

  /**
   * A held ref. It is checked against the pool this resolver holds and then handed to the helper,
   * which is the authority — and it is never turned back into a query. The local check only
   * refuses EARLIER than the helper would; it can never admit something the helper would refuse.
   */
  #locateRef(ref: string): { source: "ref"; ref: string; node?: AxNode } {
    const m = REF_PATTERN.exec(ref);
    if (!m) {
      throw new AgentError("INVALID_TARGET", `'${ref}' is not an accessibility ref`, {
        remediation: "An AX ref looks like @a<generation>e<index> and comes from an AX snapshot or find.",
        details: { ref },
      });
    }
    const refs = this.#refs;
    if (refs) {
      const generation = Number(m[1]);
      if (generation !== refs.generation) {
        throw new AgentError(
          "ELEMENT_STALE",
          `ref '${ref}' is from AX generation ${generation}; this pool is generation ${refs.generation}`,
          { remediation: AX_REMEDIATION.ELEMENT_STALE, details: { ref, generation, poolGeneration: refs.generation } },
        );
      }
      const node = refs.get(ref);
      if (!node) {
        throw new AgentError("ELEMENT_NOT_FOUND", `no element '${ref}' in AX generation ${generation}`, {
          remediation: AX_REMEDIATION.ELEMENT_NOT_FOUND,
          details: { ref, generation, poolNodes: refs.all().length },
        });
      }
      return { source: "ref", ref, node };
    }
    return { source: "ref", ref };
  }

  /**
   * A query. `ax_find` is a snapshot — it mints a fresh generation — so the pool is REPLACED
   * here rather than merged: every ref the caller was holding is stale the moment this returns,
   * and a store that still claimed the old generation would refuse the ref this call just minted.
   */
  async #locateFind(target: AxFindOpts): Promise<{ source: "find"; ref: string; node: AxNode }> {
    const query = queryOf(target);
    if (Object.keys(query).length === 0) {
      throw new AgentError("INVALID_TARGET", "an accessibility query needs at least one of role, title or value", {
        remediation: "Pass {ref} for a held element, or narrow the query with a role and a title.",
        details: { target },
      });
    }
    const walk = await this.#ax.find(this.#pid, target);
    this.#refs = axRefStoreFrom(walk.generation, walk.nodes);
    const matches = walk.nodes;
    if (matches.length === 0) {
      throw new AgentError("ELEMENT_NOT_FOUND", `no accessibility element matches ${JSON.stringify(query)}`, {
        remediation:
          "Take an ax snapshot to see what the window actually exposes; AX roles are exact (AXButton, AXTextField) while title and value are case-insensitive substrings.",
        details: { query, visited: walk.total, truncated: walk.truncated, stopReason: walk.stopReason },
      });
    }
    if (matches.length > 1) {
      throw new AgentError("INVALID_TARGET", `${JSON.stringify(query)} matches ${matches.length} elements`, {
        remediation: "Pass the `ref` of the intended element, or narrow the query with role + title.",
        details: { query, candidates: matches.slice(0, 12).map(brief) },
      });
    }
    const node = matches[0] as AxNode;
    return { source: "find", ref: node.ref, node };
  }

  /** The helper's ladder, re-dressed: the same code, remediation a caller can act on. */
  async #point(ref: string): Promise<AxPointInfo> {
    try {
      return await this.#ax.point(ref);
    } catch (e) {
      if (!(e instanceof AgentError)) throw e;
      const better = AX_REMEDIATION[e.code];
      if (better === undefined) throw e;
      throw new AgentError(e.code, e.message, {
        remediation: better,
        details: { ...e.details, ref, pid: this.#pid },
        cause: e,
      });
    }
  }

  /**
   * The part the helper cannot know: whether that point is inside a window THIS application owns.
   *
   * The windows are read fresh on every resolve, not cached — a panel that opened between the
   * snapshot and now is exactly the window a native target is most likely to be in. Only
   * on-screen windows count: a click cannot reach one that is not.
   *
   * Known limit, stated rather than hidden: a SANDBOXED application's open/save panel is hosted
   * by a different process, so it is neither in this pid's AX tree nor in this list, and a point
   * in it is refused. OneCAD's panels are in-process.
   */
  async #gate(point: AxPointInfo, opts: ResolveAxOpts): Promise<Pt> {
    const owned: OwnedWindow[] = (await this.#windows.list(this.#pid))
      .filter((w) => w.onscreen)
      .map((w) => ({ windowId: w.windowId, bounds: w.bounds }));
    try {
      return checkGlobalPoint(point.center, owned, opts.occlusion ?? null);
    } catch (e) {
      if (!(e instanceof AgentError) || e.code !== "POINT_OUTSIDE_WINDOW") throw e;
      throw new AgentError(e.code, e.message, {
        remediation: e.remediation,
        details: {
          ...e.details,
          ref: point.ref,
          pid: this.#pid,
          // The AX-reported container, when the helper could correlate one. A known id that is
          // absent from `windows` is direct evidence the element is not this application's.
          axWindowId: axWindowIdValue(point.window),
          axWindowIdSource: point.window.source,
          frontmost: point.frontmost,
        },
        cause: e,
      });
    }
  }
}
