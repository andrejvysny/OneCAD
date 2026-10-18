use super::{
    quality_acceptance::{fixture_view, idle, render},
    *,
};

#[test]
#[ignore = "requires real Metal hardware and repository NEMA fixture"]
fn headless_quality_nema_admission_capture() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: None,
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("real Metal adapter required");
    assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: required_device_features(adapter.features()),
        ..Default::default()
    }))
    .unwrap();
    let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let limits = RenderLimits {
        max_cpu_prepared_bytes: 64 << 20,
        ..Default::default()
    };
    let mut renderer = Renderer::new(&device, &queue, [320, 240], limits).unwrap();
    idle(&device, &mut renderer);
    let scene = nema_scene();
    let preparation = std::time::Instant::now();
    let prepared = PreparedScene::new(&scene, renderer.preparation_budget()).unwrap();
    eprintln!(
        "NEMA preparation: {} bytes, {:?}",
        prepared.cpu_bytes(),
        preparation.elapsed()
    );
    assert!(prepared.cpu_bytes() <= 64 << 20);
    renderer.replace_prepared_scene(prepared).unwrap();
    let estimate = renderer.memory_estimate();
    eprintln!("NEMA admitted resource accounting: {estimate:?}");
    assert!(estimate.peak_total_bytes <= limits.max_total_bytes);
    assert!(
        estimate.geometry_bytes + estimate.target_bytes + estimate.ibl_bytes
            <= limits.max_gpu_bytes
    );
    for projection in [Projection::Perspective, Projection::Orthographic] {
        let view = fixture_view(&scene, projection, 1.0);
        for theme in [Theme::Dark, Theme::Light] {
            let style = RenderStyle {
                theme,
                ..Default::default()
            };
            let name = format!("NEMA-{projection:?}-{theme:?}");
            let first = render(&mut renderer, &device, &queue, &view, style, &name);
            let second = render(&mut renderer, &device, &queue, &view, style, &name);
            assert_eq!(first, second, "NEMA same-device repeated capture");
        }
    }
    assert_eq!(
        estimate.geometry_bytes,
        renderer.memory_estimate().geometry_bytes
    );
    idle(&device, &mut renderer);
    let validation = pollster::block_on(errors.pop());
    assert!(validation.is_none(), "{validation:?}");
}

fn nema_scene() -> SceneSnapshot {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spikes/fixtures/nema17");
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "mesh1")
        })
        .collect();
    paths.sort();
    let bodies = paths
        .into_iter()
        .map(|path| {
            SceneBody::new(
                BodyId::new(),
                1,
                Arc::from(std::fs::read(path).unwrap()),
                onecad_scene::MeshLimits::default(),
            )
            .unwrap()
        })
        .collect();
    let scene = SceneSnapshot::new(
        SceneIdentity {
            session: 20,
            scene: 1,
        },
        bodies,
        SceneLimits::default(),
    )
    .unwrap();
    assert_eq!(scene.bodies().len(), 67);
    assert_eq!(
        scene
            .bodies()
            .iter()
            .map(|body| u64::from(body.mesh().unwrap().header().triangle_count))
            .sum::<u64>(),
        381_524
    );
    scene
}
