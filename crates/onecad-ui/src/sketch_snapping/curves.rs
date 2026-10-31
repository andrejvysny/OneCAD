use super::*;
use onecad_core::{ids::EntityId, sketch::SketchEntity};
use onecad_scene::DVec2;
use std::f64::consts::TAU;

/// Planar geometry shared by real curves and unbounded inference lines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Shape {
    /// `origin + dir * t` for `t` within `range`; infinite bounds model rays and lines.
    Line {
        origin: DVec2,
        dir: DVec2,
        range: (f64, f64),
    },
    Circle {
        center: DVec2,
        radius: f64,
        start: f64,
        sweep: f64,
    },
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Curve {
    pub(super) id: EntityId,
    pub(super) shape: Shape,
}

pub(super) fn vector(point: Vec2) -> DVec2 {
    DVec2::new(point.x, point.y)
}
pub(super) fn point(point: DVec2) -> Option<Vec2> {
    Vec2::new(point.x, point.y)
}

pub(super) fn raw_point(sketch: &Sketch, id: EntityId) -> Option<DVec2> {
    SketchTarget::Point {
        entity: id,
        role: onecad_core::sketch::CurvePosition::Arbitrary,
    }
    .point_coordinates(sketch)
    .map(vector)
}

/// Normalised sweep of an arc, so a full turn stays a full circle.
pub(super) fn arc_sweep(start_angle: f64, end_angle: f64) -> f64 {
    let delta = end_angle - start_angle;
    if delta.abs() >= TAU {
        TAU
    } else {
        delta.rem_euclid(TAU)
    }
}

impl Curve {
    pub(super) fn from_entity(sketch: &Sketch, entity: &SketchEntity) -> Option<Self> {
        let shape = match entity {
            SketchEntity::Line { start, end, .. } => {
                let a = raw_point(sketch, *start)?;
                Shape::Line {
                    origin: a,
                    dir: raw_point(sketch, *end)? - a,
                    range: (0.0, 1.0),
                }
            }
            SketchEntity::Circle { center, radius, .. } if *radius > 0.0 => Shape::Circle {
                center: raw_point(sketch, *center)?,
                radius: *radius,
                start: 0.0,
                sweep: TAU,
            },
            SketchEntity::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                ..
            } if *radius > 0.0 => Shape::Circle {
                center: raw_point(sketch, *center)?,
                radius: *radius,
                start: *start_angle,
                sweep: arc_sweep(*start_angle, *end_angle),
            },
            _ => return None,
        };
        Some(Self {
            id: entity.id(),
            shape,
        })
    }
    /// Samples a bounded curve; `parameter` spans the segment or the arc sweep.
    fn at(self, parameter: f64) -> DVec2 {
        match self.shape {
            Shape::Line { origin, dir, .. } => origin + dir * parameter,
            Shape::Circle {
                center,
                radius,
                start,
                sweep,
            } => {
                let angle = start + sweep * parameter;
                center + DVec2::new(angle.cos(), angle.sin()) * radius
            }
        }
    }
    pub(super) fn nearest(self, request: &SnapRequest<'_>) -> Option<Vec2> {
        let samples: usize = match self.shape {
            Shape::Line { .. } => 1,
            Shape::Circle { .. } => 64,
        };
        let distance = |t: f64| {
            project(request, point(self.at(t))?)
                .map(|screen| f64::from(screen.distance_sq(request.pointer)))
        };
        let index = (0..=samples)
            .filter_map(|i| distance(i as f64 / samples as f64).map(|d| (i, d)))
            .min_by(|a, b| a.1.total_cmp(&b.1))?
            .0;
        let mut low = index.saturating_sub(1) as f64 / samples as f64;
        let mut high = ((index + 1) as f64 / samples as f64).min(1.0);
        for _ in 0..24 {
            let a = low + (high - low) / 3.0;
            let b = high - (high - low) / 3.0;
            if distance(a)? <= distance(b)? {
                high = b;
            } else {
                low = a;
            }
        }
        point(self.at((low + high) / 2.0))
    }
}

