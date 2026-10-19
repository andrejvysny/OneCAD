//! Detached, visit-scoped native gestures. Legacy gesture callers remain separate.
use super::*;
use crate::dto::{NativeGestureScope, NativeGestureSource};
use crate::worker::NativeGestureWorker;

pub(crate) mod control;
pub(crate) mod driver;
mod validation;

pub(crate) struct PreparedNativeGesture {
    pub scope: NativeGestureScope,
    pub before: Sketch,
    pub geometry: String,
    pub worker: Arc<dyn NativeGestureWorker>,
    pub claim: Option<Claim>,
}

pub(crate) struct NativeGestureOwner {
    pub id: u64,
    pub sketch: SketchId,
    pub control: Arc<control::Control>,
}

impl Drop for NativeGestureOwner {
    fn drop(&mut self) {
        self.control.cancel();
    }
}

impl DocumentRuntime {
    /// Captures authority for a caller that already owns the current edit visit.
    /// Native hosts must compare this scope with their accepted session before admission.
    pub fn native_gesture_scope(
        &self,
        sketch_id: SketchId,
        source: NativeGestureSource,
    ) -> Result<NativeGestureScope, EngineError> {
        Ok(NativeGestureScope {
            document_id: self.document_uuid(),
            runtime_session: self.runtime_session(),
            sketch_id,
            edit_visit: self
                .sketch_edit_token(sketch_id)
                .ok_or_else(|| op_failed("Native gesture requires a current sketch edit visit"))?,
            document_revision: self.revision(),
            worker_epoch: self.engine.current_epoch(),
            snapshot: self.head_snapshot_id(),
            source,
            expected_base_hash: self.native_gesture_base_hash(),
        })
    }

    fn native_gesture_base_hash(&self) -> onecad_core::regen::HistoryPrefixHash {
        let count = self
            .latest_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.step_index)
            .map_or(0, |step| step.saturating_add(1));
        onecad_core::regen::history_prefix_hash(
            &self.regen.timeline.records()[..count.min(self.regen.timeline.records().len())],
        )
    }

    pub(crate) fn prepare_native_gesture(
        &self,
        scope: NativeGestureScope,
        target: wire::GestureTarget,
    ) -> Result<PreparedNativeGesture, EngineError> {
        self.validate_native_scope(&scope)?;
        if self.active_gesture.is_some() || self.native_gesture.is_some() {
            return Err(op_failed(
                "Native gesture: another gesture owns the document",
            ));
        }
        let before = self.sketch_or_err(scope.sketch_id, "Native gesture")?;
        validation::target(&before, target)?;
        let claim = self
            .solver_lane
            .claim(scope.sketch_id, LaneOwner::Gesture)
            .map_err(|held| {
                op_failed(format!("Native gesture: {} is in flight", held.describe()))
            })?;
        let worker = self.solver.bind_native_gesture()?;
        if !worker.is_current() {
            return Err(op_failed("Native gesture: worker lease is stale"));
        }
        Ok(PreparedNativeGesture {
            geometry: sketch_geometry_token(&before),
            before,
            scope,
            worker,
            claim: Some(claim),
        })
    }

    pub(crate) fn validate_native_scope(
        &self,
        scope: &NativeGestureScope,
    ) -> Result<(), EngineError> {
        if self.read_only {
            return Err(op_failed("Native gesture: document is read-only"));
        }
        same_axis("document", scope.document_id, self.document_uuid())?;
        same_axis("runtime", &scope.runtime_session, &self.runtime_session())?;
        same_axis(
            "edit visit",
            Some(scope.edit_visit),
            self.sketch_edit_token(scope.sketch_id),
        )?;
        same_axis("revision", scope.document_revision, self.revision())?;
        same_axis(
            "adopted worker epoch",
            scope.worker_epoch,
            self.fencing.get().1,
        )?;
        same_axis(
            "worker epoch",
            scope.worker_epoch,
            self.engine.current_epoch(),
        )?;
        // PreviewContext represents the unpublished head as snapshot 0; the runtime stores None.
        same_axis(
            "snapshot",
            scope.snapshot.unwrap_or(SnapshotId(0)),
            self.head_snapshot_id().unwrap_or(SnapshotId(0)),
        )?;
        Ok(())
    }

    pub(crate) fn validate_native_gesture(
        &self,
        prepared: &PreparedNativeGesture,
        id: u64,
    ) -> Result<(), EngineError> {
        self.validate_native_scope(&prepared.scope)?;
        if !prepared.worker.is_current()
            || !self
                .native_gesture
                .as_ref()
                .is_some_and(|owner| owner.id == id)
            || sketch_geometry_token(
                &self.sketch_or_err(prepared.scope.sketch_id, "Native gesture")?,
            ) != prepared.geometry
        {
            return Err(op_failed("Native gesture: stale owner or geometry"));
        }
        Ok(())
    }

    pub(crate) fn commit_native_gesture(
        &mut self,
        prepared: &PreparedNativeGesture,
        id: u64,
        baseline: &Sketch,
        mut solved: SketchUpsertDto,
        pending: SketchUpsertDto,
    ) -> Result<(SketchUpsertDto, Option<CommandOutcome>), EngineError> {
        self.validate_native_gesture(prepared, id)?;
        let after = self.adopt_candidate_solve(baseline, &solved)?;
        sketch_adoption::merge_pending_solve(&mut solved, pending);
        self.native_gesture.take();
        let outcome = if after == prepared.before {
            None
        } else {
            Some(
                self.apply(EditCommand::SketchDragGesture {
                    sketch: prepared.scope.sketch_id,
                    before: prepared.before.clone(),
                    after,
                })
                .map_err(|error| op_failed(format!("Native gesture commit: {error}")))?,
            )
        };
        self.record_upsert(prepared.scope.sketch_id, &solved);
        Ok((solved, outcome))
    }

    /// Releases an End whose final intent equals Begin. Like Cancel, it adopts no
    /// re-solve correction and records no worker revision; the worker store keeps a
    /// solved cache of the unchanged accepted sketch.
    pub(crate) fn release_unchanged_native_gesture(
        &mut self,
        prepared: &PreparedNativeGesture,
        id: u64,
        mut pending: SketchUpsertDto,
    ) -> Result<SketchUpsertDto, EngineError> {
        self.validate_native_gesture(prepared, id)?;
        self.native_gesture.take();
        pending.solved_positions.clear();
        pending.solved_curves.clear();
        Ok(pending)
    }
}

#[cfg(test)]
mod tests;

fn same_axis<T: std::fmt::Debug + PartialEq>(
    axis: &str,
    expected: T,
    current: T,
) -> Result<(), EngineError> {
    if expected != current {
        return Err(op_failed(format!(
            "Native gesture: stale {axis} (expected {expected:?}, current {current:?})"
        )));
    }
    Ok(())
}
