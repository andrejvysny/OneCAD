//! Uniform access to a constraint's entity references, so the remap table can
//! rewrite "the trimmed entity" to a piece without a 21-arm match per rule.

use crate::ids::{ConstraintId, EntityId};
use crate::sketch::{Constraint, CurvePosition};

/// How a constraint names an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Slot {
    /// The entity as a whole: a curve operand, or a point entity itself.
    Whole,
    /// One point OF the entity: a line/arc `Start`/`End` or a `Center`.
    Role(CurvePosition),
}

fn point_slot(role: CurvePosition) -> Slot {
    if role.is_arbitrary() {
        Slot::Whole
    } else {
        Slot::Role(role)
    }
}

/// Every `(entity, slot)` reference of `c`, in field order.
pub(super) fn refs(c: &Constraint) -> Vec<(EntityId, Slot)> {
    let mut out = Vec::new();
    map_refs(c, c.id(), &mut |e, slot| {
        out.push((e, slot));
        e
    });
    out
}

/// `c` with id `id` and every entity reference passed through `f`. Roles are
/// carried over unchanged — only the owning entity is rewritten.
pub(super) fn map_refs(
    c: &Constraint,
    id: ConstraintId,
    f: &mut dyn FnMut(EntityId, Slot) -> EntityId,
) -> Constraint {
    use Constraint as C;
    let w = Slot::Whole;
    match c.clone() {
        C::Coincident {
            point1,
            point2,
            point1_position,
            point2_position,
            ..
        } => C::Coincident {
            id,
            point1: f(point1, point_slot(point1_position)),
            point2: f(point2, point_slot(point2_position)),
            point1_position,
            point2_position,
        },
        C::Horizontal { line, .. } => C::Horizontal {
            id,
            line: f(line, w),
        },
        C::Vertical { line, .. } => C::Vertical {
            id,
            line: f(line, w),
        },
        C::Fixed {
            point,
            point_position,
            at,
            ..
        } => C::Fixed {
            id,
            point: f(point, point_slot(point_position)),
            point_position,
            at,
        },
        C::Midpoint {
            point,
            point_position,
            line,
            ..
        } => C::Midpoint {
            id,
            point: f(point, point_slot(point_position)),
            point_position,
            line: f(line, w),
        },
        C::OnCurve {
            point,
            point_position,
            curve,
            position,
            ..
        } => C::OnCurve {
            id,
            point: f(point, point_slot(point_position)),
            point_position,
            curve: f(curve, point_slot(position)),
            position,
        },
        C::Parallel { line1, line2, .. } => C::Parallel {
            id,
            line1: f(line1, w),
            line2: f(line2, w),
        },
        C::Perpendicular { line1, line2, .. } => C::Perpendicular {
            id,
            line1: f(line1, w),
            line2: f(line2, w),
        },
        C::Tangent {
            entity1, entity2, ..
        } => C::Tangent {
            id,
            entity1: f(entity1, w),
            entity2: f(entity2, w),
        },
        C::TangentAtPoint {
            entity1,
            entity2,
            entity1_position,
            entity2_position,
            ..
        } => C::TangentAtPoint {
            id,
            entity1: f(entity1, point_slot(entity1_position)),
            entity2: f(entity2, point_slot(entity2_position)),
            entity1_position,
            entity2_position,
        },
        C::Concentric {
            entity1, entity2, ..
        } => C::Concentric {
            id,
            entity1: f(entity1, w),
            entity2: f(entity2, w),
        },
        C::Equal {
            entity1, entity2, ..
        } => C::Equal {
            id,
            entity1: f(entity1, w),
            entity2: f(entity2, w),
        },
        other => map_dimension_refs(other, id, f),
    }
}

