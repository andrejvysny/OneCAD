use crate::{
    highlights::{highlight_budget, HighlightDraw, HighlightTone},
    *,
};
use onecad_scene::{Camera, SceneBody, SceneLimits, Viewport};
use std::sync::Arc;

fn scene() -> onecad_scene::SceneSnapshot {
    let bytes = include_bytes!(
        "../../../spikes/fixtures/fillet-block/00000000-0000-0000-0000-00000000f101.mesh1"
    );
    let body = SceneBody::new(
        onecad_scene::BodyId::new(),
        7,
        Arc::from(bytes.as_slice()),
        onecad_scene::MeshLimits::default(),
    )
    .unwrap();
    onecad_scene::SceneSnapshot::new(
        onecad_scene::SceneIdentity {
            session: 4,
            scene: 8,
        },
        vec![body],
        SceneLimits::default(),
    )
    .unwrap()
}
fn budget() -> PreparationBudget {
    PreparationBudget {
        limits: RenderLimits::default(),
        resident: MemoryEstimate::default(),
        max_buffer: u64::MAX,
    }
}
fn target<'a>(
    scene: &onecad_scene::SceneSnapshot,
    id: &'a str,
    kind: HighlightKind,
) -> HighlightTarget<'a> {
    HighlightTarget {
        scene: scene.identity(),
        body_id: scene.bodies()[0].body_id(),
        mesh_revision: 7,
        kind,
        topology_id: id,
    }
}

#[test]
fn checked_faces_use_index_units_edges_use_segment_units() {
    let scene = scene();
    let mesh = scene.bodies()[0].mesh().unwrap();
    let face = target(&scene, mesh.face_id(0).unwrap(), HighlightKind::Face);
    let edge = target(&scene, mesh.edge_id(1).unwrap(), HighlightKind::Edge);
    let prepared = PreparedHighlights::new(&scene, &[face], Some(edge), budget()).unwrap();
    let face_span = mesh.face_range(0).unwrap();
    assert_eq!(prepared.draws[0].first, face_span.first * 3);
    assert_eq!(prepared.draws[0].count, face_span.count * 3);
    assert_eq!(
        prepared.draws[1].first,
        mesh.edge_range(0).unwrap().count - 1
    );
    assert_eq!(
        prepared.draws[1].count,
        mesh.edge_range(1).unwrap().count - 1
    );
    assert_eq!(prepared.draws[1].mesh_revision, 7);
}

#[test]
fn selected_wins_hover_duplicate_targets_never_double_blend() {
    let scene = scene();
    let mesh = scene.bodies()[0].mesh().unwrap();
    let face = target(&scene, mesh.face_id(0).unwrap(), HighlightKind::Face);
    let prepared = PreparedHighlights::new(&scene, &[face, face], Some(face), budget()).unwrap();
    assert_eq!(prepared.len(), 1);
    assert_eq!(prepared.draws[0].tone, HighlightTone::Selected);
    let hover = PreparedHighlights::new(&scene, &[], Some(face), budget()).unwrap();
    assert_eq!(hover.draws[0].tone, HighlightTone::Hover);
}

#[test]
fn stale_scene_revision_and_unknown_ids_fail_closed() {
    let scene = scene();
    let mesh = scene.bodies()[0].mesh().unwrap();
    let face = target(&scene, mesh.face_id(0).unwrap(), HighlightKind::Face);
    let mut stale = face;
    stale.scene.scene += 1;
    assert!(matches!(
        PreparedHighlights::new(&scene, &[stale], None, budget()),
        Err(RenderError::CaptureMismatch)
    ));
    stale = face;
    stale.mesh_revision += 1;
    assert!(matches!(
        PreparedHighlights::new(&scene, &[stale], None, budget()),
        Err(RenderError::CaptureMismatch)
    ));
    let unknown = HighlightTarget {
        topology_id: "not-a-mesh-ID",
        ..face
    };
    assert!(matches!(
        PreparedHighlights::new(&scene, &[unknown], None, budget()),
        Err(RenderError::Highlight("unknown topology ID"))
    ));
    let unknown = HighlightTarget {
        body_id: onecad_scene::BodyId::new(),
        ..face
    };
    assert!(matches!(
        PreparedHighlights::new(&scene, &[unknown], None, budget()),
        Err(RenderError::Highlight("unknown body"))
    ));
    let prepared = PreparedHighlights::new(&scene, &[face], None, budget()).unwrap();
    assert!(prepared
        .validate_scene(Some(onecad_scene::SceneIdentity {
            session: 5,
            ..scene.identity()
        }))
        .is_err());
    assert!(crate::highlight_prepare::face_indices(&mesh, usize::MAX).is_err());
    assert!(crate::highlight_prepare::edge_segments(&mesh, usize::MAX).is_err());
}

