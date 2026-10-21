use super::*;
use onecad_scene::{Camera, Projection, SceneIdentity, Viewport};

fn view(camera: Camera, scale_factor: f64) -> ViewSnapshot {
    ViewSnapshot::new(
        SceneIdentity {
            session: 1,
            scene: 1,
        },
        1,
        camera,
        Viewport {
            width: 900.0,
            height: 560.0,
            scale_factor,
        },
    )
    .unwrap()
}

fn plane_view(plane: onecad_core::sketch::SketchPlane) -> ViewSnapshot {
    let mut camera = Camera::default();
    crate::sketch_entry::align_camera(&mut camera, plane);
    view(camera, 1.0)
}

fn bounded(ray: &ProjectedRay) {
    for point in [ray.start, ray.end] {
        assert!(point.is_finite());
        assert!((0.0..=900.0).contains(&point.x));
        assert!((0.0..=560.0).contains(&point.y));
    }
}

#[test]
fn principal_positive_rays_reach_viewport_edges_without_negative_segments() {
    for plane in [
        onecad_core::sketch::SketchPlane::xy(),
        onecad_core::sketch::SketchPlane::xz(),
        onecad_core::sketch::SketchPlane::yz(),
    ] {
        let view = plane_view(plane);
        let vector = |v: onecad_core::math::Vec3| DVec3::new(v.x, v.y, v.z);
        let x = project_ray(&view, DVec3::ZERO, vector(plane.x_axis)).unwrap();
        let y = project_ray(&view, DVec3::ZERO, vector(plane.y_axis)).unwrap();
        assert!((x.start - DVec2::new(450.0, 280.0)).length() < 1e-8);
        assert!((x.end - DVec2::new(900.0, 280.0)).length() < 1e-8);
        assert!((y.end - DVec2::new(450.0, 0.0)).length() < 1e-8);
        bounded(&x);
        bounded(&y);
    }
}

#[test]
fn offscreen_origin_keeps_only_visible_positive_ray() {
    let view = plane_view(onecad_core::sketch::SketchPlane::xy());
    let ray = project_ray(&view, DVec3::NEG_Y * 1000.0, DVec3::Y).unwrap();
    assert!(ray.origin.is_none());
    assert!(ray.start.x.abs() < 1e-8 && (ray.end.x - 900.0).abs() < 1e-8);
    bounded(&ray);
    assert!(project_ray(&view, DVec3::NEG_Y * 1000.0, DVec3::NEG_Y).is_none());
}

#[test]
fn tilted_offset_support_preserves_exact_local_origin_and_directions() {
    let plane = onecad_core::sketch::plane_from_point_normal(
        onecad_core::math::Vec3::new_unchecked(42.0, -18.0, 75.0),
        onecad_core::math::Vec3::new_unchecked(0.6, 0.0, 0.8),
    );
    let view = plane_view(plane);
    let vector = |v: onecad_core::math::Vec3| DVec3::new(v.x, v.y, v.z);
    let ray = project_ray(&view, vector(plane.origin), vector(plane.x_axis)).unwrap();
    assert!((ray.origin.unwrap() - DVec2::new(450.0, 280.0)).length() < 1e-8);
    assert!((ray.end - DVec2::new(900.0, 280.0)).length() < 1e-8);
}

#[test]
fn perspective_ray_crossing_near_plane_is_clipped_before_projection() {
    let view = view(Camera::default(), 1.0);
    let origin = view.eye() + view.basis().back * 10.0;
    let direction = -view.basis().back + view.basis().right * 0.1;
    let ray = project_ray(&view, origin, direction).unwrap();
    assert!(ray.origin.is_none());
    assert!((ray.start.x - 900.0).abs() < 1e-6);
    bounded(&ray);
    assert!(project_ray(&view, origin, view.basis().back).is_none());
}

#[test]
fn edge_on_axes_and_retina_remain_finite_in_both_projections() {
    for projection in [Projection::Perspective, Projection::Orthographic] {
        let camera = Camera {
            projection,
            ..Default::default()
        };
        let first = view(camera, 1.0);
        let retina = view(camera, 2.0);
        for axis in [
            first.basis().back,
            first.basis().back + first.basis().right * 1e-10,
        ] {
            let ray = project_ray(&first, DVec3::ZERO, axis).unwrap();
            let scaled = project_ray(&retina, DVec3::ZERO, axis).unwrap();
            bounded(&ray);
            assert!((ray.start - scaled.start).length() < 1e-9);
            assert!((ray.end - scaled.end).length() < 1e-9);
        }
    }
}
