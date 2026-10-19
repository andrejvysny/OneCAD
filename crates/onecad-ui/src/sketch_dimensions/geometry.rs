//! Witness geometry comes from accepted operands, including analytic point roles.
use onecad_core::{
    ids::EntityId,
    math::Vec2,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity},
};
use onecad_interaction::sketch::SketchTarget;
use std::f64::consts::{FRAC_PI_4, TAU};

#[derive(Clone, Debug)]
pub struct Witness {
    pub anchor: Vec2,
    pub segments: Vec<[Vec2; 2]>,
    pub measured: [Vec2; 2],
}

fn point(sketch: &Sketch, id: EntityId, role: CurvePosition) -> Option<Vec2> {
    SketchTarget::Point { entity: id, role }.point_coordinates(sketch)
}
fn line(sketch: &Sketch, id: EntityId) -> Option<[Vec2; 2]> {
    let SketchEntity::Line { start, end, .. } = sketch.get_entity(id)? else {
        return None;
    };
    Some([
        point(sketch, *start, CurvePosition::Arbitrary)?,
        point(sketch, *end, CurvePosition::Arbitrary)?,
    ])
}
fn midpoint(a: Vec2, b: Vec2) -> Vec2 {
    Vec2::new_unchecked(a.x * 0.5 + b.x * 0.5, a.y * 0.5 + b.y * 0.5)
}
fn linear(first: Vec2, second: Vec2, actual: [Vec2; 2]) -> Option<Witness> {
    (first.is_finite() && second.is_finite()).then_some(Witness {
        anchor: midpoint(first, second),
        segments: vec![[actual[0], first], [actual[1], second], [first, second]],
        measured: [first, second],
    })
}
fn foot(point: Vec2, line: [Vec2; 2]) -> Option<Vec2> {
    let (dx, dy) = (line[1].x - line[0].x, line[1].y - line[0].y);
    let length = dx * dx + dy * dy;
    if !length.is_finite() || length <= 1e-24 {
        return None;
    }
    let t = ((point.x - line[0].x) * dx + (point.y - line[0].y) * dy) / length;
    Vec2::new(line[0].x + t * dx, line[0].y + t * dy)
}

pub fn witness(sketch: &Sketch, constraint: &Constraint) -> Option<Witness> {
    match *constraint {
        Constraint::Distance {
            entity1,
            entity1_position,
            entity2,
            entity2_position,
            ..
        } => distance(
            sketch,
            (entity1, entity1_position),
            (entity2, entity2_position),
        ),
        Constraint::HorizontalDistance {
            point1,
            point1_position,
            point2,
            point2_position,
            ..
        } => {
            let (a, b) = (
                point(sketch, point1, point1_position)?,
                point(sketch, point2, point2_position)?,
            );
            linear(a, Vec2::new(b.x, a.y)?, [a, b])
        }
        Constraint::VerticalDistance {
            point1,
            point1_position,
            point2,
            point2_position,
            ..
        } => {
            let (a, b) = (
                point(sketch, point1, point1_position)?,
                point(sketch, point2, point2_position)?,
            );
            linear(a, Vec2::new(a.x, b.y)?, [a, b])
        }
        Constraint::Angle {
            line1,
            line2,
            ref value,
            ..
        } => angle(sketch, line1, line2, value.value),
        Constraint::Radius { entity, .. } => radial(sketch, entity, false),
        Constraint::Diameter { entity, .. } => radial(sketch, entity, true),
        _ => None,
    }
}

fn distance(
    sketch: &Sketch,
    a: (EntityId, CurvePosition),
    b: (EntityId, CurvePosition),
) -> Option<Witness> {
    let (first, second) = match (point(sketch, a.0, a.1), point(sketch, b.0, b.1)) {
        (Some(a), Some(b)) => (a, b),
        (Some(a), None) => (a, foot(a, line(sketch, b.0)?)?),
        (None, Some(b)) => (foot(b, line(sketch, a.0)?)?, b),
        (None, None) => {
            let first = line(sketch, a.0)?;
            let at = midpoint(first[0], first[1]);
            (at, foot(at, line(sketch, b.0)?)?)
        }
    };
    linear(first, second, [first, second])
}

fn radial(sketch: &Sketch, id: EntityId, diameter: bool) -> Option<Witness> {
    let (center, radius, angle) = match sketch.get_entity(id)? {
        SketchEntity::Circle { center, radius, .. } => (*center, *radius, FRAC_PI_4),
        SketchEntity::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            ..
        } => (
            *center,
            *radius,
            start_angle + (end_angle - start_angle).rem_euclid(TAU) * 0.5,
        ),
        _ => return None,
    };
    let center = point(sketch, center, CurvePosition::Arbitrary)?;
    let at = |factor: f64| {
        Vec2::new(
            center.x + factor * radius * angle.cos(),
            center.y + factor * radius * angle.sin(),
        )
    };
    let first = if diameter { at(-1.0)? } else { center };
    let second = at(1.0)?;
    linear(first, second, [first, second])
}

fn angle(sketch: &Sketch, first: EntityId, second: EntityId, value: f64) -> Option<Witness> {
    let (a, b) = (line(sketch, first)?, line(sketch, second)?);
    let (u, v) = (
        (a[1].x - a[0].x, a[1].y - a[0].y),
        (b[1].x - b[0].x, b[1].y - b[0].y),
    );
    let cross = u.0 * v.1 - u.1 * v.0;
    let vertex = if cross.abs() > 1e-12 {
        let t = ((b[0].x - a[0].x) * v.1 - (b[0].y - a[0].y) * v.0) / cross;
        Vec2::new(a[0].x + t * u.0, a[0].y + t * u.1)?
    } else {
        a[0]
    };
    let start = u.1.atan2(u.0);
    let mut sweep = cross.atan2(u.0 * v.0 + u.1 * v.1);
    if value > 0.0 && sweep < 0.0 {
        sweep += TAU;
    } else if value < 0.0 && sweep > 0.0 {
        sweep -= TAU;
    }
    let radius = (u.0.hypot(u.1).min(v.0.hypot(v.1)) * 0.3).clamp(1e-6, 1000.0);
    let at = |theta: f64| {
        Vec2::new(
            vertex.x + radius * theta.cos(),
            vertex.y + radius * theta.sin(),
        )
    };
    let mut segments = vec![a, b, [vertex, at(start)?], [vertex, at(start + sweep)?]];
    for index in 0..24 {
        segments.push([
            at(start + sweep * index as f64 / 24.0)?,
            at(start + sweep * (index + 1) as f64 / 24.0)?,
        ]);
    }
    Some(Witness {
        anchor: at(start + sweep * 0.5)?,
        segments,
        measured: [at(start)?, at(start + sweep)?],
    })
}
