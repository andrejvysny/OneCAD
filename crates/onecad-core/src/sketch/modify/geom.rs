//! Curve model for the Trim/Extend planners: every sketch curve resolved to
//! plane coordinates, plus the parameter spaces the planners cut in.
//!
//! Param spaces: a line uses `t` along `a→b` (`t∈[0,1]` is the segment); an arc
//! uses `s`, the CCW angle from its start (`s∈[0,sweep]` is the arc); a circle
//! uses the absolute angle `θ∈[0,2π)`. The sweep convention matches the worker
//! (`SketchArc::sweepAngle`: `(end − start)` folded into `[0, 2π)`).

use std::f64::consts::{PI, TAU};
use std::ops::{Add, Mul, Sub};

use super::COINCIDENT_MM;
use crate::ids::EntityId;
use crate::math::Vec2;
use crate::sketch::{CurvePosition, Sketch, SketchEntity};

/// A plane point/vector with the arithmetic the planners need.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Pt {
    pub x: f64,
    pub y: f64,
}

impl Pt {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    pub fn dot(self, o: Self) -> f64 {
        self.x * o.x + self.y * o.y
    }
    pub fn cross(self, o: Self) -> f64 {
        self.x * o.y - self.y * o.x
    }
    pub fn len(self) -> f64 {
        self.x.hypot(self.y)
    }
    pub fn dist(self, o: Self) -> f64 {
        (self - o).len()
    }
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
    pub fn to_vec2(self) -> Option<Vec2> {
        Vec2::new(self.x, self.y)
    }
}

impl From<Vec2> for Pt {
    fn from(v: Vec2) -> Self {
        Self::new(v.x, v.y)
    }
}

impl Add for Pt {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Pt {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y)
    }
}

impl Mul<f64> for Pt {
    type Output = Self;
    fn mul(self, k: f64) -> Self {
        Self::new(self.x * k, self.y * k)
    }
}

/// A sketch curve in plane coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Curve {
    Line {
        a: Pt,
        b: Pt,
    },
    Circle {
        c: Pt,
        r: f64,
    },
    Arc {
        c: Pt,
        r: f64,
        start: f64,
        sweep: f64,
    },
    Ellipse {
        c: Pt,
        major: f64,
        minor: f64,
        rot: f64,
    },
}

/// Position of a `Point` entity.
pub(super) fn point_at(sketch: &Sketch, id: EntityId) -> Option<Pt> {
    match sketch.get_entity(id)? {
        SketchEntity::Point { at, .. } => Some((*at).into()),
        _ => None,
    }
}

/// The curve an entity draws, or `None` for a point, a missing reference or a
/// non-positive size (such geometry neither cuts nor is cut).
pub(super) fn curve_of(sketch: &Sketch, entity: &SketchEntity) -> Option<Curve> {
    let curve = match *entity {
        SketchEntity::Point { .. } => return None,
        SketchEntity::Line { start, end, .. } => Curve::Line {
            a: point_at(sketch, start)?,
            b: point_at(sketch, end)?,
        },
        SketchEntity::Circle { center, radius, .. } => Curve::Circle {
            c: point_at(sketch, center)?,
            r: radius,
        },
        SketchEntity::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            ..
        } => Curve::Arc {
            c: point_at(sketch, center)?,
            r: radius,
            start: start_angle,
            sweep: sweep_of(start_angle, end_angle),
        },
        SketchEntity::Ellipse {
            center,
            major_r,
            minor_r,
            rotation,
            ..
        } => Curve::Ellipse {
            c: point_at(sketch, center)?,
            major: major_r,
            minor: minor_r,
            rot: rotation,
        },
    };
    let sized = match curve {
        Curve::Line { .. } => true,
        Curve::Circle { r, .. } | Curve::Arc { r, .. } => r > 0.0,
        Curve::Ellipse { major, minor, .. } => major > 0.0 && minor > 0.0,
    };
    sized.then_some(curve)
}

/// CCW sweep `start→end` folded into `[0, 2π)` (worker convention).
pub(super) fn sweep_of(start: f64, end: f64) -> f64 {
    (end - start).rem_euclid(TAU)
}

/// Angle of `p` about `c`, in `[0, 2π)`.
pub(super) fn angle_of(c: Pt, p: Pt) -> f64 {
    (p.y - c.y).atan2(p.x - c.x).rem_euclid(TAU)
}

/// The point at `angle` on the circle `(c, r)`.
pub(super) fn on_circle(c: Pt, r: f64, angle: f64) -> Pt {
    c + Pt::new(angle.cos(), angle.sin()) * r
}

/// Folds an angle into `(−π, π]`, the range the worker stores arc angles in.
pub(super) fn normalize_angle(a: f64) -> f64 {
    let folded = a.rem_euclid(TAU);
    if folded > PI {
        folded - TAU
    } else {
        folded
    }
}

