/*
 * Sketch drawing tool machines (PURE state machines) operating in plane (u,v)
 * coords. Each machine is a reducer over discrete events the controller feeds it:
 *
 *   click(pt) — a tap (pointerdown+up without a drag) at a snapped plane point
 *   move(pt)  — pointer moved to a snapped plane point (drives the rubber-band)
 *   esc       — cancel/end the current chain
 *   sides(n)  — polygon side count (digit keys 3-9); every OTHER machine ignores it
 *   arcToggle — the line tool's TANGENT-ARC mode (drag / `A`); ignored elsewhere
 *
 * `step` returns the next state, the PREVIEW draft entities (rubber-band, no ids)
 * and, when a gesture completes, the COMMITTED draft entities the controller
 * id-assigns → auto-constrains → sends to `sketchUpsert`. Chaining semantics:
 *   - line      : click-click-… chains segments; Esc ends the chain. `arcToggle`
 *                 makes the NEXT leg a tangent arc off the chain's own direction.
 *   - rect      : 2 clicks (corner→corner) commit 4 lines, then reset for the next.
 *   - centerRect: 2 clicks (center→corner) commit the 4 mirrored lines, then reset.
 *   - circle    : 2 clicks (center→radius) commit one circle, then reset.
 *   - ellipse   : 3 clicks (center→major-axis end→minor extent) commit one
 *                 ellipse, then reset.
 *   - arc       : 3 clicks (center→start→end) commit one arc, then reset.
 *   - arc3p     : 3 clicks (start→end→through) commit one arc, then reset.
 *   - polygon   : 2 clicks (center→vertex) commit n lines + a construction
 *                 circumcircle; the side count is sticky across commits.
 *   - slot      : 3 clicks (p0→p1→width) commit 2 walls + 2 cap arcs, then reset.
 *   - point     : 1 click commits a Point and re-arms immediately.
 *
 * A step MAY also carry `committedConstraints` — constraints the TOOL authors over
 * its own freshly committed entities (see `ToolConstraintSpec` /
 * `resolveToolConstraints`), for shapes whose semantics the generic auto-constraint
 * inference cannot express (slot tangency, polygon regularity).
 *
 * Pure ⇒ pointer sequences are unit-tested end to end.
 */
import type {
  ConstraintPosition,
  SketchConstraint,
  SketchConstraintType,
  SketchEntity,
  SketchEntityType,
} from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";
import type { DimLocks, DimValues, ToolDimension } from "./liveDimension";
import { normalizeEllipse } from "./ellipseMath";
import { arcThroughPoints, perpDistanceTo, tangentArcTo, unitDir, type TangentArc } from "./arcMath";

/** A draft entity in plane coords (no id yet — the controller assigns on commit). */
export interface DraftEntity {
  type: SketchEntityType;
  construction?: boolean;
  p0?: Point2;
  p1?: Point2;
  center?: Point2;
  radius?: number;
  start?: Point2;
  end?: Point2;
  /** Ellipse semi-major / semi-minor / rotation (radians, [0, 2π)). Always
   *  normalized to `majorR ≥ minorR` by `ellipseTool` before they appear here. */
  majorR?: number;
  minorR?: number;
  rotation?: number;
}

export type ToolEvent =
  | { kind: "click"; pt: Point2 }
  | { kind: "move"; pt: Point2 }
  | { kind: "esc" }
  /** Polygon side count (SketchController routes digit keys 3-9 here while the
   *  polygon tool is armed). Consumed ONLY by `polygonTool`; every other machine
   *  ignores it (state unchanged, empty preview). */
  | { kind: "sides"; n: number }
  /** Turn the line tool's tangent-arc mode on (`on: true`), off (`on: false`) or
   *  flip it (`on` omitted). Consumed ONLY by `lineTool`; every other machine
   *  ignores it on exactly the `sides` terms. */
  | { kind: "arcToggle"; on?: boolean };

/** Optional per-step context. `minSize` is the smallest accepted feature extent in
 *  PLANE (world) units — a click/drag below it is a degenerate no-op. The controller
 *  passes a screen-scaled value (≈4px worth of world) so the reject radius is
 *  constant on screen at any zoom; the default keeps the machines usable bare. */
export interface ToolContext {
  minSize?: number;
  /** Live-dimension fields the user has PINNED (mm · degrees · count). Read ONLY
   *  by the `withLiveDims` decorator (liveToolMachines.ts), which projects them
   *  into the event before the raw machine ever sees it — every machine below
   *  ignores this field. */
  locks?: DimLocks;
  /**
   * Numeric field values the SNAP DECISION accepted for this sample (SNAP P2) —
   * cursor rounding included. Same ownership as `locks`: read only by the
   * `withLiveDims` decorator, which applies them (locks on top) and rebuilds the
   * point once. Absent ⇒ nothing was accepted and the event passes through.
   *
   * Replaces the old `quantum`, which made this decorator the OWNER of the
   * rounding decision and therefore unable to compose it with a directional
   * intent or to report why it was refused.
   */
  dimValues?: DimValues | null;
  /** CONTROLLER-RESOLVED chain tangent: the direction an EXISTING entity leaves
   *  the anchor the chain was seeded on (`entityEndTangent`). Only a seed — a
   *  machine that has recorded its own `ToolState.chainTangent` (it committed the
   *  previous leg itself) always prefers that. Read by `lineTool` alone. */
  tangent?: Point2;
}

/** Fallback minimum feature size when no context is supplied (numerically tiny —
 *  rejects only truly coincident points). */
const DEFAULT_MIN_SIZE = 1e-9;

