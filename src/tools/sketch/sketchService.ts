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
import { sketchStore } from "@/stores/sketchStore";
import { applySolvedPositions } from "@/ipc/sketchWireMap";
import { toolChipStore } from "@/stores/toolChipStore";
import { planePointToWorld } from "@/viewport/engine/sketchBasis";
import { isConflictStatus } from "./dimensionTool";
import type { ApplicableConstraint } from "./constraintApplicability";
import { buildAppliedConstraint, buildAppliedDimension } from "./constraintAuthoring";

/*
 * ALL exported session mutators are serialized through one promise chain. Each
 * reads `sketchStore.session` INSIDE its queued turn, so a second user action
 * fired before the first `sketchUpsert` resolves rebases on the settled result
 * instead of capturing the same stale arrays (which would make `marshalUpsert`'s
 * shared id-map diff synthesize a removal of the first action's constraint and
 * let either response clobber the store).
 */
let sketchMutationChain: Promise<unknown> = Promise.resolve();
function enqueueSketchMutation<T>(fn: () => Promise<T>): Promise<T> {
  const run = sketchMutationChain.then(fn, fn);
  sketchMutationChain = run.catch(() => undefined);
  return run;
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
  const session = sketchStore.getState().session;
  if (!session) return;
  const constraints = session.constraints.map((c) =>
    c.id === constraintId ? { ...c, value } : c,
  );
  const result = await client.sketchUpsert(session.sketchId, session.entities, constraints);
  if (!sketchStore.getState().session) return; // exited during await

  const next = { ...session, constraints, dof: result.dof, status: result.status };
  sketchStore.getState().setSession(next);
  getViewportEngine()?.updateSketchSession(next.plane, next.entities, next.status);
  documentStore.getState().setSketchSolve(session.sketchId, result.dof, docSketchStatus(result.status));
  viewportStore.setState({ dofBadge: result.dof });
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
): Promise<{ rejected: boolean }> {
  return enqueueSketchMutation(() => commitDimensionConstraintNow(client, constraint));
}

async function commitDimensionConstraintNow(
  client: CadClient,
  constraint: SketchConstraint,
): Promise<{ rejected: boolean }> {
  const session = sketchStore.getState().session;
  if (!session) return { rejected: false };

  const constraints = [...session.constraints, constraint];
  const result = await client.sketchUpsert(session.sketchId, session.entities, constraints);
  if (!sketchStore.getState().session) return { rejected: false }; // exited during await

  if (isConflictStatus(result.status)) {
    // Reject-on-conflict: drop the dimension and restore the previous solve.
    const restore = await client.sketchUpsert(session.sketchId, session.entities, session.constraints);
    if (!sketchStore.getState().session) return { rejected: true };
    const reverted = { ...session, dof: restore.dof, status: restore.status };
    sketchStore.getState().setSession(reverted);
    getViewportEngine()?.updateSketchSession(reverted.plane, reverted.entities, reverted.status);
    documentStore.getState().setSketchSolve(session.sketchId, restore.dof, docSketchStatus(restore.status));
    viewportStore.setState({ dofBadge: restore.dof });
    return { rejected: true };
  }

  const solvedEntities = applySolvedPositions(session.entities, result.solvedPositions ?? {});
  const next = { ...session, entities: solvedEntities, constraints, dof: result.dof, status: result.status };
  sketchStore.getState().setSession(next);
  getViewportEngine()?.updateSketchSession(next.plane, solvedEntities, next.status);
  documentStore.getState().setSketchSolve(session.sketchId, result.dof, docSketchStatus(result.status));
  viewportStore.setState({ dofBadge: result.dof });
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
  const session = sketchStore.getState().session;
  if (!session) return;
  const doomed = new Set(ids);
  const entities = session.entities.filter((e) => !doomed.has(e.id));
  const constraints = session.constraints.filter((c) => !c.entities.some((r) => doomed.has(r)));
  if (entities.length === session.entities.length && constraints.length === session.constraints.length) {
    return; // no live id matched — nothing to solve
  }
  await commitReducedSketch(client, session, entities, constraints);
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
  const session = sketchStore.getState().session;
  if (!session) return;
  const doomed = new Set(ids);
  const constraints = session.constraints.filter((c) => !doomed.has(c.id));
  if (constraints.length === session.constraints.length) return; // nothing matched
  await commitReducedSketch(client, session, session.entities, constraints);
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
): Promise<void> {
  let result;
  try {
    result = await client.sketchUpsert(session.sketchId, entities, constraints);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    viewportStore.getState().setStatusHint(`Sketch delete failed: ${msg}`);
    return;
  }
  if (!sketchStore.getState().session) return; // exited during await

  const solvedEntities = applySolvedPositions(entities, result.solvedPositions ?? {});
  const next = { ...session, entities: solvedEntities, constraints, dof: result.dof, status: result.status };
  sketchStore.getState().setSession(next);
  getViewportEngine()?.updateSketchSession(next.plane, solvedEntities, next.status);
  documentStore.getState().setSketchSolve(session.sketchId, result.dof, docSketchStatus(result.status));
  viewportStore.setState({ dofBadge: result.dof });
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
  const session = sketchStore.getState().session;
  if (!session) return { rejected: false };

  if (applicable.dimensional) {
    openAppliedDimensionChip(client, applicable);
    return { rejected: false };
  }

  const id = sketchStore.getState().nextConstraintId();
  const constraint = buildAppliedConstraint(applicable, id);
  if (!constraint) return { rejected: false };

  const constraints = [...session.constraints, constraint];
  const result = await client.sketchUpsert(session.sketchId, session.entities, constraints);
  if (!sketchStore.getState().session) return { rejected: false }; // exited during await

  if (isConflictStatus(result.status)) {
    // Reject-on-conflict: drop the constraint and restore the previous solve.
    const restore = await client.sketchUpsert(session.sketchId, session.entities, session.constraints);
    if (!sketchStore.getState().session) return { rejected: true };
    const reverted = { ...session, dof: restore.dof, status: restore.status };
    sketchStore.getState().setSession(reverted);
    getViewportEngine()?.updateSketchSession(reverted.plane, reverted.entities, reverted.status);
    documentStore.getState().setSketchSolve(session.sketchId, restore.dof, docSketchStatus(restore.status));
    viewportStore.setState({ dofBadge: restore.dof });
    viewportStore.getState().setStatusHint("Constraint removed — it would over-constrain the sketch");
    return { rejected: true };
  }

  const solvedEntities = applySolvedPositions(session.entities, result.solvedPositions ?? {});
  const next = { ...session, entities: solvedEntities, constraints, dof: result.dof, status: result.status };
  sketchStore.getState().setSession(next);
  getViewportEngine()?.updateSketchSession(next.plane, solvedEntities, next.status);
  documentStore.getState().setSketchSolve(session.sketchId, result.dof, docSketchStatus(result.status));
  viewportStore.setState({ dofBadge: result.dof });
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
      void commitDimensionConstraint(client, constraint).then(({ rejected }) => {
        viewportStore
          .getState()
          .setStatusHint(rejected ? "Dimension removed — it would over-constrain the sketch" : null);
      });
    },
    () => toolChipStore.getState().clear(),
  );
}
