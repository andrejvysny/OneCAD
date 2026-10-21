use super::*;

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

/// Test-only readback; the product API never reads pixels back.
fn read_thumbnail(device: &wgpu::Device, queue: &wgpu::Queue, thumb: &Thumbnail) -> Vec<u8> {
    let [width, height] = thumb.size;
    let stride = (width * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test thumbnail readback only"),
        size: u64::from(stride) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &thumb.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
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
    let data = buffer.slice(..).get_mapped_range().unwrap();
    data.chunks(stride as usize)
        .flat_map(|row| row[..width as usize * 4].iter().copied())
        .collect()
}

#[test]
fn thumbnail_fit_keeps_aspect_rounds_down_and_never_upscales() {
    assert_eq!(thumbnail::fit([320, 240], [64, 40]), [53, 40]);
    assert_eq!(thumbnail::fit([240, 320], [64, 40]), [30, 40]);
    assert_eq!(thumbnail::fit([320, 240], [1000, 1000]), [320, 240]);
    assert_eq!(thumbnail::fit([4000, 10], [64, 40]), [64, 1]);
    assert_eq!(thumbnail::fit([320, 240], [0, 0]), [1, 1]);
}

#[test]
#[ignore = "requires real Metal hardware; absent GPU fails qualification"]
fn thumbnail_snapshot_survives_later_renderer_output() {
    let (device, queue) = metal_device();
    let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let scene = fixture_scene(SceneIdentity {
        session: 41,
        scene: 1,
    });
    let mut renderer = Renderer::new(&device, &queue, [320, 240], RenderLimits::default()).unwrap();
    quality_acceptance::idle(&device, &mut renderer);
    renderer.replace_scene(&scene).unwrap();
    let style = RenderStyle::default();

    let first_view = quality_acceptance::fixture_view(&scene, Projection::Perspective, 1.0);
    let first = quality_acceptance::render(
        &mut renderer,
        &device,
        &queue,
        &first_view,
        style,
        "thumbnail-first",
    );
    let thumb = renderer
        .snapshot_thumbnail(&device, &queue, [64, 40])
        .expect("output exists");
    assert_eq!(thumb.size, [53, 40]);
    assert_eq!(thumb.texture.format(), wgpu::TextureFormat::Rgba8UnormSrgb);
    let before = read_thumbnail(&device, &queue, &thumb);
    assert_eq!(before.len(), 53 * 40 * 4);
    assert!(before
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel[..3] != before[..3]));

    let second_view = quality_acceptance::fixture_view(&scene, Projection::Orthographic, 1.0);
    let second = quality_acceptance::render(
        &mut renderer,
        &device,
        &queue,
        &second_view,
        style,
        "thumbnail-second",
    );
    assert_ne!(first, second, "second scene must differ from the first");
    assert_eq!(read_thumbnail(&device, &queue, &thumb), before);
    let later = renderer
        .snapshot_thumbnail(&device, &queue, [64, 40])
        .unwrap();
    assert_ne!(read_thumbnail(&device, &queue, &later), before);

    renderer.resize([0, 0]).unwrap();
    assert!(renderer
        .snapshot_thumbnail(&device, &queue, [64, 40])
        .is_none());
    assert_eq!(read_thumbnail(&device, &queue, &thumb), before);
    assert!(pollster::block_on(errors.pop()).is_none());
}
