use super::{Constraint, CurvePosition, Sketch, SketchEntity, SketchError};
use crate::ids::EntityId;

impl Sketch {
    /// Checks endpoint-tangency roles and structural welds, including loaded sketches.
    /// Coordinates alone never establish a weld.
    pub fn validate_endpoint_tangencies(&self) -> Result<(), SketchError> {
        for constraint in self.constraints() {
            self.validate_endpoint_tangency(constraint)?;
        }
        Ok(())
    }

    /// Appends an atomic batch, validating welds against the complete candidate.
    /// Constraint order is preserved, including a tangent preceding its weld.
    pub fn add_constraints(&mut self, constraints: Vec<Constraint>) -> Result<(), SketchError> {
        let mut candidate = self.clone();
        for constraint in constraints {
            let id = constraint.id();
            if candidate.contains_constraint(id) {
                return Err(SketchError::DuplicateConstraint(id));
            }
            for missing in constraint.entities() {
                if !candidate.contains_entity(missing) {
                    return Err(SketchError::DanglingConstraintRef {
                        constraint: id,
                        missing,
                    });
                }
            }
            candidate
                .constraint_index
                .insert(id, candidate.constraints.len());
            candidate.constraints.push(constraint);
        }
        candidate.validate_endpoint_tangencies()?;
        *self = candidate;
        Ok(())
    }

    pub(super) fn validate_endpoint_tangency(
        &self,
        constraint: &Constraint,
    ) -> Result<(), SketchError> {
        let Constraint::TangentAtPoint {
            id,
            entity1,
            entity2,
            entity1_position,
            entity2_position,
        } = *constraint
        else {
            return Ok(());
        };
        let fail = |reason| SketchError::InvalidEndpointTangency {
            constraint: id,
            reason,
        };
        if entity1 == entity2 {
            return Err(fail("TANGENT_DEGENERATE_OPERAND"));
        }
        if self.degenerate_tangent_operand(entity1) || self.degenerate_tangent_operand(entity2) {
            return Err(fail("TANGENT_DEGENERATE_OPERAND"));
        }
        let a = self
            .tangent_endpoint(entity1, entity1_position)
            .ok_or_else(|| fail("TANGENT_ENDPOINT_UNRESOLVED"))?;
        let b = self
            .tangent_endpoint(entity2, entity2_position)
            .ok_or_else(|| fail("TANGENT_ENDPOINT_UNRESOLVED"))?;
        let welded = a == b
            || self.constraints().iter().any(|c| {
                let Constraint::Coincident {
                    point1,
                    point2,
                    point1_position,
                    point2_position,
                    ..
                } = *c
                else {
                    return false;
                };
                let p = self.weld_reference(point1, point1_position);
                let q = self.weld_reference(point2, point2_position);
                (p == Some(a) && q == Some(b)) || (p == Some(b) && q == Some(a))
            });
        if !welded {
            return Err(fail("TANGENT_ENDPOINT_NOT_WELDED"));
        }
        Ok(())
    }

    fn degenerate_tangent_operand(&self, id: EntityId) -> bool {
        match self.get_entity(id) {
            Some(SketchEntity::Line { start, end, .. }) => {
                match (self.get_entity(*start), self.get_entity(*end)) {
                    (
                        Some(SketchEntity::Point { at: a, .. }),
                        Some(SketchEntity::Point { at: b, .. }),
                    ) => !a.is_finite() || !b.is_finite() || a == b,
                    _ => true,
                }
            }
            Some(SketchEntity::Arc {
                radius,
                start_angle,
                end_angle,
                ..
            }) => {
                !radius.is_finite()
                    || *radius <= 0.0
                    || !start_angle.is_finite()
                    || !end_angle.is_finite()
                    || start_angle == end_angle
            }
            _ => false,
        }
    }

    fn tangent_endpoint(
        &self,
        entity: EntityId,
        role: CurvePosition,
    ) -> Option<(EntityId, CurvePosition)> {
        if !matches!(role, CurvePosition::Start | CurvePosition::End) {
            return None;
        }
        match self.get_entity(entity)? {
            SketchEntity::Line { .. } | SketchEntity::Arc { .. } => {
                self.weld_reference(entity, role)
            }
            _ => None,
        }
    }

    fn weld_reference(
        &self,
        entity: EntityId,
        role: CurvePosition,
    ) -> Option<(EntityId, CurvePosition)> {
        match (self.get_entity(entity)?, role) {
            (SketchEntity::Line { start, .. }, CurvePosition::Start) => {
                Some((*start, CurvePosition::Arbitrary))
            }
            (SketchEntity::Line { end, .. }, CurvePosition::End) => {
                Some((*end, CurvePosition::Arbitrary))
            }
            (SketchEntity::Arc { .. }, CurvePosition::Start | CurvePosition::End) => {
                Some((entity, role))
            }
            (SketchEntity::Point { .. }, CurvePosition::Arbitrary) => Some((entity, role)),
            _ => None,
        }
    }
}
