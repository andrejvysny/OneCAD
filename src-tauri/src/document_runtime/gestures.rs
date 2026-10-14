use super::*;

impl DocumentRuntime {
    /// Opens a drag gesture on `drag_point` (SCHEMA §7.4 `BeginGesture`). Snapshots
    /// the pre-gesture sketch (the `before` memento) so pointer-up can commit **one**
    /// undo command for the whole drag.
    ///
    /// `target` declares WHAT the pointer grabbed (point handle / arc endpoint /
    /// radius / whole entity body). [`GestureTarget::point`](wire::GestureTarget::point)
    /// is the plain point drag every pre-SP-2 caller means. `drag_point` stays the
    /// gesture's `SolveDrag` handle for all kinds — the worker ignores `pointId`
    /// off the point path, and the runtime needs a stable per-gesture id anyway.
    ///
    /// # Errors
    /// [`EngineError`] on a read-only document, an unknown sketch, or a worker failure.
    pub async fn begin_gesture(
        &mut self,
        sketch_id: SketchId,
        drag_point: EntityId,
        target: wire::GestureTarget,
    ) -> Result<crate::dto::BeginGestureDto, EngineError> {
        if self.read_only {
            return Err(op_failed("beginGesture: read-only document"));
        }
        if self.active_gesture.is_some() {
            self.cancel_gesture().await?;
        }
        let sketch = self.sketch_or_err(sketch_id, "beginGesture")?;
        let claim = self.claim_gesture_lane(sketch_id)?;
        let fence = self.fencing.get();
        let geometry = sketch_geometry_token(&sketch);
        let (solved, baseline) = self.solve_sketch_candidate(&sketch).await?;
        let (ready, baseline, solved) = self
            .open_worker_gesture(&sketch, (baseline, solved), drag_point, target, fence)
            .await?;
        let gesture_id = ready.gesture_id;
        if baseline == sketch {
            self.record_solve(sketch_id, &solved);
        } else {
            self.sketch_solve.remove(&sketch_id);
        }
        self.active_gesture = Some(ActiveGesture {
            gesture_id,
            sketch_id,
            drag_point,
            before: sketch,
            baseline,
            pending_solve: solved,
            fence,
            geometry,
            next_seq: 1,
            _claim: claim,
        });
        Ok(ready)
    }

    async fn open_worker_gesture(
        &mut self,
        sketch: &Sketch,
        pending: (Sketch, SketchUpsertDto),
        drag_point: EntityId,
        target: wire::GestureTarget,
        fence: (DocumentRevision, WorkerEpoch),
    ) -> Result<(crate::dto::BeginGestureDto, Sketch, SketchUpsertDto), EngineError> {
        let (baseline, solved) = pending;
        let gesture_id = self.next_gesture_id();
        let receipt = self
            .solver
            .begin_gesture(
                &sketch.id.to_string(),
                solved.sketch_revision,
                gesture_id,
                drag_point,
                target,
                // solverPolicyHash: reserved, always "" — see wire::begin_gesture_args.
                "",
            )
            .await;
        let prepared = receipt.and_then(|mut ready| {
            let (baseline, pending) = self
                .prepare_begin_receipt(sketch, baseline, solved, &mut ready, gesture_id, fence)?;
            Ok((ready, baseline, pending))
        });
        match prepared {
            Ok(prepared) => Ok(prepared),
            Err(error) => {
                self.sketch_solve.remove(&sketch.id);
                let _ = self.solver.cancel_gesture(gesture_id).await;
                Err(error)
            }
        }
    }

    /// Takes the exclusive drag lane after cancelling any previous gesture.
    fn claim_gesture_lane(&mut self, sketch_id: SketchId) -> Result<Claim, EngineError> {
        self.solver_lane
            .claim(sketch_id, LaneOwner::Gesture)
            .map_err(|held| {
                op_failed(format!(
                    "beginGesture: {} is in flight for sketch {sketch_id} — retry",
                    held.describe()
                ))
            })
    }

    /// One incremental drag solve (SCHEMA §7.4 `SolveDrag`). Fired latest-wins: the
    /// caller sends the newest `target` without awaiting each serially. The returned
    /// positions are a **preview** (not committed) — only [`end_gesture`](Self::end_gesture)
    /// mutates the document.
    ///
    /// # Errors
    /// [`EngineError`] when no gesture is active or the worker fails.
    pub async fn solve_drag(
        &mut self,
        target: [f64; 2],
    ) -> Result<crate::dto::DragSolveDto, EngineError> {
        let (gesture_id, drag_point, seq) = {
            let g = self
                .active_gesture
                .as_mut()
                .ok_or_else(|| op_failed("solveDrag: no active gesture"))?;
            let seq = g.next_seq;
            g.next_seq += 1;
            (g.gesture_id, g.drag_point, seq)
        };
        let result = self
            .solver
            .solve_drag(gesture_id, seq, drag_point, target)
            .await;
        let result = result.and_then(|solved| {
            let gesture = self
                .active_gesture
                .as_ref()
                .ok_or_else(|| op_failed("solveDrag: gesture ended"))?;
            self.validate_gesture_fence(gesture)?;
            if !solved.superseded && (solved.gesture_id != gesture_id || solved.seq != seq) {
                return Err(op_failed("solveDrag: stale gesture or sequence receipt"));
            }
            Ok(solved)
        });
        if result.is_err() {
            let _ = self.cancel_gesture().await;
        }
        result
    }

