/*
 * Sketch drawing tool machines (PURE state machines) operating in plane (u,v)
 * coords. Each machine is a reducer over discrete events the controller feeds it:
 *
 *   click(pt) — a tap (pointerdown+up without a drag) at a snapped plane point
 *   move(pt)  — pointer moved to a snapped plane point (drives the rubber-band)
 *   esc       — cancel/end the current chain
 *
 * `step` returns the next state, the PREVIEW draft entities (rubber-band, no ids)
 * and, when a gesture completes, the COMMITTED draft entities the controller
 * id-assigns → auto-constrains → sends to `sketchUpsert`. Chaining semantics:
 *   - line  : click-click-… chains segments; Esc ends the chain.
 *   - rect  : 2 clicks (corner→corner) commit 4 lines, then reset for the next.
 *   - circle: 2 clicks (center→radius) commit one circle, then reset.
 *   - arc   : 3 clicks (center→start→end) commit one arc, then reset.
 *
 * Pure ⇒ pointer sequences are unit-tested end to end.
 */
import type { SketchEntityType } from "@/ipc/types";
import type { Point2 } from "@/viewport/engine/sketchBasis";

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
}

export type ToolEvent =
  | { kind: "click"; pt: Point2 }
  | { kind: "move"; pt: Point2 }
  | { kind: "esc" };

/** Optional per-step context. `minSize` is the smallest accepted feature extent in
 *  PLANE (world) units — a click/drag below it is a degenerate no-op. The controller
 *  passes a screen-scaled value (≈4px worth of world) so the reject radius is
 *  constant on screen at any zoom; the default keeps the machines usable bare. */
export interface ToolContext {
  minSize?: number;
}

/** Fallback minimum feature size when no context is supplied (numerically tiny —
 *  rejects only truly coincident points). */
const DEFAULT_MIN_SIZE = 1e-9;

/** Accumulated anchor points placed so far + the live cursor. */
export interface ToolState {
  anchors: Point2[];
  cursor: Point2 | null;
}

export interface ToolStep {
  state: ToolState;
  preview: DraftEntity[];
  committed?: DraftEntity[];
  /** True when the current gesture ended (chain closed / entity committed). */
  done?: boolean;
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

export const lineTool: ToolMachine = {
  id: "line",
  init: emptyState,
  step(state, event, ctx) {
    if (event.kind === "esc") {
      return { state: emptyState(), preview: [], done: true };
    }
    if (event.kind === "move") {
      const last = state.anchors[state.anchors.length - 1] ?? null;
      const next = { ...state, cursor: event.pt };
      return { state: next, preview: last ? [line(last, event.pt)] : [] };
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
      return { state: emptyState(), preview: [], committed: [line(last, first)], done: true };
    }
    return {
      state: { anchors: [...state.anchors, event.pt], cursor: event.pt },
      preview: [],
      committed: [line(last, event.pt)],
    };
  },
};

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

export const TOOL_MACHINES: Record<string, ToolMachine> = {
  line: lineTool,
  rect: rectTool,
  circle: circleTool,
  arc: arcTool,
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
  };
}
