use super::*;

#[test]
#[ignore = "requires real Metal hardware; absent GPU fails qualification"]
fn retina_target_replacement_reclaims_only_after_fences() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: false,
        ..Default::default()
    }))
    .expect("real GPU required");
    assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: required_device_features(adapter.features()),
        ..Default::default()
    }))
    .unwrap();
    let mut renderer =
        Renderer::new(&device, &queue, [2560, 1496], RenderLimits::default()).unwrap();
    quality_acceptance::idle(&device, &mut renderer);
    assert!(matches!(
        renderer.resize([2562, 1498]),
        Err(RenderError::Budget(_))
    ));
    assert_eq!(renderer.size(), [2560, 1496]);
    assert!(renderer.output_view().is_some());
    for size in [[2562, 1498], [2560, 1496], [2600, 1520], [2400, 1400]] {
        renderer.resize_reclaiming(size).unwrap();
        assert_eq!(renderer.size(), size);
        assert!(renderer.output_view().is_some());
        assert!(renderer.memory_estimate().peak_total_bytes <= 768 << 20);
    }
    assert!(renderer.resize_reclaiming([4097, 2000]).is_err());
    assert_eq!(renderer.size(), [2400, 1400]);
    renderer.resize_reclaiming([1280, 748]).unwrap();
    renderer.resize([0, 0]).unwrap();
    assert!(renderer.output_view().is_none());
    renderer.resize([640, 400]).unwrap();
    for ssaa in [2.0, 1.0, 2.0, 1.0] {
        renderer
            .set_options_reclaiming(RenderOptions {
                ssaa,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(renderer.options().ssaa, ssaa);
    }
    assert!(RenderError::Allocation("out of memory".into()).recoverable_target_refusal());
    assert!(!RenderError::Device("lost".into()).recoverable_target_refusal());
}

#[test]
#[ignore = "requires real Metal hardware; absent GPU fails qualification"]
fn allocation_failure_after_retirement_preserves_output_and_allows_retry() {
    let (device, queue) = metal_device();
    let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let scene = fixture_scene(SceneIdentity {
        session: 31,
        scene: 1,
    });
    let view = quality_acceptance::fixture_view(&scene, Projection::Perspective, 1.0);
    let mut renderer = Renderer::new(&device, &queue, [320, 240], RenderLimits::default()).unwrap();
    quality_acceptance::idle(&device, &mut renderer);
    renderer.replace_scene(&scene).unwrap();
    let baseline = render_recovery(&mut renderer, &device, &queue, &view);
    let output = renderer.output_view().unwrap().clone();

    targets::tests::fail_next_allocation();
    let error = renderer.resize_reclaiming([322, 242]).unwrap_err();
    assert!(matches!(error, RenderError::Allocation(_)));
    assert!(error.recoverable_target_refusal());
    assert_retained_output(&mut renderer, &device, &queue, &view, &output, &baseline);
    // Same-size retries must rebuild targets that were retired by the failed replacement.
    renderer.resize([320, 240]).unwrap();
    assert!(renderer.has_targets());
    assert!(renderer.retained_output.is_none());
    assert_eq!(
        render_recovery(&mut renderer, &device, &queue, &view),
        baseline
    );

    verify_options_retry(&mut renderer, &device, &queue, &view, &baseline);
    assert!(pollster::block_on(errors.pop()).is_none());
}

fn verify_options_retry(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view: &ViewSnapshot,
    baseline: &[u8],
) {
    let output = renderer.output_view().unwrap().clone();
    let options = RenderOptions {
        ssaa: 2.0,
        ..Default::default()
    };
    for _ in 0..2 {
        targets::tests::fail_next_allocation();
        let error = renderer.set_options_reclaiming(options).unwrap_err();
        assert!(matches!(error, RenderError::Allocation(_)));
        assert_eq!(*renderer.options(), RenderOptions::default());
        assert_retained_output(renderer, device, queue, view, &output, baseline);
    }
    renderer.set_options_reclaiming(options).unwrap();
    assert_eq!(*renderer.options(), options);
    assert!(renderer.has_targets());
    assert!(renderer.retained_output.is_none());
    assert_eq!(
        render_recovery(renderer, device, queue, view).len(),
        baseline.len()
    );
}

fn metal_device() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: false,
        ..Default::default()
    }))
    .expect("real GPU required");
    assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: required_device_features(adapter.features()),
        ..Default::default()
    }))
    .unwrap()
}

fn render_recovery(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view: &ViewSnapshot,
) -> Vec<u8> {
    quality_acceptance::render(
        renderer,
        device,
        queue,
        view,
        RenderStyle::default(),
        "resize-recovery",
    )
}

fn assert_retained_output(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view: &ViewSnapshot,
    output: &wgpu::TextureView,
    baseline: &[u8],
) {
    assert!(!renderer.has_targets());
    assert_eq!(renderer.output_view(), Some(output));
    assert_eq!(renderer.size(), [320, 240]);
    assert_eq!(renderer.memory_estimate().target_bytes, 320 * 240 * 4);
    let retained = renderer.retained_output.as_ref().unwrap();
    assert_eq!(retained.bytes, 320 * 240 * 4);
    assert_eq!(
        read_retained_output(device, queue, &retained._texture),
        baseline
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(!renderer
        .encode(&mut encoder, view, RenderStyle::default())
        .unwrap());
}

fn read_retained_output(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Vec<u8> {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("retained output regression"),
        size: 320 * 240 * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(320 * 4),
                rows_per_image: Some(240),
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    buffer.slice(..).get_mapped_range().unwrap().to_vec()
}
