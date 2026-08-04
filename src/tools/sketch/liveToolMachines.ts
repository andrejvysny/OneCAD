/*
 * `withLiveDims` — the TRANSPARENT decorator that gives every draw machine live
 * dimensions without any machine knowing about them.
 *
 * Two jobs, in this order and no other:
 *   1. PROJECT the incoming pointer point through the phase's `DimFrame`, so
 *      locked fields pin exactly and unlocked ones land on the rounding quantum.
 *   2. DESCRIBE the resulting phase as chip descriptors on `step.dims`.
 *
 * ORDER IS LOAD-BEARING: locks BEFORE rounding BEFORE the machine. A typed value
 * is never re-rounded, and because the projection happens on the way IN, the
 * preview, the committed entity and the chip are all the same number by
 * construction rather than by three call sites agreeing.
 *
 * TRANSPARENCY: with no locks and no quantum the event passes through BY
 * IDENTITY. A measure/rebuild round-trip is only exact to ~1e-15, and perturbing
 * every raw click coordinate in the app to buy nothing is not a trade worth
 * making — so `LIVE_TOOL_MACHINES` is a drop-in for `TOOL_MACHINES`.
 *
 * LOCK LIFETIME is the controller's, not this module's: locks are cleared on
 * `done`/`committed`, tool change and session teardown, and deliberately PERSIST
 * across a non-committing click so an arc's typed radius survives phase 1 → 2.
 */
import {
  DIM_ALIASES,
  describeDims,
  roundTo,
  type DimDomain,
  type DimLocks,
  type DimQuantum,
  type DimValues,
} from "./liveDimension";
import { dimFrame, type DimFrame } from "./liveDimFrames";
import {
  TOOL_MACHINES,
  type ToolContext,
  type ToolEvent,
  type ToolMachine,
} from "./toolMachine";

/** Rounding step for a domain; 0 ⇒ do not round (a side COUNT is already whole). */
function quantumFor(domain: DimDomain, q: DimQuantum): number {
  if (domain === "angle") return q.angle;
  return domain === "length" ? q.length : 0;
}

/** With exactly one half of an alias pair LOCKED, the other's measured value is a
 *  stale second opinion about the same geometry — drop it so `rebuild` cannot
 *  pick the wrong one (see `DIM_ALIASES`). */
function dropAliasedSiblings(values: DimValues, locks: DimLocks | undefined): void {
  for (const [a, b] of DIM_ALIASES) {
    const lockedA = locks?.[a] !== undefined;
    const lockedB = locks?.[b] !== undefined;
    if (lockedA !== lockedB) delete values[lockedA ? b : a];
  }
}

/**
 * Re-place a pointer event's point so every field of the current phase reads its
 * locked value, or its measured value rounded to the quantum. Non-pointer verbs
 * (`esc`, `sides`) and phases with no frame pass through untouched.
 */
export function projectEvent(
  frame: DimFrame | null,
  event: ToolEvent,
  locks?: DimLocks,
  quantum?: DimQuantum | null,
): ToolEvent {
  if (!frame || (event.kind !== "click" && event.kind !== "move")) return event;
  const hasLocks = locks !== undefined && Object.keys(locks).length > 0;
  if (!hasLocks && !quantum) return event; // transparency — see the header
  const values = frame.measure(event.pt);
  for (const spec of frame.fields) {
    const locked = locks?.[spec.field];
    if (locked !== undefined) {
      values[spec.field] = locked;
      continue;
    }
    const measured = values[spec.field];
    if (!quantum || measured === undefined) continue;
    values[spec.field] = roundTo(measured, quantumFor(spec.domain, quantum));
  }
  dropAliasedSiblings(values, locks);
  return { kind: event.kind, pt: frame.rebuild(values, event.pt) };
}

/** Wrap a raw draw machine so it projects locks/quantum and emits chip
 *  descriptors. Identity behaviour when `ctx` carries neither. */
export function withLiveDims(m: ToolMachine): ToolMachine {
  return {
    id: m.id,
    init: () => m.init(),
    step(state, event, ctx?: ToolContext) {
      const frame = dimFrame(m.id, state.anchors, state.sides);
      const step = m.step(state, projectEvent(frame, event, ctx?.locks, ctx?.quantum), ctx);
      // Describe the phase the gesture landed IN, not the one it came from — and
      // never after `done`, when the machine has already reset to bare anchors.
      const nextFrame = dimFrame(m.id, step.state.anchors, step.state.sides);
      const cursor = step.state.cursor;
      const dims =
        nextFrame && cursor && !step.done ? describeDims(nextFrame, cursor, ctx?.locks ?? {}) : [];
      return dims.length > 0 ? { ...step, dims } : step;
    },
  };
}

/** `TOOL_MACHINES` with live dimensions — what SketchController selects from. */
export const LIVE_TOOL_MACHINES: Record<string, ToolMachine> = Object.fromEntries(
  Object.entries(TOOL_MACHINES).map(([id, m]) => [id, withLiveDims(m)]),
);