/// CCW angle of `p` from an arc's `start`, with a point a hair before the start
/// reported as a tiny negative rather than as almost a full turn.
pub(super) fn arc_param(c: Pt, r: f64, start: f64, p: Pt) -> f64 {
    let s = (angle_of(c, p) - start).rem_euclid(TAU);
    if s > TAU - COINCIDENT_MM / r {
        s - TAU
    } else {
        s
    }
}

/// Line parameter of `p` along `a→b` (unbounded).
pub(super) fn line_param(a: Pt, b: Pt, p: Pt) -> f64 {
    let d = b - a;
    (p - a).dot(d) / d.dot(d)
}

impl Curve {
    /// True when `p` (already on this curve's carrier) lies within the drawn
    /// extent, endpoints inclusive within [`COINCIDENT_MM`].
    pub fn contains(&self, p: Pt) -> bool {
        match *self {
            Self::Line { a, b } => {
                let len = a.dist(b);
                if len <= COINCIDENT_MM {
                    return a.dist(p) <= COINCIDENT_MM;
                }
                let tol = COINCIDENT_MM / len;
                let t = line_param(a, b, p);
                t >= -tol && t <= 1.0 + tol
            }
            Self::Arc { c, r, start, sweep } => {
                let s = arc_param(c, r, start, p);
                s >= -COINCIDENT_MM / r && s <= sweep + COINCIDENT_MM / r
            }
            Self::Circle { .. } | Self::Ellipse { .. } => true,
        }
    }

    /// The drawn endpoint of this curve at `p`, as the reference a constraint
    /// would name: a line's endpoint POINT entity, or an arc plus its role.
    pub fn endpoint_ref(&self, entity: &SketchEntity, p: Pt) -> Option<(EntityId, CurvePosition)> {
        match (*self, entity) {
            (Self::Line { a, b }, SketchEntity::Line { start, end, .. }) => {
                if a.dist(p) <= COINCIDENT_MM {
                    Some((*start, CurvePosition::Arbitrary))
                } else if b.dist(p) <= COINCIDENT_MM {
                    Some((*end, CurvePosition::Arbitrary))
                } else {
                    None
                }
            }
            (Self::Arc { c, r, start, sweep }, SketchEntity::Arc { id, .. }) => {
                if on_circle(c, r, start).dist(p) <= COINCIDENT_MM {
                    Some((*id, CurvePosition::Start))
                } else if on_circle(c, r, start + sweep).dist(p) <= COINCIDENT_MM {
                    Some((*id, CurvePosition::End))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// The canonical spelling of a constraint point reference: a line's
/// `Start`/`End` role names its endpoint POINT entity and an arc/circle/
/// ellipse `Center` role its centre POINT entity, so both spellings of the
/// same point compare equal. Arc endpoint roles and point entities are already
/// canonical.
pub(super) fn point_key(
    sketch: &Sketch,
    entity: EntityId,
    role: CurvePosition,
) -> (EntityId, CurvePosition) {
    let point = match (sketch.get_entity(entity), role) {
        (Some(SketchEntity::Line { start, .. }), CurvePosition::Start) => *start,
        (Some(SketchEntity::Line { end, .. }), CurvePosition::End) => *end,
        (
            Some(
                SketchEntity::Arc { center, .. }
                | SketchEntity::Circle { center, .. }
                | SketchEntity::Ellipse { center, .. },
            ),
            CurvePosition::Center,
        ) => *center,
        _ => return (entity, role),
    };
    (point, CurvePosition::Arbitrary)
}

/// Plane position of a constraint point reference: a point entity
/// (`Arbitrary`), a line/arc endpoint role, or a centre role.
pub(super) fn ref_point(sketch: &Sketch, entity: EntityId, role: CurvePosition) -> Option<Pt> {
    let e = sketch.get_entity(entity)?;
    match (e, role) {
        (SketchEntity::Point { at, .. }, CurvePosition::Arbitrary) => Some((*at).into()),
        (SketchEntity::Line { start, .. }, CurvePosition::Start) => point_at(sketch, *start),
        (SketchEntity::Line { end, .. }, CurvePosition::End) => point_at(sketch, *end),
        (
            SketchEntity::Arc { center, .. }
            | SketchEntity::Circle { center, .. }
            | SketchEntity::Ellipse { center, .. },
            CurvePosition::Center,
        ) => point_at(sketch, *center),
        (
            SketchEntity::Arc {
                center,
                radius,
                start_angle,
                end_angle,
                ..
            },
            CurvePosition::Start | CurvePosition::End,
        ) => {
            let angle = if role == CurvePosition::Start {
                *start_angle
            } else {
                *end_angle
            };
            Some(on_circle(point_at(sketch, *center)?, *radius, angle))
        }
        _ => None,
    }
}
