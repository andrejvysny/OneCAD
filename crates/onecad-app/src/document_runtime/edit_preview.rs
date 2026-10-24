//! ONECAD-369 edit preview: the locked phases of a non-publishing full replay of
//! an existing record's candidate edit (SCHEMA §7.6 `PreviewPlan`).
//!
//! Phase 1 ([`DocumentRuntime::plan_edit_preview`], locked) applies the edit to a
//! CLONE of the document through the same `UpdateOperationParams` validation,
//! intent hydration and effective-timeline lowering as the commit, and plans it
//! with the same gate, `from` and `editedFrom` — but from 0 and with no
//! checkpoint. Phase 2 ([`drive_edit_preview`], unlocked) runs it, plus the
//! WP-D1 isolation pass the commit would run after a halt. Phase 3
//! ([`DocumentRuntime::finish_edit_preview`], locked) drops a superseded result
//! and classifies the rest. Nothing here touches the session, the revision, the
//! dirty flag, undo, checkpoints or the mesh cache.

use std::collections::HashSet;

use onecad_core::document::record::{Operation, OperationRecord};
use onecad_core::document::repair::RepairItem;
use onecad_core::edit::session::DocumentSession;
use onecad_core::edit::{EditCommand, RegenHint};
use onecad_core::history::{DependencyGraph, Timeline};
use onecad_core::ids::{BodyId, JobId, RecordId, SnapshotId};
use onecad_core::regen::{
    CancelToken, Diagnostic, EngineError, Lod, PlanArtifacts, PlanContext, PolicyVersions,
    RegenPlanner, RegenRequest, StepSignatures,
};

use super::{lower_feature_patterns, DocumentRuntime};
use crate::dto::PreviewBodyDto;
use crate::error::ApiError;
use crate::worker::{PlanPreview, PreviewPlanRequest};

mod classify;
mod drive;

use classify::map_preview_error;

pub use drive::{drive_edit_preview, EditPreviewRun};

/// The one in-flight edit preview of a runtime. Installing a new one, a
/// publishing regen and dropping the runtime all cancel the current one.
#[derive(Debug, Default)]
pub(crate) struct EditPreviewSlot {
    active: Option<(u64, CancelToken)>,
    /// The preview a publishing regen cancelled, so its caller hears `Cancelled`
    /// rather than the `StalePreview` of a superseded draft.
    preempted: Option<u64>,
    next: u64,
}

/// How a finishing preview relates to the slot.
#[derive(Debug, PartialEq, Eq)]
enum SlotFinish {
    Current,
    Preempted,
    Superseded,
}

impl EditPreviewSlot {
    fn install(&mut self) -> (u64, CancelToken) {
        self.cancel_active();
        self.next += 1;
        let token = CancelToken::new();
        self.active = Some((self.next, token.clone()));
        (self.next, token)
    }

    /// Supersedes the current preview: it finishes as `StalePreview`.
    pub(crate) fn cancel_active(&mut self) {
        self.preempted = None;
        if let Some((_, token)) = self.active.take() {
            token.cancel();
        }
    }

    /// A publishing regen cancels the current preview: it finishes as `Cancelled`.
    fn preempt(&mut self) {
        if let Some((seq, token)) = self.active.take() {
            token.cancel();
            self.preempted = Some(seq);
        }
    }

    /// Releases the slot for `seq` and says whether it was still current.
    fn finish(&mut self, seq: u64) -> SlotFinish {
        match &self.active {
            Some((current, _)) if *current == seq => {
                self.active = None;
                SlotFinish::Current
            }
            _ if self.preempted == Some(seq) => {
                self.preempted = None;
                SlotFinish::Preempted
            }
            _ => SlotFinish::Superseded,
        }
    }
}

impl Drop for EditPreviewSlot {
    fn drop(&mut self) {
        self.cancel_active();
    }
}

