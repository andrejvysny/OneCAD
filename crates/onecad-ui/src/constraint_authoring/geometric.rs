//! Explicit geometric constructors preserve roles and ordered symmetry operands.
use super::{
    operands::{circular, curve, line, lines, point_curve, points},
    relation::Relation,
};
use onecad_core::{
    ids::ConstraintId,
    sketch::{Constraint, CurvePosition, Sketch},
};
use onecad_interaction::sketch::SketchTarget;

pub fn build(
    relation: Relation,
    sketch: &Sketch,
    targets: &[SketchTarget],
    id: ConstraintId,
) -> Option<Constraint> {
    use Relation::*;
    match relation {
        Horizontal | Vertical => axis(relation, sketch, targets, id),
        Fixed => fixed(sketch, targets, id),
        Parallel | Perpendicular => line_pair(relation, sketch, targets, id),
        Coincident | HorizontalPoints | VerticalPoints => point_pair(relation, sketch, targets, id),
        Midpoint | OnCurve => point_on(relation, sketch, targets, id),
        Symmetric => symmetric(sketch, targets, id),
        Tangent | Concentric | Equal => curve_pair(relation, sketch, targets, id),
        _ => None,
    }
}

fn axis(
    relation: Relation,
    sketch: &Sketch,
    targets: &[SketchTarget],
    id: ConstraintId,
) -> Option<Constraint> {
    let [target] = targets else { return None };
    let line = line(sketch, *target)?.0;
    Some(if relation == Relation::Horizontal {
        Constraint::Horizontal { id, line }
    } else {
        Constraint::Vertical { id, line }
    })
}

fn fixed(sketch: &Sketch, targets: &[SketchTarget], id: ConstraintId) -> Option<Constraint> {
    let [target] = targets else { return None };
    if sketch.get_entity(target.owner_id())?.is_reference_locked() {
        return None;
    }
    let (point, point_position) = target.constraint_point(sketch)?;
    Some(Constraint::Fixed {
        id,
        point,
        point_position,
        at: target.point_coordinates(sketch)?,
    })
}

fn line_pair(
    relation: Relation,
    sketch: &Sketch,
    targets: &[SketchTarget],
    id: ConstraintId,
) -> Option<Constraint> {
    let (line1, line2) = lines(sketch, targets)?;
    Some(if relation == Relation::Parallel {
        Constraint::Parallel { id, line1, line2 }
    } else {
        Constraint::Perpendicular { id, line1, line2 }
    })
}

fn point_pair(
    relation: Relation,
    sketch: &Sketch,
    targets: &[SketchTarget],
    id: ConstraintId,
) -> Option<Constraint> {
    let ((point1, point1_position), (point2, point2_position)) = points(sketch, targets)?;
    Some(match relation {
        Relation::Coincident => Constraint::Coincident {
            id,
            point1,
            point1_position,
            point2,
            point2_position,
        },
        Relation::HorizontalPoints => Constraint::HorizontalPoints {
            id,
            point1,
            point1_position,
            point2,
            point2_position,
        },
        _ => Constraint::VerticalPoints {
            id,
            point1,
            point1_position,
            point2,
            point2_position,
        },
    })
}

fn point_on(
    relation: Relation,
    sketch: &Sketch,
    targets: &[SketchTarget],
    id: ConstraintId,
) -> Option<Constraint> {
    let ((point, point_position), other) =
        point_curve(sketch, targets, relation == Relation::Midpoint)?;
    Some(if relation == Relation::Midpoint {
        Constraint::Midpoint {
            id,
            point,
            point_position,
            line: other,
        }
    } else {
        Constraint::OnCurve {
            id,
            point,
            point_position,
            curve: other,
            position: CurvePosition::Arbitrary,
        }
    })
}

fn symmetric(sketch: &Sketch, targets: &[SketchTarget], id: ConstraintId) -> Option<Constraint> {
    let [a, b, axis] = targets else { return None };
    let ((point1, point1_position), (point2, point2_position)) = points(sketch, &[*a, *b])?;
    Some(Constraint::Symmetric {
        id,
        point1,
        point1_position,
        point2,
        point2_position,
        axis: line(sketch, *axis)?.0,
    })
}

fn curve_pair(
    relation: Relation,
    sketch: &Sketch,
    targets: &[SketchTarget],
    id: ConstraintId,
) -> Option<Constraint> {
    let [a, b] = targets else { return None };
    let (entity1, entity2) = (curve(sketch, *a)?, curve(sketch, *b)?);
    if entity1 == entity2 {
        return None;
    }
    let both_circular = circular(sketch, *a).is_some() && circular(sketch, *b).is_some();
    let both_lines = line(sketch, *a).is_some() && line(sketch, *b).is_some();
    match relation {
        Relation::Tangent if !both_lines => Some(Constraint::Tangent {
            id,
            entity1,
            entity2,
        }),
        Relation::Concentric if both_circular => Some(Constraint::Concentric {
            id,
            entity1,
            entity2,
        }),
        Relation::Equal if both_circular || both_lines => Some(Constraint::Equal {
            id,
            entity1,
            entity2,
        }),
        _ => None,
    }
}
