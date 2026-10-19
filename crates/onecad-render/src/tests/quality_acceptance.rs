use super::*;

const PIXEL_CHANNEL_TOLERANCE: u8 = 2;
const MIN_CHANGED_PIXELS: usize = 8;

#[test]
#[ignore = "requires real Metal hardware; CI must run explicitly with --ignored"]
fn headless_quality_feature_acceptance() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: None,
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("real GPU required; missing adapter fails acceptance");
    assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
    let enabled = required_device_features(adapter.features());
    assert!(enabled.contains(wgpu::Features::DEPTH32FLOAT_STENCIL8));
    for features in [wgpu::Features::FLOAT32_BLENDABLE, enabled] {
        run_depth_configuration(&adapter, features);
    }
}

fn run_depth_configuration(adapter: &wgpu::Adapter, features: wgpu::Features) {
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("quality feature acceptance"),
        required_features: features,
        ..Default::default()
    }))
    .unwrap();
    let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let scene = fixture_scene(SceneIdentity {
        session: 18,
        scene: 1,
    });
    let mut renderer = Renderer::new(&device, &queue, [320, 240], RenderLimits::default()).unwrap();
    idle(&device, &mut renderer);
    super::section_regressions::verify(&mut renderer, &device, &queue);
    verify_analytic_outline(&mut renderer, &device, &queue);
    renderer.replace_scene(&scene).unwrap();
    for projection in [Projection::Perspective, Projection::Orthographic] {
        let view = fixture_view(&scene, projection, 1.0);
        for theme in [Theme::Dark, Theme::Light] {
            let style = RenderStyle {
                theme,
                ..Default::default()
            };
            let prefix = format!("{features:?}-{projection:?}-{theme:?}");
            let baseline = render(&mut renderer, &device, &queue, &view, style, &prefix);
            let repeated = render(&mut renderer, &device, &queue, &view, style, &prefix);
            assert_eq!(
                baseline, repeated,
                "same-device repeated capture must be byte exact"
            );
            verify_features(
                &mut renderer,
                &device,
                &queue,
                &scene,
                &view,
                style,
                &baseline,
            );
        }
    }
    verify_flight_guards(&mut renderer, &device, &queue, &scene);
    verify_replacement_and_stale_capture(&mut renderer, &device, &queue, &scene);
    idle(&device, &mut renderer);
    let validation = pollster::block_on(errors.pop());
    assert!(validation.is_none(), "{validation:?}");
}

pub(super) fn fixture_view(
    scene: &SceneSnapshot,
    projection: Projection,
    scale: f64,
) -> ViewSnapshot {
    let bounds = scene.bounds().unwrap();
    ViewSnapshot::new(
        scene.identity(),
        1,
        Camera {
            target: (bounds.min + bounds.max) * 0.5,
            offset: DVec3::new(90.0, -120.0, 100.0),
            projection,
            ..Default::default()
        },
        Viewport {
            width: 320.0,
            height: 240.0,
            scale_factor: scale,
        },
    )
    .unwrap()
}

pub(super) fn idle(device: &wgpu::Device, renderer: &mut Renderer) {
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    assert!(renderer.poll_idle().unwrap());
}

pub(super) fn render(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view: &ViewSnapshot,
    style: RenderStyle,
    name: &str,
) -> Vec<u8> {
    idle(device, renderer);
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(renderer.encode(&mut encoder, view, style).unwrap());
    queue.submit([encoder.finish()]);
    renderer.after_submit(|| {}).unwrap();
    static CAPTURE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = CAPTURE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let pixels = capture(device, queue, renderer, &format!("{sequence:04}-{name}"));
    idle(device, renderer);
    pixels
}

fn assert_feature_changed(before: &[u8], after: &[u8], feature: &str) {
    assert_eq!(before.len(), after.len());
    let changed = before
        .as_chunks::<4>()
        .0
        .iter()
        .zip(after.as_chunks::<4>().0)
        .filter(|(a, b)| {
            a[..3]
                .iter()
                .zip(&b[..3])
                .any(|(x, y)| x.abs_diff(*y) > PIXEL_CHANNEL_TOLERANCE)
        })
        .count();
    assert!(
        changed >= MIN_CHANGED_PIXELS,
        "{feature}: only {changed} changed pixels"
    );
}

