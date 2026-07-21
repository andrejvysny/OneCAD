/*
 * Shared sketch-edit helpers usable from React (the badge layer's dimension
 * chip) without the imperative SketchController. Keeps store + engine updates in
 * one place so an edit outside the controller stays consistent.
 */
import type { CadClient } from "@/ipc/client";
import type { SketchConstraint, SketchEntity, SketchSession } from "@/ipc/types";
import { getViewportEngine } from "@/viewport/engineBridge";
import { documentStore, docSketchStatus } from "@/stores/documentStore";
import { viewportStore } from "@/stores/viewportStore";
import { sketchStore, type SketchSnapshot } from "@/stores/sketchStore";
import { applySolvedPositions } from "@/ipc/sketchWireMap";
import { toolChipStore } from "@/stores/toolChipStore";
import { planePointToWorld } from "@/viewport/engine/sketchBasis";
import { isConflictStatus } from "./dimensionTool";
import type { ApplicableConstraint } from "./constraintApplicability";
import { buildAppliedConstraint, buildAppliedDimension } from "./constraintAuthoring";

/*
 * ALL session mutators are serialized through one promise chain. Each reads
 * `sketchStore.session` INSIDE its queued turn, so a second user action fired before
 * the first `sketchUpsert` resolves rebases on the settled result instead of
 * capturing the same stale arrays (which would make `marshalUpsert`'s shared id-map
 * diff synthesize a removal of the first action's constraint and let either response
 * clobber the store). The SketchController wraps its own commit/mirror/drag write-
 * backs through the SAME queue (it imports `enqueueSketchMutation`), so drawing and
 * constraint edits share one ordering.
 */
let sketchMutationChain: Promise<unknown> = Promise.resolve();
export function enqueueSketchMutation<T>(fn: () => Promise<T>): Promise<T> {
  const run = sketchMutationChain.then(fn, fn);
  sketchMutationChain = run.catch(() => undefined);
  return run;
}

/**
 * Resolve once the CURRENT mutation chain settles (drawing, constraint edits, undo).
 * `finishSketch` awaits this before flipping to model mode + computing regions so a
 * still-in-flight upsert can't publish geometry after the profile was captured. The
 * queue is NEVER cleared as a cancellation signal — this only drains what's pending.
 */
export function flushSketchMutations(): Promise<void> {
  return sketchMutationChain.then(
    () => undefined,
    () => undefined,
  );
}

/**
 * Fencing gate (see sketchStore.sessionGeneration). A queued mutation captures the
 * generation at its turn start and calls this after every await: a bumped generation
 * (a newer setSession superseded it) or a torn-down session (null) means the write
 * must be silently dropped. Subsumes the old `if (!session) return` post-await guard.
 */
function sessionSuperseded(gen: number): boolean {
  const s = sketchStore.getState();
  return s.session === null || s.sessionGeneration !== gen;
}

/**
 * Build a reject hint that NAMES the constraint the rejected one clashed with. The
 * solve's `conflicting` ids (frontend ids) resolve against `constraints` (the array
 * INCLUDING the just-authored one); the authored id is excluded so the hint points
 * at the OTHER party. Falls back to the generic over-constrain text when no
 * conflicting id is available (e.g. the mock lane, which never reports ids).
 */
function rejectConflictHint(
  constraints: SketchConstraint[],
  conflicting: string[] | undefined,
  authoredId: string,
  noun: "Constraint" | "Dimension",
): string {
  const generic = `${noun} removed — it would over-constrain the sketch`;
  const ids = conflicting ?? [];
  const pick = ids.find((id) => id !== authoredId) ?? ids[0];
  if (!pick) return generic;
  const type = constraints.find((c) => c.id === pick)?.type ?? "constraint";
  return `${noun} removed — conflicts with ${type} (${pick})`;
}

/** Edit a dimensional constraint's value → re-solve → refresh geometry + DOF. */
export function editConstraintValue(
  client: CadClient,
  constraintId: string,
  value: number,
): Promise<void> {
  return enqueueSketchMutation(() => editConstraintValueNow(client, constraintId, value));
}