    /// Pointer-up final exact solve (SCHEMA §7.4 `EndGesture`): applies the solved
    /// positions to the `before` memento and commits **one** [`EditCommand::SketchDragGesture`]
    /// (single undo step for the whole drag). Returns the final dof/status/positions.
    ///
    /// # Errors
    /// [`EngineError`] when no gesture is active, the commit is invalid, or the
    /// worker fails.
    pub async fn end_gesture(
        &mut self,
        final_target: Option<[f64; 2]>,
    ) -> Result<SketchUpsertDto, EngineError> {
        self.end_gesture_with_outcome(final_target)
            .await
            .map(|(solved, _)| solved)
    }

    /// [`end_gesture`](Self::end_gesture) plus the exact edit outcome the app
    /// scheduler must consume.
    pub async fn end_gesture_with_outcome(
        &mut self,
        final_target: Option<[f64; 2]>,
    ) -> Result<(SketchUpsertDto, CommandOutcome), EngineError> {
        let gesture = self
            .active_gesture
            .take()
            .ok_or_else(|| op_failed("endGesture: no active gesture"))?;
        self.sketch_solve.remove(&gesture.sketch_id);
        self.validate_gesture_fence(&gesture)?;
        let result = self
            .solver
            .end_gesture(
                &gesture.sketch_id.to_string(),
                gesture.gesture_id,
                final_target,
            )
            .await;
        let solved = match result {
            Ok(solved) => solved,
            Err(error) => {
                let _ = self.solver.cancel_gesture(gesture.gesture_id).await;
                return Err(error);
            }
        };
        self.commit_gesture_result(gesture, solved)
    }

    fn commit_gesture_result(
        &mut self,
        gesture: ActiveGesture,
        mut solved: SketchUpsertDto,
    ) -> Result<(SketchUpsertDto, CommandOutcome), EngineError> {
        self.validate_gesture_fence(&gesture)?;
        if solved.sketch_id != gesture.sketch_id.to_string() {
            return Err(op_failed("endGesture: response belongs to another sketch"));
        }
        crate::worker::sketch_truth::validate(&gesture.baseline, &solved).map_err(op_failed)?;
        let mut after = gesture.baseline.clone();
        after.apply_solved_positions(&typed_positions(&solved.solved_positions));
        after.apply_solved_curves(&typed_curves(&solved.solved_curves));
        sketch_adoption::merge_pending_solve(&mut solved, gesture.pending_solve);
        let outcome = self
            .apply(EditCommand::SketchDragGesture {
                sketch: gesture.sketch_id,
                before: gesture.before,
                after,
            })
            .map_err(|e| op_failed(format!("endGesture commit: {e}")))?;
        self.record_solve(gesture.sketch_id, &solved);
        Ok((solved, outcome))
    }

    fn prepare_begin_receipt(
        &self,
        before: &Sketch,
        baseline: Sketch,
        pending: SketchUpsertDto,
        ready: &mut crate::dto::BeginGestureDto,
        gesture_id: u64,
        fence: (DocumentRevision, WorkerEpoch),
    ) -> Result<(Sketch, SketchUpsertDto), EngineError> {
        if self.fencing.get() != fence
            || self.engine.current_epoch() != fence.1
            || sketch_geometry_token(&self.sketch_or_err(before.id, "beginGesture")?)
                != sketch_geometry_token(before)
            || ready.gesture_id != gesture_id
            || !ready.ready
            || ready
                .point_offset
                .is_some_and(|offset| offset.iter().any(|n| !n.is_finite()))
        {
            return Err(op_failed("beginGesture: stale or invalid gesture receipt"));
        }
        let mut correction = pending.clone();
        correction.solved_positions.clone_from(&ready.positions);
        correction.solved_curves.clone_from(&ready.curves);
        let baseline =
            crate::worker::sketch_result::adopted(&baseline, &correction).map_err(op_failed)?;
        sketch_adoption::merge_pending_solve(&mut correction, pending);
        ready.positions.clone_from(&correction.solved_positions);
        ready.curves.clone_from(&correction.solved_curves);
        Ok((baseline, correction))
    }

    fn validate_gesture_fence(&self, gesture: &ActiveGesture) -> Result<(), EngineError> {
        if self.fencing.get() != gesture.fence
            || self.engine.current_epoch() != gesture.fence.1
            || sketch_geometry_token(&self.sketch_or_err(gesture.sketch_id, "gesture")?)
                != gesture.geometry
        {
            return Err(op_failed(
                "Gesture response belongs to a stale document or worker",
            ));
        }
        Ok(())
    }

    /// Discard scratch, pending Begin corrections and its exclusive lane claim.
    /// No edit, solve event or document projection is produced.
    pub async fn cancel_gesture(&mut self) -> Result<(), EngineError> {
        let Some(gesture) = self.active_gesture.take() else {
            return Ok(());
        };
        self.sketch_solve.remove(&gesture.sketch_id);
        self.validate_gesture_fence(&gesture)?;
        self.solver.cancel_gesture(gesture.gesture_id).await?;
        self.validate_gesture_fence(&gesture)
    }
}