fn verify_features(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneSnapshot,
    view: &ViewSnapshot,
    style: RenderStyle,
    baseline: &[u8],
) {
    for (name, options) in [
        (
            "hidden-edges",
            RenderOptions {
                display_mode: DisplayMode::Xray,
                ..Default::default()
            },
        ),
        (
            "opacity",
            RenderOptions {
                display_mode: DisplayMode::Xray,
                xray_opacity: 0.35,
                ..Default::default()
            },
        ),
    ] {
        renderer.set_options(options).unwrap();
        let pixels = render(renderer, device, queue, view, style, name);
        assert_feature_changed(baseline, &pixels, name);
    }
    renderer.set_options(RenderOptions::default()).unwrap();
    verify_highlights(renderer, device, queue, scene, view, style, baseline);
    verify_section(renderer, device, queue, scene, view, style, baseline);
    verify_supersampling(renderer, device, queue, scene, style);
}

fn verify_highlights(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneSnapshot,
    view: &ViewSnapshot,
    style: RenderStyle,
    baseline: &[u8],
) {
    let body = &scene.bodies()[0];
    assert!(renderer.set_body_style(body.body_id(), BodyStyle { selected: true }));
    let selected = render(renderer, device, queue, view, style, "selected-body");
    assert_feature_changed(baseline, &selected, "body selection");
    renderer.set_body_style(body.body_id(), BodyStyle::default());
    let mesh = body.mesh().unwrap();
    let targets: Vec<_> = (0..mesh.header().face_count)
        .map(|index| HighlightTarget {
            scene: scene.identity(),
            body_id: body.body_id(),
            mesh_revision: body.mesh_revision(),
            kind: HighlightKind::Face,
            topology_id: mesh.face_id(index as usize).unwrap(),
        })
        .collect();
    let prepared =
        PreparedHighlights::new(scene, &targets, None, renderer.preparation_budget()).unwrap();
    renderer.replace_highlights(prepared).unwrap();
    let selected = render(renderer, device, queue, view, style, "selected-faces");
    assert_feature_changed(baseline, &selected, "face selection");
    let both = PreparedHighlights::new(
        scene,
        &targets,
        targets.first().copied(),
        renderer.preparation_budget(),
    )
    .unwrap();
    renderer.replace_highlights(both).unwrap();
    let precedence = render(renderer, device, queue, view, style, "selected-hover");
    assert_eq!(
        selected, precedence,
        "matching hover must not double blend selection"
    );
    renderer.clear_highlights().unwrap();
    renderer
        .replace_preview(
            PreviewIdentity {
                base: scene.identity(),
                generation: renderer.latest_preview_generation.unwrap_or(0) + 1,
            },
            scene,
        )
        .unwrap();
    let preview = render(renderer, device, queue, view, style, "preview");
    assert_feature_changed(baseline, &preview, "preview");
    renderer.clear_preview().unwrap();
}

fn verify_section(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneSnapshot,
    view: &ViewSnapshot,
    style: RenderStyle,
    baseline: &[u8],
) {
    let bounds = scene.bounds().unwrap();
    let section = [0.0, 0.0, 1.0, ((bounds.min.z + bounds.max.z) * 0.5) as f32];
    section_options(
        renderer,
        scene,
        RenderOptions {
            section,
            cap: [1.0, 0.0, 0.0, 1.0],
            ..Default::default()
        },
    );
    let red = render(renderer, device, queue, view, style, "section-red");
    assert_feature_changed(baseline, &red, "section clipping");
    section_options(
        renderer,
        scene,
        RenderOptions {
            section,
            cap: [0.0, 1.0, 0.0, 1.0],
            ..Default::default()
        },
    );
    let green = render(renderer, device, queue, view, style, "section-green");
    assert_feature_changed(&red, &green, "section cap color");
    renderer.set_options(RenderOptions::default()).unwrap();
}

