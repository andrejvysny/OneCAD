//! Phase 3 classification: how a replay's stop relates to the edited step `k`,
//! and how an engine error maps to the API.

use onecad_core::regen::{EngineError, OpFailureCode, StepResult, StepStatus, StoppedReason};

use super::{EditPreviewJob, EditPreviewOutcome};
use crate::error::ApiError;
use crate::worker::PlanPreview;

/// Classifies the HALTED (first) pass relative to the edited step `k`. A halt
/// BEFORE `k` that the WP-D1 isolation pass routed around — it ran `k` — is not
/// `NotReached`: Apply publishes the edit there, so the outcome reads from the
/// isolation pass at `k` and reports the earlier halt as another feature's.
pub(super) fn classify(
    job: &EditPreviewJob,
    halted: &PlanPreview,
    isolated: Option<&PlanPreview>,
) -> EditPreviewOutcome {
    let Some(stop) = stop_of(halted) else {
        return EditPreviewOutcome::Ready;
    };
    let k = job.edited_step;
    if stop.step_index >= k {
        return at_stop(job, halted, stop);
    }
    let reached = isolated.and_then(|pass| {
        pass.per_step
            .iter()
            .find(|s| s.step_index == k)
            .map(|s| (pass, s))
    });
    match reached {
        Some((pass, at_k)) if at_k.status != StepStatus::Ok => at_stop(job, pass, at_k),
        Some(_) => other_feature(job, stop),
        None => EditPreviewOutcome::NotReached {
            ceiling: stop.step_index.checked_sub(1),
        },
    }
}

/// The step a pass stopped at, `None` when it completed.
fn stop_of(pass: &PlanPreview) -> Option<&StepResult> {
    if pass.stopped_reason == StoppedReason::Completed {
        return None;
    }
    pass.per_step.last()
}

/// `stop` (at or after `k`) of `pass`: the edited feature's own failure or another's.
fn at_stop(job: &EditPreviewJob, pass: &PlanPreview, stop: &StepResult) -> EditPreviewOutcome {
    let k = job.edited_step;
    if stop.step_index != k {
        return other_feature(job, stop);
    }
    let needs_repair = stop.status == StepStatus::NeedsRepair;
    let mut diagnostics = stop.diagnostics.clone();
    if let Some(event) = pass.steps.iter().find(|s| s.step_index == k) {
        diagnostics.extend(event.diagnostics.iter().cloned());
    }
    let message = if !stop.message.is_empty() {
        stop.message.clone()
    } else if needs_repair {
        "The edited feature needs repair".to_string()
    } else {
        "The edited feature failed".to_string()
    };
    EditPreviewOutcome::EditedFailed {
        message,
        diagnostics,
        needs_repair,
    }
}

fn other_feature(job: &EditPreviewJob, stop: &StepResult) -> EditPreviewOutcome {
    let step = stop.step_index;
    let record = job.records.get(step).map_or(job.record, |r| r.record_id);
    if stop.status == StepStatus::NeedsRepair {
        EditPreviewOutcome::DownstreamNeedsRepair { step, record }
    } else {
        EditPreviewOutcome::DownstreamFailed {
            step,
            record,
            message: stop.message.clone(),
        }
    }
}

/// The explicit engine → API mapping for a preview (the generic `From` folds
/// `Cancelled` and `StalePreview` into `OpFailed`). A top-level `OpFailed` is
/// never the edited feature's own failure — that rides `perStepResults` and is
/// classified `EditedFailed` — so every other code means "no preview", which
/// must not block Apply: `Unsupported`.
pub(super) fn map_preview_error(job: &EditPreviewJob, error: EngineError) -> ApiError {
    match error {
        EngineError::Cancelled => ApiError::Cancelled("edit preview cancelled".into()),
        EngineError::OpFailed {
            code: OpFailureCode::StalePreview,
            message,
            ..
        } => ApiError::StalePreview(message),
        EngineError::OpFailed {
            code: OpFailureCode::Unsupported,
            message,
            ..
        } => ApiError::Unsupported(message),
        EngineError::OpFailed { code, message, .. } => {
            tracing::warn!(
                runtime_session = %job.runtime_session,
                record = %job.record,
                job = %job.request.plan.job_id.0,
                ?code,
                %message,
                "edit preview: worker refused the replay — preview unavailable"
            );
            ApiError::Unsupported(format!("edit preview unavailable ({code:?}): {message}"))
        }
        other => {
            tracing::warn!(
                runtime_session = %job.runtime_session,
                record = %job.record,
                job = %job.request.plan.job_id.0,
                error = %other,
                "edit preview: worker-class failure"
            );
            other.into()
        }
    }
}