async function editConstraintValueNow(
  client: CadClient,
  constraintId: string,
  value: number,
): Promise<void> {
  const gen = sketchStore.getState().sessionGeneration;
  const session = sketchStore.getState().session;
  if (!session) return;
  const before: SketchSnapshot = { entities: session.entities, constraints: session.constraints };
  const constraints = session.constraints.map((c) =>
    c.id === constraintId ? { ...c, value } : c,
  );
  const result = await client.sketchUpsert(session.sketchId, session.entities, constraints);
  if (sessionSuperseded(gen)) return;

  const next = { ...session, constraints, dof: result.dof, status: result.status };
  sketchStore.getState().setSession(next);
  sketchStore.getState().setConflicting(result.conflicting ?? []);
  getViewportEngine()?.updateSketchSession(next.plane, next.entities, next.status);
  documentStore.getState().setSketchSolve(session.sketchId, result.dof, docSketchStatus(result.status));
  viewportStore.setState({ dofBadge: result.dof });

  // COALESCE consecutive edits of the SAME constraint into one undo entry: if the
  // top undo snapshot was pushed by an edit to this constraint, keep it (undo returns
  // to the value BEFORE the run of edits). An intervening op / undo resets provenance.
  const store = sketchStore.getState();
  const last = store.lastUndoPush;
  const coalesce =
    last?.kind === "editConstraintValue" &&
    last.constraintId === constraintId &&
    store.undoStack.length > 0;
  if (!coalesce) store.pushUndoSnapshot(before, { kind: "editConstraintValue", constraintId });
}

/**
 * Author a NEW dimensional constraint (Dimension tool) → re-solve, refresh
 * geometry + DOF. If the solve reports over-constrained/conflicting, REJECT it:
 * remove the constraint, re-solve to the prior state, and surface a status hint
 * (`{ rejected: true }`). The solver's status is the only signal the mock lane
 * exposes — see `isConflictStatus` for the granularity seam.
 */
export function commitDimensionConstraint(
  client: CadClient,
  constraint: SketchConstraint,
): Promise<{ rejected: boolean; hint?: string }> {
  return enqueueSketchMutation(() => commitDimensionConstraintNow(client, constraint));
}

async function commitDimensionConstraintNow(
  client: CadClient,
  constraint: SketchConstraint,
): Promise<{ rejected: boolean; hint?: string }> {
  const gen = sketchStore.getState().sessionGeneration;
  const session = sketchStore.getState().session;
  if (!session) return { rejected: false };
  const before: SketchSnapshot = { entities: session.entities, constraints: session.constraints };

  const constraints = [...session.constraints, constraint];
  const result = await client.sketchUpsert(session.sketchId, session.entities, constraints);
  if (sessionSuperseded(gen)) return { rejected: false };

  if (isConflictStatus(result.status)) {
    // Reject-on-conflict: name the clashing constraint from the failed solve BEFORE
    // dropping the dimension and restoring the previous solve.
    const hint = rejectConflictHint(constraints, result.conflicting, constraint.id, "Dimension");
    const restore = await client.sketchUpsert(session.sketchId, session.entities, session.constraints);
    if (sessionSuperseded(gen)) return { rejected: true, hint };
    const reverted = { ...session, dof: restore.dof, status: restore.status };
    sketchStore.getState().setSession(reverted);
    sketchStore.getState().setConflicting(restore.conflicting ?? []);
    getViewportEngine()?.updateSketchSession(reverted.plane, reverted.entities, reverted.status);
    documentStore.getState().setSketchSolve(session.sketchId, restore.dof, docSketchStatus(restore.status));
    viewportStore.setState({ dofBadge: restore.dof });
    return { rejected: true, hint }; // rejected: no undo snapshot pushed
  }

  const solvedEntities = applySolvedPositions(session.entities, result.solvedPositions ?? {});
  const next = { ...session, entities: solvedEntities, constraints, dof: result.dof, status: result.status };
  sketchStore.getState().setSession(next);
  sketchStore.getState().setConflicting(result.conflicting ?? []);
  getViewportEngine()?.updateSketchSession(next.plane, solvedEntities, next.status);
  documentStore.getState().setSketchSolve(session.sketchId, result.dof, docSketchStatus(result.status));
  viewportStore.setState({ dofBadge: result.dof });
  sketchStore.getState().pushUndoSnapshot(before, { kind: "commitDimension" });
  return { rejected: false };
}

