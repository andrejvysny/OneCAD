//! History edit candidates share the native checked ghost-mesh admission path.
use super::*;
use onecad_app::{
    api::{EditPreview, EditPreviewOutcome},
    error::ApiError,
};
use onecad_core::document::record::Operation;
use onecad_interaction::RequestTag;

pub(super) async fn candidate(
    backend: &Backend,
    generation: u64,
    tag: RequestTag,
    operation: Operation,
) {
    let lease = backend
        .shared
        .host_budget
        .reserve(crate::budget::PREVIEW_RESERVATION);
    let mut delivery = CandidateDelivery {
        action: Action::EditPreviewFinished {
            tag: tag.clone(),
            result: Err(ApiError::Cancelled("not started".into())),
        },
        prepared: None,
        lease: None,
        scene: None,
    };
    let result = match lease {
        Ok(lease) => {
            delivery.lease = Some(lease);
            preview(backend, &tag, operation).await.and_then(|result| {
                prepare(backend, &tag, &result, &mut delivery)?;
                Ok(result)
            })
        }
        Err(error) => Err(ApiError::Unsupported(error)),
    };
    delivery.action = Action::EditPreviewFinished { tag, result };
    backend.shared.publish_candidate(generation, delivery);
}

fn prepare(
    backend: &Backend,
    tag: &RequestTag,
    result: &EditPreview,
    delivery: &mut CandidateDelivery,
) -> Result<(), ApiError> {
    if matches!(
        result.outcome,
        EditPreviewOutcome::Unchanged | EditPreviewOutcome::NotReached { .. }
    ) {
        return Ok(());
    }
    if result.suppressed.len() > crate::budget::MAX_BODIES
        || result.bodies.len() > crate::budget::MAX_BODIES
    {
        return Err(ApiError::Unsupported(
            "Native edit preview body count budget exceeded".into(),
        ));
    }
    let scene = super::preview_scene(tag, &result.meshes).map_err(ApiError::Internal)?;
    let budget = backend
        .shared
        .preparation_budget()
        .map_err(ApiError::Unsupported)?;
    delivery.prepared = Some(
        onecad_render::PreparedScene::new(
            &scene,
            budget.limit_cpu_prepared_bytes(crate::budget::MAX_PREPARED_BYTES),
        )
        .map_err(|error| ApiError::Internal(error.to_string()))?,
    );
    delivery.scene = Some(Arc::new(scene));
    Ok(())
}

async fn preview(
    backend: &Backend,
    tag: &RequestTag,
    operation: Operation,
) -> Result<EditPreview, ApiError> {
    let fence = super::check_tag(&backend.state, tag)
        .await
        .map_err(ApiError::StalePreview)?;
    let result = api::preview_existing_edit(
        &backend.state,
        fence.clone(),
        tag.record_id,
        operation,
        Lod::Coarse,
    )
    .await?;
    if api::preview_context(&backend.state).await? != fence
        || result.snapshot_id != fence.snapshot_id
    {
        return Err(ApiError::StalePreview(
            "edit preview completion belongs to an obsolete worker head".into(),
        ));
    }
    Ok(result)
}
