//! Explicit constraint admission solves detached candidates before authoring.
use super::*;
use crate::dto::SketchConstraintAdmission;
use onecad_core::{
    document::variables::Scalar,
    expr::Dimension,
    ids::ConstraintId,
    regen::evaluate_expression,
    sketch::{Constraint, SketchEntity},
};

pub struct PreparedSketchConstraints {
    control: Option<crate::sketch_candidate::SketchCandidateControl>,
    candidate: Sketch,
    ops: Vec<SketchEditOp>,
    solver: Arc<dyn SolverEngine>,
    instance: Uuid,
    fencing: (DocumentRevision, WorkerEpoch),
    snapshot: Option<SnapshotId>,
    geometry: String,
    session: Option<Uuid>,
    policy: AdmissionPolicy,
    _claim: Claim,
}

#[derive(Clone, Copy)]
enum AdmissionPolicy {
    Strict,
    Primitive,
    Repair,
}

impl AdmissionPolicy {
    fn allows(self, op: &SketchEditOp) -> bool {
        match self {
            Self::Strict => matches!(
                op,
                SketchEditOp::AddConstraint { .. } | SketchEditOp::SetDimension { .. }
            ),
            Self::Primitive => matches!(
                op,
                SketchEditOp::AddEntity { .. } | SketchEditOp::AddConstraint { .. }
            ),
            Self::Repair => matches!(
                op,
                SketchEditOp::RemoveConstraint { .. }
                    | SketchEditOp::RemoveEntity { .. }
                    | SketchEditOp::SetEntityConstruction { .. }
            ),
        }
    }
}

pub struct DrivenSketchConstraints {
    prepared: PreparedSketchConstraints,
    solve: SketchUpsertDto,
}

impl PreparedSketchConstraints {
    pub fn with_control(
        mut self,
        control: crate::sketch_candidate::SketchCandidateControl,
    ) -> Self {
        self.control = Some(control);
        self
    }
    /// Worker I/O runs outside the runtime lock; the lane remains claimed
    /// until the fenced decision, including refusal, is complete.
    pub async fn drive(self) -> Result<DrivenSketchConstraints, EngineError> {
        if self
            .control
            .as_ref()
            .is_some_and(|control| control.cancelled())
        {
            return Err(op_failed("Sketch candidate cancelled before admission"));
        }
        let solve = self.solver.sketch_upsert(&self.candidate).await?;
        Ok(DrivenSketchConstraints {
            prepared: self,
            solve,
        })
    }
}

impl DocumentRuntime {
    pub async fn prepare_sketch_primitive(
        &self,
        id: SketchId,
        mut ops: Vec<SketchEditOp>,
    ) -> Result<PreparedSketchConstraints, EngineError> {
        validate_primitive_ops(&self.sketch_or_err(id, "Sketch primitive")?, &ops)?;
        for op in &mut ops {
            if let SketchEditOp::AddConstraint { constraint } = op {
                resolve_dimension(constraint, self.variables_table())?;
            }
        }
        self.prepare_constraint_ops(id, ops, AdmissionPolicy::Primitive)
            .await
    }

    pub async fn prepare_sketch_constraints(
        &self,
        id: SketchId,
        constraints: Vec<Constraint>,
    ) -> Result<PreparedSketchConstraints, EngineError> {
        if constraints.is_empty() || constraints.len() > 128 {
            return Err(op_failed(
                "Constraint batch must contain 1 to 128 constraints",
            ));
        }
        let ops = constraints
            .into_iter()
            .map(|mut constraint| {
                resolve_dimension(&mut constraint, self.variables_table())?;
                Ok(SketchEditOp::AddConstraint { constraint })
            })
            .collect::<Result<Vec<_>, EngineError>>()?;
        self.prepare_constraint_ops(id, ops, AdmissionPolicy::Strict)
            .await
    }

