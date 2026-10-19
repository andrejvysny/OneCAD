use super::*;

#[test]
fn profile_behind_body_interior_is_selectable_only_in_wireframe() {
    use onecad_render::DisplayMode;
    let (scene, view, profile) = box_and_hidden_profile();
    let cursor = DVec2::new(400.0, 300.0);
    let (_, distance) = profile.pick(&view, cursor).unwrap();
    for mode in [
        DisplayMode::Wireframe,
        DisplayMode::Shaded,
        DisplayMode::ShadedEdges,
        DisplayMode::Xray,
    ] {
        let body = onecad_scene::pick_with_policy(
            &scene,
            &view,
            view.identity(),
            cursor,
            pick_policy(mode),
        )
        .unwrap();
        assert_eq!(
            profile_visible(body.as_ref(), distance),
            mode == DisplayMode::Wireframe
        );
    }
}

fn box_and_hidden_profile() -> (
    onecad_scene::SceneSnapshot,
    ViewSnapshot,
    crate::profiles::Profile,
) {
    use onecad_scene::{BodyId, Camera, DVec3, MeshLimits, SceneBody, SceneLimits, SceneSnapshot};
    let body = SceneBody::new(
        BodyId::new(),
        1,
        std::sync::Arc::from(&include_bytes!("../../../fixtures/render/edge-policy/box.mesh1")[..]),
        MeshLimits::default(),
    )
    .unwrap();
    let bounds = body.bounds();
    let center = (bounds.min + bounds.max) * 0.5;
    let scene = SceneSnapshot::new(
        onecad_scene::SceneIdentity {
            session: 1,
            scene: 1,
        },
        vec![body],
        SceneLimits::default(),
    )
    .unwrap();
    let view = ViewSnapshot::new(
        scene.identity(),
        1,
        Camera {
            target: center,
            offset: DVec3::Z * 100.0,
            ..Camera::default()
        },
        Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: 1.0,
        },
    )
    .unwrap();
    let profile = profile_behind_box(center, bounds.min.z - 1.0);
    (scene, view, profile)
}

fn profile_behind_box(center: onecad_scene::DVec3, z: f64) -> crate::profiles::Profile {
    use onecad_app::dto::{FinishSketchDto, PreviewTrianglesDto, SketchRegionDto};
    use onecad_core::{ids::SketchId, math::Vec3, sketch::SketchPlane};
    let plane = SketchPlane {
        origin: Vec3::new(center.x, center.y, z).unwrap(),
        ..SketchPlane::xy()
    };
    crate::profiles::decode_profiles(
        SketchId::new(),
        plane,
        FinishSketchDto {
            region_identity_version: 3,
            diagnostics: vec![],
            regions: vec![SketchRegionDto {
                region_id: "cell-v3:behind-box".into(),
                outer_loop: vec![],
                holes: vec![],
                preview_triangles: Some(PreviewTrianglesDto {
                    positions: vec![-1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0],
                    indices: vec![0, 1, 2, 0, 2, 3],
                    holes_subtracted: 0,
                }),
            }],
        },
    )
    .unwrap()
    .remove(0)
}

#[test]
fn physical_target_rounding_suspends_subpixel_dimensions() {
    let viewport = Viewport {
        width: 0.2,
        height: 600.0,
        scale_factor: 1.25,
    };
    assert_eq!(physical_size(viewport), [0, 750]);
    assert_eq!(
        physical_size(Viewport {
            width: 800.0,
            ..viewport
        }),
        [1000, 750]
    );
}

#[test]
fn drawable_capture_retains_rendered_camera_until_next_submission() {
    let scene = onecad_scene::SceneIdentity {
        session: 1,
        scene: 1,
    };
    let viewport = Viewport {
        width: 800.0,
        height: 600.0,
        scale_factor: 1.25,
    };
    let camera = onecad_scene::Camera::default();
    let view = ViewSnapshot::new(scene, 1, camera, viewport).unwrap();
    let mut pending = camera;
    pending.roll_radians = 0.5;
    let drawable = drawable_capture(Some(view), scene, viewport).unwrap();
    assert_eq!(drawable.camera(), camera);
    assert_ne!(drawable.camera(), pending);
    assert_eq!(drawable.identity(), view.identity());
}

#[test]
fn stale_scene_resize_and_monitor_scale_reject_drawable_actions() {
    let scene = onecad_scene::SceneIdentity {
        session: 1,
        scene: 1,
    };
    let viewport = Viewport {
        width: 800.0,
        height: 600.0,
        scale_factor: 1.25,
    };
    let view = ViewSnapshot::new(scene, 1, onecad_scene::Camera::default(), viewport).unwrap();
    assert!(
        drawable_capture(
            Some(view),
            onecad_scene::SceneIdentity {
                session: 2,
                ..scene
            },
            viewport
        )
        .is_none()
    );
    for next in [
        Viewport {
            width: 801.0,
            ..viewport
        },
        Viewport {
            scale_factor: 2.0,
            ..viewport
        },
    ] {
        assert!(drawable_capture(Some(view), scene, next).is_none());
    }
    assert!(drawable_capture(None, scene, viewport).is_none());
}
