#[path = "../../../onecad-scene/tests/support/mod.rs"]
mod scene_support;
use super::quality_acceptance::{fixture_view, idle, render};
use super::*;
use onecad_scene::instances::{OccurrenceKey, OccurrencePath};

fn placed(
    definition: &SceneBody,
    id: BodyId,
    occurrence: u128,
    x: f64,
    visible: bool,
) -> SceneBody {
    definition
        .instance(
            id,
            OccurrencePath(vec![OccurrenceKey(occurrence)]),
            onecad_scene::DMat4::from_translation(DVec3::new(x, 0.0, 0.0)),
            visible,
        )
        .unwrap()
}

#[test]
fn preparation_shares_one_geometry_and_admits_placement_metadata() {
    let base = fixture_scene(SceneIdentity {
        session: 71,
        scene: 1,
    });
    let definition = &base.bodies()[0];
    let one = placed(definition, BodyId::new(), 1, -30.0, true);
    let two = placed(definition, BodyId::new(), 2, 30.0, true);
    let scene =
        SceneSnapshot::new(base.identity(), vec![one, two], SceneLimits::default()).unwrap();
    let budget = PreparationBudget {
        limits: RenderLimits::default(),
        resident: MemoryEstimate::default(),
        max_buffer: u64::MAX,
    };
    let prepared = PreparedScene::new(&scene, budget).unwrap();
    assert!(Arc::ptr_eq(
        &prepared.bodies[0].1.geometry,
        &prepared.bodies[1].1.geometry
    ));
    assert_eq!(
        prepared.gpu_bytes,
        mesh::body_estimate(definition, u64::MAX, true).unwrap() + 128
    );
}

#[test]
fn pose_only_preparation_reuses_packed_geometry() {
    let id = SceneIdentity {
        session: 72,
        scene: 1,
    };
    let base = fixture_scene(id);
    let definition = &base.bodies()[0];
    let display = BodyId::new();
    let scene = |identity, x| {
        SceneSnapshot::new(
            identity,
            vec![placed(definition, display, 1, x, true)],
            SceneLimits::default(),
        )
        .unwrap()
    };
    let budget = PreparationBudget {
        limits: RenderLimits::default(),
        resident: MemoryEstimate::default(),
        max_buffer: u64::MAX,
    };
    let mut cache = PreparationCache::default();
    let first = PreparedScene::new_cached(&scene(id, 1.0), budget, &mut cache).unwrap();
    let next = PreparedScene::new_cached(
        &scene(SceneIdentity { scene: 2, ..id }, 10.0),
        budget,
        &mut cache,
    )
    .unwrap();
    assert!(Arc::ptr_eq(
        &first.bodies[0].1.geometry,
        &next.bodies[0].1.geometry
    ));
    assert_ne!(first.bodies[0].1.model, next.bodies[0].1.model);
}

#[test]
fn distant_occurrences_are_refused_before_gpu_upload() {
    let base = fixture_scene(SceneIdentity {
        session: 73,
        scene: 1,
    });
    let definition = &base.bodies()[0];
    let scene = SceneSnapshot::new(
        base.identity(),
        vec![
            placed(definition, BodyId::new(), 1, 1_000_000_000.0, true),
            placed(definition, BodyId::new(), 2, 1_000_000_001.0, true),
        ],
        SceneLimits::default(),
    )
    .unwrap();
    let budget = PreparationBudget {
        limits: RenderLimits::default(),
        resident: MemoryEstimate::default(),
        max_buffer: u64::MAX,
    };
    assert!(matches!(
        PreparedScene::new(&scene, budget),
        Err(RenderError::Mesh(
            "occurrence exceeds 1/1024 mm GPU precision bound"
        ))
    ));
}

