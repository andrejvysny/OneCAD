//! Strict constraint authoring keeps refused candidates outside document history.
use super::*;
use crate::dto::SketchConstraintAdmission;
use onecad_core::sketch::Constraint;

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
    let driven = prepared.drive().await?;
    let (result, outcome, projection) = {
        let mut guard = state.runtime.lock().await;
        let runtime = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("addSketchConstraints".into()))?;
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
