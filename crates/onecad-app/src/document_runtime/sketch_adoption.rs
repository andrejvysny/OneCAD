//! Single-writer adoption for the three authoritative non-drag upsert sites.

use super::*;

pub(super) struct CachedSketchSolve {
    pub(super) dof: u32,
    pub(super) status: SketchSolveStatus,
    pub(super) token: String,
    pub(super) conflicting: Vec<String>,
    pub(super) truth: crate::dto::SketchTruthDto,
}

impl DocumentRuntime {
    pub(super) async fn claim_upsert_lane(&self, sketch: SketchId) -> Result<Claim, EngineError> {
        if self.active_gesture.is_some() {
            return Err(op_failed("SketchUpsert: a drag gesture is active"));
        }
        let tokens = self.fencing.get();
        let epoch = self.engine.current_epoch();
        let claim = self
            .solver_lane
            .claim_upsert(sketch)
            .await
            .map_err(|held| {
                op_failed(format!(
                    "SketchUpsert: {} is in flight for sketch {sketch} — retry",
                    held.describe()
                ))
            })?;
        if self.fencing.get() != tokens || self.engine.current_epoch() != epoch {
            return Err(op_failed(
                "SketchUpsert: document or worker changed while waiting for region queries",
            ));
        }
        Ok(claim)
    }

    pub(super) async fn solve_sketch_candidate(
        &self,
        candidate: &Sketch,
    ) -> Result<(SketchUpsertDto, Sketch), EngineError> {
        let tokens = self.fencing.get();
        let epoch = self.engine.current_epoch();
        if tokens.1 != epoch {
            return Err(op_failed(
                "SketchUpsert: document has not adopted the worker epoch",
            ));
        }
        let current = self.sketch_or_err(candidate.id, "SketchUpsert")?;
        let geometry = sketch_geometry_token(&current);
        let solved = self.solver.sketch_upsert(candidate).await?;
        if self.fencing.get() != tokens
            || self.engine.current_epoch() != epoch
            || sketch_geometry_token(&self.sketch_or_err(candidate.id, "SketchUpsert")?) != geometry
        {
            return Err(op_failed(
                "SketchUpsert: solve belongs to a stale document or worker",
            ));
        }
        if solved.sketch_revision == 0
            || self.sketch_upsert_revisions.get(&candidate.id).is_some_and(
                |(prior_epoch, revision)| {
                    *prior_epoch == epoch && solved.sketch_revision <= *revision
                },
            )
        {
            return Err(op_failed("SketchUpsert: stale sketch revision"));
        }
        let after = crate::worker::sketch_result::adopted(candidate, &solved).map_err(op_failed)?;
        Ok((solved, after))
    }

    pub(super) fn commit_sketch_solve(
        &mut self,
        candidate: Sketch,
        after: Sketch,
        ops: Vec<SketchEditOp>,
    ) -> Result<Option<CommandOutcome>, EngineError> {
        let id = candidate.id;
        let label = if ops.is_empty() {
            "Resolve sketch"
        } else {
            "Edit Sketch"
        };
        let mut commands = Vec::new();
        if !ops.is_empty() {
            commands.push(EditCommand::SketchEdit { sketch: id, ops });
        }
        if after != candidate {
            commands.push(EditCommand::SketchDragGesture {
                sketch: id,
                before: candidate,
                after,
            });
        }
        if commands.is_empty() {
            return Ok(None);
        }
        self.apply_transaction(label, commands)
            .map(Some)
            .map_err(|e| op_failed(format!("SketchUpsert commit: {e}")))
    }

    pub(super) fn record_upsert(&mut self, sketch: SketchId, solved: &SketchUpsertDto) {
        self.sketch_upsert_revisions.insert(
            sketch,
            (self.engine.current_epoch(), solved.sketch_revision),
        );
        self.record_solve(sketch, solved);
    }
}

#[cfg(test)]
mod tests;

// End deltas are relative to the worker's solved Begin baseline. Publish the
// cumulative correction so consumers using the original document get final values.
pub(super) fn merge_pending_solve(final_solve: &mut SketchUpsertDto, mut pending: SketchUpsertDto) {
    pending
        .solved_positions
        .extend(std::mem::take(&mut final_solve.solved_positions));
    final_solve.solved_positions = pending.solved_positions;
    for (id, final_curve) in std::mem::take(&mut final_solve.solved_curves) {
        let curve = pending.solved_curves.entry(id).or_default();
        if final_curve.radius.is_some() {
            curve.radius = final_curve.radius;
        }
        if final_curve.start_angle.is_some() {
            curve.start_angle = final_curve.start_angle;
        }
        if final_curve.end_angle.is_some() {
            curve.end_angle = final_curve.end_angle;
        }
    }
    final_solve.solved_curves = pending.solved_curves;
}
