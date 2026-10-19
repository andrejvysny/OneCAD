//! Explicit constraint admission solves detached candidates before authoring.
use super::*;
use crate::dto::SketchConstraintAdmission;
use onecad_core::{expr::Dimension, regen::evaluate_expression, sketch::Constraint};

pub struct PreparedSketchConstraints {
    candidate: Sketch,
    ops: Vec<SketchEditOp>,
    solver: Arc<dyn SolverEngine>,
    instance: Uuid,
    fencing: (DocumentRevision, WorkerEpoch),
    snapshot: Option<SnapshotId>,
    geometry: String,
    session: Option<Uuid>,
    _claim: Claim,
}

pub struct DrivenSketchConstraints {
    prepared: PreparedSketchConstraints,
    solve: SketchUpsertDto,
}

impl PreparedSketchConstraints {
    /// Worker I/O runs outside the runtime lock; the lane remains claimed
    /// until the fenced decision, including refusal, is complete.
    pub async fn drive(self) -> Result<DrivenSketchConstraints, EngineError> {
        let solve = self.solver.sketch_upsert(&self.candidate).await?;
        Ok(DrivenSketchConstraints {
            prepared: self,
            solve,
        })
    }
}

impl DocumentRuntime {
    pub async fn prepare_sketch_constraints(
        &self,
        id: SketchId,
        constraints: Vec<Constraint>,
    ) -> Result<PreparedSketchConstraints, EngineError> {
        if self.read_only {
            return Err(op_failed("Sketch constraints: read-only document"));
        }
        if constraints.is_empty() || constraints.len() > 128 {
            return Err(op_failed(
                "Constraint batch must contain 1 to 128 constraints",
            ));
        }
        let claim = self.claim_upsert_lane(id).await?;
        let ops = constraints
            .into_iter()
            .map(|mut constraint| {
                resolve_dimension(&mut constraint, self.variables_table())?;
                Ok(SketchEditOp::AddConstraint { constraint })
            })
            .collect::<Result<Vec<_>, EngineError>>()?;
        let candidate = self
            .session
            .prepare_sketch_edit(id, &ops)
            .map_err(|error| op_failed(format!("Sketch constraints: {error}")))?;
        let fencing = self.fencing.get();
        if fencing.1 != self.engine.current_epoch() {
            return Err(op_failed(
                "Sketch constraints: document has not adopted the worker epoch",
            ));
        }
        Ok(PreparedSketchConstraints {
            candidate,
            ops,
            solver: self.solver.clone(),
            instance: self.instance,
            fencing,
            snapshot: self.head_snapshot_id(),
            session: self.sketch_edit_token(id),
            geometry: sketch_geometry_token(&self.sketch_or_err(id, "Sketch constraints")?),
            _claim: claim,
        })
    }

    pub fn commit_sketch_constraints(
        &mut self,
        driven: DrivenSketchConstraints,
    ) -> Result<(SketchConstraintAdmission, Option<CommandOutcome>), EngineError> {
        let prepared = driven.prepared;
        if self.read_only
            || self.instance != prepared.instance
            || self.fencing.get() != prepared.fencing
            || self.engine.current_epoch() != prepared.fencing.1
            || self.head_snapshot_id() != prepared.snapshot
            || self.sketch_edit_token(prepared.candidate.id) != prepared.session
            || sketch_geometry_token(
                &self.sketch_or_err(prepared.candidate.id, "Sketch constraints")?,
            ) != prepared.geometry
        {
            return Err(op_failed(
                "Sketch constraints: candidate belongs to a stale document or worker",
            ));
        }
        let after = self.adopt_candidate_solve(&prepared.candidate, &driven.solve)?;
        let accepted = matches!(
            driven.solve.status,
            SketchSolveStatus::UnderConstrained | SketchSolveStatus::FullyConstrained
        );
        let outcome = if accepted {
            let id = prepared.candidate.id;
            let outcome = self.commit_sketch_solve(prepared.candidate, after, prepared.ops)?;
            self.record_upsert(id, &driven.solve);
            outcome
        } else {
            None
        };
        Ok((
            SketchConstraintAdmission {
                accepted,
                solve: driven.solve,
            },
            outcome,
        ))
    }

    pub async fn add_sketch_constraints_with_outcome(
        &mut self,
        id: SketchId,
        constraints: Vec<Constraint>,
    ) -> Result<(SketchConstraintAdmission, Option<CommandOutcome>), EngineError> {
        let driven = self
            .prepare_sketch_constraints(id, constraints)
            .await?
            .drive()
            .await?;
        self.commit_sketch_constraints(driven)
    }
}

fn resolve_dimension(
    constraint: &mut Constraint,
    variables: &VariableTable,
) -> Result<(), EngineError> {
    let site = if matches!(constraint, Constraint::Angle { .. }) {
        Dimension::Angle
    } else {
        Dimension::Length
    };
    let value = match constraint {
        Constraint::Distance { value, .. }
        | Constraint::HorizontalDistance { value, .. }
        | Constraint::VerticalDistance { value, .. }
        | Constraint::Angle { value, .. }
        | Constraint::Radius { value, .. }
        | Constraint::Diameter { value, .. } => value,
        _ => return Ok(()),
    };
    if let Some(expression) = &value.expr {
        value.value = evaluate_expression(expression, site, variables)
            .map_err(|(_, message)| op_failed(format!("Sketch dimension: {message}")))?
            .number;
    }
    Ok(())
}