/// The dimensional / point-pair half of [`map_refs`] (split for length).
fn map_dimension_refs(
    c: Constraint,
    id: ConstraintId,
    f: &mut dyn FnMut(EntityId, Slot) -> EntityId,
) -> Constraint {
    use Constraint as C;
    let w = Slot::Whole;
    match c {
        C::Distance {
            entity1,
            entity1_position,
            entity2,
            entity2_position,
            value,
            ..
        } => C::Distance {
            id,
            entity1: f(entity1, point_slot(entity1_position)),
            entity1_position,
            entity2: f(entity2, point_slot(entity2_position)),
            entity2_position,
            value,
        },
        C::HorizontalDistance {
            point1,
            point1_position,
            point2,
            point2_position,
            value,
            ..
        } => C::HorizontalDistance {
            id,
            point1: f(point1, point_slot(point1_position)),
            point1_position,
            point2: f(point2, point_slot(point2_position)),
            point2_position,
            value,
        },
        C::VerticalDistance {
            point1,
            point1_position,
            point2,
            point2_position,
            value,
            ..
        } => C::VerticalDistance {
            id,
            point1: f(point1, point_slot(point1_position)),
            point1_position,
            point2: f(point2, point_slot(point2_position)),
            point2_position,
            value,
        },
        C::Angle {
            line1,
            line2,
            value,
            ..
        } => C::Angle {
            id,
            line1: f(line1, w),
            line2: f(line2, w),
            value,
        },
        C::Radius { entity, value, .. } => C::Radius {
            id,
            entity: f(entity, w),
            value,
        },
        C::Diameter { entity, value, .. } => C::Diameter {
            id,
            entity: f(entity, w),
            value,
        },
        C::Symmetric {
            point1,
            point1_position,
            point2,
            point2_position,
            axis,
            ..
        } => C::Symmetric {
            id,
            point1: f(point1, point_slot(point1_position)),
            point1_position,
            point2: f(point2, point_slot(point2_position)),
            point2_position,
            axis: f(axis, w),
        },
        C::HorizontalPoints {
            point1,
            point1_position,
            point2,
            point2_position,
            ..
        } => C::HorizontalPoints {
            id,
            point1: f(point1, point_slot(point1_position)),
            point1_position,
            point2: f(point2, point_slot(point2_position)),
            point2_position,
        },
        C::VerticalPoints {
            point1,
            point1_position,
            point2,
            point2_position,
            ..
        } => C::VerticalPoints {
            id,
            point1: f(point1, point_slot(point1_position)),
            point1_position,
            point2: f(point2, point_slot(point2_position)),
            point2_position,
        },
        geometric => unreachable!("map_refs maps {geometric:?} itself"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::variables::Scalar;
    use uuid::Uuid;

    fn eid(n: u128) -> EntityId {
        EntityId(Uuid::from_u128(n))
    }

    #[test]
    fn refs_follow_field_order_and_roles() {
        let c = Constraint::OnCurve {
            id: ConstraintId(Uuid::from_u128(1)),
            point: eid(1),
            point_position: CurvePosition::End,
            curve: eid(2),
            position: CurvePosition::Arbitrary,
        };
        assert_eq!(
            refs(&c),
            vec![
                (eid(1), Slot::Role(CurvePosition::End)),
                (eid(2), Slot::Whole)
            ]
        );
    }

    #[test]
    fn map_refs_rewrites_ids_and_keeps_values() {
        let c = Constraint::Distance {
            id: ConstraintId(Uuid::from_u128(1)),
            entity1: eid(1),
            entity1_position: CurvePosition::Start,
            entity2: eid(2),
            entity2_position: CurvePosition::Arbitrary,
            value: Scalar::new(7.0),
        };
        let fresh = ConstraintId(Uuid::from_u128(9));
        let out = map_refs(&c, fresh, &mut |e, _| if e == eid(1) { eid(5) } else { e });
        assert_eq!(out.id(), fresh);
        assert_eq!(out.entities(), vec![eid(5), eid(2)]);
        assert_eq!(out.value().unwrap().value, 7.0);
    }
}
