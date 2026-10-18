use super::*;
use onecad_scene::Projection;

fn viewport() -> Viewport {
    Viewport {
        width: 800.0,
        height: 600.0,
        scale_factor: 2.0,
    }
}

fn scene() -> SceneIdentity {
    SceneIdentity {
        session: 7,
        scene: 19,
    }
}

fn camera(projection: Projection) -> Camera {
    Camera {
        target: DVec3::new(2.0, 3.0, 4.0),
        offset: DVec3::new(0.0, -10.0, 0.0),
        projection,
        ..Camera::default()
    }
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
}

fn view(camera: Camera) -> ViewSnapshot {
    ViewSnapshot::new(scene(), 0, camera, viewport()).unwrap()
}

fn assert_anchor(mut camera: Camera, factor: f64, expected_distance: f64) {
    let cursor = DVec2::new(650.0, 180.0);
    let anchor = target_plane_anchor(&view(camera), cursor).unwrap();
    let projection = camera.projection;
    assert!(apply(
        &mut camera,
        viewport(),
        scene(),
        Some(cursor),
        Motion::Zoom(factor),
    ));
    close(camera.offset.length(), expected_distance);
    let projected = view(camera).project(anchor).unwrap().logical;
    close(projected.x, cursor.x);
    close(projected.y, cursor.y);
    assert_eq!(camera.projection, projection);
}

#[test]
fn zoom_preserves_cursor_anchor_for_both_projections() {
    for projection in [Projection::Perspective, Projection::Orthographic] {
        assert_anchor(camera(projection), 0.4, 4.0);
        assert_anchor(camera(projection), 2.0, 20.0);
    }
}

#[test]
fn distance_clamps_use_effective_anchor_ratio() {
    for projection in [Projection::Perspective, Projection::Orthographic] {
        let mut c = camera(projection);
        c.offset = -DVec3::Y * 0.6;
        assert_anchor(c, 0.2, CAMERA_MIN_DISTANCE);
        c.offset = -DVec3::Y * 40_000.0;
        assert_anchor(c, 5.0, CAMERA_MAX_DISTANCE);
    }
}

#[test]
fn center_zoom_keeps_target_and_clamps_factor() {
    for projection in [Projection::Perspective, Projection::Orthographic] {
        for cursor in [None, Some(DVec2::new(400.0, 300.0))] {
            for (factor, distance) in [(0.001, 2.0), (1_000.0, 50.0)] {
                let mut c = camera(projection);
                let target = c.target;
                assert!(apply(
                    &mut c,
                    viewport(),
                    scene(),
                    cursor,
                    Motion::Zoom(factor)
                ));
                close(c.offset.length(), distance);
                assert_eq!(c.target, target);
            }
        }
    }
}

#[test]
fn pan_matches_legacy_direction_scale_and_z_up() {
    let mut c = camera(Projection::Perspective);
    let original = c;
    assert!(apply(
        &mut c,
        viewport(),
        scene(),
        None,
        Motion::Pan(egui::vec2(30.0, 60.0)),
    ));
    close(c.target.x - original.target.x, -0.7);
    close(c.target.y - original.target.y, 0.0);
    close(c.target.z - original.target.z, 1.4);
    assert_eq!(c.offset, original.offset);
    assert_eq!(c.pole_right, original.pole_right);
}

#[test]
fn orbit_matches_direction_and_preserves_distance_and_target() {
    let mut c = camera(Projection::Perspective);
    let original = c;
    assert!(apply(
        &mut c,
        viewport(),
        scene(),
        None,
        Motion::Orbit(egui::vec2(10.0, 20.0)),
    ));
    close(c.offset.x, -0.08_f64.sin() * 0.16_f64.cos() * 10.0);
    close(c.offset.y, -0.08_f64.cos() * 0.16_f64.cos() * 10.0);
    close(c.offset.z, 0.16_f64.sin() * 10.0);
    close(c.offset.length(), original.offset.length());
    assert_eq!(c.target, original.target);
    assert_eq!(c.projection, original.projection);
}

#[test]
fn orbit_caps_pitch_and_retains_pole_heading() {
    for (height, direction) in [(10.0, 1.0), (-10.0, -1.0)] {
        let mut c = Camera {
            offset: DVec3::Z * height,
            pole_right: DVec3::Y,
            ..camera(Projection::Perspective)
        };
        assert!(apply(
            &mut c,
            viewport(),
            scene(),
            None,
            Motion::Orbit(egui::vec2(0.0, direction * 1e8)),
        ));
        close(c.offset.length(), 10.0);
        close(c.offset.z, f64::from(direction) * MAX_PITCH.sin() * 10.0);
        assert!(c.offset.x > 0.0);
        close(c.offset.y, 0.0);
        assert!((c.pole_right - DVec3::Y).length() < 1e-8);
    }
}

#[test]
fn hostile_motion_and_cursor_leave_camera_unchanged() {
    for motion in [
        Motion::Pan(egui::vec2(f32::INFINITY, 1.0)),
        Motion::Orbit(egui::vec2(1.0, f32::NAN)),
        Motion::Zoom(f64::NAN),
        Motion::Zoom(f64::INFINITY),
        Motion::Zoom(0.0),
        Motion::Zoom(-1.0),
    ] {
        let mut c = camera(Projection::Perspective);
        let before = c;
        assert!(!apply(&mut c, viewport(), scene(), None, motion));
        assert_eq!(c, before);
    }
    for cursor in [DVec2::splat(f64::NAN), DVec2::new(801.0, 20.0)] {
        let mut c = camera(Projection::Perspective);
        let before = c;
        assert!(!apply(
            &mut c,
            viewport(),
            scene(),
            Some(cursor),
            Motion::Zoom(0.5)
        ));
        assert_eq!(c, before);
    }
}

#[test]
fn invalid_viewport_and_camera_leave_input_unchanged() {
    for bad_viewport in [
        Viewport {
            height: 0.0,
            ..viewport()
        },
        Viewport {
            width: f64::INFINITY,
            ..viewport()
        },
        Viewport {
            scale_factor: -1.0,
            ..viewport()
        },
    ] {
        let mut c = camera(Projection::Perspective);
        let before = c;
        assert!(!apply(
            &mut c,
            bad_viewport,
            scene(),
            None,
            Motion::Zoom(0.5)
        ));
        assert_eq!(c, before);
    }
    let mut c = Camera {
        offset: DVec3::ZERO,
        ..camera(Projection::Perspective)
    };
    let before = c;
    assert!(!apply(
        &mut c,
        viewport(),
        scene(),
        None,
        Motion::Pan(egui::vec2(1.0, 2.0))
    ));
    assert_eq!(c, before);
}

#[test]
fn finite_extreme_input_never_commits_invalid_view() {
    for motion in [
        Motion::Pan(egui::Vec2::splat(f32::MAX)),
        Motion::Orbit(egui::Vec2::splat(f32::MAX)),
        Motion::Zoom(f64::MAX),
    ] {
        let mut c = camera(Projection::Perspective);
        apply(&mut c, viewport(), scene(), None, motion);
        assert!(ViewSnapshot::new(scene(), 0, c, viewport()).is_ok());
    }
}