#[test]
fn invalid_resolved_ranges_and_changed_revisions_are_refused() {
    let scene = scene();
    let draw = HighlightDraw {
        body_id: scene.bodies()[0].body_id(),
        body_index: 0,
        mesh_revision: 7,
        kind: HighlightKind::Face,
        first: 0,
        count: 3,
        tone: HighlightTone::Selected,
    };
    assert!(draw.validate(7, 3, 0).is_ok());
    assert!(matches!(
        draw.validate(8, 3, 0),
        Err(RenderError::CaptureMismatch)
    ));
    assert!(HighlightDraw { first: 1, ..draw }
        .validate(7, 6, 0)
        .is_err());
    assert!(HighlightDraw {
        first: u32::MAX,
        count: 3,
        ..draw
    }
    .validate(7, u32::MAX, 0)
    .is_err());
    assert!(HighlightDraw { count: 0, ..draw }
        .validate(7, 3, 0)
        .is_err());
    assert!(HighlightDraw {
        kind: HighlightKind::Edge,
        count: 4,
        ..draw
    }
    .validate(7, 30, 3)
    .is_err());
}

#[test]
fn preparation_and_old_new_uniform_residency_obey_existing_caps() {
    let scene = scene();
    let mesh = scene.bodies()[0].mesh().unwrap();
    let face = target(&scene, mesh.face_id(0).unwrap(), HighlightKind::Face);
    let limits = RenderLimits {
        max_highlights: 1,
        ..Default::default()
    };
    assert!(matches!(
        PreparedHighlights::new(
            &scene,
            &[face],
            Some(face),
            PreparationBudget { limits, ..budget() }
        ),
        Err(RenderError::Budget("highlight count"))
    ));
    let limits = RenderLimits {
        max_cpu_prepared_bytes: 1,
        ..Default::default()
    };
    assert!(matches!(
        PreparedHighlights::new(
            &scene,
            &[face],
            None,
            PreparationBudget { limits, ..budget() }
        ),
        Err(RenderError::Budget("CPU preparation"))
    ));
    let uniforms = 2 * std::mem::size_of::<crate::FrameU>() as u64;
    let resident = MemoryEstimate {
        geometry_bytes: uniforms,
        ..Default::default()
    };
    let limits = RenderLimits {
        max_gpu_bytes: uniforms * 2 - 1,
        ..Default::default()
    };
    assert!(matches!(
        highlight_budget(
            PreparationBudget {
                limits,
                resident,
                ..budget()
            },
            64,
            uniforms
        ),
        Err(RenderError::Budget("GPU replacement overlap"))
    ));
    let limits = RenderLimits {
        max_upload_bytes: uniforms - 1,
        ..Default::default()
    };
    assert!(matches!(
        highlight_budget(PreparationBudget { limits, ..budget() }, 64, uniforms),
        Err(RenderError::Budget("upload staging"))
    ));
    let estimate = highlight_budget(budget(), 64, uniforms).unwrap();
    assert_eq!(estimate.peak_total_bytes, uniforms * 2 + 128);
}

#[test]
fn face_edge_styles_keep_onecad_opacity_and_reversed_depth_rules() {
    let scene = scene();
    let view = onecad_scene::ViewSnapshot::new(
        scene.identity(),
        1,
        Camera::default(),
        Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 2.0,
        },
    )
    .unwrap();
    for theme in [Theme::Light, Theme::Dark] {
        let style = RenderStyle {
            theme,
            ..Default::default()
        };
        let base = crate::frame::uniform(&view, None, style, BodyStyle::default(), false).unwrap();
        let selected =
            crate::frame::highlight_uniform(base, &view, style, HighlightTone::Selected).unwrap();
        let hover =
            crate::frame::highlight_uniform(base, &view, style, HighlightTone::Hover).unwrap();
        assert_eq!(selected.overlay[3], 0.55);
        assert_eq!(hover.overlay[3], 0.45);
        assert_eq!(hover.overlay[..3], hover.edge[..3]);
        assert_ne!(selected.overlay[..3], hover.overlay[..3]);
        assert_eq!(selected.lines[0], 6.0);
        assert_eq!(hover.lines[0], 6.0);
    }
    let face = crate::pipelines::highlight_depth().unwrap();
    assert_eq!(face.depth_write_enabled, Some(false));
    assert_eq!(
        face.depth_compare,
        Some(wgpu::CompareFunction::GreaterEqual)
    );
    assert_eq!(face.bias.constant, 1);
    assert_eq!(face.bias.slope_scale, 1.0);
    let edge = crate::pipelines::highlight_line_depth().unwrap();
    assert_eq!(edge.depth_write_enabled, Some(false));
    assert_eq!(edge.depth_compare, Some(wgpu::CompareFunction::Always));
}

#[test]
fn highlight_shader_entry_validates_without_device_or_capture() {
    let source = [
        include_str!("shaders/common.wgsl"),
        include_str!("shaders/body.wgsl"),
    ]
    .concat();
    let module = naga::front::wgsl::parse_str(&source).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    assert!(module
        .entry_points
        .iter()
        .any(|entry| entry.name == "fs_highlight" && entry.stage == naga::ShaderStage::Fragment));
}