/** Accumulated anchor points placed so far + the live cursor. */
export interface ToolState {
  anchors: Point2[];
  cursor: Point2 | null;
  /** Polygon side count (polygonTool only). Sticky across commits + Esc. */
  sides?: number;
  /** The direction the chain LEAVES its last anchor travelling in — recorded on
   *  every `lineTool` commit, because an arc's exit direction is NOT its chord
   *  and the next tangent arc has to continue the real curve. Cleared with the
   *  chain (Esc / close / chain-end). */
  chainTangent?: Point2;
  /** `lineTool`: the NEXT leg is a tangent arc. Auto-clears on the arc's commit —
   *  one arc per toggle, matching Fusion's line tool. */
  arcMode?: boolean;
}

/**
 * A constraint the TOOL itself authors over the entities of the SAME step.
 *
 * `refs` index into that step's `committed` array (not entity ids — the machine is
 * pure and has never seen an id); the controller resolves them right after the
 * id-assign (`resolveToolConstraints`). `positions` aligns 1:1 with `refs`;
 * `undefined` in a slot means the ref is ENTITY-level (whole line / circle / arc).
 *
 * Only positions the marshaller can express may be used — Line Start/End,
 * Circle/Arc Center, entity-level, or (Coincident ONLY) an Arc Start/End. An arc
 * endpoint has no synthesized point uuid of its own: it is minted by the WORKER,
 * so `toWireConstraint` sends the arc's uuid plus a position, and only the
 * Coincident wire shape carries those position fields (W0b). Under any other
 * kind an Arc Start/End still resolves to null and `marshalUpsert` SILENTLY DROPS
 * the constraint (sketchWireMap.ts).
 */
export interface ToolConstraintSpec {
  type: SketchConstraintType;
  /** Indices into THIS step's `committed` array. */
  refs: number[];
  /** Per-ref point selector, aligned with `refs` (`undefined` ⇒ entity-level). */
  positions?: (ConstraintPosition | undefined)[];
}

export interface ToolStep {
  state: ToolState;
  preview: DraftEntity[];
  committed?: DraftEntity[];
  /** Tool-authored constraints over `committed` (see `ToolConstraintSpec`). */
  committedConstraints?: ToolConstraintSpec[];
  /** True when the current gesture ended (chain closed / entity committed). */
  done?: boolean;
  /** Live dimension chips for the gesture's CURRENT phase. Emitted only by the
   *  `withLiveDims` decorator; a raw machine never sets it. */
  dims?: ToolDimension[];
}

export interface ToolMachine {
  readonly id: string;
  init(): ToolState;
  step(state: ToolState, event: ToolEvent, ctx?: ToolContext): ToolStep;
}

const emptyState = (): ToolState => ({ anchors: [], cursor: null });

const asPair = (p: Point2): [number, number] => [p.x, p.y];
const line = (a: Point2, b: Point2): DraftEntity => ({ type: "Line", p0: a, p1: b });
const dist = (a: Point2, b: Point2): number => Math.hypot(a.x - b.x, a.y - b.y);

/** The TOOL-SPECIFIC verbs: `sides` belongs to the polygon, `arcToggle` to the
 *  line. Every machine that does not own one is a no-op for it. */
type ToolVerb = Extract<ToolEvent, { kind: "sides" } | { kind: "arcToggle" }>;

const isToolVerb = (e: ToolEvent): e is ToolVerb => e.kind === "sides" || e.kind === "arcToggle";

/** No-op for a verb this machine does not own — the SAME state object back, so a
 *  caller can identity-check that nothing happened. */
const ignoreVerb = (state: ToolState): ToolStep => ({ state, preview: [] });

// ── Line tool: click-click chaining, Esc ends the chain ──────────────────────
//
// Anchors ACCUMULATE the whole chain (not just the last segment) so the
// controller's snap engine sees every placed vertex as an H/V guide reference.
// A click reuses one of three outcomes, checked in this order:
//   1. within minSize of the LAST anchor  → END CHAIN (no commit). Covers the
//      1-anchor case too (first === last), and makes a double-click at the same
//      point end the chain right after the first click of the pair committed
//      the segment.
//   2. within minSize of the FIRST anchor (chain has ≥2 anchors) → CLOSE LOOP:
//      commit the closing segment back to the first anchor.
//   3. otherwise → commit last→pt and keep chaining.
//
// TANGENT-ARC MODE (SP-4 W3) is a MODE of this machine, not a separate tool —
// Fusion's rule, and the reason it can be: the arc has to continue the chain the
// line tool is already holding, and a second machine would have to be handed that
// chain on every toggle. `arcToggle` arms it; the next click commits ONE Arc and
// the mode auto-clears, so the common "line, arc, line" run needs no untoggle.
//
// The direction the arc leaves along is `chainTangent` — RECORDED on every commit
// rather than derived from the anchors, because an arc's exit direction is not its
// chord: chaining an arc off an arc off a line is exact only if each leg reports
// where it actually ended up pointing. `ctx.tangent` seeds the very first leg of a
// chain the user started ON existing geometry (the controller resolves it there).
// With no tangent at all the mode is INERT — a straight-line-only chain has no
// direction to be tangent to, and guessing the chord would be a lie.

/** A committed/previewed arc leg. `Arc` fields only — the CCW convention every
 *  `arcMath` construction already states its result in. */
const arcDraft = (a: TangentArc): DraftEntity => ({
  type: "Arc",
  center: a.center,
  radius: a.radius,
  start: a.start,
  end: a.end,
});

/** The leg the chain is drawing right now: a tangent arc in arc mode, or the
 *  straight rubber-band — which is also the FALLBACK whenever the cursor sits on
 *  the tangent ray and no finite arc exists (previewing nothing there would read
 *  as a broken tool). */
