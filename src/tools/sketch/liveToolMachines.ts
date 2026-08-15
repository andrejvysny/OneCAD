/*
 * `withLiveDims` — the TRANSPARENT decorator that gives every draw machine live
 * dimensions without any machine knowing about them.
 *
 * Two jobs, in this order and no other:
 *   1. PROJECT the incoming pointer point through the phase's `DimFrame`, so
 *      locked fields pin exactly and ACCEPTED numeric values land where the snap
 *      decision said they should.
 *   2. DESCRIBE the resulting phase as chip descriptors on `step.dims`.
 *
 * ── This module no longer OWNS the rounding decision (SNAP P2) ───────────────
 * It used to take a `DimQuantum` and round every measured value itself. That
 * made cursor rounding a downstream rule with no way to compose with a
 * directional intent and no way to report why it did not fire: the snap engine
 * had to fake it by handing back `kind: "none"` and hoping. Rounding is now an
 * explicit CANDIDATE resolved in `snapArbitration.ts` alongside every geometry
 * and guide candidate, and this decorator applies the values it accepted.
 * A typed lock still wins over an accepted value for the same field.
 *
 * TRANSPARENCY: with no locks and no accepted values the event passes through BY
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
  type DimFieldId,
  type DimLocks,
  type DimValues,
} from "./liveDimension";
import { dimFrame, type DimChain, type DimFrame } from "./liveDimFrames";
import {
  TOOL_MACHINES,
  type ToolContext,
  type ToolEvent,
  type ToolMachine,
  type ToolState,
} from "./toolMachine";

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
 * locked value, or the value the snap decision accepted for it. Non-pointer
 * verbs (`esc`, `sides`) and phases with no frame pass through untouched.
 *
 * Covers EVERY value `measure` returns, not only the phase's exposed chips — a
 * first line leg hides its ∠ field until a reference leg exists, yet an accepted
 * whole-degree angle must still steer the geometry.
 */
export function projectEvent(
  frame: DimFrame | null,
  event: ToolEvent,
  locks?: DimLocks,
  accepted?: DimValues | null,
): ToolEvent {
  if (!frame || (event.kind !== "click" && event.kind !== "move")) return event;
  const hasLocks = locks !== undefined && Object.keys(locks).length > 0;
  const hasAccepted = accepted !== undefined && accepted !== null && Object.keys(accepted).length > 0;
  if (!hasLocks && !hasAccepted) return event; // transparency — see the header
  const values = frame.measure(event.pt);
  // ACCEPTED values first, then locks on top. A lock is exact and outranks
  // everything, including an accepted cursor rounding for the same field.
  for (const id of Object.keys(values) as DimFieldId[]) {
    const value = accepted?.[id];
    if (value !== undefined) values[id] = value;
    const locked = locks?.[id];
    if (locked !== undefined) values[id] = locked;
  }
  dropAliasedSiblings(values, locks);
  return { kind: event.kind, pt: frame.rebuild(values, event.pt) };
}

/** The line tool's chain facts for `dimFrame` — the machine's OWN recorded
 *  tangent first, the controller's seed only while it has none (same precedence
 *  `lineTool` itself applies, so the chip and the geometry cannot disagree). */
const chainOf = (state: ToolState, ctx?: ToolContext): DimChain => ({
  arcMode: state.arcMode,
  tangent: state.chainTangent ?? ctx?.tangent,
});

/** Wrap a raw draw machine so it projects locks + accepted values and emits
 *  chip descriptors. Identity behaviour when `ctx` carries neither. */
export function withLiveDims(m: ToolMachine): ToolMachine {
  return {
    id: m.id,
    init: () => m.init(),
    step(state, event, ctx?: ToolContext) {
      const frame = dimFrame(m.id, state.anchors, state.sides, chainOf(state, ctx));
      const step = m.step(state, projectEvent(frame, event, ctx?.locks, ctx?.dimValues), ctx);
      // Describe the phase the gesture landed IN, not the one it came from — and
      // never after `done`, when the machine has already reset to bare anchors.
      const nextFrame = dimFrame(m.id, step.state.anchors, step.state.sides, chainOf(step.state, ctx));
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
