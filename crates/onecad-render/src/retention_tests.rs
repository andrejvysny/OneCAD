use super::*;
use std::sync::{Arc, Weak};

fn scene() -> SceneSnapshot {
    let bytes = include_bytes!(
        "../../../fixtures/render/fillet-block/00000000-0000-0000-0000-00000000f101.mesh1"
    );
    let body = SceneBody::new(
        BodyId::new(),
        1,
        Arc::from(bytes.as_slice()),
        Default::default(),
    )
    .unwrap();
    SceneSnapshot::new(
        SceneIdentity {
            session: 1,
            scene: 1,
        },
        vec![body],
        Default::default(),
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
fn retention() -> (PreparationRetention, Weak<()>, Weak<()>) {
    let source = Arc::new(());
    let preparation = Arc::new(());
    let weak_source = Arc::downgrade(&source);
    let weak_preparation = Arc::downgrade(&preparation);
    (
        PreparationRetention {
            source: Some(source),
            preparation: Some(preparation),
        },
        weak_source,
        weak_preparation,
    )
}

#[test]
fn reused_geometry_keeps_original_guards_after_cache_replacement() {
    let scene = scene();
    let mut cache = PreparationCache::default();
    let (guard, source, preparation) = retention();
    let first = PreparedScene::new_cached_retained(&scene, budget(), &mut cache, guard).unwrap();
    let (guard, next_source, next_preparation) = retention();
    let second = PreparedScene::new_cached_retained(&scene, budget(), &mut cache, guard).unwrap();
    assert!(Arc::ptr_eq(
        &first.bodies[0].1.geometry,
        &second.bodies[0].1.geometry
    ));
    assert!(next_source.upgrade().is_none()); // Every source allocation was reused.
    drop(first);
    assert!(source.upgrade().is_some());
    assert!(preparation.upgrade().is_some());
    assert!(next_preparation.upgrade().is_some()); // New scene/cache vector metadata.
    let replacement = PreparedScene::new_cached(&self::scene(), budget(), &mut cache).unwrap();
    assert!(source.upgrade().is_some()); // The reused result still owns the old geometry.
    drop(second);
    assert!(source.upgrade().is_none());
    assert!(preparation.upgrade().is_none());
    assert!(next_preparation.upgrade().is_none());
    drop(replacement);
}

#[test]
fn cache_alone_retains_source_and_preparation_after_result_drops() {
    let (guard, source, preparation) = retention();
    let mut cache = PreparationCache::default();
    let prepared =
        PreparedScene::new_cached_retained(&scene(), budget(), &mut cache, guard).unwrap();
    drop(prepared);
    assert!(source.upgrade().is_some());
    assert!(preparation.upgrade().is_some());
    drop(cache);
    assert!(source.upgrade().is_none());
    assert!(preparation.upgrade().is_none());
}

#[test]
fn refused_preparation_releases_incoming_guards_and_preserves_cached_guards() {
    let (guard, source, preparation) = retention();
    let mut cache = PreparationCache::default();
    drop(PreparedScene::new_cached_retained(&scene(), budget(), &mut cache, guard).unwrap());
    let (guard, incoming_source, incoming_preparation) = retention();
    assert!(PreparedScene::new_cached_retained(
        &scene(),
        budget().limit_cpu_prepared_bytes(0),
        &mut cache,
        guard
    )
    .is_err());
    assert!(incoming_source.upgrade().is_none());
    assert!(incoming_preparation.upgrade().is_none());
    assert!(source.upgrade().is_some());
    assert!(preparation.upgrade().is_some());
    drop(cache);
    assert!(source.upgrade().is_none());
    assert!(preparation.upgrade().is_none());
}

#[test]
fn retained_preparation_rebuilds_legacy_cache_geometry_with_missing_guards() {
    let scene = scene();
    for requested in [
        PreparationRetention {
            source: Some(Arc::new(())),
            preparation: None,
        },
        PreparationRetention {
            source: None,
            preparation: Some(Arc::new(())),
        },
    ] {
        let mut cache = PreparationCache::default();
        let legacy = PreparedScene::new_cached(&scene, budget(), &mut cache).unwrap();
        let retained =
            PreparedScene::new_cached_retained(&scene, budget(), &mut cache, requested).unwrap();
        assert!(!Arc::ptr_eq(
            &legacy.bodies[0].1.geometry,
            &retained.bodies[0].1.geometry
        ));
    }
    let mut cache = PreparationCache::default();
    let legacy = PreparedScene::new_cached(&scene, budget(), &mut cache).unwrap();
    let (guard, source, preparation) = retention();
    let retained = PreparedScene::new_cached_retained(&scene, budget(), &mut cache, guard).unwrap();
    drop(legacy);
    drop(retained);
    assert!(source.upgrade().is_some());
    assert!(preparation.upgrade().is_some());
    drop(cache);
    assert!(source.upgrade().is_none());
    assert!(preparation.upgrade().is_none());
}

#[test]
fn legacy_cache_upgrade_admits_rebuild_peak_before_replacing_the_cache() {
    let scene = scene();
    let mut cache = PreparationCache::default();
    let legacy = PreparedScene::new_cached(&scene, budget(), &mut cache).unwrap();
    let reused = PreparedScene::new_cached(&scene, budget(), &mut cache).unwrap();
    let reuse_only = budget().limit_cpu_prepared_bytes(reused.cpu_bytes());
    let (guard, source, preparation) = retention();
    assert!(matches!(
        PreparedScene::new_cached_retained(&scene, reuse_only, &mut cache, guard),
        Err(RenderError::Budget("CPU preparation"))
    ));
    assert!(source.upgrade().is_none());
    assert!(preparation.upgrade().is_none());
    assert!(Arc::ptr_eq(
        &legacy.bodies[0].1.geometry,
        &cache.geometries[0]
    ));
}

fn gpu_renderer() -> Renderer {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: crate::required_device_features(adapter.features()),
        ..Default::default()
    }))
    .unwrap();
    let mut renderer = Renderer::new(&device, &queue, [0, 0], RenderLimits::default()).unwrap();
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    assert!(renderer.poll_idle().unwrap());
    renderer
}
fn highlights(
    scene: &SceneSnapshot,
    budget: PreparationBudget,
) -> (crate::PreparedHighlights, Weak<()>) {
    let body = &scene.bodies()[0];
    let mesh = body.mesh().unwrap();
    let target = crate::HighlightTarget {
        scene: scene.identity(),
        body_id: body.body_id(),
        mesh_revision: body.mesh_revision(),
        kind: crate::HighlightKind::Face,
        topology_id: mesh.face_id(0).unwrap(),
    };
    let guard = Arc::new(());
    let weak = Arc::downgrade(&guard);
    let prepared =
        crate::PreparedHighlights::new_retained(scene, &[target], None, budget, Some(guard))
            .unwrap();
    (prepared, weak)
}