function lineLeg(from: Point2, to: Point2, tangent: Point2 | undefined): DraftEntity {
  if (!tangent) return line(from, to);
  const arc = tangentArcTo(from, tangent, to);
  return arc ? arcDraft(arc) : line(from, to);
}

/** `arcToggle`: inert with nothing placed or no tangent to leave along, and a
 *  no-change toggle hands back the SAME state (nothing to repaint). */
function lineArcToggle(state: ToolState, on: boolean | undefined, tangent: Point2 | undefined): ToolStep {
  if (state.anchors.length === 0 || !tangent) return ignoreVerb(state);
  const next = on ?? !(state.arcMode ?? false);
  if (next === (state.arcMode ?? false)) return ignoreVerb(state);
  return { state: { ...state, arcMode: next }, preview: [] };
}

export const lineTool: ToolMachine = {
  id: "line",
  init: emptyState,
  step(state, event, ctx) {
    if (event.kind === "esc") {
      return { state: emptyState(), preview: [], done: true };
    }
    if (event.kind === "sides") return ignoreVerb(state);
    // The machine's OWN record wins over the controller's seed — see the header.
    const tangent = state.chainTangent ?? ctx?.tangent;
    if (event.kind === "arcToggle") return lineArcToggle(state, event.on, tangent);
    const arcTangent = state.arcMode === true ? tangent : undefined;
    if (event.kind === "move") {
      const last = state.anchors[state.anchors.length - 1] ?? null;
      const next = { ...state, cursor: event.pt };
      return { state: next, preview: last ? [lineLeg(last, event.pt, arcTangent)] : [] };
    }
    // click
    const minSize = ctx?.minSize ?? DEFAULT_MIN_SIZE;
    const last = state.anchors[state.anchors.length - 1] ?? null;
    if (!last) {
      // First anchor of a fresh chain.
      return { state: { anchors: [event.pt], cursor: event.pt }, preview: [] };
    }
    if (dist(last, event.pt) < minSize) {
      return { state: emptyState(), preview: [], done: true };
    }
    const first = state.anchors[0];
    if (state.anchors.length > 1 && dist(first, event.pt) < minSize) {
      // Closing leg: an ARC in arc mode (the mode is about this leg, and the loop
      // has to close with the shape the user is looking at), else the segment.
      return { state: emptyState(), preview: [], committed: [lineLeg(last, first, arcTangent)], done: true };
    }
    if (arcTangent) return arcLegClick(state, last, arcTangent, event.pt);
    const dir = unitDir(last, event.pt);
    return {
      state: {
        anchors: [...state.anchors, event.pt],
        cursor: event.pt,
        ...(dir ? { chainTangent: dir } : {}),
      },
      preview: [],
      committed: [line(last, event.pt)],
    };
  },
};

/**
 * The arc-mode committing click. No arc-LENGTH floor is needed: a circular arc is
 * never shorter than its chord, and the chord already cleared `minSize` above — so
 * the only rejection left is a cursor ON the tangent ray, where no finite circle
 * exists. That click is IGNORED (stay armed, arc mode still on) rather than
 * quietly committing a straight line the mode did not promise.
 */
function arcLegClick(state: ToolState, last: Point2, tangent: Point2, pt: Point2): ToolStep {
  const arc = tangentArcTo(last, tangent, pt);
  if (!arc) return { state: { ...state, cursor: pt }, preview: [] };
  return {
    state: {
      anchors: [...state.anchors, pt],
      cursor: pt,
      chainTangent: arc.exitTangent,
      arcMode: false, // one arc per toggle
    },
    preview: [],
    committed: [arcDraft(arc)],
  };
}

// ── Rectangle tool: 2 corner clicks → 4 lines ────────────────────────────────

function rectLines(a: Point2, b: Point2): DraftEntity[] {
  const c1 = a;
  const c2 = { x: b.x, y: a.y };
  const c3 = b;
  const c4 = { x: a.x, y: b.y };
  return [line(c1, c2), line(c2, c3), line(c3, c4), line(c4, c1)];
}

export const rectTool: ToolMachine = {
  id: "rect",
  init: emptyState,
  step(state, event, ctx) {
    if (event.kind === "esc") return { state: emptyState(), preview: [], done: true };
    if (isToolVerb(event)) return ignoreVerb(state);
    if (event.kind === "move") {
      const corner = state.anchors[0] ?? null;
      return { state: { ...state, cursor: event.pt }, preview: corner ? rectLines(corner, event.pt) : [] };
    }
    const minSize = ctx?.minSize ?? DEFAULT_MIN_SIZE;
    const corner = state.anchors[0] ?? null;
    if (!corner) return { state: { anchors: [event.pt], cursor: event.pt }, preview: [] };
    if (Math.abs(event.pt.x - corner.x) < minSize || Math.abs(event.pt.y - corner.y) < minSize) {
      // Degenerate rectangle (zero-extent on either axis) — ignore this click, keep
      // waiting for a real corner.
      return { state: { anchors: [corner], cursor: event.pt }, preview: [] };
    }
    return { state: emptyState(), preview: [], committed: rectLines(corner, event.pt), done: true };
  },
};

// ── Circle tool: center → radius ─────────────────────────────────────────────

const radiusOf = (c: Point2, edge: Point2): number => Math.hypot(edge.x - c.x, edge.y - c.y);

