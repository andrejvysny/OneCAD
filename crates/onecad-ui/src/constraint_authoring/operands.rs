//! Operand roles are checked before building a detached candidate; the application still admits it.
use super::relation::Relation;
use onecad_core::{
    document::variables::Scalar,
    ids::{ConstraintId, EntityId},
    math::Vec2,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity},
};
use onecad_interaction::sketch::SketchTarget;

type PointRef = (EntityId, CurvePosition);

pub fn raw_point(sketch: &Sketch, id: EntityId) -> Option<Vec2> {
    match sketch.get_entity(id)? {
        SketchEntity::Point { at, .. } => Some(*at),
        _ => None,
    }
}

pub fn line(sketch: &Sketch, target: SketchTarget) -> Option<(EntityId, Vec2, Vec2)> {
    let SketchTarget::Entity(id) = target else {
        return None;
    };
    let SketchEntity::Line { start, end, .. } = sketch.get_entity(id)? else {
        return None;
    };
    let a = raw_point(sketch, *start)?;
    let b = raw_point(sketch, *end)?;
    ((b.x - a.x).hypot(b.y - a.y) > 1e-12).then_some((id, a, b))
}

pub fn circular(sketch: &Sketch, target: SketchTarget) -> Option<(EntityId, f64)> {
    let SketchTarget::Entity(id) = target else {
        return None;
    };
    match sketch.get_entity(id)? {
        SketchEntity::Circle { radius, .. } | SketchEntity::Arc { radius, .. }
            if radius.is_finite() && *radius > 0.0 =>
        {
            Some((id, *radius))
        }
        _ => None,
    }
}

pub(super) fn curve(sketch: &Sketch, target: SketchTarget) -> Option<EntityId> {
    line(sketch, target)
        .map(|(id, _, _)| id)
        .or_else(|| circular(sketch, target).map(|(id, _)| id))
}

pub(super) fn points(sketch: &Sketch, targets: &[SketchTarget]) -> Option<(PointRef, PointRef)> {
    let [a, b] = targets else { return None };
    let pair = (a.constraint_point(sketch)?, b.constraint_point(sketch)?);
    (pair.0 != pair.1).then_some(pair)
}

pub(super) fn lines(sketch: &Sketch, targets: &[SketchTarget]) -> Option<(EntityId, EntityId)> {
    let [a, b] = targets else { return None };
    let (a, b) = (line(sketch, *a)?.0, line(sketch, *b)?.0);
    (a != b).then_some((a, b))
}

pub(super) fn point_curve(
    sketch: &Sketch,
    targets: &[SketchTarget],
    line_only: bool,
) -> Option<(PointRef, EntityId)> {
    let [a, b] = targets else { return None };
    let get_curve = |target| {
        if line_only {
            line(sketch, target).map(|(id, _, _)| id)
        } else {
            curve(sketch, target)
        }
    };
    let pair = a
        .constraint_point(sketch)
        .zip(get_curve(*b))
        .or_else(|| b.constraint_point(sketch).zip(get_curve(*a)))?;
    (pair.0.0 != pair.1).then_some(pair)
}

pub fn build(
    relation: Relation,
    sketch: &Sketch,
    targets: &[SketchTarget],
    value: Option<Scalar>,
) -> Result<Constraint, String> {
    build_with_id(
        relation,
        sketch,
        targets,
        value,
        ConstraintId(uuid::Uuid::nil()),
    )
}

pub fn create(
    relation: Relation,
    sketch: &Sketch,
    targets: &[SketchTarget],
    value: Option<Scalar>,
) -> Result<Constraint, String> {
    build_with_id(relation, sketch, targets, value, ConstraintId::new())
}

fn build_with_id(
    relation: Relation,
    sketch: &Sketch,
    targets: &[SketchTarget],
    value: Option<Scalar>,
    id: ConstraintId,
) -> Result<Constraint, String> {
    if targets.iter().any(|target| !target.is_valid(sketch)) {
        return Err("Selection is no longer available in the accepted sketch".into());
    }
    let constraint = if relation.dimensional() {
        dimensional(
            relation,
            sketch,
            targets,
            id,
            value.unwrap_or_else(|| Scalar::new(1.0)),
        )
    } else {
        super::geometric::build(relation, sketch, targets, id)
    };
    constraint.ok_or_else(|| requirement(relation).into())
}

fn dimensional(
    relation: Relation,
    sketch: &Sketch,
    targets: &[SketchTarget],
    id: ConstraintId,
    value: Scalar,
) -> Option<Constraint> {
    use Relation::*;
    match relation {
        Radius | Diameter => {
            let [target] = targets else { return None };
            let entity = circular(sketch, *target)?.0;
            Some(if relation == Radius {
                Constraint::Radius { id, entity, value }
            } else {
                Constraint::Diameter { id, entity, value }
            })
        }
        Angle => {
            let (line1, line2) = lines(sketch, targets)?;
            Some(Constraint::Angle {
                id,
                line1,
                line2,
                value,
            })
        }
        HorizontalDistance | VerticalDistance => {
            let ((point1, point1_position), (point2, point2_position)) = points(sketch, targets)?;
            Some(if relation == HorizontalDistance {
                Constraint::HorizontalDistance {
                    id,
                    point1,
                    point1_position,
                    point2,
                    point2_position,
                    value,
                }
            } else {
                Constraint::VerticalDistance {
                    id,
                    point1,
                    point1_position,
                    point2,
                    point2_position,
                    value,
                }
            })
        }
        Distance => distance(sketch, targets, id, value),
        _ => None,
    }
}

