//! Read-only history preparation clones one bounded sketch, never the document.
use super::*;
use crate::edit::HistoryDirection;

impl DocumentSession {
    pub fn sketch_history_depth(
        &self,
        direction: HistoryDirection,
        id: SketchId,
        max: usize,
    ) -> usize {
        self.undo.sketch_history_depth(direction, id, max)
    }

    pub fn prepare_sketch_history(
        &self,
        id: SketchId,
        direction: HistoryDirection,
    ) -> Result<Option<Sketch>, DomainError> {
        if self.transaction_open() {
            return Err(DomainError::Validation(
                "Sketch history: transaction is open".into(),
            ));
        }
        let Some(txn) = self.undo.history_txn(direction) else {
            return Ok(None);
        };
        if !txn.edits.iter().all(|edit| matches!(&edit.forward,
            EditCommand::SketchEdit { sketch, .. } | EditCommand::SketchDragGesture { sketch, .. } if *sketch == id)) {
            return Err(DomainError::Validation("Sketch history: next step belongs to another operation".into()));
        }
        let mut candidate =
            self.document.sketches.get(&id).cloned().ok_or_else(|| {
                DomainError::Validation("Sketch history: sketch is missing".into())
            })?;
        match direction {
            HistoryDirection::Undo => {
                for edit in txn.edits.iter().rev() {
                    restore(&mut candidate, &edit.inverse)?;
                }
            }
            HistoryDirection::Redo => {
                for edit in &txn.edits {
                    candidate = match &edit.forward {
                        EditCommand::SketchEdit { ops, .. } => apply_sketch_ops(&candidate, ops)?,
                        EditCommand::SketchDragGesture { after, .. } => after.clone(),
                        _ => {
                            return Err(DomainError::Validation(
                                "Sketch history: unsupported forward edit".into(),
                            ));
                        }
                    };
                }
            }
        }
        Ok(Some(candidate))
    }

    /// Keeps an existing sketch record refresh in the finished sketch's net edit.
    /// Refuses to append across an interleaved command or caller-owned transaction.
    pub fn refresh_finished_sketch_record(
        &mut self,
        sketch: SketchId,
        record: RecordId,
        op: Operation,
    ) -> Result<Option<CommandOutcome>, DomainError> {
        let owns_sketch = |op: &Operation| {
            matches!(op,
            Operation::Known(KnownOperation::Sketch(params)) if params.sketch == sketch)
        };
        let valid_record = self
            .document
            .timeline
            .index_of(record)
            .and_then(|index| self.document.timeline.record(index))
            .is_some_and(|record| owns_sketch(&record.op));
        if !valid_record || !owns_sketch(&op) {
            return Err(DomainError::Validation(
                "finished sketch record does not match sketch".into(),
            ));
        }
        let eligible = !self.transaction_open()
            && self
                .undo
                .history_txn(HistoryDirection::Undo)
                .is_some_and(|txn| {
                    txn.edits.len() == 1
                        && matches!(txn.edits[0].forward,
                EditCommand::SketchEdit { sketch: id, .. } if id == sketch)
                });
        if !eligible {
            return Ok(None);
        }
        let forward = EditCommand::UpdateOperationParams { record, op };
        let (outcome, inverse) = self.apply_forward(&forward)?;
        if !matches!(inverse, Inverse::Noop) {
            self.undo.append_latest(AppliedEdit { forward, inverse });
        }
        Ok(Some(outcome))
    }

    /// The application has validated a fresh solver result for the prepared target.
    /// Updating its derived coordinates belongs to this history move, not another edit.
    pub fn commit_sketch_history(
        &mut self,
        id: SketchId,
        direction: HistoryDirection,
        solved: Sketch,
    ) -> Result<Option<UndoOutcome>, DomainError> {
        if solved.id != id || self.prepare_sketch_history(id, direction)?.is_none() {
            return Err(DomainError::Validation(
                "Sketch history: target is no longer available".into(),
            ));
        }
        let outcome = match direction {
            HistoryDirection::Undo => self.undo(),
            HistoryDirection::Redo => self.redo()?,
        };
        if outcome.is_some() {
            self.document.sketches.insert(id, solved);
            self.rebuild_graph();
        }
        Ok(outcome)
    }
}

fn restore(candidate: &mut Sketch, inverse: &Inverse) -> Result<(), DomainError> {
    match inverse {
        Inverse::RestoreSketch {
            id,
            prior: Some(prior),
        } if *id == candidate.id => {
            *candidate = prior.as_ref().clone();
            Ok(())
        }
        Inverse::Composite(inverses) => {
            for inverse in inverses.iter().rev() {
                restore(candidate, inverse)?;
            }
            Ok(())
        }
        Inverse::RestoreRepair { .. } | Inverse::Noop => Ok(()),
        _ => Err(DomainError::Validation(
            "Sketch history: inverse is not confined to this sketch".into(),
        )),
    }
}

#[cfg(test)]
#[path = "sketch_history/finish_tests.rs"]
mod finish_tests;