export const circleTool: ToolMachine = {
  id: "circle",
  init: emptyState,
  step(state, event, ctx) {
    if (event.kind === "esc") return { state: emptyState(), preview: [], done: true };
    if (isToolVerb(event)) return ignoreVerb(state);
    if (event.kind === "move") {
      const center = state.anchors[0] ?? null;
      return {
        state: { ...state, cursor: event.pt },
        preview: center ? [{ type: "Circle", center, radius: radiusOf(center, event.pt) }] : [],
      };
    }
    const minSize = ctx?.minSize ?? DEFAULT_MIN_SIZE;
    const center = state.anchors[0] ?? null;
    if (!center) return { state: { anchors: [event.pt], cursor: event.pt }, preview: [] };
    const radius = radiusOf(center, event.pt);
    if (radius < minSize) return { state: { anchors: [center], cursor: event.pt }, preview: [] };
    return {
      state: emptyState(),
      preview: [],
      committed: [{ type: "Circle", center, radius }],
      done: true,
    };
  },
};

// ── Ellipse tool: center → major-axis endpoint → minor extent ────────────────
//
// Ported from the oracle `OneCAD-CPP/src/core/sketch/tools/EllipseTool.cpp:28-171`
// (3-click FSM, Esc cancels):
//   click 1  centre. While placing the major axis, the rubber-band is a CIRCLE of
//            radius |cursor − centre| (EllipseTool.cpp:130-136: `minorRadius_ =
//            majorRadius_` "circle preview until second point").
//   click 2  the major-axis ENDPOINT. `majorR = |cursor − centre|` (rejected below
//            `ctx.minSize`), `rotation = atan2(Δy, Δx)`. The very first preview after
//            this click uses `minorR = majorR · 0.5` (EllipseTool.cpp:63).
//   move     `minorR` = the PERPENDICULAR distance from the cursor to the major-axis
//            line (`perpDistance`, shared with the slot tool).
//   click 3  commits (rejected below `ctx.minSize`).
//
// LIVE SWAP-NORMALIZATION. The oracle applies the SAME major≥minor swap to the
// preview as to the commit ("Apply same swap logic as final creation for consistent
// preview", EllipseTool.cpp:151) — so dragging the cursor past the major radius
// visibly re-labels the axes instead of drawing an impossible minor>major ellipse,
// and what is on screen is byte-for-byte what commits. `normalizeEllipse`
// (ellipseMath.ts) is the single implementation of that rule, called on every
// preview AND on the commit.
//
// The ellipse authors NO tool constraints: its CURVE takes none (legacy parity —
// `worker/tests/prototypes/proto_sketch_constraint_applicability.cpp:62-67` grants an
// ellipse curve no constraints at all), and its CENTRE gets the generic Coincident
// inference for free like a circle's.

/** The single ellipse draft for a centre + major-axis endpoint + RAW minor extent
 *  (normalized: `majorR ≥ minorR`, `rotation` folded into [0, 2π)). */
export function ellipseDraft(center: Point2, majorEnd: Point2, minorRaw: number): DraftEntity {
  const { majorR, minorR, rotation } = normalizeEllipse(
    radiusOf(center, majorEnd),
    minorRaw,
    Math.atan2(majorEnd.y - center.y, majorEnd.x - center.x),
  );
  return { type: "Ellipse", center, majorR, minorR, rotation };
}

export const ellipseTool: ToolMachine = {
  id: "ellipse",
  init: emptyState,
  step(state, event, ctx) {
    if (event.kind === "esc") return { state: emptyState(), preview: [], done: true };
    if (isToolVerb(event)) return ignoreVerb(state);
    const [center, majorEnd] = state.anchors;

    if (event.kind === "move") {
      if (center && majorEnd) {
        return {
          state: { ...state, cursor: event.pt },
          preview: [ellipseDraft(center, majorEnd, perpDistance(center, majorEnd, event.pt))],
        };
      }
      if (center) {
        // Circle rubber-band while the major axis is being aimed (oracle parity).
        return {
          state: { ...state, cursor: event.pt },
          preview: [{ type: "Circle", center, radius: radiusOf(center, event.pt) }],
        };
      }
      return { state: { ...state, cursor: event.pt }, preview: [] };
    }

    // click
    const minSize = ctx?.minSize ?? DEFAULT_MIN_SIZE;
    if (!center) return { state: { anchors: [event.pt], cursor: event.pt }, preview: [] };
    if (!majorEnd) {
      if (radiusOf(center, event.pt) < minSize) {
        return { state: { anchors: [center], cursor: event.pt }, preview: [] };
      }
      // Seed the minor at half the major so the ellipse reads as one immediately.
      return {
        state: { anchors: [center, event.pt], cursor: event.pt },
        preview: [ellipseDraft(center, event.pt, radiusOf(center, event.pt) * 0.5)],
      };
    }
    const minorRaw = perpDistance(center, majorEnd, event.pt);
    if (minorRaw < minSize) {
      return { state: { anchors: [center, majorEnd], cursor: event.pt }, preview: [] };
    }
    return {
      state: emptyState(),
      preview: [],
      committed: [ellipseDraft(center, majorEnd, minorRaw)],
      done: true,
    };
  },
};

// ── Arc tool: center → start → end (documented choice: center-start-end) ──────
//
// End is projected onto the circle (radius fixed by center→start); the sweep runs
// CCW from `start` to `end`. A center-start-end arc is the most predictable for a
// mouse (radius is locked after the 2nd click, so only the sweep tracks the
// cursor). SCHEMA leaves the arc gesture to the design; this is the pick.

function projectToCircle(center: Point2, radius: number, toward: Point2): Point2 {
  const a = Math.atan2(toward.y - center.y, toward.x - center.x);
  return { x: center.x + radius * Math.cos(a), y: center.y + radius * Math.sin(a) };
}

