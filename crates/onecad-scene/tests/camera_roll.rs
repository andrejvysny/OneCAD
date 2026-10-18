mod support;
use onecad_scene::*;
use support::*;

#[test]
fn quarter_roll_rotates_basis_and_capture_with_no_geometry_transform() {
    let original = camera(Projection::Perspective);
    let rolled = Camera {
        roll_radians: std::f64::consts::FRAC_PI_2,
        ..original
    };
    let a = ViewSnapshot::new(
        identity(),
        1,
        original,
        view(Projection::Perspective, 1.0).viewport(),
    )
    .unwrap();
    let b = ViewSnapshot::new(identity(), 1, rolled, a.viewport()).unwrap();
    assert!((b.basis().right - DVec3::Z).length() < 1e-12);
    assert!((b.basis().up + DVec3::X).length() < 1e-12);
    assert_eq!(b.camera().target, a.camera().target);
    assert_eq!(b.camera().offset, a.camera().offset);
    assert_ne!(b.identity(), a.identity());
    let p = b.project(DVec3::new(2.0, 0.0, 1.0)).unwrap().logical;
    assert!((p - DVec2::new(430.0, 360.0)).length() < 1e-9);
}

#[test]
fn rolled_projection_ray_and_matrix_agree_in_both_projections() {
    for projection in [Projection::Perspective, Projection::Orthographic] {
        let c = Camera {
            roll_radians: 0.73,
            ..camera(projection)
        };
        let v = ViewSnapshot::new(identity(), 1, c, view(projection, 2.0).viewport()).unwrap();
        let point = DVec3::new(2.0, 0.0, 1.0);
        let logical = v.project(point).unwrap().logical;
        let ray = v.ray(logical).unwrap();
        let intersect = ray.origin + ray.direction * (-ray.origin.y / ray.direction.y);
        assert!((intersect - point).length() < 1e-9);
        let clip = v.view_projection() * point.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        let matrix_pixel = DVec2::new((ndc.x + 1.0) * 400.0, (1.0 - ndc.y) * 300.0);
        assert!((matrix_pixel - logical).length() < 1e-9);
        assert!((v.basis().right.cross(v.basis().up) - v.basis().back).length() < 1e-12);
    }
}

#[test]
fn invalid_roll_fails_before_capture() {
    let viewport = view(Projection::Perspective, 1.0).viewport();
    for roll_radians in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let c = Camera {
            roll_radians,
            ..Camera::default()
        };
        assert!(c.basis().is_err());
        assert!(ViewSnapshot::new(identity(), 1, c, viewport).is_err());
    }
}

#[test]
fn rolled_visible_pick_uses_the_same_capture_as_projection() {
    let geometry = scene(
        &[[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 0.0, 4.0]],
        &[[0, 1, 2]],
        &[],
    );
    for projection in [Projection::Perspective, Projection::Orthographic] {
        let c = Camera {
            roll_radians: std::f64::consts::FRAC_PI_2,
            ..camera(projection)
        };
        let capture =
            ViewSnapshot::new(identity(), 3, c, view(projection, 2.0).viewport()).unwrap();
        let point = DVec3::new(1.0, 0.0, 1.0);
        let logical = capture.project(point).unwrap().logical;
        let picked = pick(&geometry, &capture, capture.identity(), logical)
            .unwrap()
            .unwrap();
        assert!((picked.world_position - point).length() < 1e-9);
        assert_eq!(picked.capture, capture.identity());
        let unrolled =
            ViewSnapshot::new(identity(), 3, camera(projection), capture.viewport()).unwrap();
        assert!(pick(&geometry, &capture, unrolled.identity(), logical).is_err());
    }
}
