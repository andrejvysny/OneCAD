use super::*;
use onecad_core::{ids::EntityId, sketch::SketchEntity};
use onecad_scene::DVec2;
use std::f64::consts::TAU;

#[derive(Clone, Copy, Debug)]
pub(super) enum Curve {
    Line {
        id: EntityId,
        a: DVec2,
        b: DVec2,
    },
    Circle {
        id: EntityId,
        center: DVec2,
        radius: f64,
        start: f64,
        sweep: f64,
    },
}

pub(super) fn vector(point: Vec2) -> DVec2 {
    DVec2::new(point.x, point.y)
}
pub(super) fn point(point: DVec2) -> Option<Vec2> {
    Vec2::new(point.x, point.y)
}

impl Curve {
    pub(super) fn from_entity(sketch: &Sketch, entity: &SketchEntity) -> Option<Self> {
        let raw = |id| {
            SketchTarget::Point {
                entity: id,
                role: onecad_core::sketch::CurvePosition::Arbitrary,
            }
            .point_coordinates(sketch)
            .map(vector)
        };
        Some(match entity {
            SketchEntity::Line { id, start, end, .. } => Self::Line {
                id: *id,
                a: raw(*start)?,
                b: raw(*end)?,
            },
            SketchEntity::Circle {
                id, center, radius, ..
            } if *radius > 0.0 => Self::Circle {
                id: *id,
                center: raw(*center)?,
                radius: *radius,
                start: 0.0,
                sweep: TAU,
            },
            SketchEntity::Arc {
                id,
                center,
                radius,
                start_angle,
                end_angle,
                ..
            } if *radius > 0.0 => {
                let delta = end_angle - start_angle;
                Self::Circle {
                    id: *id,
                    center: raw(*center)?,
                    radius: *radius,
                    start: *start_angle,
                    sweep: if delta.abs() >= TAU {
                        TAU
                    } else {
                        delta.rem_euclid(TAU)
                    },
                }
            }
            _ => return None,
        })
    }
    pub(super) fn id(self) -> EntityId {
        match self {
            Self::Line { id, .. } | Self::Circle { id, .. } => id,
        }
    }
    pub(super) fn at(self, parameter: f64) -> DVec2 {
        match self {
            Self::Line { a, b, .. } => a + (b - a) * parameter,
            Self::Circle {
                center,
                radius,
                start,
                sweep,
                ..
            } => {
                let angle = start + sweep * parameter;
                center + DVec2::new(angle.cos(), angle.sin()) * radius
            }
        }
    }
    pub(super) fn contains(self, value: DVec2) -> bool {
        match self {
            Self::Line { a, b, .. } => {
                let length = (b - a).length_squared();
                length > 1e-20 && (-1e-9..=1.0 + 1e-9).contains(&((value - a).dot(b - a) / length))
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
    pub(super) fn nearest(self, request: &SnapRequest<'_>) -> Option<Vec2> {
        let samples: usize = match self {
            Self::Line { .. } => 1,
            Self::Circle { .. } => 64,
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

fn cross(a: DVec2, b: DVec2) -> f64 {
    a.x * b.y - a.y * b.x
}

pub(super) fn intersections(a: Curve, b: Curve) -> Vec<Vec2> {
    let candidates = match (a, b) {
        (Curve::Line { a, b, .. }, Curve::Line { a: c, b: d, .. }) => {
            let denominator = cross(b - a, d - c);
            if denominator.abs() <= 1e-12 * (b - a).length() * (d - c).length() {
                vec![]
            } else {
                vec![a + (b - a) * (cross(c - a, d - c) / denominator)]
            }
        }
        (Curve::Line { a, b, .. }, Curve::Circle { center, radius, .. })
        | (Curve::Circle { center, radius, .. }, Curve::Line { a, b, .. }) => {
            line_circle(a, b, center, radius)
        }
        (
            Curve::Circle {
                center: a,
                radius: r,
                ..
            },
            Curve::Circle {
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

fn line_circle(a: DVec2, b: DVec2, center: DVec2, radius: f64) -> Vec<DVec2> {
    let delta = b - a;
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
