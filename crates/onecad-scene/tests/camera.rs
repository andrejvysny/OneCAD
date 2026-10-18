mod support;
use onecad_scene::*;
use support::*;

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}

#[test]
fn z_up_right_handed_projection_and_ray_are_analytical() {
    let v = view(Projection::Perspective, 1.0);
    let p = v.project(DVec3::new(2.0, 0.0, 1.0)).unwrap();
    close(p.logical.x, 460.0);
    close(p.logical.y, 270.0);
    close(p.depth, 10.0);
    assert_eq!(v.basis().right, DVec3::X);
    assert_eq!(v.basis().up, DVec3::Z);
    let ray = v.ray(p.logical).unwrap();
    let at_plane = ray.origin + ray.direction * (10.0 / ray.direction.y);
    assert!((at_plane - DVec3::new(2.0, 0.0, 1.0)).length() < 1e-9);
    let clip = v.view_projection() * DVec3::ZERO.extend(1.0);
    assert!(clip.z / clip.w > 0.0 && clip.z / clip.w < 1.0);
}

#[test]
fn ortho_switch_preserves_pivot_size_and_parallel_rays() {
    let p = view(Projection::Perspective, 1.0);
    let o = view(Projection::Orthographic, 2.0);
    let point = DVec3::new(2.0, 0.0, 1.0);
    assert!(
        (p.project(point).unwrap().logical - o.project(point).unwrap().logical).length() < 1e-9
    );
    assert_eq!(
        o.ray(DVec2::ZERO).unwrap().direction,
        o.ray(DVec2::new(800.0, 600.0)).unwrap().direction
    );
    close(o.world_per_pixel(1.0), o.world_per_pixel(900.0));
    close(p.world_per_pixel(20.0), 2.0 * p.world_per_pixel(10.0));
}

#[test]
fn default_camera_and_pole_heading_match_contract() {
    let c = Camera::default();
    assert_eq!(
        (c.vertical_fov_degrees, c.near, c.far),
        (76.0, 0.1, 100_000.0)
    );
    let top = Camera {
        offset: DVec3::Z * 10.0,
        pole_right: DVec3::Y,
        ..c
    };
    assert_eq!(top.basis().unwrap().right, DVec3::Y);
    assert_eq!(top.basis().unwrap().up, -DVec3::X);
}

#[test]
fn fit_matches_corner_contract_and_safe_work_area() {
    let bounds = Bounds::new(DVec3::splat(-1.0), DVec3::splat(1.0)).unwrap();
    let c = camera(Projection::Perspective);
    let viewport = view(Projection::Perspective, 1.0).viewport();
    let full = fit_camera(bounds, c, viewport, None).unwrap();
    close(full.distance, 2.0 * 1.15);
    let safe = SafeRect {
        x: 400.0,
        y: 0.0,
        width: 400.0,
        height: 600.0,
    };
    let fitted = fit_camera(bounds, c, viewport, Some(safe)).unwrap();
    let c = Camera {
        target: fitted.target,
        offset: c.offset.normalize() * fitted.distance,
        ..c
    };
    let captured = ViewSnapshot::new(identity(), 11, c, viewport).unwrap();
    for corner in bounds.corners() {
        let p = captured.project(corner).unwrap().logical;
        assert!((400.0..=800.0).contains(&p.x) && (0.0..=600.0).contains(&p.y));
    }
    assert!(fit_camera(bounds, c, viewport, Some(SafeRect { width: 0.0, ..safe })).is_err());
    assert!(fit_camera(
        Bounds::new(DVec3::splat(-1e6), DVec3::splat(1e6)).unwrap(),
        c,
        viewport,
        None
    )
    .is_err());
}

#[test]
fn invalid_views_and_projection_clipping_fail_safely() {
    let v = view(Projection::Perspective, 1.0);
    for (width, height, scale_factor) in [
        (0.0, 600.0, 1.0),
        (800.0, f64::NAN, 1.0),
        (800.0, 600.0, -1.0),
        (f64::MAX, 600.0, f64::MAX),
    ] {
        assert!(ViewSnapshot::new(
            identity(),
            11,
            camera(Projection::Perspective),
            Viewport {
                width,
                height,
                scale_factor
            }
        )
        .is_err());
    }
    for c in [
        Camera {
            offset: DVec3::ZERO,
            ..v.camera()
        },
        Camera {
            target: DVec3::splat(f64::INFINITY),
            ..v.camera()
        },
        Camera {
            near: 0.0,
            ..v.camera()
        },
        Camera {
            vertical_fov_degrees: 180.0,
            ..v.camera()
        },
    ] {
        assert!(ViewSnapshot::new(identity(), 11, c, v.viewport()).is_err());
    }
    assert!(v.project(DVec3::new(0.0, -9.95, 0.0)).is_none());
    assert!(v.project(DVec3::new(0.0, 100_001.0, 0.0)).is_none());
    assert!(v.ray(DVec2::new(f64::NAN, 2.0)).is_err());
}
