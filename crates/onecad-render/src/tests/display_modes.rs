use super::{
    quality_acceptance::{idle, render},
    *,
};

fn view(scene: &SceneSnapshot) -> ViewSnapshot {
    ViewSnapshot::new(
        scene.identity(),
        1,
        Camera {
            target: DVec3::ZERO,
            offset: DVec3::new(0.0, -100.0, 70.0),
            projection: Projection::Orthographic,
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
fn scene(bodies: Vec<SceneBody>) -> SceneSnapshot {
    SceneSnapshot::new(
        SceneIdentity {
            session: 30,
            scene: 1,
        },
        bodies,
        SceneLimits::default(),
    )
    .unwrap()
}
fn difference(a: &[u8], b: &[u8], area: [usize; 4]) -> usize {
    let [x0, y0, x1, y1] = area;
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (y * 320 + x) * 4))
        .filter(|i| {
            a[*i..*i + 3]
                .iter()
                .zip(&b[*i..*i + 3])
                .any(|(a, b)| a.abs_diff(*b) > 2)
        })
        .count()
}
fn full_image_tolerance(a: &[u8], b: &[u8]) {
    for y in 0..240 {
        for x in 0..320 {
            let i = (y * 320 + x) * 4;
            for channel in 0..3 {
                assert!(
                    a[i + channel].abs_diff(b[i + channel]) <= 2,
                    "order-dependent pixel ({x},{y}) channel {channel}: {} vs {}",
                    a[i + channel],
                    b[i + channel]
                );
            }
        }
    }
}

#[test]
#[ignore = "requires real Metal hardware; missing hardware fails required qualification"]
fn headless_four_display_modes_and_internal_xray_layers() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: None,
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("real Metal GPU required");
    assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
    for features in [
        wgpu::Features::FLOAT32_BLENDABLE,
        required_device_features(adapter.features()),
    ] {
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: features,
            ..Default::default()
        }))
        .unwrap();
        let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let outer = super::analytic_fixture::sphere_body([0.0; 3], 35.0);
        let inner = super::analytic_fixture::sphere_body([3.0, 0.0, 0.0], 20.0);
        let single = scene(vec![outer.clone()]);
        let nested = scene(vec![outer.clone(), inner.clone()]);
        let reverse = scene(vec![inner, outer]);
        let empty = scene(Vec::new());
        let view = view(&single);
        let mut renderer =
            Renderer::new(&device, &queue, [320, 240], RenderLimits::default()).unwrap();
        idle(&device, &mut renderer);
        assert!(
            pollster::block_on(errors.pop()).is_none(),
            "renderer pipeline validation failed"
        );
        let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
        renderer
            .set_options(RenderOptions {
                display_mode: DisplayMode::Wireframe,
                ..Default::default()
            })
            .unwrap();
        renderer.replace_scene(&empty).unwrap();
        let blank = render(
            &mut renderer,
            &device,
            &queue,
            &view,
            RenderStyle::default(),
            "wire-blank",
        );
        renderer.replace_scene(&single).unwrap();
        highlight_replacement_accounting(&mut renderer, &single);
        let geometry = renderer.committed[0].positions.clone();
        eprintln!(
            "silhouette count {}",
            renderer.committed[0].silhouette_count
        );
        let wire = render(
            &mut renderer,
            &device,
            &queue,
            &view,
            RenderStyle::default(),
            "wire-sphere",
        );
        assert_eq!(
            difference(&blank, &wire, [135, 95, 185, 145]),
            0,
            "wireframe must not fill sphere interior"
        );
        eprintln!(
            "changed wire {}",
            difference(&blank, &wire, [0, 0, 320, 240])
        );
        assert!(
            difference(&blank, &wire, [0, 0, 320, 240]) > 100,
            "wire sphere silhouette needs substantial coverage"
        );
        renderer
            .set_options(RenderOptions {
                display_mode: DisplayMode::Shaded,
                ..Default::default()
            })
            .unwrap();
        let shaded = render(
            &mut renderer,
            &device,
            &queue,
            &view,
            RenderStyle::default(),
            "shaded-sphere",
        );
        assert!(
            difference(&wire, &shaded, [100, 60, 220, 180]) > 2000,
            "shaded faces need substantial coverage"
        );
        assert_eq!(
            geometry, renderer.committed[0].positions,
            "mode changes reuse geometry"
        );
        renderer.replace_scene(&nested).unwrap();
        let opaque_nested = render(
            &mut renderer,
            &device,
            &queue,
            &view,
            RenderStyle::default(),
            "opaque-nested",
        );
        full_image_tolerance(&shaded, &opaque_nested);
        renderer
            .set_options(RenderOptions {
                display_mode: DisplayMode::Xray,
                ..Default::default()
            })
            .unwrap();
        let both = render(
            &mut renderer,
            &device,
            &queue,
            &view,
            RenderStyle::default(),
            "xray-nested",
        );
        renderer.replace_scene(&single).unwrap();
        let one = render(
            &mut renderer,
            &device,
            &queue,
            &view,
            RenderStyle::default(),
            "xray-single",
        );
        assert!(
            difference(&one, &both, [120, 80, 200, 160]) > 1000,
            "internal surfaces must contribute in Xray"
        );
        renderer.replace_scene(&reverse).unwrap();
        let reordered = render(
            &mut renderer,
            &device,
            &queue,
            &view,
            RenderStyle::default(),
            "xray-reordered",
        );
        full_image_tolerance(&both, &reordered);
        intersecting_reorder(&mut renderer, &device, &queue, &view);

        for opacity in [0.0, 1.0] {
            renderer
                .set_options(RenderOptions {
                    display_mode: DisplayMode::Xray,
                    xray_opacity: opacity,
                    ..Default::default()
                })
                .unwrap();
            let pixels = render(
                &mut renderer,
                &device,
                &queue,
                &view,
                RenderStyle::default(),
                "xray-endpoint",
            );
            assert!(pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        }
        high_overdraw(&mut renderer, &device, &queue, &view);
        preview_caps(&mut renderer, &device, &queue);
        idle(&device, &mut renderer);
        assert!(pollster::block_on(errors.pop()).is_none());
    }
}
fn high_overdraw(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view: &ViewSnapshot,
) {
    let source = super::analytic_fixture::sphere_body([0.0; 3], 35.0);
    let bodies: Vec<_> = (0..128)
        .map(|_| {
            SceneBody::new(
                BodyId::new(),
                1,
                source.bytes().clone(),
                onecad_scene::MeshLimits::default(),
            )
            .unwrap()
        })
        .collect();
    let selected: Vec<_> = bodies.iter().step_by(2).map(SceneBody::body_id).collect();
    let mut images = Vec::new();
    for reverse in [false, true] {
        let mut ordered = bodies.clone();
        if reverse {
            ordered.reverse();
        }
        renderer.replace_scene(&scene(ordered)).unwrap();
        renderer
            .set_options(RenderOptions {
                display_mode: DisplayMode::Xray,
                ..Default::default()
            })
            .unwrap();
        for id in &selected {
            renderer.set_body_style(*id, BodyStyle { selected: true });
        }
        images.push(render(
            renderer,
            device,
            queue,
            view,
            RenderStyle::default(),
            "xray-high-overdraw",
        ));
    }
    full_image_tolerance(&images[0], &images[1]);
    assert!(images
        .iter()
        .flat_map(|image| image.as_chunks::<4>().0)
        .all(|pixel| pixel[3] == 255));
}