#[test]
#[ignore = "requires real Metal hardware; missing hardware fails qualification"]
fn metal_occurrences_share_buffers_and_move_hide_and_highlight_independently() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("Metal adapter required");
    assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: required_device_features(adapter.features()),
        ..Default::default()
    }))
    .unwrap();
    let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut renderer = Renderer::new(&device, &queue, [320, 240], RenderLimits::default()).unwrap();
    idle(&device, &mut renderer);
    let id = SceneIdentity {
        session: 71,
        scene: 1,
    };
    let base = fixture_scene(id);
    let definition = base.bodies()[0].clone();
    let a = BodyId::new();
    let b = BodyId::new();
    let make = |identity, x, visible| {
        SceneSnapshot::new(
            identity,
            vec![
                placed(&definition, a, 1, -30.0, true),
                placed(&definition, b, 2, x, visible),
            ],
            SceneLimits::default(),
        )
        .unwrap()
    };
    let scene = make(id, 30.0, true);
    renderer.replace_scene(&scene).unwrap();
    assert_eq!(
        renderer.committed[0].positions,
        renderer.committed[1].positions
    );
    assert_ne!(
        renderer.committed[0].model_buffer,
        renderer.committed[1].model_buffer
    );
    let geometry = renderer.committed[0].positions.clone();
    let view = fixture_view(&scene, Projection::Perspective, 1.0);
    let all = render(
        &mut renderer,
        &device,
        &queue,
        &view,
        RenderStyle::default(),
        "occurrences-all",
    );
    let topology = scene.bodies()[1]
        .mesh()
        .unwrap()
        .face_id(0)
        .unwrap()
        .to_owned();
    let target = HighlightTarget {
        scene: scene.identity(),
        body_id: b,
        mesh_revision: definition.mesh_revision(),
        kind: HighlightKind::Face,
        topology_id: &topology,
    };
    let highlights =
        PreparedHighlights::new(&scene, &[target], None, renderer.preparation_budget()).unwrap();
    renderer.replace_highlights(highlights).unwrap();
    let selected = render(
        &mut renderer,
        &device,
        &queue,
        &view,
        RenderStyle::default(),
        "occurrence-selected",
    );
    assert_ne!(all, selected);
    let mut section = *renderer.options();
    section.section = [0.0, 0.0, 1.0, 0.0];
    assert!(matches!(
        renderer.set_options(section),
        Err(RenderError::Mesh(_))
    ));
    assert_eq!(renderer.scene_identity(), Some(id));
    let stale = make(id, 50.0, true);
    assert!(matches!(
        renderer.replace_scene(&stale),
        Err(RenderError::CaptureMismatch)
    ));
    assert_eq!(renderer.scene_identity(), Some(id));
    assert_eq!(renderer.committed[0].positions, geometry);
    let heavy = scene_support::body(
        &[[-1.0, 0.0, -1.0], [1.0, 0.0, -1.0], [0.0, 0.0, 1.0]],
        &vec![[0, 1, 2]; 250_001],
        &[],
    );
    let heavy_copies = vec![
        placed(&heavy, BodyId::new(), 3, -10.0, true),
        placed(&heavy, BodyId::new(), 4, 10.0, true),
    ];
    assert!(heavy_copies[0].shares_geometry(&heavy_copies[1]));
    assert!(matches!(
        SceneSnapshot::new(
            SceneIdentity { scene: 4, ..id },
            heavy_copies,
            SceneLimits::default()
        ),
        Err(onecad_scene::SceneError::BudgetExceeded(
            "visible triangles"
        ))
    ));
    let permissive = SceneSnapshot::new(
        SceneIdentity { scene: 5, ..id },
        vec![
            placed(&heavy, BodyId::new(), 5, -10.0, true),
            placed(&heavy, BodyId::new(), 6, 10.0, true),
        ],
        SceneLimits {
            max_triangles: 600_000,
            ..SceneLimits::default()
        },
    )
    .unwrap();
    assert!(matches!(
        renderer.replace_scene(&permissive),
        Err(RenderError::Budget("visible draw work"))
    ));
    assert_eq!(renderer.scene_identity(), Some(id));
    assert_eq!(renderer.committed[0].positions, geometry);
    assert_eq!(renderer.committed[1].positions, geometry);
    let distant = make(SceneIdentity { scene: 3, ..id }, 1_000_000_000.0, true);
    assert!(matches!(
        renderer.replace_scene(&distant),
        Err(RenderError::Mesh(
            "occurrence exceeds 1/1024 mm GPU precision bound"
        ))
    ));
    assert_eq!(renderer.scene_identity(), Some(id));
    assert_eq!(renderer.committed[0].positions, geometry);
    assert_eq!(renderer.committed[1].positions, geometry);
    let next_id = SceneIdentity { scene: 2, ..id };
    let next = make(next_id, 50.0, false);
    let old_limit = renderer.limits.max_upload_bytes;
    renderer.limits.max_upload_bytes = 1;
    assert!(matches!(
        renderer.replace_scene(&next),
        Err(RenderError::Budget(_))
    ));
    assert_eq!(renderer.scene_identity(), Some(id));
    assert_eq!(renderer.committed[0].positions, geometry);
    renderer.limits.max_upload_bytes = old_limit;
    renderer.replace_scene(&next).unwrap();
    assert!(matches!(
        PreparedHighlights::new(&next, &[target], None, renderer.preparation_budget()),
        Err(RenderError::CaptureMismatch)
    ));
    assert_eq!(renderer.committed[0].positions, geometry);
    assert_eq!(renderer.committed[1].positions, geometry);
    assert_eq!(renderer.memory_estimate().upload_bytes, 128);
    let next_view = ViewSnapshot::new(next_id, 2, view.camera(), view.viewport()).unwrap();
    let hidden = render(
        &mut renderer,
        &device,
        &queue,
        &next_view,
        RenderStyle::default(),
        "occurrence-hidden",
    );
    assert_ne!(all, hidden);
    assert!(pollster::block_on(errors.pop()).is_none());
}