fn distance(
    sketch: &Sketch,
    targets: &[SketchTarget],
    id: ConstraintId,
    value: Scalar,
) -> Option<Constraint> {
    let refs = if let Some(pair) = points(sketch, targets) {
        pair
    } else if let Some((point, line)) = point_curve(sketch, targets, true) {
        (point, (line, CurvePosition::Arbitrary))
    } else if let [target] = targets {
        let SketchTarget::Entity(line_id) = target else {
            return None;
        };
        let SketchEntity::Line { start, end, .. } = sketch.get_entity(*line_id)? else {
            return None;
        };
        (
            (*start, CurvePosition::Arbitrary),
            (*end, CurvePosition::Arbitrary),
        )
    } else {
        let [first, second] = targets else {
            return None;
        };
        let (id1, a, b) = line(sketch, *first)?;
        let (id2, c, d) = line(sketch, *second)?;
        let cross = (b.x - a.x) * (d.y - c.y) - (b.y - a.y) * (d.x - c.x);
        if id1 == id2
            || cross.abs() > 1e-8 * ((b.x - a.x).hypot(b.y - a.y) * (d.x - c.x).hypot(d.y - c.y))
        {
            return None;
        }
        (
            (id1, CurvePosition::Arbitrary),
            (id2, CurvePosition::Arbitrary),
        )
    };
    Some(Constraint::Distance {
        id,
        entity1: refs.0.0,
        entity1_position: refs.0.1,
        entity2: refs.1.0,
        entity2_position: refs.1.1,
        value,
    })
}

pub fn requirement(relation: Relation) -> &'static str {
    use Relation::*;
    match relation {
        Horizontal | Vertical => {
            "Select one whole line; point alignment uses Horizontal points or Vertical points"
        }
        Fixed => "Select one editable point or supported endpoint/center role",
        Coincident | HorizontalPoints | VerticalPoints | HorizontalDistance | VerticalDistance => {
            "Select two distinct point roles in the intended order"
        }
        Parallel | Perpendicular | Angle => "Select two distinct whole lines",
        Tangent => {
            "Select a line and circle/arc, or two circles/arcs; line-line tangency is unsupported"
        }
        Concentric => "Select two whole circles or circular arcs",
        Equal => "Select two whole lines, or two circles/circular arcs",
        Midpoint => "Select a point role and a whole line",
        Symmetric => "Select two point roles, then the whole axis line",
        OnCurve => "Select a point role and a whole line, circle or circular arc",
        Distance => "Select one line, two point roles, a point and line, or two parallel lines",
        Radius | Diameter => "Select one whole circle or circular arc",
    }
}

pub fn already_applied(sketch: &Sketch, candidate: &Constraint) -> bool {
    let candidate_signature = signature(candidate);
    sketch.constraints().iter().any(|existing| {
        std::mem::discriminant(existing) == std::mem::discriminant(candidate)
            && signature(existing) == candidate_signature
    })
}

fn signature(constraint: &Constraint) -> serde_json::Value {
    let mut result = serde_json::to_value(constraint).unwrap_or_default();
    let Some(object) = result.as_object_mut() else {
        return result;
    };
    object.remove("id");
    object.remove("value");
    if matches!(constraint, Constraint::Fixed { .. }) {
        object.remove("at");
    }
    let pair = match constraint {
        Constraint::Coincident { .. }
        | Constraint::HorizontalPoints { .. }
        | Constraint::VerticalPoints { .. }
        | Constraint::Symmetric { .. } => {
            Some(("point1", "point2", "point1Position", "point2Position"))
        }
        Constraint::Parallel { .. } | Constraint::Perpendicular { .. } => {
            Some(("line1", "line2", "unused1", "unused2"))
        }
        Constraint::Tangent { .. }
        | Constraint::Concentric { .. }
        | Constraint::Equal { .. }
        | Constraint::Distance { .. } => {
            Some(("entity1", "entity2", "entity1Position", "entity2Position"))
        }
        _ => None,
    };
    if let Some((a, b, ar, br)) = pair {
        let first = (
            object.get(a).cloned().unwrap_or_default(),
            object.get(ar).cloned().unwrap_or_default(),
        );
        let second = (
            object.get(b).cloned().unwrap_or_default(),
            object.get(br).cloned().unwrap_or_default(),
        );
        if format!("{first:?}") > format!("{second:?}") {
            object.insert(a.into(), second.0);
            object.insert(b.into(), first.0);
            let av = object.remove(ar);
            let bv = object.remove(br);
            if let Some(v) = av {
                object.insert(br.into(), v);
            }
            if let Some(v) = bv {
                object.insert(ar.into(), v);
            }
        }
    }
    result
}
