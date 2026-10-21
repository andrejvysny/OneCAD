//! Hosts confirm a current semantic deletion closure before one atomic document edit.
use super::*;
pub use crate::document_runtime::deletion::{DeleteTarget, DeletionItem, DeletionPlan};

pub async fn plan_delete_document_item(
    state: &AppState,
    target: DeleteTarget,
    expected: PreviewContext,
) -> Result<DeletionPlan, ApiError> {
    if preview_context(state).await? != expected {
        return Err(ApiError::StalePreview(
            "Deletion belongs to an obsolete worker head".into(),
        ));
    }
    let guard = state.runtime.lock().await;
    let runtime = guard
        .as_ref()
        .ok_or_else(|| ApiError::NoDocument("planDeleteDocumentItem".into()))?;
    runtime.plan_delete_document_item(target, expected)
}

pub async fn delete_document_item(
    state: &AppState,
    app: HostContext,
    plan: DeletionPlan,
) -> Result<CommandCompletion<DocumentProjection>, ApiError> {
    if preview_context(state).await? != plan.expected {
        return Err(ApiError::StalePreview(
            "Deletion belongs to an obsolete worker head".into(),
        ));
    }
    let (outcome, projection) = {
        let mut guard = state.runtime.lock().await;
        let runtime = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("deleteDocumentItem".into()))?;
        let outcome = runtime.delete_document_item(&plan)?;
        (outcome, runtime.projection())
    };
    let _ = app.emit(AppEvent::ProjectionUpdated(projection.clone()));
    let regen_expected = schedule_command_outcome(state, &outcome);
    state.note_mutation();
    Ok(CommandCompletion {
        result: Ok(projection.clone()),
        projection,
        regen_expected,
    })
}
