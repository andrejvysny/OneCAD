//! Sketch-local history reuses bounded document mementos behind an edit-visit fence.
use super::*;
use crate::dto::{SketchHistoryDto, SketchHistoryScope};
use onecad_core::edit::HistoryDirection;

pub struct PreparedSketchHistory {
    scope: SketchHistoryScope,
    direction: HistoryDirection,
    candidate: Sketch,
    solver: Arc<dyn SolverEngine>,
    fencing: (DocumentRevision, WorkerEpoch),
    _claim: Claim,
}

pub struct DrivenSketchHistory {
    prepared: PreparedSketchHistory,
    solve: SketchUpsertDto,
}

impl PreparedSketchHistory {
    pub async fn drive(self) -> Result<DrivenSketchHistory, EngineError> {
        let solve = self.solver.sketch_upsert(&self.candidate).await?;
        Ok(DrivenSketchHistory {
            prepared: self,
            solve,
        })
    }
}

impl DocumentRuntime {
    pub(super) fn sketch_edit_token(&self, id: SketchId) -> Option<Uuid> {
        self.sketch_session
            .as_ref()
            .filter(|session| session.sketch_id == id)
            .map(|session| session.token)
    }

    pub(super) fn sketch_history_availability(
        &self,
        id: SketchId,
    ) -> Option<crate::dto::SketchHistoryAvailability> {
        let session = self.sketch_session.as_ref().filter(|session| {
            session.sketch_id == id
                && session.history_valid
                && session.history_epoch == self.engine.current_epoch()
                && self.session.undo_depth() >= session.retained_watermark(self.session.evictions())
        })?;
        Some(crate::dto::SketchHistoryAvailability {
            undo_depth: self.session.sketch_history_depth(
                HistoryDirection::Undo,
                id,
                self.session
                    .undo_depth()
                    .saturating_sub(session.retained_watermark(self.session.evictions())),
            ),
            redo_depth: self.session.sketch_history_depth(
                HistoryDirection::Redo,
                id,
                session.history_redo,
            ),
        })
    }

    fn check_sketch_history(
        &self,
        scope: &SketchHistoryScope,
    ) -> Result<&SketchSession, EngineError> {
        let session = self
            .sketch_session
            .as_ref()
            .filter(|session| {
                session.sketch_id == scope.sketch_id
                    && session.token.to_string() == scope.edit_session
            })
            .ok_or_else(|| op_failed("Sketch history: edit session is no longer current"))?;
        if session.history_epoch != self.engine.current_epoch()
            || !session.history_valid
            || self.read_only
            || self.instance.to_string() != scope.runtime_session
            || self.revision().0 != scope.revision
            || self.head_snapshot_id().unwrap_or(SnapshotId(0)) != scope.snapshot_id
            || self.fencing.get().1 != self.engine.current_epoch()
            || self.session.undo_depth() < session.retained_watermark(self.session.evictions())
        {
            return Err(op_failed(
                "Sketch history: document, worker or history boundary changed",
            ));
        }
        Ok(session)
    }

    pub async fn prepare_sketch_history(
        &self,
        scope: SketchHistoryScope,
        direction: HistoryDirection,
    ) -> Result<Option<PreparedSketchHistory>, EngineError> {
        let session = self.check_sketch_history(&scope)?;
        let empty = match direction {
            HistoryDirection::Undo => {
                self.session.undo_depth() == session.retained_watermark(self.session.evictions())
            }
            HistoryDirection::Redo => session.history_redo == 0 || self.session.redo_depth() == 0,
        };
        if empty {
            return Ok(None);
        }
        let claim = self.claim_upsert_lane(scope.sketch_id).await?;
        self.check_sketch_history(&scope)?;
        let candidate = self
            .session
            .prepare_sketch_history(scope.sketch_id, direction)
            .map_err(|error| op_failed(format!("{error}")))?
            .ok_or_else(|| op_failed("Sketch history: history entry disappeared"))?;
        Ok(Some(PreparedSketchHistory {
            scope,
            direction,
            candidate,
            solver: self.solver.clone(),
            fencing: self.fencing.get(),
            _claim: claim,
        }))
    }

    pub fn sketch_history_result(
        &self,
        scope: &SketchHistoryScope,
        direction: HistoryDirection,
        moved: bool,
    ) -> Result<SketchHistoryDto, EngineError> {
        let session = self.check_sketch_history(scope)?;
        Ok(SketchHistoryDto {
            moved,
            direction,
            undo_depth: self
                .session
                .undo_depth()
                .saturating_sub(session.retained_watermark(self.session.evictions())),
            redo_depth: session.history_redo,
        })
    }

    pub fn commit_sketch_history(
        &mut self,
        driven: DrivenSketchHistory,
    ) -> Result<(SketchHistoryDto, RevertReport, SketchUpsertDto), EngineError> {
        let prepared = driven.prepared;
        let session = self.check_sketch_history(&prepared.scope)?;
        let redo = session.history_redo;
        if self.fencing.get() != prepared.fencing {
            return Err(op_failed("Sketch history: worker fence changed"));
        }
        let after = self.adopt_candidate_solve(&prepared.candidate, &driven.solve)?;
        if !matches!(
            driven.solve.status,
            SketchSolveStatus::UnderConstrained | SketchSolveStatus::FullyConstrained
        ) {
            return Err(op_failed(
                "Sketch history: restored sketch did not solve cleanly",
            ));
        }
        let id = prepared.scope.sketch_id;
        let outcome = self
            .session
            .commit_sketch_history(id, prepared.direction, after)
            .map_err(|error| op_failed(format!("Sketch history: {error}")))?
            .ok_or_else(|| op_failed("Sketch history: history entry disappeared"))?;
        let dirty_floor = outcome.dirty_floor;
        let report = self.revert_report(outcome);
        self.after_mutation(dirty_floor);
        if let Some(session) = &mut self.sketch_session {
            session.history_redo = match prepared.direction {
                HistoryDirection::Undo => redo + 1,
                HistoryDirection::Redo => redo.saturating_sub(1),
            };
        }
        self.record_upsert(id, &driven.solve);
        let mut current = prepared.scope;
        current.revision = self.revision().0;
        current.snapshot_id = self.head_snapshot_id().unwrap_or(SnapshotId(0));
        let result = self.sketch_history_result(&current, prepared.direction, true)?;
        Ok((result, report, driven.solve))
    }
}