#[test]
fn pending_highlight_output_keeps_admission_until_consumed_or_discarded() {
    let (prepared, guard) = highlights(&scene(), budget());
    assert!(guard.upgrade().is_some());
    drop(prepared);
    assert!(guard.upgrade().is_none());
}

#[test]
#[ignore = "requires a real GPU; run explicitly with ONECAD_REQUIRE_GPU=1"]
fn upload_retains_source_but_releases_prepared_arrays_and_keeps_highlight_metadata_guard() {
    let mut renderer = gpu_renderer();
    let scene = scene();
    renderer.replace_scene(&scene).unwrap(); // Reuse an initially unretained GPU body.
    let mut cache = PreparationCache::default();
    drop(PreparedScene::new_cached(&scene, renderer.preparation_budget(), &mut cache).unwrap());
    let (guard, source, preparation) = retention();
    let prepared = PreparedScene::new_cached_retained(
        &scene,
        renderer.preparation_budget(),
        &mut cache,
        guard,
    )
    .unwrap();
    renderer.replace_prepared_scene(prepared).unwrap();
    drop(cache);
    assert!(source.upgrade().is_some());
    assert!(preparation.upgrade().is_none());
    renderer.replace_scene(&scene).unwrap(); // Legacy callers cannot shed an existing source guard.
    assert!(source.upgrade().is_some());
    let (prepared, highlight) = highlights(&scene, renderer.preparation_budget());
    renderer.replace_highlights(prepared).unwrap();
    assert!(highlight.upgrade().is_some());
    renderer.clear_highlights().unwrap();
    assert!(highlight.upgrade().is_none());
    let empty = SceneSnapshot::new(
        SceneIdentity {
            session: 1,
            scene: 2,
        },
        Vec::new(),
        Default::default(),
    )
    .unwrap();
    renderer
        .replace_prepared_scene(PreparedScene::new(&empty, renderer.preparation_budget()).unwrap())
        .unwrap();
    assert!(source.upgrade().is_none());
}