/// How an edit preview relates to the edited feature (step `k`).
#[derive(Debug, Clone, PartialEq)]
pub enum EditPreviewOutcome {
    /// Every planned step succeeded.
    Ready,
    /// The candidate equals the stored operation; nothing was run.
    Unchanged,
    /// The edited step cannot execute: it is suppressed or rolled back, a repair
    /// or variable gate sits at or below it, or an earlier step stopped the
    /// replay and the edited step depends on it. `ceiling` is the last step the
    /// replay may or did reach.
    NotReached { ceiling: Option<usize> },
    /// The edited feature itself failed or needs repair at its own step.
    EditedFailed {
        message: String,
        diagnostics: Vec<Diagnostic>,
        needs_repair: bool,
    },
    /// Another feature failed at `step`: a later one, or an earlier one the
    /// WP-D1 isolation pass routed around (Apply still publishes the edit).
    DownstreamFailed {
        step: usize,
        record: RecordId,
        message: String,
    },
    /// Another feature needs repair at `step` (later, or routed around as above).
    DownstreamNeedsRepair { step: usize, record: RecordId },
}

/// A classified, display-ready edit preview.
#[derive(Debug, Clone)]
pub struct EditPreview {
    pub record: RecordId,
    /// The edited record's timeline index, when it exists.
    pub edited_step: Option<usize>,
    /// The worker head the preview was fenced against.
    pub snapshot_id: SnapshotId,
    pub outcome: EditPreviewOutcome,
    /// The candidate body set (the isolation pass's when one ran).
    pub bodies: Vec<BodyId>,
    /// Ghost meshes: candidate bodies that differ from the head, visible only.
    pub meshes: Vec<PreviewBodyDto>,
    /// Head bodies to hide while the preview is shown: replaced by a mesh,
    /// absent from the candidate, or hidden by the user.
    pub suppressed: Vec<BodyId>,
    /// NeedsRepair items the halted pass reported.
    pub needs_repair: Vec<RepairItem>,
    /// Per-step signatures of the displayed pass (evidence for preview==commit).
    pub step_signatures: Vec<(usize, StepSignatures)>,
    /// Whether the WP-D1 isolation pass supplied the displayed bodies.
    pub isolated: bool,
}

impl EditPreview {
    fn idle(record: RecordId, edited_step: Option<usize>, outcome: EditPreviewOutcome) -> Self {
        Self {
            record,
            edited_step,
            snapshot_id: SnapshotId(0),
            outcome,
            bodies: Vec::new(),
            meshes: Vec::new(),
            suppressed: Vec::new(),
            needs_repair: Vec::new(),
            step_signatures: Vec::new(),
            isolated: false,
        }
    }
}

/// Phase 1's result.
#[derive(Debug)]
pub enum EditPreviewPrep {
    /// Classified without the worker (`Unchanged` / `NotReached`).
    Done(Box<EditPreview>),
    /// Run [`drive_edit_preview`] on this, then [`DocumentRuntime::finish_edit_preview`].
    Run(Box<EditPreviewJob>),
}

/// What phase 2 runs and phase 3 needs back. Owned, so no lock is held across IO.
#[derive(Debug)]
pub struct EditPreviewJob {
    seq: u64,
    cancel: CancelToken,
    /// For failure surfacing only.
    runtime_session: String,
    record: RecordId,
    edited_step: usize,
    request: PreviewPlanRequest,
    /// The candidate document's records and graph — the inputs the commit's
    /// `begin_isolation_regen` reads — plus the effective mirror it re-plans.
    records: Vec<OperationRecord>,
    graph: DependencyGraph,
    mirror: Timeline,
    isolation_job: JobId,
    head_bodies: Vec<BodyId>,
}

impl EditPreviewJob {
    /// The job's cancel token; cancelling it stops the worker replay even after
    /// the driving future is dropped.
    #[must_use]
    pub fn cancel_token(&self) -> CancelToken {
        self.cancel.clone()
    }
}

impl DocumentRuntime {
    /// Whether `op` equals the stored operation of `record`, as authored or as
    /// the commit would hydrate it.
    ///
    /// # Errors
    /// [`ApiError::InvalidCommand`] for an unknown record.
    pub fn reedit_is_unchanged(&self, record: RecordId, op: &Operation) -> Result<bool, ApiError> {
        let prior = self
            .session
            .document()
            .timeline
            .records()
            .iter()
            .find(|r| r.record_id == record)
            .ok_or_else(|| ApiError::InvalidCommand(format!("unknown record {record}")))?;
        if prior.op == *op {
            return Ok(true);
        }
        let hydrated = self.hydrate_ref_intents(EditCommand::UpdateOperationParams {
            record,
            op: op.clone(),
        });
        Ok(matches!(hydrated, EditCommand::UpdateOperationParams { op, .. } if op == prior.op))
    }

