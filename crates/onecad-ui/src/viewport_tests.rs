use super::*;

#[path = "viewport_fit_tests.rs"]
mod fit_tests;

#[test]
fn acute_profile_fill_preserves_exact_vertices_without_stroke_joins() {
    let points = [
        Some(egui::pos2(100.0, 100.0)),
        Some(egui::pos2(500.0, 100.0)),
        Some(egui::pos2(100.01, 100.001)),
        Some(egui::pos2(100.0, 300.0)),
    ];
    let triangles = [[0, 1, 2], [0, 2, 3]];
    let mesh = profile_fill(&points, &triangles, egui::Color32::LIGHT_BLUE);
    assert_eq!(mesh.indices, [0, 1, 2, 0, 2, 3]);
    assert_eq!(mesh.vertices.len(), points.len());
    for (vertex, point) in mesh.vertices.iter().zip(points) {
        assert_eq!(Some(vertex.pos), point);
    }
    let context = egui::Context::default();
    let mut output = context.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let primitives = context.tessellate(
        vec![egui::epaint::ClippedShape {
            clip_rect: egui::Rect::EVERYTHING,
            shape: egui::Shape::mesh(mesh),
            transform_after_tessellation: Default::default(),
        }],
        1.0,
    );
    let egui::epaint::Primitive::Mesh(tessellated) = &primitives[0].primitive else {
        panic!("fill mesh expected")
    };
    assert!(tessellated.vertices.iter().all(|vertex| {
        vertex.pos.x >= 100.0
            && vertex.pos.x <= 500.0
            && vertex.pos.y >= 100.0
            && vertex.pos.y <= 300.0
    }));
}

#[test]
fn outlined_acute_triangles_reproduce_finish_spike_mechanism() {
    let context = egui::Context::default();
    let mut output = context.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let old = egui::Shape::convex_polygon(
        vec![
            egui::pos2(100.0, 100.0),
            egui::pos2(500.0, 100.0),
            egui::pos2(100.01, 100.001),
        ],
        egui::Color32::LIGHT_BLUE,
        egui::Stroke::new(1.0, egui::Color32::BLUE),
    );
    let primitives = context.tessellate(
        vec![egui::epaint::ClippedShape {
            clip_rect: egui::Rect::EVERYTHING,
            shape: old,
            transform_after_tessellation: Default::default(),
        }],
        1.0,
    );
    let egui::epaint::Primitive::Mesh(mesh) = &primitives[0].primitive else {
        panic!("triangle expected")
    };
    assert!(
        mesh.vertices
            .iter()
            .any(|vertex| vertex.pos.x.abs() > 1000.0 || vertex.pos.y.abs() > 1000.0),
        "acute filled path stroke must reproduce the unbounded miter spike"
    );
}

#[test]
fn clipped_profile_triangles_never_reference_unprojected_vertices() {
    let mesh = profile_fill(
        &[Some(egui::pos2(0.0, 0.0)), None, Some(egui::pos2(1.0, 1.0))],
        &[[0, 1, 2]],
        egui::Color32::WHITE,
    );
    assert!(mesh.indices.is_empty());
}

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

#[test]
fn desktop_renders_at_the_display_scale_and_mobile_caps_large_viewports() {
    use onecad_render::RenderProfile::{Desktop, Mobile};
    let rect = |w: f32, h: f32| egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h));
    for ppp in [1.0, 2.0, 1.5] {
        assert_eq!(
            scale_factor(Desktop, 1.0, rect(1366.0, 1024.0), ppp).0,
            f64::from(ppp)
        );
    }
    assert_eq!(
        scale_factor(Mobile, 1.0, rect(100.0, 100.0), 2.0),
        (2.0, 1.0)
    );
    // 13-inch iPad viewport in points: the scene image keeps the 3 Mpx cap.
    let (factor, scale) = scale_factor(Mobile, 1.0, rect(1366.0, 1024.0), 2.0);
    assert!(scale < 1.0);
    let pixels = 1366.0 * factor * 1024.0 * factor;
    assert!((pixels - 3e6).abs() < 1.0, "{pixels}");
    // A memory-pressure ceiling lowers it further, never below half native.
    assert_eq!(
        scale_factor(Mobile, 0.625, rect(1366.0, 1024.0), 2.0),
        (1.25, 0.625)
    );
    assert_eq!(
        scale_factor(Mobile, 0.1, rect(800.0, 600.0), 2.0),
        (1.0, 0.5)
    );
    assert_eq!(scale_factor(Mobile, 1.0, rect(0.0, 0.0), 2.0), (2.0, 1.0));
}
