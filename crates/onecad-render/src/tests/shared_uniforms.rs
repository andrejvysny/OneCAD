use super::{quality_acceptance::idle, *};

fn mixed_scene(count: usize) -> SceneSnapshot {
    let bodies = (0..count)
        .map(|index| {
            analytic_fixture::sphere_body(
                [
                    ((index % 4) as f32 - 1.5) * 32.0,
                    (index / 4) as f32 * 35.0,
                    25.0,
                ],
                19.0,
            )
        })
        .collect();
    SceneSnapshot::new(
        SceneIdentity {
            session: 62,
            scene: 1,
        },
        bodies,
        SceneLimits::default(),
    )
    .unwrap()
}

fn prepare_mixed(renderer: &mut Renderer, scene: &SceneSnapshot) {
    renderer.replace_scene(scene).unwrap();
    renderer.set_body_style(scene.bodies()[0].body_id(), BodyStyle { selected: true });
    let target = |index: usize| HighlightTarget {
        scene: scene.identity(),
        body_id: scene.bodies()[index].body_id(),
        mesh_revision: 1,
        kind: HighlightKind::Face,
        topology_id: "f:1",
    };
    renderer
        .replace_highlights(
            PreparedHighlights::new(
                scene,
                &[target(1)],
                Some(target(2)),
                renderer.preparation_budget(),
            )
            .unwrap(),
        )
        .unwrap();
    let preview = SceneSnapshot::new(
        scene.identity(),
        vec![
            analytic_fixture::sphere_body([-30.0, 26.0, 30.0], 23.0),
            analytic_fixture::sphere_body([24.0, -8.0, 22.0], 22.0),
        ],
        SceneLimits::default(),
    )
    .unwrap();
    renderer
        .replace_preview(
            PreviewIdentity {
                base: scene.identity(),
                generation: 1,
            },
            &preview,
        )
        .unwrap();
}

fn legacy_body_bindings(
    renderer: &mut Renderer,
    view: &ViewSnapshot,
    style: RenderStyle,
) -> Vec<(wgpu::Buffer, wgpu::BindGroup)> {
    let gpu = gpu::Gpu {
        device: &renderer.device,
        queue: &renderer.queue,
    };
    let mut previous = Vec::new();
    for (bodies, preview) in [
        (&mut renderer.committed, false),
        (&mut renderer.preview, true),
    ] {
        for body in bodies {
            previous.push((body.uniform.clone(), body.frame_bind.clone()));
            let mut frame = frame::with_options(
                frame::uniform(view, renderer.committed_bounds, style, body.style, preview)
                    .unwrap(),
                renderer.options,
                renderer.profile,
            );
            if preview {
                frame.display[2] = frame::PREVIEW_HIDDEN_EDGE_ALPHA;
            }
            body.uniform = renderer.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("independent legacy body frame"),
                size: std::mem::size_of::<FrameU>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            body.frame_bind = gpu.bind(
                "independent legacy body frame",
                &renderer.pipelines.frame_layout,
                &[body.uniform.as_entire_binding()],
            );
            renderer
                .queue
                .write_buffer(&body.uniform, 0, bytemuck::bytes_of(&frame));
        }
    }
    previous
}

fn reference_pixels(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view: &ViewSnapshot,
    style: RenderStyle,
) -> Vec<u8> {
    idle(device, renderer);
    let previous = legacy_body_bindings(renderer, view, style);
    let pixels =
        quality_acceptance::render(renderer, device, queue, view, style, "per-body-reference");
    for (body, (uniform, bind)) in renderer
        .committed
        .iter_mut()
        .chain(&mut renderer.preview)
        .zip(previous)
    {
        body.uniform = uniform;
        body.frame_bind = bind;
    }
    pixels
}

fn verify_resident_and_uploads(renderer: &Renderer) {
    let expected = renderer
        .committed
        .iter()
        .chain(&renderer.preview)
        .map(mesh::gpu_bytes)
        .sum::<u64>()
        + limits::shared_uniform_bytes()
        + renderer.highlight_gpu_bytes();
    assert_eq!(renderer.memory_estimate().geometry_bytes, expected);
    assert_eq!(
        renderer.frame_stats(),
        FrameStats {
            uniform_writes: 5,
            uniform_upload_bytes: 5 * std::mem::size_of::<FrameU>() as u64,
        }
    );
    for (bodies, preview) in [(&renderer.committed, false), (&renderer.preview, true)] {
        for body in bodies {
            assert_eq!(
                body.uniform,
                renderer.frame_bindings.body(body.style, preview).buffer
            );
        }
    }
    assert_ne!(renderer.committed[0].uniform, renderer.committed[1].uniform);
    assert_ne!(renderer.committed[1].uniform, renderer.preview[0].uniform);
    assert_eq!(renderer.preview[0].uniform, renderer.preview[1].uniform);
}

