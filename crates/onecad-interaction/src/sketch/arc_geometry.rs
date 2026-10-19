//! Analytic arc candidates. These constructors neither author nor solve a sketch.

use super::primitives::MIN_SIZE;
use onecad_core::{math::Vec2, sketch::CurvePosition};
use std::f64::consts::TAU;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalyticArc {
    pub center: Vec2,
    pub radius: f64,
    /// Stored arc travels counterclockwise from `start_angle` to `end_angle`.
    pub start_angle: f64,
    /// Unwrapped: greater than start, less than start + one full turn.
    pub end_angle: f64,
    pub sweep: f64,
    /// Input/travel order can oppose the stored counterclockwise order.
    pub clockwise: bool,
    pub travel_start_role: CurvePosition,
    pub travel_end_role: CurvePosition,
    pub exit_tangent: Vec2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArcGeometryError {
    NonFinite,
    CoordinateOverflow,
    DegenerateRadius,
    EndAtCenter,
    DegenerateChord,
    DegenerateSweep,
    Collinear,
    IllConditioned,
    ZeroTangent,
    TangentLine,
}

impl std::fmt::Display for ArcGeometryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NonFinite => "Arc input must be finite",
            Self::CoordinateOverflow => "Arc coordinates exceed finite precision",
            Self::DegenerateRadius => "Arc radius must be at least 1e-9",
            Self::EndAtCenter => "Arc end cursor must differ from its center",
            Self::DegenerateChord => "Arc endpoints must be distinct by at least 1e-9",
            Self::DegenerateSweep => "Arc sweep must be nonzero and less than a full turn",
            Self::Collinear => "Three arc points must not be collinear",
            Self::IllConditioned => "Arc fit is too ill-conditioned",
            Self::ZeroTangent => "Arc travel tangent must be nonzero",
            Self::TangentLine => "No finite tangent arc reaches a point on the tangent line",
        })
    }
}

impl std::error::Error for ArcGeometryError {}

type Result<T> = std::result::Result<T, ArcGeometryError>;

/// Radius comes from `start`; only the direction of `end_cursor` is retained.
pub fn center_start_end(center: Vec2, start: Vec2, end_cursor: Vec2) -> Result<AnalyticArc> {
    finite(&[center, start, end_cursor])?;
    let radial = difference(start, center)?;
    let radius = length(radial)?;
    valid_radius(radius)?;
    let direction = unit(
        difference(end_cursor, center)?,
        ArcGeometryError::EndAtCenter,
    )?;
    let end = translated(center, scaled(direction, radius))?;
    build(center, radius, start, end, false)
}

/// Click order is start, end, through. The stored CCW arc contains `through`.
pub fn through_points(start: Vec2, end: Vec2, through: Vec2) -> Result<AnalyticArc> {
    finite(&[start, end, through])?;
    let u = difference(end, start)?;
    let v = difference(through, start)?;
    if length(u)? < MIN_SIZE
        || length(v)? < MIN_SIZE
        || length(difference(end, through)?)? < MIN_SIZE
    {
        return Err(ArcGeometryError::DegenerateChord);
    }
    let center = circumcenter(start, u, v)?;
    let radius = length(difference(start, center)?)?;
    let a = unit(
        difference(start, center)?,
        ArcGeometryError::DegenerateRadius,
    )?;
    let b = unit(difference(end, center)?, ArcGeometryError::DegenerateRadius)?;
    let m = unit(
        difference(through, center)?,
        ArcGeometryError::DegenerateRadius,
    )?;
    build(
        center,
        radius,
        start,
        end,
        ccw_angle(a, m) > ccw_angle(a, b),
    )
}

/// `travel_tangent` points away from anchor along the intended travel direction.
pub fn tangent_to(anchor: Vec2, travel_tangent: Vec2, end: Vec2) -> Result<AnalyticArc> {
    finite(&[anchor, travel_tangent, end])?;
    let tangent = unit(travel_tangent, ArcGeometryError::ZeroTangent)?;
    let normal = Vec2::new_unchecked(-tangent.y, tangent.x);
    let chord = difference(end, anchor)?;
    let chord_length = length(chord)?;
    if chord_length < MIN_SIZE {
        return Err(ArcGeometryError::DegenerateChord);
    }
    let direction = unit(chord, ArcGeometryError::DegenerateChord)?;
    let normal_component = dot(direction, normal);
    if normal_component == 0.0 || chord_length * normal_component.abs() < MIN_SIZE {
        return Err(ArcGeometryError::TangentLine);
    }
    let offset = (chord_length * 0.5) / normal_component;
    let center = translated(anchor, scaled(normal, offset))?;
    build(center, offset.abs(), anchor, end, offset < 0.0)
}