/**
 * Delete sketch entities (user-facing delete). Builds the reduced authoritative
 * arrays — `entities` minus `ids` AND every constraint whose `entities` references
 * a deleted id — then round-trips through `sketchUpsert`. The constraint predicate
 * MIRRORS the Rust `cascade_remove_entity` (session.rs) so the frontend panels
 * never diverge from the server projection: a constraint is dropped iff any of its
 * `entities` refs a doomed id. Point-picks (`{entityId, position}`) reference the
 * OWNING entity id in `entities`, so a child-point ref (e.g. a line's Start↔End
 * Distance) is covered by the same predicate. No-op on empty `ids` / no session /
 * nothing matched; a solve failure surfaces a status hint and leaves state intact.
 */
export function deleteEntities(client: CadClient, ids: string[]): Promise<void> {
  return enqueueSketchMutation(() => deleteEntitiesNow(client, ids));
}

async function deleteEntitiesNow(client: CadClient, ids: string[]): Promise<void> {
  if (ids.length === 0) return;
  const gen = sketchStore.getState().sessionGeneration;
  const session = sketchStore.getState().session;
  if (!session) return;
  const doomed = new Set(ids);
  const entities = session.entities.filter((e) => !doomed.has(e.id));
  const constraints = session.constraints.filter((c) => !c.entities.some((r) => doomed.has(r)));
  if (entities.length === session.entities.length && constraints.length === session.constraints.length) {
    return; // no live id matched — nothing to solve
  }
  const before: SketchSnapshot = { entities: session.entities, constraints: session.constraints };
  if (await commitReducedSketch(client, session, entities, constraints, gen)) {
    sketchStore.getState().pushUndoSnapshot(before, { kind: "deleteEntities" });
  }
}

/**
 * Delete sketch constraints (user-facing delete). Removes `ids` from the
 * constraint array (entities untouched) and re-solves. No-op on empty `ids` / no
 * session / nothing matched; a solve failure surfaces a status hint.
 */
export function deleteConstraints(client: CadClient, ids: string[]): Promise<void> {
  return enqueueSketchMutation(() => deleteConstraintsNow(client, ids));
}

async function deleteConstraintsNow(client: CadClient, ids: string[]): Promise<void> {
  if (ids.length === 0) return;
  const gen = sketchStore.getState().sessionGeneration;
  const session = sketchStore.getState().session;
  if (!session) return;
  const doomed = new Set(ids);
  const constraints = session.constraints.filter((c) => !doomed.has(c.id));
  if (constraints.length === session.constraints.length) return; // nothing matched
  const before: SketchSnapshot = { entities: session.entities, constraints: session.constraints };
  if (await commitReducedSketch(client, session, session.entities, constraints, gen)) {
    sketchStore.getState().pushUndoSnapshot(before, { kind: "deleteConstraints" });
  }
}

/**
 * Shared tail for the delete verbs: upsert the reduced arrays, apply the returned
 * solve (dof/status/positions), and refresh session + engine + DOF badges — the
 * same write-back shape `commit`/`commitDimensionConstraint` use. A rejected upsert
 * surfaces a status hint (matching SketchController.commit's error path) and leaves
 * the session untouched.
 */
