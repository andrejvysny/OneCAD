//! Existing edits replay their history in an isolated worker candidate.
use crate::*;
use onecad_app::{
    api::{EditPreview, EditPreviewOutcome},
    error::ApiError,
};

impl Interaction {
    pub(crate) fn edit_preview_finished(
        &mut self,
        tag: RequestTag,
        result: Result<EditPreview, ApiError>,
    ) -> Vec<Effect> {
        if !self.matches_editing(&tag) {
            return Vec::new();
        }
        let Some(tool) = self.tool.as_mut().filter(|tool| {
            tool.editing_existing
                && tool.option_draft_error.is_none()
                && tool.preview == PreviewStatus::Pending
                && tool.params.readiness() == Readiness::Ready
        }) else {
            return Vec::new();
        };
        let result = match admit(&tag, result) {
            Ok(result) => result,
            Err(status) => {
                if let Some(status) = status {
                    tool.preview = status;
                }
                return Vec::new();
            }
        };
        tool.preview = status(&result.outcome);
        if matches!(
            result.outcome,
            EditPreviewOutcome::Unchanged | EditPreviewOutcome::NotReached { .. }
        ) {
            return Vec::new();
        }
        vec![Effect::PublishPreview {
            tag,
            result: Box::new(geometry(result)),
        }]
    }
}

fn admit(
    tag: &RequestTag,
    result: Result<EditPreview, ApiError>,
) -> Result<EditPreview, Option<PreviewStatus>> {
    match result {
        Ok(result)
            if result.record == tag.record_id && result.snapshot_id == tag.context.snapshot_id =>
        {
            Ok(result)
        }
        Ok(_) => Err(Some(PreviewStatus::Failed(
            "Edit preview identity changed".into(),
        ))),
        Err(ApiError::Unsupported(_)) => Err(Some(PreviewStatus::UnavailableForEdit)),
        Err(ApiError::StalePreview(_) | ApiError::Cancelled(_)) => Err(None),
        Err(error) => Err(Some(PreviewStatus::Failed(error.to_string()))),
    }
}

fn status(outcome: &EditPreviewOutcome) -> PreviewStatus {
    match outcome {
        EditPreviewOutcome::Ready | EditPreviewOutcome::Unchanged => PreviewStatus::Ready,
        EditPreviewOutcome::NotReached { .. } => PreviewStatus::UnavailableForEdit,
        EditPreviewOutcome::EditedFailed { message, .. } => PreviewStatus::Failed(message.clone()),
        EditPreviewOutcome::DownstreamFailed {
            record, message, ..
        } => PreviewStatus::EditWarning(format!(
            "Feature {record} failed: {message}. Apply retains the edit."
        )),
        EditPreviewOutcome::DownstreamNeedsRepair { record, .. } => PreviewStatus::EditWarning(
            format!("Feature {record} needs repair. Apply retains the edit."),
        ),
    }
}

fn geometry(result: EditPreview) -> PreviewResultDto {
    PreviewResultDto {
        snapshot_id: result.snapshot_id.0,
        bodies: result.meshes,
        body_events: Vec::new(),
        changed_bodies: result
            .suppressed
            .into_iter()
            .map(|id| id.to_string())
            .collect(),
        deleted_bodies: Vec::new(),
        needs_repair: Vec::new(),
    }
}