export const arcTool: ToolMachine = {
  id: "arc",
  init: emptyState,
  step(state, event, ctx) {
    if (event.kind === "esc") return { state: emptyState(), preview: [], done: true };
    if (isToolVerb(event)) return ignoreVerb(state);
    const minSize = ctx?.minSize ?? DEFAULT_MIN_SIZE;
    const [center, start] = state.anchors;
    if (event.kind === "move") {
      if (center && start) {
        const radius = radiusOf(center, start);
        const end = projectToCircle(center, radius, event.pt);
        return {
          state: { ...state, cursor: event.pt },
          preview: [{ type: "Arc", center, radius, start, end }],
        };
      }
      if (center) {
        // Preview the radius as a construction line while placing the start.
        return { state: { ...state, cursor: event.pt }, preview: [{ ...line(center, event.pt), construction: true }] };
      }
      return { state: { ...state, cursor: event.pt }, preview: [] };
    }
    // click
    if (!center) return { state: { anchors: [event.pt], cursor: event.pt }, preview: [] };
    if (!start) {
      if (radiusOf(center, event.pt) < minSize) return { state: { anchors: [center], cursor: event.pt }, preview: [] };
      return { state: { anchors: [center, event.pt], cursor: event.pt }, preview: [] };
    }
    const radius = radiusOf(center, start);
    const end = projectToCircle(center, radius, event.pt);
    return {
      state: emptyState(),
      preview: [],
      committed: [{ type: "Arc", center, radius, start, end }],
      done: true,
    };
  },
};

// ── 3-point arc: start → end → through (SP-4 W2) ─────────────────────────────
//
// The complement of `arcTool`: the two ENDPOINTS are placed first (so they can
// be snapped onto existing geometry and weld to it), and the third click bends
// the arc through the cursor. Radius and sweep are both derived — which is what
// makes it the right tool for closing a profile between two known points, and
// `arcTool` the right one when the centre is what is known.
//
// The through point selects WHICH of the two arcs the chord cuts (see
// `arcThroughPoints`' swap rule), so dragging the cursor across the chord flips
// the bulge instead of jumping to the far arc.
//
// It authors NO tool constraints: the endpoints are ordinary snapped points, so
// the generic Coincident inference welds them to whatever they landed on for
// free — and since a batch of one entity has no intra-batch inference to filter,
// authoring nothing is what KEEPS those welds (see `resolveToolConstraints`).

/** The Arc through S, E and `p`, or null when the gesture is degenerate: the
 *  cursor on the chord, three collinear points, or an arc shorter than the
 *  minimum feature size. */
function arc3pDraft(s: Point2, e: Point2, p: Point2, minSize: number): DraftEntity | null {
  if (perpDistanceTo(s, e, p) < minSize) return null;
  const a = arcThroughPoints(s, e, p);
  // `radius · sweep` is the arc LENGTH — the honest extent to floor, since a
  // huge-radius sliver and a tiny-radius sliver are equally undrawable.
  if (!a || a.radius * a.sweep < minSize) return null;
  return { type: "Arc", center: a.center, radius: a.radius, start: a.start, end: a.end };
}

/** Phase-appropriate rubber-band: the chord as CONSTRUCTION while the second
 *  endpoint is being placed, and again whenever the cursor makes no arc. */
function arc3pPreview(s: Point2 | undefined, e: Point2 | undefined, p: Point2, minSize: number): DraftEntity[] {
  if (!s) return [];
  if (!e) return [{ ...line(s, p), construction: true }];
  const arc = arc3pDraft(s, e, p, minSize);
  return arc ? [arc] : [{ ...line(s, e), construction: true }];
}

export const arc3pTool: ToolMachine = {
  id: "arc3p",
  init: emptyState,
  step(state, event, ctx) {
    if (event.kind === "esc") return { state: emptyState(), preview: [], done: true };
    if (isToolVerb(event)) return ignoreVerb(state);
    const minSize = ctx?.minSize ?? DEFAULT_MIN_SIZE;
    const [start, end] = state.anchors;
    if (event.kind === "move") {
      return {
        state: { ...state, cursor: event.pt },
        preview: arc3pPreview(start, end, event.pt, minSize),
      };
    }
    // click
    if (!start) return { state: { anchors: [event.pt], cursor: event.pt }, preview: [] };
    if (!end) {
      if (dist(start, event.pt) < minSize) return { state: { anchors: [start], cursor: event.pt }, preview: [] };
      return { state: { anchors: [start, event.pt], cursor: event.pt }, preview: [] };
    }
    const arc = arc3pDraft(start, end, event.pt, minSize);
    // Degenerate third click: stay armed on the two endpoints rather than commit
    // a sliver the user cannot see (same rule as every other tool's floor).
    if (!arc) return { state: { anchors: [start, end], cursor: event.pt }, preview: [] };
    return { state: emptyState(), preview: [], committed: [arc], done: true };
  },
};

// ── Point tool: one click commits a Point and re-arms ────────────────────────
//
// The simplest machine there is: no anchors, no preview, `done` on every click so
// the tool immediately re-arms for the next point. Snapping (the controller's
// snapAt) and the Coincident inference come free through the shared commit path —
// a point dropped on an existing endpoint auto-constrains to it.

export const pointTool: ToolMachine = {
  id: "point",
  init: emptyState,
  step(state, event) {
    if (event.kind === "esc") return { state: emptyState(), preview: [], done: true };
    if (isToolVerb(event)) return ignoreVerb(state);
    if (event.kind === "move") return { state: { ...state, cursor: event.pt }, preview: [] };
    return {
      state: emptyState(),
      preview: [],
      committed: [{ type: "Point", p0: event.pt }],
      done: true,
    };
  },
};