fn preview_caps(renderer: &mut Renderer, device: &wgpu::Device, queue: &wgpu::Queue) {
    renderer.set_options(RenderOptions::default()).unwrap();
    let empty = scene(Vec::new());
    renderer.replace_scene(&empty).unwrap();
    let view = ViewSnapshot::new(
        empty.identity(),
        1,
        Camera {
            offset: DVec3::new(0.0, 0.0, 120.0),
            projection: Projection::Orthographic,
            ..Default::default()
        },
        Viewport {
            width: 320.0,
            height: 240.0,
            scale_factor: 1.0,
        },
    )
    .unwrap();
    let preview = scene(vec![super::analytic_fixture::sphere_body([0.0; 3], 35.0)]);
    let options = RenderOptions {
        display_mode: DisplayMode::Shaded,
        section: [0.0, 0.0, 1.0, 0.0],
        ..Default::default()
    };
    renderer.set_options(options).unwrap();
    let blank = render(
        renderer,
        device,
        queue,
        &view,
        RenderStyle::default(),
        "preview-cap-empty",
    );
    let identity = PreviewIdentity {
        base: empty.identity(),
        generation: 1,
    };
    renderer.replace_preview(identity, &preview).unwrap();
    let filled = render(
        renderer,
        device,
        queue,
        &view,
        RenderStyle::default(),
        "preview-cap-filled",
    );
    let frame = frame::uniform(
        &view,
        None,
        RenderStyle::default(),
        BodyStyle::default(),
        true,
    )
    .unwrap();
    let linear = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let i = (120 * 320 + 160) * 4;
    for channel in 0..3 {
        // One retained back wall and exactly one cap, each at preview opacity 0.18 (wall faces the eye at the probe pixel, so unshaded).
        let expected = linear(blank[i + channel]) * 0.6724 + frame.overlay[channel] * 0.3276;
        assert!(
            (linear(filled[i + channel]) - expected).abs() < 0.015,
            "preview cap must composite once: channel {channel}, got {} expected {expected}",
            linear(filled[i + channel])
        );
    }
    renderer
        .set_options(RenderOptions {
            display_mode: DisplayMode::Wireframe,
            ..options
        })
        .unwrap();
    let wire = render(
        renderer,
        device,
        queue,
        &view,
        RenderStyle::default(),
        "preview-cap-wire",
    );
    assert_eq!(
        difference(&blank, &wire, [145, 105, 175, 135]),
        0,
        "wire preview must have neither caps nor fills"
    );
    renderer.clear_preview().unwrap();
    renderer
        .set_options(RenderOptions {
            display_mode: DisplayMode::Wireframe,
            ..Default::default()
        })
        .unwrap();
    let blank = render(
        renderer,
        device,
        queue,
        &view,
        RenderStyle::default(),
        "preview-silhouette-empty",
    );
    renderer
        .replace_preview(
            PreviewIdentity {
                generation: 2,
                ..identity
            },
            &preview,
        )
        .unwrap();
    assert_eq!(preview.bodies()[0].mesh().unwrap().header().edge_count, 0);
    let wire = render(
        renderer,
        device,
        queue,
        &view,
        RenderStyle::default(),
        "preview-silhouette-wire",
    );
    assert!(
        difference(&blank, &wire, [0, 0, 320, 240]) > 100,
        "smooth preview silhouette must render without topology edges or section contours"
    );
    assert_eq!(
        difference(&blank, &wire, [145, 105, 175, 135]),
        0,
        "smooth wire preview must not fill interior"
    );
}

