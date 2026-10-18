use super::{CapturedSketchPoint, SketchSession};
use onecad_core::{math::Vec2, sketch::SketchPlane};
use onecad_scene::{DVec2, DVec3, Ray, ViewSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SketchPointerError {
    CaptureMismatch,
    InvalidPlane,
    NoIntersection,
    InvalidCoordinate,
}

pub fn capture_sketch_pointer(
    session: &SketchSession,
    view: &ViewSnapshot,
    logical: DVec2,
) -> Result<CapturedSketchPoint, SketchPointerError> {
    if session.document.scene != view.identity().scene {
        return Err(SketchPointerError::CaptureMismatch);
    }
    let ray = view
        .ray(logical)
        .map_err(|_| SketchPointerError::InvalidCoordinate)?;
    let world =
        intersect_sketch_plane(session.plane, ray).ok_or(SketchPointerError::NoIntersection)?;
    let depth = view.depth(world);
    if !depth.is_finite() || depth < view.camera().near || depth > view.camera().far {
        return Err(SketchPointerError::NoIntersection);
    }
    let point =
        sketch_plane_coordinates(session.plane, world).ok_or(SketchPointerError::InvalidPlane)?;
    Ok(CapturedSketchPoint {
        key: session.key.clone(),
        capture: view.identity(),
        point,
    })
}

pub fn validate_sketch_plane(plane: SketchPlane) -> bool {
    valid_plane(plane)
}

pub fn intersect_sketch_plane(plane: SketchPlane, ray: Ray) -> Option<DVec3> {
    if !valid_plane(plane) {
        return None;
    }
    intersect_plane(plane, ray)
}

pub fn sketch_plane_coordinates(plane: SketchPlane, world: DVec3) -> Option<Vec2> {
    if !valid_plane(plane) || !world.is_finite() {
        return None;
    }
    plane_coordinates(plane, world)
}

// Adapted from SolidCraft crates/geom/src/plane.rs::Plane::intersect_ray,
// 6bde126a5cabd1f5af0c68e6955bd0176fab805a (MIT OR Apache-2.0).
// Keep OneCAD's captured axes; guard finite rays and relative parallel roundoff.
fn intersect_plane(plane: SketchPlane, ray: Ray) -> Option<DVec3> {
    let x = vector(plane.x_axis);
    let y = vector(plane.y_axis);
    let normal = x.cross(y);
    let denominator = ray.direction.dot(normal);
    let scale = ray.direction.length() * normal.length();
    if !scale.is_finite() || scale == 0.0 || denominator.abs() <= f64::EPSILON * scale {
        return None;
    }
    let distance = (vector(plane.origin) - ray.origin).dot(normal) / denominator;
    let result = ray.origin + ray.direction * distance;
    (distance.is_finite() && distance >= 0.0 && result.is_finite()).then_some(result)
}

fn plane_coordinates(plane: SketchPlane, world: DVec3) -> Option<Vec2> {
    let x = vector(plane.x_axis);
    let y = vector(plane.y_axis);
    let relative = world - vector(plane.origin);
    let (xx, xy, yy) = (x.dot(x), x.dot(y), y.dot(y));
    let determinant = x.cross(y).length_squared();
    if !determinant.is_finite() || determinant <= 0.0 {
        return None;
    }
    Vec2::new(
        (relative.dot(x) * yy - relative.dot(y) * xy) / determinant,
        (relative.dot(y) * xx - relative.dot(x) * xy) / determinant,
    )
}

pub(crate) fn valid_plane(plane: SketchPlane) -> bool {
    let x = vector(plane.x_axis);
    let y = vector(plane.y_axis);
    let n = vector(plane.normal);
    let cross = x.cross(y);
    let scale = cross.length() * n.length();
    plane.origin.is_finite()
        && x.is_finite()
        && y.is_finite()
        && n.is_finite()
        && scale.is_finite()
        && scale > 0.0
        && cross.dot(n) > 0.0
        && cross.cross(n).length() <= 64.0 * f64::EPSILON * scale
        && plane_coordinates(plane, vector(plane.origin)).is_some()
}

fn vector(value: onecad_core::math::Vec3) -> DVec3 {
    DVec3::new(value.x, value.y, value.z)
}