    /// Phase 1 (**locked**). See the module docs.
    ///
    /// # Errors
    /// [`ApiError::InvalidCommand`] when the document cannot be edited now or the
    /// candidate fails the commit's own validation.
    pub fn plan_edit_preview(
        &mut self,
        record: RecordId,
        op: Operation,
        lod: Lod,
    ) -> Result<EditPreviewPrep, ApiError> {
        self.refuse_edit_preview_now()?;
        // Latest-wins for EVERY draft, including the ones answered without the
        // worker: reverting a field must not leave the obsolete replay running.
        self.edit_preview.cancel_active();
        if self.reedit_is_unchanged(record, &op)? {
            return Ok(EditPreviewPrep::Done(Box::new(EditPreview::idle(
                record,
                self.session.document().timeline.index_of(record),
                EditPreviewOutcome::Unchanged,
            ))));
        }
        let command = self.hydrate_ref_intents(EditCommand::UpdateOperationParams { record, op });
        self.plan_command_preview(record, command, lod)
    }

    pub(crate) fn plan_input_repair_preview(
        &mut self,
        record: RecordId,
        command: EditCommand,
        lod: Lod,
    ) -> Result<EditPreviewPrep, ApiError> {
        self.refuse_edit_preview_now()?;
        self.edit_preview.cancel_active();
        self.plan_command_preview(record, command, lod)
    }

    fn plan_command_preview(
        &mut self,
        record: RecordId,
        command: EditCommand,
        lod: Lod,
    ) -> Result<EditPreviewPrep, ApiError> {
        let mut candidate = DocumentSession::new(self.session.document().clone());
        let outcome = candidate.apply(command)?;
        let doc = candidate.document();
        let edited_step = doc.timeline.index_of(record);
        let idle = |outcome| {
            Ok(EditPreviewPrep::Done(Box::new(EditPreview::idle(
                record,
                edited_step,
                outcome,
            ))))
        };
        if !matches!(outcome.regen, RegenHint::ToEnd) {
            return idle(EditPreviewOutcome::Unchanged);
        }
        // The commit's own `from` (scheduler `handle`): the dirty floor.
        let from = outcome.dirty.map_or(0, |d| d.from);
        let Some(k) = edited_step
            .filter(|&k| k < doc.timeline.cursor() && !doc.timeline.records()[k].suppressed)
        else {
            return idle(EditPreviewOutcome::NotReached { ceiling: None });
        };
        let (mut mirror, unresolved) =
            onecad_core::regen::substituted_timeline(&doc.timeline, &doc.variables);
        let pattern_errors = lower_feature_patterns(&mut mirror);
        // `begin_regen`'s gate, read from the candidate.
        let gate = [
            doc.repair.first_seeded_step(),
            unresolved.keys().next().copied(),
            pattern_errors.keys().next().copied(),
        ]
        .into_iter()
        .flatten()
        .min();
        let ceiling = gate.map(|g| g.checked_sub(1));
        if let Some(ceiling) = ceiling {
            if ceiling.is_none_or(|c| c < k) {
                return idle(EditPreviewOutcome::NotReached { ceiling });
            }
        }
        let ctx = PlanContext {
            policy_versions: PolicyVersions::default(),
            occt_fingerprint: self.occt_fingerprint.clone(),
        };
        let request = RegenRequest::ToEnd { from };
        let plan = RegenPlanner::plan_with_ceiling(
            &mirror,
            &DependencyGraph::new(),
            &[],
            request,
            &ctx,
            ceiling.flatten(),
        );
        if plan.is_empty() {
            return idle(EditPreviewOutcome::NotReached {
                ceiling: ceiling.flatten(),
            });
        }
        let records = doc.timeline.records().to_vec();
        let graph = candidate.graph().clone();
        let job = self.next_job_id();
        let isolation_job = self.next_job_id();
        let (revision, epoch) = self.fencing.get();
        let plan = plan
            .into_request(
                job,
                revision,
                epoch,
                PolicyVersions::default(),
                PlanArtifacts::default(),
            )
            .with_edited_from(request.edited_from());
        let (seq, cancel) = self.edit_preview.install();
        Ok(EditPreviewPrep::Run(Box::new(EditPreviewJob {
            seq,
            cancel,
            runtime_session: self.runtime_session(),
            record,
            edited_step: k,
            request: PreviewPlanRequest {
                plan,
                expected_snapshot: self.head_snapshot_id().unwrap_or(SnapshotId(0)),
                lod,
            },
            records,
            graph,
            mirror,
            isolation_job,
            head_bodies: self.head_body_ids(),
        })))
    }

