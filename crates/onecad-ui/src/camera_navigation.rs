use crate::navigation::Motion;
use onecad_scene::{
    CAMERA_MAX_DISTANCE, CAMERA_MIN_DISTANCE, Camera, DVec2, DVec3, SceneIdentity, ViewSnapshot,
    Viewport,
};

const ORBIT_SPEED: f64 = 0.008;
const PAN_SENSITIVITY: f64 = 1.4;
const MAX_PITCH: f64 = std::f64::consts::FRAC_PI_2 - 0.001;

pub fn apply(
    camera: &mut Camera,
    viewport: Viewport,
    scene: SceneIdentity,
    cursor: Option<DVec2>,
    motion: Motion,
) -> bool {
    let Ok(view) = ViewSnapshot::new(scene, 0, *camera, viewport) else {
        return false;
    };
    if cursor.is_some_and(|point| !point.is_finite()) {
        return false;
    }
    let mut candidate = *camera;
    let valid = match motion {
        Motion::Pan(delta) => pan(&mut candidate, &view, delta),
        Motion::Orbit(delta) => orbit(&mut candidate, delta),
        Motion::Zoom(factor) => zoom(&mut candidate, &view, cursor, factor),
    };
    if !valid || candidate == *camera || ViewSnapshot::new(scene, 0, candidate, viewport).is_err() {
        return false;
    }
    *camera = candidate;
    true
}

fn pan(camera: &mut Camera, view: &ViewSnapshot, delta: egui::Vec2) -> bool {
    if !delta.is_finite() || delta == egui::Vec2::ZERO {
        return false;
    }
    let basis = view.basis();
    let scale = camera.offset.length() * PAN_SENSITIVITY / view.viewport().height;
    camera.target += (-basis.right * f64::from(delta.x) + basis.up * f64::from(delta.y)) * scale;
    true
}

fn orbit(camera: &mut Camera, delta: egui::Vec2) -> bool {
    if !delta.is_finite() || delta == egui::Vec2::ZERO {
        return false;
    }
    let distance = camera.offset.length();
    let Ok(basis) = Camera {
        roll_radians: 0.0,
        ..*camera
    }
    .basis() else {
        return false;
    };
    let right = basis.right;
    let yaw = (-right.x).atan2(right.y) - f64::from(delta.x) * ORBIT_SPEED;
    let pitch = ((camera.offset.z / distance).clamp(-1.0, 1.0).asin()
        + f64::from(delta.y) * ORBIT_SPEED)
        .clamp(-MAX_PITCH, MAX_PITCH);
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    let (sin_pitch, cos_pitch) = pitch.sin_cos();
    camera.offset = DVec3::new(cos_yaw * cos_pitch, sin_yaw * cos_pitch, sin_pitch) * distance;
    camera.pole_right = DVec3::new(-sin_yaw, cos_yaw, 0.0);
    true
}

fn zoom(camera: &mut Camera, view: &ViewSnapshot, cursor: Option<DVec2>, factor: f64) -> bool {
    if !factor.is_finite() || factor <= 0.0 {
        return false;
    }
    let distance = camera.offset.length();
    let next = (distance * factor.clamp(0.2, 5.0)).clamp(CAMERA_MIN_DISTANCE, CAMERA_MAX_DISTANCE);
    if next == distance {
        return false;
    }
    let ratio = next / distance;
    if let Some(cursor) = cursor {
        let Some(anchor) = target_plane_anchor(view, cursor) else {
            return false;
        };
        camera.target += (anchor - camera.target) * (1.0 - ratio);
    }
    camera.offset = view.basis().back * next;
    true
}

fn target_plane_anchor(view: &ViewSnapshot, cursor: DVec2) -> Option<DVec3> {
    let ray = view.ray(cursor).ok()?;
    let normal = view.basis().back;
    let denominator = ray.direction.dot(normal);
    if !denominator.is_finite() || denominator.abs() <= f64::EPSILON {
        return None;
    }
    let distance = (view.camera().target - ray.origin).dot(normal) / denominator;
    let anchor = ray.origin + ray.direction * distance;
    (distance.is_finite() && anchor.is_finite()).then_some(anchor)
}

#[cfg(test)]
#[path = "camera_navigation_tests.rs"]
mod tests;
