//! Unit coverage for the edit-preview phases over worker-free fakes. The real
//! worker equivalence (preview == commit) lives in `tests/edit_preview.rs`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, RecordId, SnapshotId};
use onecad_core::regen::{
    EngineError, GeometryEngine, Lod, OpFailureCode, RegenRequest, StepResult, StepStatus,
    StoppedReason,
};

use super::*;
use crate::document_runtime::PreparedWork;
use crate::worker::{
    MeshProvider, PendingBackend, PlanPreview, PreviewEngine, PreviewPlanRequest, SolverEngine,
};

fn extrude(distance: f64) -> Operation {
    Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: None,
        distance: Scalar::new(distance),
        draft_angle_deg: Scalar::new(0.0),
        mode: ExtrudeMode::Blind,
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.0),
        target_face2: None,
        extra: Default::default(),
    }))
}

fn rec(seed: u128) -> RecordId {
    RecordId(Uuid::from_u128(seed))
}

/// Three extrudes at steps 0..=2; the edit targets step 1.
fn runtime() -> DocumentRuntime {
    let backend = Arc::new(PendingBackend);
    let engine: Arc<dyn GeometryEngine> = backend.clone();
    let meshes: Arc<dyn MeshProvider> = backend.clone();
    let solver: Arc<dyn SolverEngine> = backend;
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);
    for (seed, distance) in [(0x10, 10.0), (0x11, 10.0), (0x12, 4.0)] {
        rt.apply(EditCommand::AddOperation {
            record: OperationRecord::new(rec(seed), 0, "Extrude", extrude(distance)),
            at_cursor: true,
        })
        .unwrap();
    }
    rt
}

fn run_job(rt: &mut DocumentRuntime, distance: f64) -> Box<EditPreviewJob> {
    match rt
        .plan_edit_preview(rec(0x11), extrude(distance), Lod::Coarse)
        .unwrap()
    {
        EditPreviewPrep::Run(job) => job,
        EditPreviewPrep::Done(done) => panic!("expected a run, got {:?}", done.outcome),
    }
}

#[test]
fn a_new_preview_supersedes_and_cancels_the_previous_one() {
    let mut slot = EditPreviewSlot::default();
    let (first, first_token) = slot.install();
    let (second, second_token) = slot.install();
    assert!(
        first_token.is_cancelled(),
        "install cancels the previous preview"
    );
    assert!(!second_token.is_cancelled());
    assert!(!slot.finish(first), "a superseded preview is not current");
    assert!(slot.finish(second));
    assert!(!slot.finish(second), "finish releases the slot");
    let (_, token) = slot.install();
    drop(slot);
    assert!(
        token.is_cancelled(),
        "dropping the runtime cancels the preview"
    );
}

#[test]
fn a_publishing_regen_preempts_the_preview() {
    let mut rt = runtime();
    let job = run_job(&mut rt, 5.0);
    let _ = rt.begin_regen(RegenRequest::ToEnd { from: 0 });
    assert!(job.cancel_token().is_cancelled());
}

#[test]
fn the_preview_plans_exactly_what_the_commit_executes() {
    let mut rt = runtime();
    let revision = rt.revision();
    let job = run_job(&mut rt, 5.0);
    assert_eq!(
        rt.revision(),
        revision,
        "planning a preview never bumps the revision"
    );
    assert!(!rt.session.transaction_open());
    let preview = job.request.plan.clone();

    let outcome = rt
        .apply(EditCommand::UpdateOperationParams {
            record: rec(0x11),
            op: extrude(5.0),
        })
        .unwrap();
    let from = outcome.dirty.map_or(0, |d| d.from);
    let prepared = rt.begin_regen(RegenRequest::ToEnd { from }).unwrap();
    let PreparedWork::Plan { plan_req, .. } = prepared.work else {
        panic!("commit compiled no plan");
    };
    assert_eq!(preview.ops, plan_req.ops, "same records, same lowering");
    assert_eq!(preview.prefix_hashes, plan_req.prefix_hashes);
    assert_eq!(preview.expected_base_hash, plan_req.expected_base_hash);
    assert_eq!(preview.target_step, plan_req.target_step);
    assert_eq!(preview.edited_from, Some(1));
    assert_eq!(preview.edited_from, plan_req.edited_from);
    assert!(preview.base_checkpoint.is_none() && preview.artifacts.tessellate.is_none());
    assert_eq!(job.edited_step, 1);
}

#[test]
fn an_unchanged_or_unreachable_edit_never_reaches_the_worker() {
    let mut rt = runtime();
    let EditPreviewPrep::Done(same) = rt
        .plan_edit_preview(rec(0x11), extrude(10.0), Lod::Coarse)
        .unwrap()
    else {
        panic!("unchanged must not run");
    };
    assert_eq!(same.outcome, EditPreviewOutcome::Unchanged);
    assert!(rt.reedit_is_unchanged(rec(0x11), &extrude(10.0)).unwrap());
    assert!(!rt.reedit_is_unchanged(rec(0x11), &extrude(9.0)).unwrap());

    rt.apply(EditCommand::SetRollback { cursor: 1 }).unwrap();
    let EditPreviewPrep::Done(rolled) = rt
        .plan_edit_preview(rec(0x11), extrude(5.0), Lod::Coarse)
        .unwrap()
    else {
        panic!("a rolled-back record must not run");
    };
    assert_eq!(
        rolled.outcome,
        EditPreviewOutcome::NotReached { ceiling: None }
    );
}

/// Records every request and answers from a script.
#[derive(Default)]
struct ScriptedPreview {
    answers: Mutex<Vec<Result<PlanPreview, EngineError>>>,
    seen: Mutex<Vec<PreviewPlanRequest>>,
}

