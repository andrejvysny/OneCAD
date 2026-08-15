/**
 * Poll an assertion by TURNS of the event loop, never by wall clock.
 *
 * Why this exists. `vi.waitFor` and RTL's `waitFor` both budget MILLISECONDS.
 * Vitest runs test files in parallel workers, so a starved event loop can blow
 * that budget while the work under test — pure microtasks against
 * `mockResolvedValue` stubs — has long since finished. Two tests went red in CI
 * exactly this way, and raising the budget is not a fix: `InspectorPanel.test.tsx`
 * had already been raised to 4 s and went red anyway. A larger number only makes
 * the red rarer and the suite slower.
 *
 * Counting turns decouples the wait from the clock. Under load a turn takes
 * longer in wall-clock terms, but it still happens, so the number of turns
 * needed to drain N chained promises is a property of the CODE and not of the
 * machine.
 *
 * This is still polling, and it is deliberately not dressed up as more than
 * that. The strictly deterministic alternative would be for the production
 * controller to expose its in-flight commit promise — but those commits are
 * fire-and-forget BY DESIGN (a row must not block on its own edit landing), so
 * exposing one would change production shape to serve a test. Turn-counting
 * removes the load sensitivity, which is the actual failure mode, without
 * touching production.
 *
 * @param assert  Throws while the condition does not hold. Called once per turn.
 * @param turns   How many event-loop turns to allow. The default is far above
 *                any real path here (the deepest is two chained round-trips);
 *                it is a runaway guard, not a budget to tune.
 * @param turn    How to advance one turn. React callers must pass an `act`-aware
 *                turn so renders scheduled by the resolved promises are flushed
 *                inside `act` — see `settleUntilAct` in the React tests.
 */
export async function settleUntil(
  assert: () => void,
  options: { turns?: number; turn?: () => Promise<void> } = {},
): Promise<void> {
  const turns = options.turns ?? 50;
  const turn = options.turn ?? (() => new Promise<void>((resolve) => setTimeout(resolve, 0)));
  for (let i = 0; i < turns; i += 1) {
    try {
      assert();
      return;
    } catch {
      await turn();
    }
  }
  // Out of turns: run it once more OUTSIDE the catch so the real assertion
  // error surfaces with its own message and diff, not a generic timeout.
  assert();
}
