use super::{
    quality_acceptance::{idle, render},
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
        let view = fitted_view(&scene, projection);
        for theme in [Theme::Dark, Theme::Light] {
            let style = RenderStyle {
                theme,
                ..Default::default()
            };
            renderer
                .replace_scene(
                    &SceneSnapshot::new(scene.identity(), Vec::new(), SceneLimits::default())
                        .unwrap(),
                )
                .unwrap();
            let blank = render(&mut renderer, &device, &queue, &view, style, "NEMA-empty");
            renderer.replace_scene(&scene).unwrap();
            for display_mode in [
                DisplayMode::Wireframe,
                DisplayMode::Shaded,
                DisplayMode::ShadedEdges,
                DisplayMode::Xray,
            ] {
                renderer
                    .set_options(RenderOptions {
                        display_mode,
                        ..Default::default()
                    })
                    .unwrap();
                let started = std::time::Instant::now();
                let configuration =
                    capture_artifacts::configuration_name(projection, theme, display_mode);
                let name = format!("nema-{configuration}");
                let first = render(&mut renderer, &device, &queue, &view, style, &name);
                let frame_time = measured_frame(&mut renderer, &device, &queue, &view, style);
                let second = render(&mut renderer, &device, &queue, &view, style, &name);
                assert_eq!(first, second, "NEMA same-device repeated capture");
                if display_mode == DisplayMode::Shaded {
                    let coverage = first
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(blank.as_chunks::<4>().0)
                        .filter(|(a, b)| {
                            a[..3].iter().zip(&b[..3]).any(|(a, b)| a.abs_diff(*b) > 8)
                        })
                        .count();
                    assert!(coverage > 10_000, "NEMA coverage too small: {coverage}");
                    eprintln!("NEMA shaded changed model region {coverage} pixels");
                }
                eprintln!(
                    "{name} warm frame completion {frame_time:?}; two captures plus frame {:?}, memory {:?}",
                    started.elapsed(),
                    renderer.memory_estimate()
                );
            }
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
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/render/nema17");
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

fn fitted_view(scene: &SceneSnapshot, projection: Projection) -> ViewSnapshot {
    let bounds = scene.bounds().unwrap();
    let center = (bounds.min + bounds.max) * 0.5;
    let back = DVec3::new(90.0, -120.0, 100.0).normalize();
    let right = back.cross(DVec3::Z).normalize();
    let up = right.cross(back).normalize();
    let tangent = (onecad_scene::CAMERA_FOV_DEGREES.to_radians() * 0.5).tan();
    let mut distance = 0.0f64;
    // Fit actual vertices instead of empty corners of the assembly bounding box.
    for body in scene.bodies() {
        let mesh = body.mesh().unwrap();
        for index in 0..mesh.header().vertex_count as usize {
            let point = glam::Vec3::from_array(mesh.position(index).unwrap()).as_dvec3() - center;
            let height = point
                .dot(up)
                .abs()
                .max(point.dot(right).abs() / (320.0 / 240.0));
            let depth = if projection == Projection::Perspective {
                point.dot(back)
            } else {
                0.0
            };
            distance = distance.max(depth + height / (tangent * 0.85));
        }
    }
    ViewSnapshot::new(
        scene.identity(),
        1,
        Camera {
            target: (bounds.min + bounds.max) * 0.5,
            offset: back * distance,
            projection,
            ..Default::default()
        },
        Viewport {
            width: 320.0,
            height: 240.0,
            scale_factor: 1.0,
        },
    )
    .unwrap()
}

fn measured_frame(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view: &ViewSnapshot,
    style: RenderStyle,
) -> std::time::Duration {
    let started = std::time::Instant::now();
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(renderer.encode(&mut encoder, view, style).unwrap());
    queue.submit([encoder.finish()]);
    renderer.after_submit(|| {}).unwrap();
    idle(device, renderer);
    started.elapsed()
}