#[async_trait]
impl PreviewEngine for ScriptedPreview {
    async fn preview_op(
        &self,
        _operation: Operation,
        _op_id: String,
        _sketch_id: Option<String>,
        _expected_snapshot: Option<SnapshotId>,
        _lod: Lod,
    ) -> Result<crate::dto::PreviewResultDto, EngineError> {
        unreachable!("edit preview never uses PreviewOp")
    }

    async fn preview_plan(
        &self,
        req: PreviewPlanRequest,
        _cancel: CancelToken,
    ) -> Result<PlanPreview, EngineError> {
        self.seen.lock().unwrap().push(req);
        let mut answers = self.answers.lock().unwrap();
        if answers.is_empty() {
            // A dead worker for any unscripted (isolation) pass: the halted
            // pass's result must survive it, as `adopt_isolated` keeps it.
            return Err(EngineError::Crashed {
                message: "script exhausted".into(),
            });
        }
        answers.remove(0)
    }
}

fn halted_at(step: usize, status: StepStatus) -> PlanPreview {
    let mut per_step: Vec<StepResult> = (0..step)
        .map(|s| StepResult {
            step_index: s,
            status: StepStatus::Ok,
            body_ids: Vec::new(),
            message: String::new(),
            diagnostics: Vec::new(),
        })
        .collect();
    per_step.push(StepResult {
        step_index: step,
        status,
        body_ids: Vec::new(),
        message: "boom".into(),
        diagnostics: Vec::new(),
    });
    PlanPreview {
        snapshot_id: SnapshotId(0),
        last_valid_step: step.checked_sub(1),
        stopped_reason: if status == StepStatus::OpFailed {
            StoppedReason::OpFailed
        } else {
            StoppedReason::NeedsRepair
        },
        per_step,
        steps: Vec::new(),
        bodies: Vec::new(),
        meshes: Vec::new(),
    }
}

async fn finish_with(answer: Result<PlanPreview, EngineError>) -> Result<EditPreview, ApiError> {
    let mut rt = runtime();
    let job = run_job(&mut rt, 5.0);
    let engine = ScriptedPreview {
        answers: Mutex::new(vec![answer]),
        ..Default::default()
    };
    let run = drive_edit_preview(&job, &engine).await;
    rt.finish_edit_preview(*job, run)
}

#[tokio::test]
async fn failures_are_classified_relative_to_the_edited_step() {
    let edited = finish_with(Ok(halted_at(1, StepStatus::OpFailed)))
        .await
        .unwrap();
    assert!(matches!(
        edited.outcome,
        EditPreviewOutcome::EditedFailed { needs_repair: false, ref message, .. } if message == "boom"
    ));
    let downstream = finish_with(Ok(halted_at(2, StepStatus::NeedsRepair)))
        .await
        .unwrap();
    assert_eq!(
        downstream.outcome,
        EditPreviewOutcome::DownstreamNeedsRepair {
            step: 2,
            record: rec(0x12)
        }
    );
    let earlier = finish_with(Ok(halted_at(0, StepStatus::OpFailed)))
        .await
        .unwrap();
    assert_eq!(
        earlier.outcome,
        EditPreviewOutcome::NotReached { ceiling: None }
    );
}

#[tokio::test]
async fn engine_errors_map_explicitly() {
    let unavailable = PreviewEngine::preview_plan(
        &PendingBackendWithoutPlan,
        run_job(&mut runtime(), 5.0).request.clone(),
        CancelToken::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        map_preview_error(unavailable),
        ApiError::Unsupported(_)
    ));
    assert!(matches!(
        finish_with(Err(EngineError::Cancelled)).await,
        Err(ApiError::Cancelled(_))
    ));
    let stale = EngineError::OpFailed {
        code: OpFailureCode::StalePreview,
        recoverable: true,
        message: "moved".into(),
        diagnostics: Vec::new(),
    };
    assert!(matches!(
        finish_with(Err(stale)).await,
        Err(ApiError::StalePreview(_))
    ));
    let crashed = EngineError::Crashed {
        message: "gone".into(),
    };
    assert!(matches!(
        finish_with(Err(crashed)).await,
        Err(ApiError::Worker(_))
    ));
}

#[tokio::test]
async fn a_superseded_result_is_stale_whatever_it_carries() {
    let mut rt = runtime();
    let job = run_job(&mut rt, 5.0);
    let _newer = run_job(&mut rt, 6.0);
    assert!(job.cancel_token().is_cancelled());
    let result = rt.finish_edit_preview(
        *job,
        Ok(EditPreviewRun {
            halted: halted_at(1, StepStatus::OpFailed),
            isolated: None,
        }),
    );
    assert!(matches!(result, Err(ApiError::StalePreview(_))));
}

#[tokio::test]
async fn an_unmeshed_body_the_head_cannot_show_is_refused() {
    let mut pass = halted_at(0, StepStatus::Ok);
    pass.stopped_reason = StoppedReason::Completed;
    pass.bodies = vec![BodyId(Uuid::from_u128(0x99))];
    let refused = finish_with(Ok(pass)).await;
    assert!(
        matches!(refused, Err(ApiError::Unsupported(_))),
        "{refused:?}"
    );
}

/// A backend that never implemented `preview_plan` (the trait default).
struct PendingBackendWithoutPlan;

#[async_trait]
impl PreviewEngine for PendingBackendWithoutPlan {
    async fn preview_op(
        &self,
        _operation: Operation,
        _op_id: String,
        _sketch_id: Option<String>,
        _expected_snapshot: Option<SnapshotId>,
        _lod: Lod,
    ) -> Result<crate::dto::PreviewResultDto, EngineError> {
        unreachable!()
    }
}
