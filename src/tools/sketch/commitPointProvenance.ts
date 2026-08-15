/*
 * commitPointProvenance — which COMMITTED point each placement click became.
 *
 * A snap decision knows what the user aimed at; the commit knows what geometry
 * was authored. Persisting an accepted relation needs both halves joined, and
 * the join is per-TOOL: a rectangle's first diagonal corner becomes line 0's
 * Start, an arc's second click becomes `Arc.Start`, a circle's radius click
 * becomes no addressable point at all.
 *
 * That mapping used to not exist. Inference re-derived relations from final
 * COORDINATES instead, which cannot tell "the user snapped here" from "these
 * happen to coincide" — the defect the whole intent pipeline exists to close.
 *
 * ── Placement-only clicks ────────────────────────────────────────────────────
 * Several clicks steer geometry without producing a point anything can be
 * constrained to: a circle's radius click, an ellipse's two axis clicks, a
 * 3-point arc's through-point, a slot's width click, a centre-rectangle's
 * centre (no centre entity exists to address). Those map to `null` and their
 * accepted intent is dropped with a traceable reason rather than being attached
 * to a nearby point it did not mean.
 */
import type { SketchEntity } from "@/ipc/types";
import type { FrontendPointRef } from "./snapTypes";
import { canonicalRole } from "./sketchTopology";
import { pointCoordOf } from "./sketchTopology";

/** How the committing gesture's clicks line up with the batch it produced. */
export interface CommitProvenanceInput {
  toolId: string;
  /** The entities this commit authored, in the machine's own `committed` order. */
  entities: readonly SketchEntity[];
  /** Anchors placed BEFORE the committing click, in placement order. */
  priorAnchorCount: number;
  /** The line tool committed a TANGENT ARC, not a segment. */
  arcMode?: boolean;
  /** Polygon side count — the construction circumcircle is the LAST entity. */
  sides?: number;
}

/**
 * The point role each placement index maps to, or `null` for placement-only.
 *
 * Index 0 is the first anchor the gesture placed; the LAST index is the click
 * that committed. Length always equals `priorAnchorCount + 1`.
 */
export function commitPointRefs(input: CommitProvenanceInput): (FrontendPointRef | null)[] {
  const { toolId, entities } = input;
  const n = input.priorAnchorCount + 1;
  const at = (i: number): SketchEntity | undefined => entities[i];
  const ref = (e: SketchEntity | undefined, position: "Start" | "End" | "Center"):
    | FrontendPointRef
    | null => {
    if (!e) return null;
    const role = canonicalRole(e.type, position);
    return role ? { entityId: e.id, position: role } : null;
  };
  const none = (): (FrontendPointRef | null)[] => new Array(n).fill(null);

  switch (toolId) {
    case "line": {
      // A chain commits ONE entity per click pair: the prior anchor is its
      // entry point and the committing click its exit. In arc mode the entity
      // is an Arc, whose Start/End are worker-minted but addressable as
      // owner+role, so the same two roles apply.
      const e = at(0);
      const out = none();
      out[n - 2] = ref(e, "Start");
      out[n - 1] = ref(e, "End");
      return out;
    }
    case "rect":
    case "centerRect": {
      // Four lines, corner-ordered. The FIRST placement is line 0's Start for a
      // corner-to-corner rect; a centre rectangle's centre addresses nothing (no
      // centre entity exists), so only the committing corner maps.
      const out = none();
      if (toolId === "rect") out[0] = ref(at(0), "Start");
      out[n - 1] = ref(at(1), "End");
      return out;
    }
    case "circle": {
      const out = none();
      out[0] = ref(at(0), "Center");
      return out; // the radius click is not an addressable point
    }
    case "arc": {
      // centre → Arc.Center, second anchor → Arc.Start, final click → Arc.End.
      const e = at(0);
      const out = none();
      out[0] = ref(e, "Center");
      if (n >= 2) out[1] = ref(e, "Start");
      out[n - 1] = ref(e, "End");
      return out;
    }
    case "arc3p": {
      // first anchor → Arc.Start, second → Arc.End; the through-point steers
      // the bulge and addresses nothing.
      const e = at(0);
      const out = none();
      out[0] = ref(e, "Start");
      if (n >= 2) out[1] = ref(e, "End");
      return out;
    }
    case "ellipse": {
      const out = none();
      out[0] = ref(at(0), "Center");
      return out; // both axis clicks are placement-only
    }
    case "polygon": {
      // The construction circumcircle is the LAST entity of the batch; the
      // first vertex is line 0's Start.
      const out = none();
      const circle = entities[entities.length - 1];
      out[0] = circle && circle.type === "Circle" ? ref(circle, "Center") : null;
      out[n - 1] = ref(at(0), "Start");
      return out;
    }
    case "slot": {
      // p0 and p1 are the CAP CENTRES. `slotDrafts` emits [wallA, wallB, capA,
      // capB]; the cap at p0 is whichever arc's centre is nearer p0, resolved by
      // coordinate because the machine does not label them.
      const out = none();
      const arcs = entities.filter((e) => e.type === "Arc");
      if (arcs.length === 2) {
        out[0] = ref(arcs[0], "Center");
        if (n >= 2) out[1] = ref(arcs[1], "Center");
      }
      return out; // the width click is placement-only
    }
    case "point": {
      const out = none();
      out[n - 1] = ref(at(0), "Start");
      return out;
    }
    default:
      return none();
  }
}

/**
 * Re-order a slot's two cap refs so index 0 really is the cap at the FIRST
 * placement.
 *
 * `slotDrafts` emits its arcs in its own order, which is not guaranteed to
 * match the click order. Resolving by coordinate is safe here and only here:
 * the two caps are the gesture's OWN geometry, placed microseconds earlier at
 * exactly the anchors being matched, so this is identity recovery, not the
 * proximity guessing the intent pipeline exists to remove.
 */
export function orderSlotCaps(
  refs: (FrontendPointRef | null)[],
  entities: readonly SketchEntity[],
  anchors: readonly { x: number; y: number }[],
): (FrontendPointRef | null)[] {
  if (refs.length < 2 || anchors.length < 2) return refs;
  const byId = new Map(entities.map((e) => [e.id, e]));
  const coordOf = (r: FrontendPointRef | null): [number, number] | null => {
    const e = r ? byId.get(r.entityId) : undefined;
    return e && r ? pointCoordOf(e, r.position) : null;
  };
  const c0 = coordOf(refs[0]);
  const c1 = coordOf(refs[1]);
  if (!c0 || !c1) return refs;
  const d = (c: [number, number], p: { x: number; y: number }): number =>
    Math.hypot(c[0] - p.x, c[1] - p.y);
  if (d(c0, anchors[0]) <= d(c1, anchors[0])) return refs;
  const swapped = [...refs];
  swapped[0] = refs[1];
  swapped[1] = refs[0];
  return swapped;
}
