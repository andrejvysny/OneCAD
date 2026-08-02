/*
 * fsmLog — the model-tool state machines' observability wrapper.
 *
 * `withPhaseLog(tool, step)` returns a step function with the SAME signature, so
 * ModelToolController wraps the eight reducers once at the import alias and not
 * one of its ~200 call sites changes.
 *
 * ── Why only on a PHASE CHANGE ───────────────────────────────────────────────
 * A drag feeds `{kind:"drag"}` at pointer-move frequency and every one of those
 * events keeps `phase === "dragging"`, so the phase-change guard makes the
 * no-drag-frequency policy structural rather than a rule someone has to
 * remember. What survives is exactly the interesting skeleton: idle → armed →
 * dragging → armed → committing → idle, with the event that caused each hop.
 *
 * `regionSelectStep` is deliberately NOT wrappable — `RegionSelectState` has no
 * `phase` (it is an `{active, selected}` pick set), so there is nothing to
 * transition on.
 */
import { logDebug } from "@/debug/log";

/** The shape every wrapped reducer's state satisfies. */
interface PhaseState {
  phase: string;
}

/** The shape every wrapped reducer's event satisfies. */
interface KindEvent {
  kind: string;
}

/** The shape every wrapped reducer returns. */
interface StepResult<S> {
  state: S;
  effect: string;
}

/**
 * Wrap a pure `(state, event) => {state, effect}` reducer so a PHASE TRANSITION
 * emits one `fsm` debug event. Same-phase steps (drags, parameter edits) are
 * silent by construction.
 */
export function withPhaseLog<S extends PhaseState, E extends KindEvent, R extends StepResult<S>>(
  tool: string,
  step: (state: S, event: E) => R,
): (state: S, event: E) => R {
  return (state, event) => {
    const out = step(state, event);
    if (out.state.phase !== state.phase) {
      logDebug("fsm", `${tool}: ${state.phase} → ${out.state.phase} (${event.kind})`, {
        tool,
        event: event.kind,
        from: state.phase,
        to: out.state.phase,
        effect: out.effect,
      });
    }
    return out;
  };
}