    fn refuse_edit_preview_now(&self) -> Result<(), ApiError> {
        if self.read_only {
            return Err(ApiError::InvalidCommand("document is read-only".into()));
        }
        if self.active_gesture.is_some() || self.native_gesture.is_some() {
            return Err(ApiError::InvalidCommand(
                "cannot preview an edit during a sketch drag".into(),
            ));
        }
        if self.session.transaction_open() {
            return Err(ApiError::InvalidCommand(
                "cannot preview an edit inside an open transaction".into(),
            ));
        }
        Ok(())
    }

    /// Cancels the in-flight edit preview, if any; it then reports
    /// `ApiError::Cancelled`. Every publishing regen calls this first: commit
    /// preempts preview.
    pub fn cancel_edit_preview(&mut self) {
        self.edit_operands.cancel();
        self.edit_preview.preempt();
    }

    /// Phase 3 (**locked**). A result that is no longer the current preview is
    /// `Cancelled` when a publishing regen preempted it and `StalePreview` when a
    /// newer draft superseded it, whatever it carries.
    ///
    /// # Errors
    /// `StalePreview` (superseded / fenced out), `Cancelled` (preempted by a
    /// publishing regen), `Unsupported` (no capability, mesh budget, an unmeshed
    /// body the head cannot show, any other worker refusal of the preview), or
    /// the worker-class error as-is.
    pub fn finish_edit_preview(
        &mut self,
        job: EditPreviewJob,
        run: Result<EditPreviewRun, EngineError>,
    ) -> Result<EditPreview, ApiError> {
        match self.edit_preview.finish(job.seq) {
            SlotFinish::Current => {}
            SlotFinish::Preempted => {
                return Err(ApiError::Cancelled(
                    "edit preview preempted by a publishing regen".into(),
                ))
            }
            SlotFinish::Superseded => {
                return Err(ApiError::StalePreview("edit preview superseded".into()))
            }
        }
        let run = run.map_err(|error| map_preview_error(&job, error))?;
        let hidden: HashSet<BodyId> = self
            .session
            .document()
            .bodies
            .bodies()
            .iter()
            .filter(|meta| !meta.visible)
            .map(|meta| meta.id)
            .collect();
        build_preview(&job, run, &hidden)
    }
}

fn build_preview(
    job: &EditPreviewJob,
    run: EditPreviewRun,
    hidden: &HashSet<BodyId>,
) -> Result<EditPreview, ApiError> {
    let outcome = classify::classify(job, &run.halted, run.isolated.as_ref());
    let needs_repair = run
        .halted
        .steps
        .iter()
        .flat_map(|step| step.needs_repair.iter().cloned())
        .collect();
    let isolated = run.isolated.is_some();
    let shown: PlanPreview = run.isolated.unwrap_or(run.halted);
    let meshed: HashSet<BodyId> = shown.meshes.iter().map(|m| m.body).collect();
    let present: HashSet<BodyId> = shown.bodies.iter().copied().collect();
    let head: HashSet<BodyId> = job.head_bodies.iter().copied().collect();
    if let Some(body) = shown
        .bodies
        .iter()
        .find(|b| !meshed.contains(b) && !head.contains(b))
    {
        return Err(ApiError::Unsupported(format!(
            "preview incomplete: body {body} has no mesh and no head geometry"
        )));
    }
    let suppressed = job
        .head_bodies
        .iter()
        .copied()
        .filter(|b| meshed.contains(b) || !present.contains(b) || hidden.contains(b))
        .collect();
    let meshes = shown
        .meshes
        .iter()
        .filter(|m| !hidden.contains(&m.body))
        .map(|m| PreviewBodyDto {
            body_id: m.body.to_string(),
            mesh: m.bytes.to_vec(),
        })
        .collect();
    Ok(EditPreview {
        record: job.record,
        edited_step: Some(job.edited_step),
        snapshot_id: shown.snapshot_id,
        outcome,
        bodies: shown.bodies.clone(),
        meshes,
        suppressed,
        needs_repair,
        step_signatures: shown
            .steps
            .iter()
            .map(|s| (s.step_index, s.signatures.clone()))
            .collect(),
        isolated,
    })
}

#[cfg(test)]
mod tests;
