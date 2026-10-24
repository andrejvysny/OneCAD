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
        if !self.matches_submitted_preview(&tag) {
            return Vec::new();
        }
        let Some(tool) = self.tool.as_ref().filter(|tool| tool.editing_existing) else {
            return Vec::new();
        };
        let repair = tool.repair.is_some();
        let result = match admit(&tag, result) {
            Ok(result) => result,
            Err(status) => {
                let status = status.map(|status| {
                    if repair && status == PreviewStatus::UnavailableForEdit {
                        PreviewStatus::Failed("Repair requires a successful preview".into())
                    } else {
                        status
                    }
                });
                return self.finish_preview(tag, status, None);
            }
        };
        let status = if repair {
            if !onecad_app::api::repair_preview_ready(&result) {
                return self.finish_preview(
                    tag,
                    Some(PreviewStatus::Failed(format!(
                        "Repair preview did not complete: {:?}",
                        result.outcome
                    ))),
                    None,
                );
            }
            match result.outcome {
                EditPreviewOutcome::DownstreamNeedsRepair { record, .. } => {
                    PreviewStatus::RepairWarning(format!(
                        "This input repaired successfully. Feature {record} still needs explicit repair."
                    ))
                }
                _ => PreviewStatus::Ready,
            }
        } else {
            status(&result.outcome)
        };
        let geometry = if matches!(
            result.outcome,
            EditPreviewOutcome::Unchanged | EditPreviewOutcome::NotReached { .. }
        ) {
            None
        } else {
            Some(geometry(result))
        };
        self.finish_preview(tag, Some(status), geometry)
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