impl Shape {
    pub(super) fn contains(self, value: DVec2) -> bool {
        match self {
            Self::Line { origin, dir, range } => {
                let length = dir.length_squared();
                length > 1e-20 && {
                    let t = (value - origin).dot(dir) / length;
                    t >= range.0 - 1e-9 && t <= range.1 + 1e-9
                }
            }
            Self::Circle {
                center,
                start,
                sweep,
                ..
            } => {
                let delta = value - center;
                let angle = (delta.y.atan2(delta.x) - start).rem_euclid(TAU);
                sweep >= TAU - 1e-10 || angle <= sweep + 1e-9 || TAU - angle < 1e-9
            }
        }
    }
    /// Orthogonal foot of `value` in sketch space, or `None` outside the shape's extent.
    pub(super) fn project(self, value: DVec2) -> Option<DVec2> {
        let foot = match self {
            Self::Line { origin, dir, .. } => {
                let length = dir.length_squared();
                if length <= 1e-20 {
                    return None;
                }
                origin + dir * ((value - origin).dot(dir) / length)
            }
            Self::Circle { center, radius, .. } => {
                let delta = value - center;
                let length = delta.length();
                if length <= 1e-12 {
                    return None;
                }
                center + delta * (radius / length)
            }
        };
        (foot.is_finite() && self.contains(foot)).then_some(foot)
    }
}

fn cross(a: DVec2, b: DVec2) -> f64 {
    a.x * b.y - a.y * b.x
}

pub(super) fn intersections(a: Shape, b: Shape) -> Vec<Vec2> {
    let candidates = match (a, b) {
        (
            Shape::Line { origin, dir, .. },
            Shape::Line {
                origin: other,
                dir: other_dir,
                ..
            },
        ) => {
            let denominator = cross(dir, other_dir);
            if denominator.abs() <= 1e-12 * dir.length() * other_dir.length() {
                vec![]
            } else {
                vec![origin + dir * (cross(other - origin, other_dir) / denominator)]
            }
        }
        (Shape::Line { origin, dir, .. }, Shape::Circle { center, radius, .. })
        | (Shape::Circle { center, radius, .. }, Shape::Line { origin, dir, .. }) => {
            line_circle(origin, dir, center, radius)
        }
        (
            Shape::Circle {
                center: a,
                radius: r,
                ..
            },
            Shape::Circle {
                center: b,
                radius: s,
                ..
            },
        ) => circle_circle(a, r, b, s),
    };
    let mut result = Vec::new();
    for value in candidates
        .into_iter()
        .filter(|value| a.contains(*value) && b.contains(*value))
    {
        if let Some(value) = point(value)
            && !result
                .iter()
                .any(|old: &Vec2| (old.x - value.x).hypot(old.y - value.y) < 1e-9)
        {
            result.push(value);
        }
    }
    result
}

fn line_circle(a: DVec2, delta: DVec2, center: DVec2, radius: f64) -> Vec<DVec2> {
    let length = delta.length_squared();
    if length <= 1e-20 {
        return vec![];
    }
    let t = (center - a).dot(delta) / length;
    let foot = a + delta * t;
    let height = radius * radius - (foot - center).length_squared();
    if height < -1e-10 * radius * radius {
        return vec![];
    }
    let offset = (height.max(0.0) / length).sqrt();
    vec![a + delta * (t - offset), a + delta * (t + offset)]
}

fn circle_circle(a: DVec2, r: f64, b: DVec2, s: f64) -> Vec<DVec2> {
    let delta = b - a;
    let distance = delta.length();
    if distance <= 1e-12 || distance > r + s + 1e-10 || distance < (r - s).abs() - 1e-10 {
        return vec![];
    }
    let x = (r * r - s * s + distance * distance) / (2.0 * distance);
    let height = r * r - x * x;
    if height < -1e-10 * r * r {
        return vec![];
    }
    let unit = delta / distance;
    let foot = a + unit * x;
    let offset = DVec2::new(-unit.y, unit.x) * height.max(0.0).sqrt();
    vec![foot + offset, foot - offset]
}