async function commitReducedSketch(
  client: CadClient,
  session: SketchSession,
  entities: SketchEntity[],
  constraints: SketchConstraint[],
  gen: number,
): Promise<boolean> {
  let result;
  try {
    result = await client.sketchUpsert(session.sketchId, entities, constraints);
  } catch (e) {
    if (sessionSuperseded(gen)) return false; // superseded — don't surface a stale hint
    const msg = e instanceof Error ? e.message : String(e);
    viewportStore.getState().setStatusHint(`Sketch delete failed: ${msg}`);
    return false;
  }
  if (sessionSuperseded(gen)) return false;

  const solvedEntities = applySolvedPositions(entities, result.solvedPositions ?? {});
  const next = { ...session, entities: solvedEntities, constraints, dof: result.dof, status: result.status };
  sketchStore.getState().setSession(next);
  sketchStore.getState().setConflicting(result.conflicting ?? []);
  getViewportEngine()?.updateSketchSession(next.plane, solvedEntities, next.status);
  documentStore.getState().setSketchSolve(session.sketchId, result.dof, docSketchStatus(result.status));
  viewportStore.setState({ dofBadge: result.dof });
  return true;
}

// ── User-applied constraints (S4b: toolbar + context chips) ───────────────────

/**
 * Apply a constraint the user picked from the applicable set for the current
 * selection (`evaluateApplicability` output). Two paths (design items 2 + 3):
 *
 *   - DIMENSIONAL (Distance/Radius/Diameter/Angle/H-/V-Distance): does NOT
 *     silent-commit — opens the seeded Dimension chip (`toolChipStore`, the same
 *     pending-dimension surface the Dimension tool uses); its Enter authors through
 *     `commitDimensionConstraint` (reject-on-conflict). Returns `{rejected:false}`
 *     (the pending chip owns the eventual commit/reject).
 *   - GEOMETRIC: builds the `SketchConstraint` from the applicable's ordered
 *     targets, appends it, upserts, and REJECTS-ON-CONFLICT — if the solve reports
 *     Conflicting/OverConstrained, the constraint is dropped and the prior solve
 *     restored (generalizes `commitDimensionConstraint`'s pattern).
 */
export function applyConstraint(
  client: CadClient,
  applicable: ApplicableConstraint,
): Promise<{ rejected: boolean }> {
  return enqueueSketchMutation(() => applyConstraintNow(client, applicable));
}

async function applyConstraintNow(
  client: CadClient,
  applicable: ApplicableConstraint,
): Promise<{ rejected: boolean }> {
  const gen = sketchStore.getState().sessionGeneration;
  const session = sketchStore.getState().session;
  if (!session) return { rejected: false };

  if (applicable.dimensional) {
    openAppliedDimensionChip(client, applicable);
    return { rejected: false };
  }

  const id = sketchStore.getState().nextConstraintId();
  const constraint = buildAppliedConstraint(applicable, id);
  if (!constraint) return { rejected: false };

  const before: SketchSnapshot = { entities: session.entities, constraints: session.constraints };
  const constraints = [...session.constraints, constraint];
  const result = await client.sketchUpsert(session.sketchId, session.entities, constraints);
  if (sessionSuperseded(gen)) return { rejected: false };

  if (isConflictStatus(result.status)) {
    // Reject-on-conflict: name the clashing constraint, then drop this one + restore.
    const hint = rejectConflictHint(constraints, result.conflicting, constraint.id, "Constraint");
    const restore = await client.sketchUpsert(session.sketchId, session.entities, session.constraints);
    if (sessionSuperseded(gen)) return { rejected: true };
    const reverted = { ...session, dof: restore.dof, status: restore.status };
    sketchStore.getState().setSession(reverted);
    sketchStore.getState().setConflicting(restore.conflicting ?? []);
    getViewportEngine()?.updateSketchSession(reverted.plane, reverted.entities, reverted.status);
    documentStore.getState().setSketchSolve(session.sketchId, restore.dof, docSketchStatus(restore.status));
    viewportStore.setState({ dofBadge: restore.dof });
    viewportStore.getState().setStatusHint(hint);
    return { rejected: true }; // rejected: no undo snapshot pushed
  }

  const solvedEntities = applySolvedPositions(session.entities, result.solvedPositions ?? {});
  const next = { ...session, entities: solvedEntities, constraints, dof: result.dof, status: result.status };
  sketchStore.getState().setSession(next);
  sketchStore.getState().setConflicting(result.conflicting ?? []);
  getViewportEngine()?.updateSketchSession(next.plane, solvedEntities, next.status);
  documentStore.getState().setSketchSolve(session.sketchId, result.dof, docSketchStatus(result.status));
  viewportStore.setState({ dofBadge: result.dof });
  sketchStore.getState().pushUndoSnapshot(before, { kind: "applyConstraint" });
  return { rejected: false };
}

