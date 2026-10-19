//! Strict constraint authoring keeps refused candidates outside document history.
use super::*;
use crate::dto::SketchConstraintAdmission;
use onecad_core::sketch::Constraint;

/// Solves a fresh native primitive outside the application lock before fenced admission.
pub async fn author_sketch_primitive_with_completion(
    state: &AppState,
    app: HostContext,
    sketch_id: String,
    ops: Vec<SketchEditOp>,
) -> Result<CommandCompletion<SketchConstraintAdmission>, ApiError> {
    let id = parse_sketch_id(&sketch_id)?;
    let prepared = {
        let guard = state.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("authorSketchPrimitive".into()))?;
        runtime.prepare_sketch_primitive(id, ops).await?
    };
    complete_constraints(state, app, prepared, "authorSketchPrimitive").await
}

pub async fn add_sketch_constraints_with_completion(
    state: &AppState,
    app: HostContext,
    sketch_id: String,
    constraints: Vec<Constraint>,
) -> Result<CommandCompletion<SketchConstraintAdmission>, ApiError> {
    let id = parse_sketch_id(&sketch_id)?;
    let prepared = {
        let guard = state.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("addSketchConstraints".into()))?;
        runtime.prepare_sketch_constraints(id, constraints).await?
    };
    complete_constraints(state, app, prepared, "addSketchConstraints").await
}

/// Edits one dimension through detached solving; literals intentionally clear expression bindings.
/// Angle literals use radians. Angle expressions resolve in degrees and convert once.
pub async fn set_sketch_dimension_with_completion(
    state: &AppState,
    app: HostContext,
    sketch_id: String,
    constraint_id: ConstraintId,
    value: Scalar,
) -> Result<CommandCompletion<SketchConstraintAdmission>, ApiError> {
    let id = parse_sketch_id(&sketch_id)?;
    let prepared = {
        let guard = state.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("setSketchDimension".into()))?;
        runtime
            .prepare_sketch_dimension(id, constraint_id, value)
            .await?
    };
    complete_constraints(state, app, prepared, "setSketchDimension").await
}

/// Repairs existing sketch geometry without admitting arbitrary unhealthy authoring.
pub async fn repair_sketch_with_completion(
    state: &AppState,
    app: HostContext,
    sketch_id: String,
    ops: Vec<SketchEditOp>,
) -> Result<CommandCompletion<SketchConstraintAdmission>, ApiError> {
    let id = parse_sketch_id(&sketch_id)?;
    let prepared = {
        let guard = state.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("repairSketch".into()))?;
        runtime.prepare_sketch_repair(id, ops).await?
    };
    complete_constraints(state, app, prepared, "repairSketch").await
}

async fn complete_constraints(
    state: &AppState,
    app: HostContext,
    prepared: crate::document_runtime::PreparedSketchConstraints,
    action: &str,
) -> Result<CommandCompletion<SketchConstraintAdmission>, ApiError> {
    let driven = prepared.drive().await?;
    let (result, outcome, projection) = {
        let mut guard = state.runtime.lock().await;
        let runtime = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument(action.into()))?;
        let (result, outcome) = runtime.commit_sketch_constraints(driven)?;
        (result, outcome, runtime.projection())
    };
    if result.accepted {
        let _ = app.emit(AppEvent::SketchSolved(result.solve.clone()));
        let _ = app.emit(AppEvent::ProjectionUpdated(projection.clone()));
    }
    let mut regen_expected = false;
    if let Some(outcome) = outcome {
        regen_expected = schedule_command_outcome(state, &outcome);
        state.note_mutation();
    }
    Ok(CommandCompletion {
        result: Ok(result),
        projection,
        regen_expected,
    })
}
