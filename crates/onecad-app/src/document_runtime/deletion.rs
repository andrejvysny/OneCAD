//! Deletion follows semantic dependencies and retains the original producer boundary.
use super::*;
use crate::{error::ApiError, read::PreviewContext};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteTarget {
    Body(BodyId),
    Sketch(SketchId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionItem {
    pub id: String,
    pub name: String,
}

/// Confirmation data is a proposal, never an authority to bypass a current dependency check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionPlan {
    pub expected: PreviewContext,
    pub target: DeleteTarget,
    pub records: Vec<DeletionItem>,
    pub sketches: Vec<DeletionItem>,
    pub bodies: Vec<DeletionItem>,
}

#[derive(Default)]
struct Closure {
    records: BTreeSet<RecordId>,
    sketches: BTreeSet<SketchId>,
    bodies: BTreeSet<BodyId>,
}

impl DocumentRuntime {
    pub fn plan_delete_document_item(
        &self,
        target: DeleteTarget,
        expected: PreviewContext,
    ) -> Result<DeletionPlan, ApiError> {
        self.check_deletion_context(&expected)?;
        let mut closure = self.deletion_seed(target)?;
        self.expand_deletion(&mut closure)?;
        let doc = self.session.document();
        let records = doc
            .timeline
            .records()
            .iter()
            .filter(|record| closure.records.contains(&record.record_id))
            .map(|record| item(record.record_id, &record.name))
            .collect();
        let sketches = doc
            .sketches
            .values()
            .filter(|sketch| closure.sketches.contains(&sketch.id))
            .map(|sketch| item(sketch.id, &sketch.name))
            .collect();
        let bodies = doc
            .bodies
            .bodies()
            .iter()
            .filter(|body| closure.bodies.contains(&body.id))
            .map(|body| item(body.id, &body.name))
            .collect();
        Ok(DeletionPlan {
            expected,
            target,
            records,
            sketches,
            bodies,
        })
    }

    pub fn delete_document_item(
        &mut self,
        plan: &DeletionPlan,
    ) -> Result<CommandOutcome, ApiError> {
        let current = self.plan_delete_document_item(plan.target, plan.expected.clone())?;
        if current != *plan {
            return Err(ApiError::StalePreview(
                "Deletion dependencies changed; review again".into(),
            ));
        }
        let mut commands = Vec::new();
        for record in plan.records.iter().rev() {
            commands.push(EditCommand::RemoveOperation {
                record: record.id.parse().map_err(invalid_id)?,
            });
        }
        for sketch in &plan.sketches {
            commands.push(EditCommand::DeleteSketch {
                sketch: sketch.id.parse().map_err(invalid_id)?,
            });
        }
        for body in &plan.bodies {
            commands.push(EditCommand::DeleteBody {
                body: body.id.parse().map_err(invalid_id)?,
            });
        }
        let mut outcome = self.apply_transaction("Delete document item", commands)?;
        if !plan.records.is_empty() {
            // Removing the tail still changes the worker head. A request starting at
            // the removed index would be an empty plan and retain deleted geometry.
            if let Some(dirty) = &mut outcome.dirty {
                dirty.from = 0;
            }
        }
        Ok(outcome)
    }

    fn check_deletion_context(&self, expected: &PreviewContext) -> Result<(), ApiError> {
        if self.document_id() != expected.document_id
            || !self.matches_runtime_session(&expected.runtime_session)
            || self.revision() != expected.revision
            || self.fencing.get().1 != expected.worker_epoch
            || self.engine.current_epoch() != expected.worker_epoch
            || self.head_snapshot_id().unwrap_or(SnapshotId(0)) != expected.snapshot_id
        {
            return Err(ApiError::StalePreview(
                "Deletion belongs to an obsolete document head".into(),
            ));
        }
        if self.read_only
            || self.sketch_session.is_some()
            || self.pending_sketch_finish.is_some()
            || self.active_gesture.is_some()
            || self.native_gesture.is_some()
            || self.session.transaction_open()
        {
            return Err(ApiError::InvalidCommand(
                "Finish the active sketch or edit before deleting document items".into(),
            ));
        }
        Ok(())
    }

    fn deletion_seed(&self, target: DeleteTarget) -> Result<Closure, ApiError> {
        let doc = self.session.document();
        let mut closure = Closure::default();
        match target {
            DeleteTarget::Sketch(id) => {
                if !doc.sketches.contains_key(&id) {
                    return Err(ApiError::InvalidCommand(format!("sketch {id} not found")));
                }
                closure.sketches.insert(id);
            }
            DeleteTarget::Body(id) => {
                if !doc.bodies.contains(id) {
                    return Err(ApiError::InvalidCommand(format!("body {id} not found")));
                }
                if let Some(record) = doc
                    .timeline
                    .records()
                    .iter()
                    .find(|r| r.outputs.contains(&id))
                {
                    if doc.bodies.get(id).is_some_and(|body| {
                        body.created_by != record.record_id
                            && doc
                                .timeline
                                .records()
                                .iter()
                                .any(|r| r.record_id == body.created_by)
                    }) {
                        return Err(ApiError::InvalidCommand(
                            "Body producer lineage is incomplete; regenerate before deleting"
                                .into(),
                        ));
                    }
                    closure.records.insert(record.record_id);
                } else if self.regen.bodies.contains(id) {
                    return Err(ApiError::InvalidCommand(
                        "Body producer is unresolved; regenerate before deleting".into(),
                    ));
                }
                closure.bodies.insert(id);
            }
        }
        Ok(closure)
    }

    fn expand_deletion(&self, closure: &mut Closure) -> Result<(), ApiError> {
        let doc = self.session.document();
        loop {
            let sizes = (
                closure.records.len(),
                closure.sketches.len(),
                closure.bodies.len(),
            );
            self.expand_deleted_records(closure)?;
            for sketch in doc.sketches.values() {
                let host = sketch.host_face().and_then(|face| face.primary.as_ref());
                if host.is_some_and(|host| closure.bodies.contains(&host.body))
                    || sketch
                        .projections
                        .values()
                        .any(|p| closure.bodies.contains(&p.source_body))
                {
                    closure.sketches.insert(sketch.id);
                }
            }
            let mut seen = BTreeSet::new();
            for record in doc.timeline.records() {
                for body in &record.outputs {
                    if seen.insert(*body) && closure.records.contains(&record.record_id) {
                        closure.bodies.insert(*body);
                    }
                }
            }
            if sizes
                == (
                    closure.records.len(),
                    closure.sketches.len(),
                    closure.bodies.len(),
                )
            {
                break;
            }
        }
        self.reject_opaque_deletion(closure)
    }

    fn expand_deleted_records(&self, closure: &mut Closure) -> Result<(), ApiError> {
        for record in self.session.document().timeline.records() {
            let input = record.op.derive_inputs();
            let produced_sketch = match &record.op {
                Operation::Known(KnownOperation::Sketch(p)) => Some(p.sketch),
                _ => None,
            };
            let referenced_bodies = reference_bodies(&record.op)?;
            if produced_sketch.is_some_and(|id| closure.sketches.contains(&id))
                || input
                    .sketches
                    .iter()
                    .any(|id| closure.sketches.contains(id))
                || input
                    .bodies
                    .iter()
                    .chain(&referenced_bodies)
                    .any(|id| closure.bodies.contains(id))
                || input.records.iter().any(|id| closure.records.contains(id))
                || record.outputs.iter().any(|id| closure.bodies.contains(id))
            {
                closure.records.insert(record.record_id);
            }
            if closure.records.contains(&record.record_id) {
                closure
                    .records
                    .extend(self.session.graph().downstream(record.record_id));
                if let Some(sketch) = produced_sketch {
                    closure.sketches.insert(sketch);
                }
            }
        }
        Ok(())
    }

    fn reject_opaque_deletion(&self, closure: &Closure) -> Result<(), ApiError> {
        let records = self.session.document().timeline.records();
        let first = records
            .iter()
            .position(|r| closure.records.contains(&r.record_id))
            .unwrap_or(0);
        if records[first..]
            .iter()
            .any(|r| matches!(r.op, Operation::Opaque(_)))
        {
            return Err(ApiError::InvalidCommand(
                "Opaque feature lineage cannot be safely deleted".into(),
            ));
        }
        for sketch in self.session.document().sketches.values() {
            if sketch
                .host_face()
                .is_some_and(|face| face.primary.is_none())
            {
                return Err(ApiError::InvalidCommand(
                    "An unresolved sketch support prevents safe dependency deletion".into(),
                ));
            }
        }
        Ok(())
    }

    /// Timeline metadata cleanup is allowed only when the same transaction removes every owner.
    pub(super) fn transaction_body_cleanup(
        &self,
        commands: &[EditCommand],
    ) -> Result<BTreeSet<BodyId>, DomainError> {
        let removed: BTreeSet<_> = commands
            .iter()
            .filter_map(|command| match command {
                EditCommand::RemoveOperation { record } => Some(*record),
                _ => None,
            })
            .collect();
        let mut cleanup = BTreeSet::new();
        let deleted: BTreeSet<_> = commands
            .iter()
            .filter_map(|command| match command {
                EditCommand::DeleteBody { body } => Some(*body),
                _ => None,
            })
            .collect();
        let sketches: BTreeSet<_> = commands
            .iter()
            .filter_map(|command| match command {
                EditCommand::DeleteSketch { sketch } => Some(*sketch),
                _ => None,
            })
            .collect();
        for command in commands {
            let EditCommand::DeleteBody { body } = command else {
                continue;
            };
            if !self.regen.bodies.contains(*body) {
                continue;
            }
            if commands.iter().any(|command| {
                !matches!(
                    command,
                    EditCommand::RemoveOperation { .. }
                        | EditCommand::DeleteSketch { .. }
                        | EditCommand::DeleteBody { .. }
                )
            }) {
                self.reject_timeline_body_delete(command)?;
            }
            self.validate_cleanup_closure(*body, &removed, &sketches, &deleted)?;
            cleanup.insert(*body);
        }
        Ok(cleanup)
    }

    fn validate_cleanup_closure(
        &self,
        body: BodyId,
        removed: &BTreeSet<RecordId>,
        sketches: &BTreeSet<SketchId>,
        deleted: &BTreeSet<BodyId>,
    ) -> Result<(), DomainError> {
        let mut closure = self
            .deletion_seed(DeleteTarget::Body(body))
            .map_err(|error| DomainError::Validation(error.to_string()))?;
        self.expand_deletion(&mut closure)
            .map_err(|error| DomainError::Validation(error.to_string()))?;
        if closure.records.is_empty()
            || !closure.records.is_subset(removed)
            || !closure.sketches.is_subset(sketches)
            || closure.bodies.iter().any(|body| {
                self.session.document().bodies.contains(*body) && !deleted.contains(body)
            })
        {
            return Err(DomainError::Validation(
                "Deletion must include the original producer and all dependent features, sketches and collateral bodies".into()));
        }
        Ok(())
    }
}

fn reference_bodies(operation: &Operation) -> Result<Vec<BodyId>, ApiError> {
    let Operation::Known(mut operation) = operation.clone() else {
        return Ok(Vec::new());
    };
    operation
        .element_refs_mut()
        .into_iter()
        .map(|reference| {
            reference.primary.as_ref().map(|p| p.body).ok_or_else(|| {
                ApiError::InvalidCommand(
                    "Unresolved feature references prevent safe dependency deletion".into(),
                )
            })
        })
        .collect()
}

fn item(id: impl std::fmt::Display, name: &str) -> DeletionItem {
    DeletionItem {
        id: id.to_string(),
        name: name.into(),
    }
}

fn invalid_id(error: impl std::fmt::Display) -> ApiError {
    ApiError::InvalidCommand(format!("Invalid deletion identity: {error}"))
}

pub(super) async fn clear_worker_head(
    engine: &dyn GeometryEngine,
    document_id: DocumentId,
    fencing: &FencingCell,
    cancel: &CancelToken,
    expected: &mut (DocumentRevision, WorkerEpoch),
) -> Result<(), Outcome> {
    if cancel.is_cancelled() {
        return Err(Outcome::Cancelled);
    }
    if fencing.get() != *expected || engine.current_epoch() != expected.1 {
        return Err(Outcome::Superseded);
    }
    let epoch = engine
        .reset(document_id, expected.1)
        .await
        .map_err(Outcome::EngineFailed)?;
    if engine.current_epoch() != epoch
        || fencing
            .epoch
            .compare_exchange(expected.1 .0, epoch.0, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
    {
        return Err(Outcome::Superseded);
    }
    expected.1 = epoch;
    let head = engine
        .open_session(onecad_core::regen::OpenSessionRequest {
            document_id,
            document_revision: expected.0,
            worker_epoch: epoch,
            mode: onecad_core::regen::SessionMode::Determinism,
        })
        .await
        .map_err(Outcome::EngineFailed)?;
    if fencing.get() != *expected || engine.current_epoch() != epoch {
        return Err(Outcome::Superseded);
    }
    if head.worker_epoch != epoch
        || head.document_revision != expected.0
        || head.snapshot_id != SnapshotId(0)
        || head.history_prefix_hash != onecad_core::regen::HistoryPrefixHash::empty()
        || head.has_scratch
        || head.has_restored_base
    {
        return Err(Outcome::EngineFailed(EngineError::Protocol {
            message: "Empty document session returned a nonempty or obsolete worker head".into(),
        }));
    }
    Ok(())
}