fn circumcenter(anchor: Vec2, u: Vec2, v: Vec2) -> Result<Vec2> {
    // Normalize before products: finite coordinates can overflow when squared.
    let scale = u.x.abs().max(u.y.abs()).max(v.x.abs()).max(v.y.abs());
    let a = scaled(u, scale.recip());
    let b = scaled(v, scale.recip());
    let determinant = cross(a, b);
    let rounding_bound = 16.0 * f64::EPSILON * ((a.x * b.y).abs() + (a.y * b.x).abs());
    if determinant == 0.0 || (determinant.abs() / length(a)?) * scale < MIN_SIZE {
        return Err(ArcGeometryError::Collinear);
    }
    if determinant.abs() <= rounding_bound {
        return Err(ArcGeometryError::IllConditioned);
    }
    let aa = dot(a, a);
    let bb = dot(b, b);
    let x = (b.y * aa - a.y * bb) / (2.0 * determinant);
    let y = (a.x * bb - b.x * aa) / (2.0 * determinant);
    translated(anchor, Vec2::new_unchecked(x * scale, y * scale))
}

fn build(
    center: Vec2,
    radius: f64,
    travel_start: Vec2,
    travel_end: Vec2,
    clockwise: bool,
) -> Result<AnalyticArc> {
    valid_radius(radius)?;
    if length(difference(travel_end, travel_start)?)? < MIN_SIZE {
        return Err(ArcGeometryError::DegenerateChord);
    }
    let a = unit(
        difference(travel_start, center)?,
        ArcGeometryError::DegenerateRadius,
    )?;
    let b = unit(
        difference(travel_end, center)?,
        ArcGeometryError::DegenerateRadius,
    )?;
    let (start, end) = if clockwise { (b, a) } else { (a, b) };
    let sweep = ccw_angle(start, end);
    let start_angle = start.y.atan2(start.x);
    let end_angle = start_angle + sweep;
    if sweep <= 0.0 || sweep >= TAU || end_angle <= start_angle || sweep < MIN_SIZE / radius {
        return Err(ArcGeometryError::DegenerateSweep);
    }
    let sign = if clockwise { -1.0 } else { 1.0 };
    let (travel_start_role, travel_end_role) = if clockwise {
        (CurvePosition::End, CurvePosition::Start)
    } else {
        (CurvePosition::Start, CurvePosition::End)
    };
    let arc = AnalyticArc {
        center,
        radius,
        start_angle,
        end_angle,
        sweep,
        clockwise,
        travel_start_role,
        travel_end_role,
        exit_tangent: Vec2::new_unchecked(-b.y * sign, b.x * sign),
    };
    validate_extent(&arc)?;
    Ok(arc)
}

fn validate_extent(arc: &AnalyticArc) -> Result<()> {
    // A finite center/radius is insufficient if the selected arc overflows.
    for angle in [
        arc.start_angle,
        arc.end_angle,
        arc.start_angle + arc.sweep * 0.5,
    ] {
        translated(
            arc.center,
            Vec2::new_unchecked(arc.radius * angle.cos(), arc.radius * angle.sin()),
        )?;
    }
    for angle in [
        0.0,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
        3.0 * std::f64::consts::FRAC_PI_2,
    ] {
        if (angle - arc.start_angle).rem_euclid(TAU) <= arc.sweep {
            translated(
                arc.center,
                Vec2::new_unchecked(arc.radius * angle.cos(), arc.radius * angle.sin()),
            )?;
        }
    }
    Ok(())
}

fn finite(points: &[Vec2]) -> Result<()> {
    points
        .iter()
        .all(Vec2::is_finite)
        .then_some(())
        .ok_or(ArcGeometryError::NonFinite)
}

fn difference(a: Vec2, b: Vec2) -> Result<Vec2> {
    Vec2::new(a.x - b.x, a.y - b.y).ok_or(ArcGeometryError::CoordinateOverflow)
}

fn translated(a: Vec2, b: Vec2) -> Result<Vec2> {
    Vec2::new(a.x + b.x, a.y + b.y).ok_or(ArcGeometryError::CoordinateOverflow)
}

fn scaled(a: Vec2, factor: f64) -> Vec2 {
    Vec2::new_unchecked(a.x * factor, a.y * factor)
}

fn length(a: Vec2) -> Result<f64> {
    let length = a.x.hypot(a.y);
    length
        .is_finite()
        .then_some(length)
        .ok_or(ArcGeometryError::CoordinateOverflow)
}

fn valid_radius(radius: f64) -> Result<()> {
    if !radius.is_finite() {
        return Err(ArcGeometryError::CoordinateOverflow);
    }
    (radius >= MIN_SIZE)
        .then_some(())
        .ok_or(ArcGeometryError::DegenerateRadius)
}

fn unit(a: Vec2, error: ArcGeometryError) -> Result<Vec2> {
    let scale = a.x.abs().max(a.y.abs());
    if scale == 0.0 {
        return Err(error);
    }
    let a = Vec2::new_unchecked(a.x / scale, a.y / scale);
    let length = length(a)?;
    Ok(Vec2::new_unchecked(a.x / length, a.y / length))
}

fn dot(a: Vec2, b: Vec2) -> f64 {
    a.x * b.x + a.y * b.y
}
fn cross(a: Vec2, b: Vec2) -> f64 {
    a.x * b.y - a.y * b.x
}
fn ccw_angle(a: Vec2, b: Vec2) -> f64 {
    cross(a, b).atan2(dot(a, b)).rem_euclid(TAU)
}

#[cfg(test)]
mod tests;