fn verify_uniform_budget_and_busy(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    view: &ViewSnapshot,
) {
    let previous = renderer.frame_stats();
    let limit = renderer.limits.max_upload_bytes;
    renderer.limits.max_upload_bytes = previous.uniform_upload_bytes - 1;
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(matches!(
        renderer.encode(&mut encoder, view, RenderStyle::default()),
        Err(RenderError::Budget("frame upload"))
    ));
    assert_eq!(renderer.frame_stats(), previous);
    renderer.limits.max_upload_bytes = limit;
    assert!(renderer
        .encode(&mut encoder, view, RenderStyle::default())
        .unwrap());
    let mut later = device.create_command_encoder(&Default::default());
    assert!(matches!(
        renderer.encode(&mut later, view, RenderStyle::default()),
        Err(RenderError::Busy)
    ));
    assert_eq!(renderer.frame_stats(), previous);
    drop(encoder);
    renderer.abandon_frame(|| {}).unwrap();
    idle(device, renderer);
}

#[test]
#[ignore = "requires real Metal hardware; absent GPU fails qualification"]
fn headless_shared_uniforms_match_per_body_frames_and_remain_bounded() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("real GPU required");
    assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: required_device_features(adapter.features()),
        ..Default::default()
    }))
    .unwrap();
    let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
    for profile in [RenderProfile::Desktop, RenderProfile::Mobile] {
        let mut renderer = Renderer::with_profile(
            &device,
            &queue,
            [320, 240],
            RenderLimits::default(),
            profile,
        )
        .unwrap();
        idle(&device, &mut renderer);
        assert_eq!(
            renderer.memory_estimate().geometry_bytes,
            limits::shared_uniform_bytes()
        );
        for count in [4, 16] {
            let scene = mixed_scene(count);
            prepare_mixed(&mut renderer, &scene);
            for mode in [
                DisplayMode::Shaded,
                DisplayMode::ShadedEdges,
                DisplayMode::Wireframe,
                DisplayMode::Xray,
            ] {
                verify_output(&mut renderer, &device, &queue, &scene, mode);
            }
            verify_style_changes(&mut renderer, &device, &queue, &scene);
        }
    }
    assert!(pollster::block_on(errors.pop()).is_none());
}

fn verify_style_changes(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneSnapshot,
) {
    renderer.clear_preview().unwrap();
    renderer.clear_highlights().unwrap();
    for body in scene.bodies() {
        assert!(renderer.set_body_style(body.body_id(), BodyStyle::default()));
    }
    let view = quality_acceptance::fixture_view(scene, Projection::Orthographic, 1.0);
    let style = RenderStyle::default();
    let normal =
        quality_acceptance::render(renderer, device, queue, &view, style, "normal-only-shared");
    assert_eq!(renderer.frame_stats().uniform_writes, 1);
    for body in &renderer.committed {
        assert_eq!(body.uniform, renderer.frame_bindings.normal.buffer);
    }
    assert_eq!(
        renderer.memory_estimate().geometry_bytes,
        renderer.committed.iter().map(mesh::gpu_bytes).sum::<u64>()
            + limits::shared_uniform_bytes()
    );
    assert!(renderer.set_body_style(scene.bodies()[2].body_id(), BodyStyle { selected: true }));
    let selected = quality_acceptance::render(
        renderer,
        device,
        queue,
        &view,
        style,
        "selection-switched-shared",
    );
    assert_eq!(renderer.frame_stats().uniform_writes, 2);
    assert_ne!(normal, selected);
    assert_eq!(
        selected,
        reference_pixels(renderer, device, queue, &view, style)
    );
}

fn verify_output(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneSnapshot,
    mode: DisplayMode,
) {
    renderer
        .set_options(RenderOptions {
            display_mode: mode,
            ..Default::default()
        })
        .unwrap();
    let view = quality_acceptance::fixture_view(scene, Projection::Perspective, 1.0);
    let style = RenderStyle::default();
    let shared = quality_acceptance::render(renderer, device, queue, &view, style, "shared-styles");
    verify_resident_and_uploads(renderer);
    let reference = reference_pixels(renderer, device, queue, &view, style);
    assert_eq!(shared, reference, "shared uniforms changed {mode:?} output");
    verify_uniform_budget_and_busy(renderer, device, &view);
}
