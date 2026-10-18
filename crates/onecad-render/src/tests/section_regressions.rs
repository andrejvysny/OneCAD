use super::{
    quality_acceptance::{render, section_options},
    *,
};

pub(super) fn verify(renderer: &mut Renderer, device: &wgpu::Device, queue: &wgpu::Queue) {
    let scene = super::analytic_fixture::sphere_scene();
    renderer.replace_scene(&scene).unwrap();
    let view = ViewSnapshot::new(
        scene.identity(),
        1,
        Camera {
            target: DVec3::ZERO,
            offset: DVec3::new(0.0, 0.0, 150.0),
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
    verify_cap_alpha(renderer, device, queue, &scene, &view);
    verify_cap_ground_tie(renderer, device, queue, &scene, &view);
    verify_contact_height(renderer, device, queue, &scene, &view);
    verify_nested_caps(renderer, device, queue);
    renderer.set_options(RenderOptions::default()).unwrap();
}

fn verify_cap_alpha(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneSnapshot,
    view: &ViewSnapshot,
) {
    let mut samples = Vec::new();
    for alpha in [0.0, 0.5, 1.0] {
        section_options(
            renderer,
            scene,
            RenderOptions {
                section: [0.0, 0.0, 1.0, 10.0],
                cap: [1.0, 0.0, 0.0, alpha],
                display_mode: DisplayMode::Shaded,
                ..Default::default()
            },
        );
        let pixels = render(
            renderer,
            device,
            queue,
            view,
            RenderStyle::default(),
            &format!("cap-alpha-{alpha}"),
        );
        let index = (120 * 320 + 160) * 4;
        samples.push(
            pixels[index..index + 3]
                .iter()
                .copied()
                .map(linear)
                .collect::<Vec<_>>(),
        );
    }
    assert!(
        samples[0]
            .iter()
            .zip(&samples[2])
            .any(|(a, b)| (a - b).abs() > 0.1),
        "cap center must be visible: {samples:?}"
    );
    for channel in 0..3 {
        let expected = (samples[0][channel] + samples[2][channel]) * 0.5;
        assert!(
            (samples[1][channel] - expected).abs() < 0.015,
            "cap must blend once: channel {channel}, samples {samples:?}"
        );
    }
}

fn linear(value: u8) -> f32 {
    let value = f32::from(value) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn verify_contact_height(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneSnapshot,
    view: &ViewSnapshot,
) {
    section_options(
        renderer,
        scene,
        RenderOptions {
            section: [0.0, 0.0, -1.0, 5.0],
            ..Default::default()
        },
    );
    render(
        renderer,
        device,
        queue,
        view,
        RenderStyle::default(),
        "contact-section-upper-half",
    );
    let frame = frame::uniform(
        view,
        scene.bounds(),
        RenderStyle::default(),
        BodyStyle::default(),
        false,
    )
    .unwrap();
    let clip =
        glam::Mat4::from_cols_array_2d(&frame.shadow_vp) * glam::Vec4::new(0.0, 0.0, -5.0, 1.0);
    let expected = clip.z / clip.w;
    let actual = height_center(device, queue, renderer);
    assert!(
        (actual - expected).abs() < 0.001,
        "section contact height: expected {expected}, got {actual}"
    );
}

fn height_center(device: &wgpu::Device, queue: &wgpu::Queue, renderer: &Renderer) -> f32 {
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test contact height result"),
        size: 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test contact height readback"),
        size: 4,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let pipeline = height_probe_pipeline(device);
    let bind = height_probe_bind(device, &pipeline, renderer, &output);
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 4);
    queue.submit([encoder.finish()]);
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    let mapped = readback.slice(..).get_mapped_range().unwrap();
    f32::from_le_bytes(mapped[..4].try_into().unwrap())
}

fn verify_cap_ground_tie(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneSnapshot,
    view: &ViewSnapshot,
) {
    let mut samples = Vec::new();
    for cap in [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]] {
        section_options(
            renderer,
            scene,
            RenderOptions {
                section: [0.0, 0.0, 1.0, 0.0],
                cap,
                display_mode: DisplayMode::Shaded,
                ..Default::default()
            },
        );
        let pixels = render(
            renderer,
            device,
            queue,
            view,
            RenderStyle::default(),
            "cap-coplanar-ground",
        );
        let index = (120 * 320 + 160) * 4;
        samples.push(pixels[index..index + 3].to_vec());
    }
    assert!(
        samples[0]
            .iter()
            .zip(&samples[1])
            .any(|(a, b)| a.abs_diff(*b) > 32),
        "ground must not overwrite coplanar cap: {samples:?}"
    );
}

fn height_probe_pipeline(device: &wgpu::Device) -> wgpu::ComputePipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("test contact height probe"),
        source: wgpu::ShaderSource::Wgsl("@group(0) @binding(0) var height: texture_depth_2d; @group(0) @binding(1) var<storage,read_write> result: f32; @compute @workgroup_size(1) fn main() { result = textureLoad(height, vec2<i32>(textureDimensions(height) / 2u), 0); }".into()),
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    })
}

fn height_probe_bind(
    device: &wgpu::Device,
    pipeline: &wgpu::ComputePipeline,
    renderer: &Renderer,
    output: &wgpu::Buffer,
) -> wgpu::BindGroup {
    let view = renderer
        .targets
        .as_ref()
        .unwrap()
        .height
        .texture()
        .create_view(&wgpu::TextureViewDescriptor {
            aspect: wgpu::TextureAspect::DepthOnly,
            ..Default::default()
        });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ],
    })
}

fn verify_nested_caps(renderer: &mut Renderer, device: &wgpu::Device, queue: &wgpu::Queue) {
    let outer = super::analytic_fixture::sphere_body([0.0; 3], 35.0);
    let inner = super::analytic_fixture::sphere_body([0.0, 0.0, 10.0], 5.0);
    let scenes = [vec![outer.clone()], vec![outer, inner]];
    let mut centers = Vec::new();
    for (index, bodies) in scenes.into_iter().enumerate() {
        let scene = SceneSnapshot::new(
            SceneIdentity {
                session: 21,
                scene: index as u64,
            },
            bodies,
            SceneLimits::default(),
        )
        .unwrap();
        renderer.replace_scene(&scene).unwrap();
        let view = ViewSnapshot::new(
            scene.identity(),
            1,
            Camera {
                target: DVec3::new(0.0, 0.0, -5.0),
                offset: DVec3::new(0.0, 0.0, -150.0),
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
        section_options(
            renderer,
            &scene,
            RenderOptions {
                section: [0.0, 0.0, -1.0, 5.0],
                cap: [1.0, 0.0, 0.0, 1.0],
                display_mode: DisplayMode::Shaded,
                ..Default::default()
            },
        );
        let pixels = render(
            renderer,
            device,
            queue,
            &view,
            RenderStyle::default(),
            &format!("nested-cap-{index}"),
        );
        let pixel = (120 * 320 + 160) * 4;
        centers.push(pixels[pixel..pixel + 3].to_vec());
        verify_contact_height(renderer, device, queue, &scene, &view);
    }
    assert!(
        centers[0][0] > centers[0][1].saturating_add(32),
        "outer cap must be visible: {centers:?}"
    );
    assert!(
        centers[0]
            .iter()
            .zip(&centers[1])
            .all(|(a, b)| a.abs_diff(*b) <= 2),
        "fully retained nested body must not erase outer cap: {centers:?}"
    );
}