// ── Center rectangle: center click → corner click → 4 mirrored lines ─────────
//
// The second click is one CORNER; the opposite corner is its reflection through
// the center, so the rectangle grows symmetrically about the first click. It
// authors NO tool constraints: the four lines get exactly the H/V + corner
// Coincident inference `rect` gets, which is the honest V1 — there is no
// persistent "centered on this point" linkage (that needs a Symmetric pair
// against a construction point; a later WP).

/** Reflect `p` through `c` — the opposite corner of a center-drawn rectangle. */
const reflect = (c: Point2, p: Point2): Point2 => ({ x: 2 * c.x - p.x, y: 2 * c.y - p.y });

export const centerRectTool: ToolMachine = {
  id: "centerRect",
  init: emptyState,
  step(state, event, ctx) {
    if (event.kind === "esc") return { state: emptyState(), preview: [], done: true };
    if (isToolVerb(event)) return ignoreVerb(state);
    const center = state.anchors[0] ?? null;
    if (event.kind === "move") {
      return {
        state: { ...state, cursor: event.pt },
        preview: center ? rectLines(reflect(center, event.pt), event.pt) : [],
      };
    }
    const minSize = ctx?.minSize ?? DEFAULT_MIN_SIZE;
    if (!center) return { state: { anchors: [event.pt], cursor: event.pt }, preview: [] };
    // Degenerate: either HALF-extent below the floor ⇒ ignore this click and stay
    // armed on the center (mirrors rectTool's corner guard, half-extent scale).
    if (Math.abs(event.pt.x - center.x) < minSize || Math.abs(event.pt.y - center.y) < minSize) {
      return { state: { anchors: [center], cursor: event.pt }, preview: [] };
    }
    return {
      state: emptyState(),
      preview: [],
      committed: rectLines(reflect(center, event.pt), event.pt),
      done: true,
    };
  },
};

// ── Slot tool: p0 → p1 (centerline) → width point ────────────────────────────
//
// GEOMETRY. `d` is the unit centerline direction, `n` its CCW normal
// (`n = rot90(d)`). `r` is the PERPENDICULAR distance from the third click to the
// centerline. Commit order is [wall(+n), wall(−n), cap@p1, cap@p0]; each cap is a
// center-start-end arc sweeping 180° CCW over the OUTSIDE of the slot:
//   cap@p1: start p1−r·n → end p1+r·n  (passes through p1+r·d)
//   cap@p0: start p0+r·n → end p0−r·n  (passes through p0−r·d)
// (CCW from −n reaches +d, and CCW from +n reaches −d — that is what puts each
// sweep on the far side of its own end, instead of doubling back into the slot.)
// The four endpoints have degree 2 exactly, so the slot reads as ONE closed loop.
//
// AUTHORED CONSTRAINTS: Coincident ×4 (each wall endpoint welded to the cap
// endpoint it touches) + Equal ×1 (cap ↔ cap radius).
//
// The welds became expressible in W0b: the worker mints an arc's endpoints as
// real solver points and registers `<id>.start`/`<id>.end`, so `sketchWireMap`
// marshals an arc-endpoint Coincident as "the ARC's uuid + a position" instead of
// dropping it. The slot is now a genuinely CONNECTED loop in the solver — drag a
// wall and its caps follow.
//
// NO entity-level Tangent ×4 any more — REMOVED, not forgotten. An entity-level
// `Tangent(line, arc)` is `distance(arcCenter, line) == radius`. Once the wall's
// endpoint is WELDED to the arc's endpoint the wall necessarily passes through a
// point at exactly `radius` from the centre, so that distance sits at its
// MAXIMUM: the constraint's gradient vanishes on the manifold of the others and
// PlaneGCS diagnoses all four as redundant — which `upsert_state` renders as
// OverConstrained. Welds + Tangents is strictly worse than welds alone (same DOF,
// plus a bogus over-constrained badge). Pinned by the worker ctest
// `test_sketch_arc_endpoints.cpp::test_endpoint_weld_makes_entity_tangent_redundant`.
// Proper endpoint tangency needs its own constraint kind (as in FreeCAD) — open.
//
// REAL-LANE DOF 9: 2 lines (4 each) + 2 arcs (5 each) = 18 free, minus 4
// Coincident (2 each) and 1 Equal. The worker's minted arc endpoints add 4 more
// parameters per arc and its internal arc rules remove them again, so they do not
// move this number. (A slot round-tripped through the RUST typed document reads
// 14, not 9: the wire emits each arc's centre BOTH as a standalone `Point` entity
// and as an inline `center` coordinate, and the worker mints a point for the
// latter — 2 stray free points, +4 DOF. Pre-existing, tracked separately.)

interface SlotFrame {
  /** Unit centerline direction. */
  d: Point2;
  /** Unit CCW normal of `d`. */
  n: Point2;
}

function slotFrame(p0: Point2, p1: Point2): SlotFrame | null {
  const dx = p1.x - p0.x;
  const dy = p1.y - p0.y;
  const len = Math.hypot(dx, dy);
  if (len < 1e-12) return null;
  return { d: { x: dx / len, y: dy / len }, n: { x: -dy / len, y: dx / len } };
}

const offsetBy = (p: Point2, v: Point2, k: number): Point2 => ({ x: p.x + v.x * k, y: p.y + v.y * k });

/** Perpendicular distance from `p` to the infinite line p0→p1 (0 when degenerate).
 *  One implementation, in `arcMath` — the 3-point arc needs the same number. */
export const perpDistance = perpDistanceTo;

