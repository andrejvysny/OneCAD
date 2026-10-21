//! Lane-safe projection services for a native in-visit sketch session.
//!
//! Each service claims the sketch's upsert lane under the runtime lock before
//! its first kernel query, holds it across the unlocked round trips, commits
//! through the fenced [`DocumentRuntime::commit_projection_visit`] and resyncs
//! the worker solver lane with `SketchUpsert`. The commit is an ordinary
//! in-visit sketch edit: sketch history moves it and Finish squashes it.
//!
//! Provenance, the `alreadyProjected` and kernel per-source refusals, and the
//! (source element, ordinal) update match are the headless services' own
//! planning steps ([`super::projection_parts`]), so both paths answer alike.
use super::*;
use crate::document_runtime::ProjectionVisit;
use crate::dto::{DetachProjectionDto, ProjectToSketchDto, SketchProjectionReceipt};

type ProjectionCompletion<T> = CommandCompletion<SketchProjectionReceipt<T>>;

/// [`project_to_sketch`] for an open visit: the picks are promoted under the
/// claimed lane, and the commit refuses if the head moved past `snapshot_id`.
pub async fn project_to_sketch_with_completion(
    state: &AppState,
    app: HostContext,
    snapshot_id: u64,
    sketch_id: String,
    mode: String,
    sources: Vec<ProjectionSourceInput>,
) -> Result<ProjectionCompletion<ProjectToSketchDto>, ApiError> {
    const VERB: &str = "projectToSketch";
    let sketch = parse_sketch_id(&sketch_id)?;
    let (mode, addressed) = parse_projection_request(&mode, sources)?;
    let snapshot = SnapshotId(snapshot_id);
    let (visit, promoted) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument(VERB.into()))?;
        let visit = rt
            .claim_projection_visit(sketch, Some(snapshot), VERB)
            .await?;
        let promoted = promote_projection_picks(rt, snapshot, &addressed).await?;
        (visit, promoted)
    };
    let (planned, mut refusals) =
        plan_projection_sources(mode, &addressed, &promoted, &visit.sketch().projections)?;
    let mut value = ProjectToSketchDto {
        sketch_id: sketch.to_string(),
        snapshot_id,
        entities: Vec::new(),
        point_count: 0,
        refusals: Vec::new(),
    };
    let mut ops = Vec::new();
    // An empty `sources` array is not a legal §7.6 request: when every pick was
    // already projected there is nothing to ask the kernel.
    if !planned.is_empty() {
        let result = state
            .face_projection()
            .project_to_sketch_plane(
                snapshot,
                sketch,
                &visit.sketch().plane,
                mode,
                &wire_projection_sources(&planned),
            )
            .await?;
        let (edit, entities, point_count) = projected_edit(&result.geometry);
        ops = edit;
        value.entities = entities;
        value.point_count = point_count;
        refusals.extend(result.refusals);
    }
    value.refusals = refusal_dtos(refusals);
    complete_projection_visit(state, app, visit, ops, value, VERB).await
}

/// [`update_projection`] for an open visit: an explicit refresh against the
/// head the lane was claimed on. Rows that changed shape stay stale and are
/// reported as `topologyChanged` refusals; nothing is re-associated by guess.
pub async fn update_projection_with_completion(
    state: &AppState,
    app: HostContext,
    sketch_id: String,
) -> Result<ProjectionCompletion<ProjectToSketchDto>, ApiError> {
    const VERB: &str = "updateProjection";
    let sketch = parse_sketch_id(&sketch_id)?;
    let (visit, rebind) = {
        let guard = state.runtime.lock().await;
        let rt = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument(VERB.into()))?;
        let visit = rt.claim_projection_visit(sketch, None, VERB).await?;
        (visit, rt.projection_rebind_ticket(sketch))
    };
    let snapshot = visit.head();
    let plan = reproject_sketch(state, visit.sketch(), snapshot, rebind).await?;
    let value = ProjectToSketchDto {
        sketch_id,
        snapshot_id: snapshot.0,
        entities: plan.entities,
        point_count: 0,
        refusals: refusal_dtos(plan.refusals),
    };
    complete_projection_visit(state, app, visit, plan.ops, value, VERB).await
}

/// [`detach_projection`] for an open visit: the entities keep their geometry
/// and become ordinary sketch geometry without provenance, lock or pins.
pub async fn detach_projection_with_completion(
    state: &AppState,
    app: HostContext,
    sketch_id: String,
    entity_ids: Option<Vec<String>>,
) -> Result<ProjectionCompletion<DetachProjectionDto>, ApiError> {
    const VERB: &str = "detachProjection";
    let sketch = parse_sketch_id(&sketch_id)?;
    let visit = {
        let guard = state.runtime.lock().await;
        let rt = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument(VERB.into()))?;
        rt.claim_projection_visit(sketch, None, VERB).await?
    };
    let targets = detach_targets(visit.sketch(), entity_ids)?;
    let (ops, released) = sketch_projection::build_projection_detach(visit.sketch(), &targets);
    let remaining = visit
        .sketch()
        .projections
        .len()
        .saturating_sub(targets.len())
        .try_into()
        .unwrap_or(u32::MAX);
    let value = DetachProjectionDto {
        sketch_id,
        entity_ids: targets.iter().map(ToString::to_string).collect(),
        released_constraints: released,
        remaining,
    };
    complete_projection_visit(state, app, visit, ops, value, VERB).await
}

/// The fenced commit + resync, or a plain lane release when nothing changed.
async fn complete_projection_visit<T>(
    state: &AppState,
    app: HostContext,
    visit: ProjectionVisit,
    ops: Vec<SketchEditOp>,
    value: T,
    verb: &str,
) -> Result<ProjectionCompletion<T>, ApiError> {
    let (solve, outcome, projection) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument(verb.into()))?;
        if ops.is_empty() {
            drop(visit);
            (None, None, rt.projection())
        } else {
            let (solve, outcome) = rt.commit_projection_visit(visit, ops, verb).await?;
            (Some(solve), outcome, rt.projection())
        }
    };
    tracing::info!(
        "{verb}: committed={} regen={}",
        solve.is_some(),
        outcome.is_some()
    );
    if let Some(solve) = &solve {
        let _ = app.emit(AppEvent::SketchSolved(solve.clone()));
    }
    let _ = app.emit(AppEvent::ProjectionUpdated(projection.clone()));
    let mut regen_expected = false;
    if let Some(outcome) = outcome {
        regen_expected = schedule_command_outcome(state, &outcome);
        state.note_mutation();
    }
    Ok(CommandCompletion {
        result: Ok(SketchProjectionReceipt { value, solve }),
        projection,
        regen_expected,
    })
}