    pub async fn prepare_sketch_dimension(
        &self,
        id: SketchId,
        constraint_id: ConstraintId,
        mut value: Scalar,
    ) -> Result<PreparedSketchConstraints, EngineError> {
        let sketch = self.sketch_or_err(id, "Sketch dimension")?;
        let constraint = sketch
            .get_constraint(constraint_id)
            .filter(|constraint| constraint.value().is_some())
            .ok_or_else(|| op_failed("Sketch dimension: dimensional constraint not found"))?;
        resolve_value(
            &mut value,
            dimension_site(constraint),
            self.variables_table(),
        )?;
        self.prepare_constraint_ops(
            id,
            vec![SketchEditOp::SetDimension {
                constraint: constraint_id,
                value,
            }],
            AdmissionPolicy::Strict,
        )
        .await
    }

    pub async fn prepare_sketch_repair(
        &self,
        id: SketchId,
        ops: Vec<SketchEditOp>,
    ) -> Result<PreparedSketchConstraints, EngineError> {
        validate_repair_ops(&self.sketch_or_err(id, "Sketch repair")?, &ops)?;
        self.prepare_constraint_ops(id, ops, AdmissionPolicy::Repair)
            .await
    }

    async fn prepare_constraint_ops(
        &self,
        id: SketchId,
        mut ops: Vec<SketchEditOp>,
        policy: AdmissionPolicy,
    ) -> Result<PreparedSketchConstraints, EngineError> {
        if self.read_only {
            return Err(op_failed("Sketch constraints: read-only document"));
        }
        if !ops.iter().all(|op| policy.allows(op)) {
            return Err(op_failed(
                "Sketch constraints: unsupported candidate operation",
            ));
        }
        let claim = self.claim_upsert_lane(id).await?;
        let before = self.sketch_or_err(id, "Sketch constraints")?;
        let candidate = self
            .session
            .prepare_sketch_edit(id, &ops)
            .map_err(|error| op_failed(format!("Sketch constraints: {error}")))?;
        if candidate == before {
            ops.clear();
        }
        let fencing = self.fencing.get();
        if fencing.1 != self.engine.current_epoch() {
            return Err(op_failed(
                "Sketch constraints: document has not adopted the worker epoch",
            ));
        }
        Ok(PreparedSketchConstraints {
            control: None,
            candidate,
            ops,
            solver: self.solver.clone(),
            instance: self.instance,
            fencing,
            snapshot: self.head_snapshot_id(),
            session: self.sketch_edit_token(id),
            policy,
            geometry: sketch_geometry_token(&before),
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
        let accepted = match driven.solve.status {
            SketchSolveStatus::UnderConstrained | SketchSolveStatus::FullyConstrained => true,
            SketchSolveStatus::Conflicting | SketchSolveStatus::OverConstrained => {
                matches!(prepared.policy, AdmissionPolicy::Repair)
            }
        };
        let outcome = if accepted {
            if prepared
                .control
                .as_ref()
                .is_some_and(|control| !control.admit())
            {
                return Err(op_failed("Sketch candidate cancelled before admission"));
            }
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

    pub async fn set_sketch_dimension_with_outcome(
        &mut self,
        id: SketchId,
        constraint: ConstraintId,
        value: Scalar,
    ) -> Result<(SketchConstraintAdmission, Option<CommandOutcome>), EngineError> {
        let driven = self
            .prepare_sketch_dimension(id, constraint, value)
            .await?
            .drive()
            .await?;
        self.commit_sketch_constraints(driven)
    }

    pub async fn repair_sketch_with_outcome(
        &mut self,
        id: SketchId,
        ops: Vec<SketchEditOp>,
    ) -> Result<(SketchConstraintAdmission, Option<CommandOutcome>), EngineError> {
        let driven = self.prepare_sketch_repair(id, ops).await?.drive().await?;
        self.commit_sketch_constraints(driven)
    }

    pub async fn author_sketch_primitive_with_outcome(
        &mut self,
        id: SketchId,
        ops: Vec<SketchEditOp>,
    ) -> Result<(SketchConstraintAdmission, Option<CommandOutcome>), EngineError> {
        let driven = self
            .prepare_sketch_primitive(id, ops)
            .await?
            .drive()
            .await?;
        self.commit_sketch_constraints(driven)
    }
}

fn validate_primitive_ops(sketch: &Sketch, ops: &[SketchEditOp]) -> Result<(), EngineError> {
    if ops.is_empty() || ops.len() > 128 {
        return Err(op_failed(
            "Sketch primitive must contain 1 to 128 operations",
        ));
    }
    let mut entities = BTreeSet::new();
    let mut constraints = BTreeSet::new();
    for op in ops {
        match op {
            SketchEditOp::AddEntity { entity } => {
                if sketch.contains_entity(entity.id())
                    || !entities.insert(entity.id())
                    || entity.is_reference_locked()
                {
                    return Err(op_failed(
                        "Sketch primitive requires fresh unique unlocked entities",
                    ));
                }
                if !finite_primitive_entity(entity) {
                    return Err(op_failed("Sketch primitive geometry must be finite"));
                }
            }
            SketchEditOp::AddConstraint { constraint } => {
                if sketch.get_constraint(constraint.id()).is_some()
                    || !constraints.insert(constraint.id())
                {
                    return Err(op_failed(
                        "Sketch primitive requires fresh unique constraints",
                    ));
                }
            }
            _ => {
                return Err(op_failed(
                    "Sketch primitive: unsupported candidate operation",
                ))
            }
        }
    }
    if entities.is_empty() {
        return Err(op_failed(
            "Sketch primitive requires at least one fresh entity",
        ));
    }
    Ok(())
}

fn finite_primitive_entity(entity: &SketchEntity) -> bool {
    match entity {
        SketchEntity::Point { at, .. } => at.is_finite(),
        SketchEntity::Line { .. } => true,
        SketchEntity::Circle { radius, .. } => radius.is_finite(),
        SketchEntity::Arc {
            radius,
            start_angle,
            end_angle,
            ..
        } => radius.is_finite() && start_angle.is_finite() && end_angle.is_finite(),
        SketchEntity::Ellipse {
            major_r,
            minor_r,
            rotation,
            ..
        } => major_r.is_finite() && minor_r.is_finite() && rotation.is_finite(),
    }
}

fn validate_repair_ops(sketch: &Sketch, ops: &[SketchEditOp]) -> Result<(), EngineError> {
    if ops.is_empty() || ops.len() > 128 {
        return Err(op_failed("Sketch repair must contain 1 to 128 operations"));
    }
    for op in ops {
        let exists = match op {
            SketchEditOp::RemoveConstraint { constraint } => {
                sketch.get_constraint(*constraint).is_some()
            }
            SketchEditOp::RemoveEntity { entity }
            | SketchEditOp::SetEntityConstruction { entity, .. } => sketch.contains_entity(*entity),
            _ => return Err(op_failed("Sketch repair: unsupported candidate operation")),
        };
        if !exists {
            return Err(op_failed("Sketch repair: target not found"));
        }
    }
    Ok(())
}

fn resolve_dimension(
    constraint: &mut Constraint,
    variables: &VariableTable,
) -> Result<(), EngineError> {
    if matches!(constraint, Constraint::Fixed { at, .. } if !at.is_finite()) {
        return Err(op_failed("Sketch constraint position must be finite"));
    }
    let site = dimension_site(constraint);
    let value = match constraint {
        Constraint::Distance { value, .. }
        | Constraint::HorizontalDistance { value, .. }
        | Constraint::VerticalDistance { value, .. }
        | Constraint::Angle { value, .. }
        | Constraint::Radius { value, .. }
        | Constraint::Diameter { value, .. } => value,
        _ => return Ok(()),
    };
    resolve_value(value, site, variables)
}

fn dimension_site(constraint: &Constraint) -> Dimension {
    if matches!(constraint, Constraint::Angle { .. }) {
        Dimension::Angle
    } else {
        Dimension::Length
    }
}

fn resolve_value(
    value: &mut Scalar,
    site: Dimension,
    variables: &VariableTable,
) -> Result<(), EngineError> {
    if let Some(expression) = &value.expr {
        let resolved = evaluate_expression(expression, site, variables)
            .map_err(|(_, message)| op_failed(format!("Sketch dimension: {message}")))?
            .number;
        // Expressions use degrees; sketch constraint values use radians.
        value.value = if site == Dimension::Angle {
            resolved.to_radians()
        } else {
            resolved
        };
    }
    if !value.value.is_finite() {
        return Err(op_failed("Sketch dimension: value must be finite"));
    }
    Ok(())
}