/** The 4 slot drafts in commit order: [wall(+n), wall(−n), cap@p1, cap@p0]. */
export function slotDrafts(p0: Point2, p1: Point2, r: number): DraftEntity[] {
  const f = slotFrame(p0, p1);
  if (!f) return [];
  const a0 = offsetBy(p0, f.n, r);
  const a1 = offsetBy(p1, f.n, r);
  const b0 = offsetBy(p0, f.n, -r);
  const b1 = offsetBy(p1, f.n, -r);
  return [
    line(a0, a1),
    line(b0, b1),
    { type: "Arc", center: p1, radius: r, start: b1, end: a1 },
    { type: "Arc", center: p0, radius: r, start: a0, end: b0 },
  ];
}

// Refs index `slotDrafts`: 0 = wall(+n) a0→a1, 1 = wall(−n) b0→b1,
// 2 = cap@p1 (start b1, end a1), 3 = cap@p0 (start a0, end b0).
const SLOT_CONSTRAINTS: ToolConstraintSpec[] = [
  { type: "Coincident", refs: [0, 2], positions: ["End", "End"] }, // a1
  { type: "Coincident", refs: [1, 2], positions: ["End", "Start"] }, // b1
  { type: "Coincident", refs: [0, 3], positions: ["Start", "Start"] }, // a0
  { type: "Coincident", refs: [1, 3], positions: ["Start", "End"] }, // b0
  { type: "Equal", refs: [2, 3] },
];

export const slotTool: ToolMachine = {
  id: "slot",
  init: emptyState,
  step(state, event, ctx) {
    if (event.kind === "esc") return { state: emptyState(), preview: [], done: true };
    if (isToolVerb(event)) return ignoreVerb(state);
    const [p0, p1] = state.anchors;
    if (event.kind === "move") {
      if (p0 && p1) {
        return {
          state: { ...state, cursor: event.pt },
          preview: slotDrafts(p0, p1, perpDistance(p0, p1, event.pt)),
        };
      }
      if (p0) {
        // Centerline rubber-band as CONSTRUCTION (same convention as the arc
        // radius preview): it aims the gesture, it is never committed.
        return {
          state: { ...state, cursor: event.pt },
          preview: [{ ...line(p0, event.pt), construction: true }],
        };
      }
      return { state: { ...state, cursor: event.pt }, preview: [] };
    }
    // click
    const minSize = ctx?.minSize ?? DEFAULT_MIN_SIZE;
    if (!p0) return { state: { anchors: [event.pt], cursor: event.pt }, preview: [] };
    if (!p1) {
      if (dist(p0, event.pt) < minSize) return { state: { anchors: [p0], cursor: event.pt }, preview: [] };
      return { state: { anchors: [p0, event.pt], cursor: event.pt }, preview: [] };
    }
    const r = perpDistance(p0, p1, event.pt);
    if (r < minSize) return { state: { anchors: [p0, p1], cursor: event.pt }, preview: [] };
    return {
      state: emptyState(),
      preview: [],
      committed: slotDrafts(p0, p1, r),
      committedConstraints: SLOT_CONSTRAINTS,
      done: true,
    };
  },
};

// ── Polygon tool: center click → vertex click → n lines + circumcircle ────────
//
// The second click places ONE vertex; the rest of the ring is that vertex rotated
// about the center in 2π/n steps. Commit is n ring lines (index 0…n−1) plus ONE
// CONSTRUCTION circumcircle (index n) — construction so it is excluded from region
// detection and the polygon stays a single clean profile.
//
// AUTHORED CONSTRAINTS make the ring a real regular polygon rather than n loose
// lines that merely happen to be placed correctly:
//   - Coincident ×n  — line k End ↔ line k+1 Start (the closed vertex ring)
//   - OnCurve   ×n   — line k Start lies ON the circumcircle
//   - Equal     ×(n−1) — consecutive lines share a length
// DOF: (4n + 3) − 2n − n − (n − 1) = 4 (centre 2 + radius 1 + rotation 1).

export const DEFAULT_POLYGON_SIDES = 6;
export const MIN_POLYGON_SIDES = 3;
export const MAX_POLYGON_SIDES = 12;

/** Clamp a requested side count into [3, 12] (integer). */
export function clampSides(n: number): number {
  if (!Number.isFinite(n)) return DEFAULT_POLYGON_SIDES;
  return Math.max(MIN_POLYGON_SIDES, Math.min(MAX_POLYGON_SIDES, Math.round(n)));
}

/** Ring vertices of the regular n-gon whose first vertex is `vertex`. */
export function polygonVertices(center: Point2, vertex: Point2, n: number): Point2[] {
  const r = radiusOf(center, vertex);
  const a0 = Math.atan2(vertex.y - center.y, vertex.x - center.x);
  const out: Point2[] = [];
  for (let k = 0; k < n; k++) {
    const a = a0 + (2 * Math.PI * k) / n;
    out.push({ x: center.x + r * Math.cos(a), y: center.y + r * Math.sin(a) });
  }
  return out;
}

/** n ring lines (0…n−1) + the CONSTRUCTION circumcircle (index n). */
export function polygonDrafts(center: Point2, vertex: Point2, n: number): DraftEntity[] {
  const v = polygonVertices(center, vertex, n);
  const drafts: DraftEntity[] = v.map((p, k) => line(p, v[(k + 1) % n]));
  drafts.push({ type: "Circle", center, radius: radiusOf(center, vertex), construction: true });
  return drafts;
}

/** Coincident ring ×n + OnCurve ×n + Equal chain ×(n−1) — see the header. */
export function polygonConstraints(n: number): ToolConstraintSpec[] {
  const out: ToolConstraintSpec[] = [];
  for (let k = 0; k < n; k++) {
    out.push({ type: "Coincident", refs: [k, (k + 1) % n], positions: ["End", "Start"] });
  }
  for (let k = 0; k < n; k++) {
    out.push({ type: "OnCurve", refs: [k, n], positions: ["Start", undefined] });
  }
  for (let k = 0; k < n - 1; k++) out.push({ type: "Equal", refs: [k, k + 1] });
  return out;
}