/** Open the seeded Dimension chip for a dimensional applicable (design item 3).
 *  Enter mints the constraint via the seed's `build` and commits it through
 *  `commitDimensionConstraint`; Esc/cancel clears the chip. */
function openAppliedDimensionChip(client: CadClient, applicable: ApplicableConstraint): void {
  const session = sketchStore.getState().session;
  if (!session) return;
  const dim = buildAppliedDimension(applicable, session.entities);
  if (!dim) return;
  const world = planePointToWorld(session.plane, dim.anchor).toArray() as [number, number, number];
  toolChipStore.getState().showDimension(
    dim.value,
    dim.suffix,
    world,
    (v) => {
      const cid = sketchStore.getState().nextConstraintId();
      const constraint = dim.build(v, cid);
      toolChipStore.getState().clear();
      void commitDimensionConstraint(client, constraint).then(({ rejected, hint }) => {
        viewportStore
          .getState()
          .setStatusHint(rejected ? hint ?? "Dimension removed — it would over-constrain the sketch" : null);
      });
    },
    () => toolChipStore.getState().clear(),
  );
}

// ── Sketch-scoped undo / redo (C2) ────────────────────────────────────────────
//
// Independent of the model history. Both flip through the SAME serialized mutation
// queue + generation fence as every other edit, so an undo racing a live edit is
// ordered and last-write-wins. Works standalone — it does not depend on any backend
// history-squash landing separately (the mock/real `sketchUpsert` just re-solves the
// restored arrays). The write-back tail matches `commitReducedSketch`.

/** Undo the last confirmed sketch edit (pop the undo stack, re-solve, refresh). */
export function undoSketch(client: CadClient): Promise<void> {
  return enqueueSketchMutation(() => undoRedoNow(client, "undo"));
}

/** Redo the last undone sketch edit. */
export function redoSketch(client: CadClient): Promise<void> {
  return enqueueSketchMutation(() => undoRedoNow(client, "redo"));
}

async function undoRedoNow(client: CadClient, dir: "undo" | "redo"): Promise<void> {
  const gen = sketchStore.getState().sessionGeneration;
  const session = sketchStore.getState().session;
  if (!session) return;
  // Move the CURRENT arrays onto the opposite stack and take the target snapshot.
  const current: SketchSnapshot = { entities: session.entities, constraints: session.constraints };
  const store = sketchStore.getState();
  const snapshot = dir === "undo" ? store.popUndo(current) : store.popRedo(current);
  if (!snapshot) return; // nothing to undo/redo

  let result;
  try {
    result = await client.sketchUpsert(session.sketchId, snapshot.entities, snapshot.constraints);
  } catch (e) {
    // Un-pop: geometry didn't change, so the stacks must not lose the entry
    // (popUndo/popRedo already swapped `current` onto the opposite stack).
    if (!sessionSuperseded(gen)) {
      if (dir === "undo") sketchStore.getState().popRedo(snapshot);
      else sketchStore.getState().popUndo(snapshot);
      const msg = e instanceof Error ? e.message : String(e);
      viewportStore.getState().setStatusHint(`Sketch ${dir} failed: ${msg}`);
    }
    return;
  }
  if (sessionSuperseded(gen)) return;

  const solvedEntities = applySolvedPositions(snapshot.entities, result.solvedPositions ?? {});
  const next = {
    ...session,
    entities: solvedEntities,
    constraints: snapshot.constraints,
    dof: result.dof,
    status: result.status,
  };
  sketchStore.getState().setSession(next);
  sketchStore.getState().setConflicting(result.conflicting ?? []);
  getViewportEngine()?.updateSketchSession(next.plane, solvedEntities, next.status);
  documentStore.getState().setSketchSolve(session.sketchId, result.dof, docSketchStatus(result.status));
  viewportStore.setState({ dofBadge: result.dof });
}
