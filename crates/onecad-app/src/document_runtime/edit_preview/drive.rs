//! Phase 2 of an edit preview (**unlocked**): the replay and, after a halt, the
//! same WP-D1 isolation pass the commit would run — so the preview shows what
//! Apply will publish, including independent features above a broken one.

use std::collections::HashSet;

use uuid::Uuid;

use onecad_core::ids::BodyId;
use onecad_core::regen::{
    isolation_scope, EngineError, OpFailureCode, PlanArtifacts, PolicyVersions, RegenPlanner,
    StepStatus, StoppedReason,
};

use super::EditPreviewJob;
use crate::worker::{validate_created, PlanPreview, PreviewEngine, PreviewPlanRequest};

/// The engine results of one edit preview.
#[derive(Debug)]
pub struct EditPreviewRun {
    /// The pass over the full candidate plan; classification reads this one.
    pub halted: PlanPreview,
    /// The WP-D1 isolation pass, when the halted pass stopped and independent
    /// later features exist; display reads this one when present.
    pub isolated: Option<PlanPreview>,
}

/// Runs `job` on `engine`. Never holds the runtime lock.
///
/// # Errors
/// The engine's error for the first pass, or for an isolation pass that was
/// cancelled or fenced out; a D1 adoption violation as `Protocol`.
pub async fn drive_edit_preview(
    job: &EditPreviewJob,
    engine: &dyn PreviewEngine,
) -> Result<EditPreviewRun, EngineError> {
    let halted = run_pass(engine, job.request.clone(), job).await?;
    let isolated = match isolation_request(job, &halted) {
        None => None,
        Some(request) => match run_pass(engine, request, job).await {
            Ok(pass) => Some(pass),
            Err(error @ EngineError::Cancelled) => return Err(error),
            Err(
                error @ EngineError::OpFailed {
                    code: OpFailureCode::StalePreview,
                    ..
                },
            ) => return Err(error),
            // `adopt_isolated`: an isolation pass that does not complete costs
            // only the isolation benefit, never the halted pass's result.
            Err(error) => {
                tracing::warn!(%error, "edit preview: isolation pass failed — showing the halted pass");
                None
            }
        },
    };
    Ok(EditPreviewRun { halted, isolated })
}

async fn run_pass(
    engine: &dyn PreviewEngine,
    request: PreviewPlanRequest,
    job: &EditPreviewJob,
) -> Result<PlanPreview, EngineError> {
    let known: HashSet<Uuid> = request
        .plan
        .ops
        .iter()
        .map(|o| o.record_id.as_uuid())
        .collect();
    let pass = engine.preview_plan(request, job.cancel.clone()).await?;
    // D1 parity: the commit's `AdoptingEngine` rejects a malformed or colliding
    // worker-minted id, so a preview that showed it would promise a publish
    // that fails. From-0 base ⇒ no pre-existing bodies.
    let existing: HashSet<BodyId> = HashSet::new();
    let mut seen = HashSet::new();
    for step in &pass.steps {
        validate_created(&step.body_events, &known, &existing, &mut seen)
            .map_err(|message| EngineError::Protocol { message })?;
    }
    Ok(pass)
}

/// The commit's `begin_isolation_regen`, over the candidate document: `None`
/// unless the pass halted and some later feature is provably independent.
fn isolation_request(job: &EditPreviewJob, halted: &PlanPreview) -> Option<PreviewPlanRequest> {
    if halted.stopped_reason == StoppedReason::Completed {
        return None;
    }
    let halted_step = halted
        .per_step
        .last()
        .filter(|s| s.status != StepStatus::Ok)?
        .step_index;
    let target = job.request.plan.target_step;
    let scope = isolation_scope(&job.records, &job.graph, halted_step, target);
    if !scope.is_worth_running() {
        return None;
    }
    let plan = RegenPlanner::without_checkpoint_excluding(&job.mirror, target, &scope.excluded);
    if plan.is_empty() {
        return None;
    }
    let base = &job.request.plan;
    let plan = plan
        .into_request(
            job.isolation_job,
            base.document_revision,
            base.worker_epoch,
            PolicyVersions::default(),
            PlanArtifacts::default(),
        )
        .with_edited_from(base.edited_from);
    Some(PreviewPlanRequest {
        plan,
        expected_snapshot: job.request.expected_snapshot,
        lod: job.request.lod,
    })
}
