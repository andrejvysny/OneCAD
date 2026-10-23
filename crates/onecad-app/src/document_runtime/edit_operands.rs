//! A separate latest-only read slot; candidate previews never replace prefix context.
use super::*;
use crate::worker::{EditOperandRequest, PreviewPlanRequest};
use tokio::task::JoinHandle;

#[derive(Default)]
pub(crate) struct OperandReadSlot {
    next: u64,
    serialization: std::sync::Arc<tokio::sync::Mutex<()>>,
    active: Option<(u64, CancelToken, Option<JoinHandle<()>>)>,
}
impl OperandReadSlot {
    pub fn cancel(&mut self) {
        if let Some((_, cancel, task)) = self.active.take() {
            cancel.cancel();
            if let Some(task) = task {
                task.abort();
            }
        }
    }
    fn install(&mut self) -> (u64, CancelToken) {
        self.cancel();
        self.next += 1;
        let cancel = CancelToken::new();
        self.active = Some((self.next, cancel.clone(), None));
        (self.next, cancel)
    }
    pub fn attach(&mut self, seq: u64, task: JoinHandle<()>) {
        if let Some((_, _, stored)) = self.active.as_mut().filter(|a| a.0 == seq) {
            if stored.is_some() {
                task.abort();
                return;
            }
            *stored = Some(task);
        } else {
            task.abort();
        }
    }
    pub fn finish(&mut self, seq: u64) -> bool {
        if self.active.as_ref().is_some_and(|a| a.0 == seq) {
            self.active.take();
            true
        } else {
            false
        }
    }
}
impl Drop for OperandReadSlot {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl DocumentRuntime {
    pub(crate) fn plan_edit_operands(
        &mut self,
        record: RecordId,
        fence: &crate::read::PreviewContext,
    ) -> Result<(u64, CancelToken, EditOperandRequest), ApiError> {
        let document = self.session.document();
        let k = document
            .timeline
            .index_of(record)
            .filter(|&k| {
                k < document.timeline.cursor() && !document.timeline.records()[k].suppressed
            })
            .ok_or_else(|| {
                ApiError::Unsupported("edited feature is outside the active timeline".into())
            })?;
        crate::worker::edit_operands::validate_operation(&document.timeline.records()[k].op)
            .map_err(ApiError::Unsupported)?;
        let (mut mirror, unresolved) =
            onecad_core::regen::substituted_timeline(&document.timeline, &document.variables);
        let pattern_errors = lower_feature_patterns(&mut mirror);
        if document
            .repair
            .first_seeded_step()
            .is_some_and(|step| step < k)
            || unresolved.keys().any(|&step| step < k)
            || pattern_errors.keys().any(|&step| step < k)
        {
            return Err(ApiError::Unsupported(
                "feature prefix requires repair before operands can be read".into(),
            ));
        }
        let operation = mirror.records()[k].op.clone();
        mirror.set_cursor(k);
        let context = PlanContext {
            policy_versions: PolicyVersions::default(),
            occt_fingerprint: self.occt_fingerprint.clone(),
        };
        let plan = RegenPlanner::plan(
            &mirror,
            &DependencyGraph::new(),
            &[],
            RegenRequest::ToStep(k.saturating_sub(1)),
            &context,
        );
        let request = EditOperandRequest {
            prefix: PreviewPlanRequest {
                plan: plan.into_request(
                    self.next_job_id(),
                    fence.revision,
                    fence.worker_epoch,
                    PolicyVersions::default(),
                    PlanArtifacts::default(),
                ),
                expected_snapshot: fence.snapshot_id,
                lod: Lod::Coarse,
            },
            record,
            operation,
            serialization: self.edit_operands.serialization.clone(),
        };
        crate::worker::edit_operands::operation(&request).map_err(ApiError::Unsupported)?;
        let (seq, cancel) = self.edit_operands.install();
        Ok((seq, cancel, request))
    }
    pub(crate) fn attach_operand_read(&mut self, seq: u64, task: JoinHandle<()>) {
        self.edit_operands.attach(seq, task);
    }
    pub(crate) fn finish_operand_read(&mut self, seq: u64) -> bool {
        self.edit_operands.finish(seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn latest_operand_read_cancels_replacement_and_rejects_late_attachment() {
        let mut slot = OperandReadSlot::default();
        let (first, cancelled) = slot.install();
        let (second, active) = slot.install();
        assert!(cancelled.is_cancelled());
        assert!(!active.is_cancelled());
        let task = tokio::spawn(std::future::pending::<()>());
        let abort = task.abort_handle();
        slot.attach(first, task);
        assert!(!slot.finish(first));
        assert!(slot.finish(second));
        assert!(!slot.finish(second));
        let (_, dropped) = slot.install();
        drop(slot);
        assert!(dropped.is_cancelled());
        tokio::task::yield_now().await;
        assert!(abort.is_finished());
    }
}
