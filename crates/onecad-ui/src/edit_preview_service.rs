//! History edit candidates share the native checked ghost-mesh admission path.
use super::*;
use onecad_app::{
    api::{EditPreview, EditPreviewOutcome},
    error::ApiError,
};
use onecad_core::document::record::Operation;
use onecad_interaction::RequestTag;

pub(super) enum Edit {
    Parameters(Box<Operation>),
    Repair(Box<onecad_app::api::InputRepair>),
}

pub(super) async fn candidate(
    backend: &CandidateBackend,
    generation: u64,
    tag: RequestTag,
    edit: Edit,
) {
    crate::performance::record(
        crate::performance::Stage::PreviewSubmitted,
        tag.context.scene.session,
        [tag.sequence, tag.context.scene.scene, tag.gesture],
    );
    let _span = super::phase(crate::performance::Stage::PreviewService, &tag);
    let lease = backend
        .shared
        .host_budget
        .admit_while(
            crate::budget::PREVIEW_RESERVATION,
            crate::budget::Kind::Preview,
            || {
                backend.shared.is_current(generation)
                    && !backend
                        .shared
                        .stopping
                        .load(std::sync::atomic::Ordering::Acquire)
            },
        )
        .await
        .and_then(|lease| {
            lease.split_kinds(
                crate::budget::PREVIEW_SOURCE_RESERVATION,
                crate::budget::Kind::PreviewSource,
                crate::budget::Kind::Preparation,
            )
        });
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
        Ok((source, preparation)) => {
            delivery.lease = Some(Arc::new(source));
            let preparation = Arc::new(preparation);
            match preview(backend, &tag, edit).await {
                Ok(result) => {
                    prepare(
                        backend,
                        generation,
                        &tag,
                        result,
                        &mut delivery,
                        preparation,
                    )
                    .await
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(ApiError::Unsupported(error)),
    };
    delivery.action = Action::EditPreviewFinished { tag, result };
    backend.shared.publish_candidate(generation, delivery);
}

async fn prepare(
    backend: &CandidateBackend,
    generation: u64,
    tag: &RequestTag,
    mut result: EditPreview,
    delivery: &mut CandidateDelivery,
    preparation: Arc<crate::budget::Lease>,
) -> Result<EditPreview, ApiError> {
    if matches!(
        result.outcome,
        EditPreviewOutcome::Unchanged | EditPreviewOutcome::NotReached { .. }
    ) {
        return Ok(result);
    }
    if result.suppressed.len() > crate::budget::MAX_BODIES
        || result.bodies.len() > crate::budget::MAX_BODIES
    {
        return Err(ApiError::Unsupported(
            "Native edit preview body count budget exceeded".into(),
        ));
    }
    let retention = onecad_render::PreparationRetention {
        source: delivery
            .lease
            .clone()
            .map(|lease| lease as Arc<dyn Send + Sync>),
        preparation: Some(preparation),
    };
    let meshes = std::mem::take(&mut result.meshes);
    let (meshes, prepared, scene) =
        super::prepare_meshes(backend, generation, tag.clone(), meshes, retention)
            .await
            .map_err(ApiError::Internal)?;
    result.meshes = meshes;
    delivery.prepared = Some(prepared);
    delivery.scene = Some(scene);
    Ok(result)
}

async fn preview(
    backend: &CandidateBackend,
    tag: &RequestTag,
    edit: Edit,
) -> Result<EditPreview, ApiError> {
    let fence = super::check_tag(&backend.state, tag)
        .await
        .map_err(ApiError::StalePreview)?;
    let _kernel = super::phase(crate::performance::Stage::PreviewKernel, tag);
    let result = match edit {
        Edit::Parameters(operation) => {
            api::preview_existing_edit(
                &backend.state,
                fence.clone(),
                tag.record_id,
                *operation,
                Lod::Coarse,
            )
            .await?
        }
        Edit::Repair(repair) => {
            api::preview_input_repair(&backend.state, &repair, Lod::Coarse).await?
        }
    };
    drop(_kernel);
    if api::preview_context(&backend.state).await? != fence
        || result.snapshot_id != fence.snapshot_id
    {
        return Err(ApiError::StalePreview(
            "edit preview completion belongs to an obsolete worker head".into(),
        ));
    }
    Ok(result)
}