fn verify_supersampling(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneSnapshot,
    style: RenderStyle,
) {
    renderer
        .set_options(RenderOptions {
            ssaa: 2.0,
            ..Default::default()
        })
        .unwrap();
    let view = fixture_view(scene, Projection::Perspective, 1.5);
    renderer.resize([480, 360]).unwrap();
    let pixels = render(renderer, device, queue, &view, style, "ssaa-fractional-dpi");
    assert_eq!(pixels.len(), 480 * 360 * 4);
    renderer.resize([0, 0]).unwrap();
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(!renderer.encode(&mut encoder, &view, style).unwrap());
    drop(encoder);
    renderer.set_options(RenderOptions::default()).unwrap();
    renderer.resize([320, 240]).unwrap();
}

fn verify_analytic_outline(renderer: &mut Renderer, device: &wgpu::Device, queue: &wgpu::Queue) {
    let scene = super::analytic_fixture::sphere_scene();
    renderer.replace_scene(&scene).unwrap();
    for projection in [Projection::Perspective, Projection::Orthographic] {
        let view = fixture_view(&scene, projection, 1.0);
        for theme in [Theme::Dark, Theme::Light] {
            {
                let silhouettes = SilhouetteMode::Geometric;
                let style = RenderStyle { theme, silhouettes };
                let outlined = render(renderer, device, queue, &view, style, "sphere-outline");
                renderer
                    .set_options(RenderOptions {
                        display_mode: DisplayMode::Shaded,
                        ..Default::default()
                    })
                    .unwrap();
                let plain = render(renderer, device, queue, &view, style, "sphere-no-outline");
                assert_feature_changed(&outlined, &plain, "smooth sphere outline");
                renderer.set_options(RenderOptions::default()).unwrap();
            }
        }
    }
    verify_sphere_hover(renderer, device, queue, &scene);
}

#[test]
fn display_options_reject_invalid_values_before_allocation() {
    for options in [
        RenderOptions {
            xray_opacity: f32::NAN,
            ..Default::default()
        },
        RenderOptions {
            xray_opacity: -0.1,
            ..Default::default()
        },
        RenderOptions {
            ssaa: 1.5,
            ..Default::default()
        },
        RenderOptions {
            section: [f32::INFINITY; 4],
            ..Default::default()
        },
        RenderOptions {
            cap: [1.1; 4],
            ..Default::default()
        },
    ] {
        assert!(options.validate().is_err());
    }
    let options = RenderOptions {
        ssaa: 2.0,
        ..Default::default()
    };
    assert_eq!(options.render_size([320, 240]).unwrap(), [640, 480]);
    assert!(matches!(
        options.render_size([u32::MAX, 1]),
        Err(RenderError::Budget("target dimension"))
    ));
    assert_eq!(
        required_device_features(wgpu::Features::empty()),
        wgpu::Features::empty()
    );
    assert_eq!(
        required_device_features(wgpu::Features::all()),
        wgpu::Features::DEPTH32FLOAT_STENCIL8 | wgpu::Features::FLOAT32_BLENDABLE
    );
}

fn verify_sphere_hover(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneSnapshot,
) {
    let view = fixture_view(scene, Projection::Perspective, 1.0);
    let baseline = render(
        renderer,
        device,
        queue,
        &view,
        RenderStyle::default(),
        "sphere-no-hover",
    );
    let body = &scene.bodies()[0];
    let target = HighlightTarget {
        scene: scene.identity(),
        body_id: body.body_id(),
        mesh_revision: body.mesh_revision(),
        kind: HighlightKind::Face,
        topology_id: "f:1",
    };
    let prepared =
        PreparedHighlights::new(scene, &[], Some(target), renderer.preparation_budget()).unwrap();
    renderer.replace_highlights(prepared).unwrap();
    let hover = render(
        renderer,
        device,
        queue,
        &view,
        RenderStyle::default(),
        "sphere-hover",
    );
    assert_feature_changed(&baseline, &hover, "hover-only face overlay");
    renderer.clear_highlights().unwrap();
}

/// Tests prepare contours synchronously; the native host uses the bounded worker.
pub(super) fn section_options(
    renderer: &mut Renderer,
    scene: &SceneSnapshot,
    options: RenderOptions,
) {
    let contours =
        PreparedSectionContours::new(scene, renderer.preparation_budget(), options.section)
            .unwrap();
    renderer
        .set_options_with_section_contours(options, contours, None)
        .unwrap();
}