fn highlight_replacement_accounting(renderer: &mut Renderer, scene: &SceneSnapshot) {
    let baseline = renderer.memory_estimate();
    let body = &scene.bodies()[0];
    let mesh = body.mesh().unwrap();
    let selected = HighlightTarget {
        scene: scene.identity(),
        body_id: body.body_id(),
        mesh_revision: body.mesh_revision(),
        kind: HighlightKind::Face,
        topology_id: mesh.face_id(0).unwrap(),
    };
    let mut previous = None;
    for _ in 0..3 {
        let prepared =
            PreparedHighlights::new(scene, &[selected], None, renderer.preparation_budget())
                .unwrap();
        renderer.replace_highlights(prepared).unwrap();
        let now = renderer.memory_estimate();
        let charge = (now.geometry_bytes, now.resident_metadata_bytes);
        if let Some(previous) = previous {
            assert_eq!(
                previous, charge,
                "highlight replacement must not accumulate discarded CPU boundary payload"
            );
        }
        previous = Some(charge);
    }
    renderer.clear_highlights().unwrap();
    let cleared = renderer.memory_estimate();
    assert_eq!(cleared.geometry_bytes, baseline.geometry_bytes);
    assert_eq!(
        cleared.resident_metadata_bytes,
        baseline.resident_metadata_bytes
    );
}

fn intersecting_reorder(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view: &ViewSnapshot,
) {
    let a = super::analytic_fixture::sphere_body([-12.5, 0.0, 0.0], 35.0);
    let b = super::analytic_fixture::sphere_body([12.5, 0.0, 0.0], 25.0);
    let mut images = Vec::new();
    for bodies in [vec![a.clone(), b.clone()], vec![b, a]] {
        renderer.replace_scene(&scene(bodies)).unwrap();
        images.push(render(
            renderer,
            device,
            queue,
            view,
            RenderStyle::default(),
            "xray-intersecting",
        ));
    }
    full_image_tolerance(&images[0], &images[1]);
}
