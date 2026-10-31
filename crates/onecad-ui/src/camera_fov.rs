//! Perspective field-of-view preference applied to document cameras.

use onecad_scene::Camera;

pub const MIN_FOV_DEGREES: u16 = 15;
pub const MAX_FOV_DEGREES: u16 = 90;
pub const DEFAULT_FOV_DEGREES: u16 = 45;

/// Default camera with the preferred field of view.
pub fn default_camera(fov_degrees: u16) -> Camera {
    let mut camera = Camera::default();
    set_fov(&mut camera, f64::from(fov_degrees));
    camera
}

/// Changes the field of view while keeping the apparent size of the target:
/// the half-height at the target, `|offset| * tan(fov / 2)`, is preserved.
pub fn set_fov(camera: &mut Camera, degrees: f64) {
    if !degrees.is_finite() {
        return;
    }
    let new = degrees.clamp(f64::from(MIN_FOV_DEGREES), f64::from(MAX_FOV_DEGREES));
    let old = camera.vertical_fov_degrees;
    if !old.is_finite() || old <= 0.0 || old >= 180.0 {
        return;
    }
    camera.offset *= (old.to_radians() / 2.0).tan() / (new.to_radians() / 2.0).tan();
    camera.vertical_fov_degrees = new;
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_scene::{DVec3, Projection};

    fn half_height(camera: &Camera) -> f64 {
        camera.offset.length() * (camera.vertical_fov_degrees.to_radians() / 2.0).tan()
    }

    #[test]
    fn apparent_size_at_target_is_unchanged() {
        for new in [15.0, 30.0, 45.0, 76.0, 90.0] {
            let mut camera = Camera::default();
            let before = half_height(&camera);
            set_fov(&mut camera, new);
            assert!((half_height(&camera) - before).abs() < 1e-9, "fov {new}");
            assert_eq!(camera.vertical_fov_degrees, new);
        }
    }

    #[test]
    fn clamps_and_ignores_non_finite() {
        let mut camera = Camera::default();
        set_fov(&mut camera, 5.0);
        assert_eq!(camera.vertical_fov_degrees, 15.0);
        set_fov(&mut camera, 170.0);
        assert_eq!(camera.vertical_fov_degrees, 90.0);
        let before = camera;
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            set_fov(&mut camera, bad);
            assert_eq!(camera, before);
        }
    }

    #[test]
    fn resulting_cameras_stay_valid() {
        for projection in [Projection::Perspective, Projection::Orthographic] {
            for degrees in [15, 45, 90] {
                let mut camera = default_camera(degrees);
                camera.projection = projection;
                camera.target = DVec3::new(10.0, 20.0, 30.0);
                assert!(camera.basis().is_ok());
                assert!(camera.offset.is_finite() && camera.offset.length() > 0.0);
                assert!(camera.vertical_fov_degrees > 0.0 && camera.vertical_fov_degrees < 180.0);
            }
        }
    }

    #[test]
    fn default_camera_uses_preferred_fov() {
        assert_eq!(default_camera(45).vertical_fov_degrees, 45.0);
    }
}