/** Reset state that PRESERVES the side count (sticky across commits + Esc). */
const polygonReset = (sides: number): ToolState => ({ anchors: [], cursor: null, sides });

export const polygonTool: ToolMachine = {
  id: "polygon",
  init: () => polygonReset(DEFAULT_POLYGON_SIDES),
  step(state, event, ctx) {
    const sides = clampSides(state.sides ?? DEFAULT_POLYGON_SIDES);
    const center = state.anchors[0] ?? null;
    if (event.kind === "sides") {
      const n = clampSides(event.n);
      return {
        state: { ...state, sides: n },
        preview: center && state.cursor ? polygonDrafts(center, state.cursor, n) : [],
      };
    }
    if (event.kind === "arcToggle") return ignoreVerb(state); // line-tool verb
    if (event.kind === "esc") return { state: polygonReset(sides), preview: [], done: true };
    if (event.kind === "move") {
      return {
        state: { ...state, cursor: event.pt },
        preview: center ? polygonDrafts(center, event.pt, sides) : [],
      };
    }
    const minSize = ctx?.minSize ?? DEFAULT_MIN_SIZE;
    if (!center) return { state: { anchors: [event.pt], cursor: event.pt, sides }, preview: [] };
    if (radiusOf(center, event.pt) < minSize) {
      return { state: { anchors: [center], cursor: event.pt, sides }, preview: [] };
    }
    return {
      state: polygonReset(sides),
      preview: [],
      committed: polygonDrafts(center, event.pt, sides),
      committedConstraints: polygonConstraints(sides),
      done: true,
    };
  },
};

export const TOOL_MACHINES: Record<string, ToolMachine> = {
  line: lineTool,
  rect: rectTool,
  centerRect: centerRectTool,
  circle: circleTool,
  ellipse: ellipseTool,
  arc: arcTool,
  arc3p: arc3pTool,
  polygon: polygonTool,
  slot: slotTool,
  point: pointTool,
};

/** DraftEntity → wire entity fields (plane coords as [u,v] pairs). */
export function draftToEntityFields(d: DraftEntity): {
  type: SketchEntityType;
  construction?: boolean;
  p0?: [number, number];
  p1?: [number, number];
  center?: [number, number];
  radius?: number;
  start?: [number, number];
  end?: [number, number];
  majorR?: number;
  minorR?: number;
  rotation?: number;
} {
  return {
    type: d.type,
    ...(d.construction ? { construction: true } : {}),
    ...(d.p0 ? { p0: asPair(d.p0) } : {}),
    ...(d.p1 ? { p1: asPair(d.p1) } : {}),
    ...(d.center ? { center: asPair(d.center) } : {}),
    ...(d.radius !== undefined ? { radius: d.radius } : {}),
    ...(d.start ? { start: asPair(d.start) } : {}),
    ...(d.end ? { end: asPair(d.end) } : {}),
    ...(d.majorR !== undefined ? { majorR: d.majorR } : {}),
    ...(d.minorR !== undefined ? { minorR: d.minorR } : {}),
    // `rotation: 0` is meaningful (an axis-aligned ellipse) — emit it whenever the
    // draft carries one, unlike the coordinate fields whose absence means "n/a".
    ...(d.rotation !== undefined ? { rotation: d.rotation } : {}),
  };
}

/**
 * Merge a step's TOOL-AUTHORED constraints with the auto-constraint inference —
 * the one place `ToolConstraintSpec.refs` become real entity ids.
 *
 * `newEntities` are this step's `committed` drafts AFTER the id-assign, in the
 * SAME order (that ordering is the whole meaning of `refs`). A spec whose ref is
 * out of range is dropped rather than mis-bound. A `positions` slot left
 * `undefined` marshals as `""`, which `sketchWireMap.resolveRef` reads as falsy
 * and therefore resolves as an ENTITY ref — the same convention the re-entry
 * hydration path uses.
 *
 * THE FILTER: when a tool authors its own constraints, every inference that only
 * relates entities from this SAME batch is dropped, keeping only inferences that
 * touch at least one PRE-EXISTING entity. The draw-commit path has no
 * reject-on-conflict (unlike `applyConstraint` / `commitDimensionConstraint`), so
 * a tool-authored constraint duplicated by an inferred one would strand the sketch
 * in OverConstrained with no way back. The cost is honest and local: an
 * axis-aligned slot/polygon does NOT also pick up the H/V it would have inferred.
 * Tools with no specs are untouched — `inferred` is returned as-is.
 */
export function resolveToolConstraints(
  specs: ToolConstraintSpec[] | undefined,
  newEntities: SketchEntity[],
  inferred: SketchConstraint[],
  nextConstraintId: () => string,
): SketchConstraint[] {
  if (!specs || specs.length === 0) return inferred;
  const authored: SketchConstraint[] = [];
  for (const spec of specs) {
    const entities = spec.refs.map((i) => newEntities[i]?.id);
    if (entities.some((id) => id === undefined)) continue; // malformed ref — never guess
    const c: SketchConstraint = {
      id: nextConstraintId(),
      type: spec.type,
      entities: entities as string[],
    };
    if (spec.positions?.some((p) => p !== undefined)) {
      c.positions = spec.positions.map((p) => p ?? "") as ConstraintPosition[];
    }
    authored.push(c);
  }
  const fresh = new Set(newEntities.map((e) => e.id));
  const kept = inferred.filter((c) => c.entities.some((id) => !fresh.has(id)));
  return [...authored, ...kept];
}
