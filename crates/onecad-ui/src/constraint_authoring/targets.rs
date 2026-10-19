//! Constraint highlights use the same typed roles as creation and picking.
use onecad_core::{
    ids::EntityId,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity},
};
use onecad_interaction::sketch::SketchTarget;

pub fn constraint_targets(sketch: &Sketch, constraint: &Constraint) -> Vec<SketchTarget> {
    let point = |entity, role| SketchTarget::Point { entity, role };
    let entity = SketchTarget::Entity;
    match *constraint {
        Constraint::Horizontal { line, .. } | Constraint::Vertical { line, .. } => {
            vec![entity(line)]
        }
        Constraint::Fixed {
            point: id,
            point_position,
            ..
        } => vec![point(id, point_position)],
        Constraint::Midpoint {
            point: id,
            point_position,
            line,
            ..
        } => vec![point(id, point_position), entity(line)],
        Constraint::OnCurve {
            point: id,
            point_position,
            curve,
            ..
        } => vec![point(id, point_position), entity(curve)],
        Constraint::Coincident {
            point1,
            point1_position,
            point2,
            point2_position,
            ..
        }
        | Constraint::HorizontalPoints {
            point1,
            point1_position,
            point2,
            point2_position,
            ..
        }
        | Constraint::VerticalPoints {
            point1,
            point1_position,
            point2,
            point2_position,
            ..
        }
        | Constraint::HorizontalDistance {
            point1,
            point1_position,
            point2,
            point2_position,
            ..
        }
        | Constraint::VerticalDistance {
            point1,
            point1_position,
            point2,
            point2_position,
            ..
        } => vec![
            point(point1, point1_position),
            point(point2, point2_position),
        ],
        Constraint::Symmetric {
            point1,
            point1_position,
            point2,
            point2_position,
            axis,
            ..
        } => vec![
            point(point1, point1_position),
            point(point2, point2_position),
            entity(axis),
        ],
        Constraint::Parallel { line1, line2, .. }
        | Constraint::Perpendicular { line1, line2, .. }
        | Constraint::Angle { line1, line2, .. } => vec![entity(line1), entity(line2)],
        Constraint::Tangent {
            entity1, entity2, ..
        }
        | Constraint::Concentric {
            entity1, entity2, ..
        }
        | Constraint::Equal {
            entity1, entity2, ..
        } => vec![entity(entity1), entity(entity2)],
        Constraint::TangentAtPoint {
            entity1,
            entity2,
            entity1_position,
            entity2_position,
            ..
        } => vec![
            point(entity1, entity1_position),
            point(entity2, entity2_position),
        ],
        Constraint::Distance {
            entity1,
            entity1_position,
            entity2,
            entity2_position,
            ..
        } => vec![
            distance_target(sketch, entity1, entity1_position),
            distance_target(sketch, entity2, entity2_position),
        ],
        Constraint::Radius { entity: id, .. } | Constraint::Diameter { entity: id, .. } => {
            vec![entity(id)]
        }
    }
}

fn distance_target(sketch: &Sketch, entity: EntityId, role: CurvePosition) -> SketchTarget {
    if role != CurvePosition::Arbitrary
        || matches!(sketch.get_entity(entity), Some(SketchEntity::Point { .. }))
    {
        SketchTarget::Point { entity, role }
    } else {
        SketchTarget::Entity(entity)
    }
}
