use super::*;
use crate::{HighlightKind, HighlightTarget, PreparedHighlights};
use onecad_scene::{Camera, DVec3, Projection, SceneBody, SceneLimits, Viewport};

fn scene() -> SceneSnapshot {
    let bytes = include_bytes!(
        "../../../spikes/fixtures/fillet-block/00000000-0000-0000-0000-00000000f101.mesh1"
    );
    let body = SceneBody::new(
        BodyId::new(),
        7,
        Arc::from(bytes.as_slice()),
        onecad_scene::MeshLimits::default(),
    )
    .unwrap();
    SceneSnapshot::new(
        SceneIdentity {
            session: 8,
            scene: 9,
        },
        vec![body],
        SceneLimits::default(),
    )
    .unwrap()
}
fn budget() -> SolidCraftPreparationBudget {
    SolidCraftPreparationBudget {
        limits: RenderLimits::default(),
        resident: MemoryEstimate::default(),
    }
}
#[test]
fn checked_mesh_preserves_winding_normals_z_up_and_topology_ranges() {
    let scene = scene();
    let mesh = scene.bodies()[0].mesh().unwrap();
    let packed = PreparedSolidCraftScene::new(&scene, budget()).unwrap();
    assert_eq!(packed.identity(), scene.identity());
    let bytes = &packed.bodies[0].scene.tris;
    assert_eq!(
        bytes.len(),
        mesh.header().triangle_count as usize * 3 * view::TRI_SIZE
    );
    let tri = mesh.triangle(0).unwrap();
    for (index, vertex) in tri.into_iter().enumerate() {
        let raw = &bytes[index * view::TRI_SIZE..][..view::TRI_SIZE];
        let values: Vec<f32> = raw[..24]
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        assert_eq!(&values[..3], &mesh.position(vertex as usize).unwrap());
        assert_eq!(&values[3..], &mesh.normal(vertex as usize).unwrap());
    }
    let target = HighlightTarget {
        scene: scene.identity(),
        body_id: scene.bodies()[0].body_id(),
        mesh_revision: 7,
        kind: HighlightKind::Face,
        topology_id: mesh.face_id(0).unwrap(),
    };
    let highlights = PreparedHighlights::new(
        &scene,
        &[target],
        None,
        crate::PreparationBudget {
            limits: RenderLimits::default(),
            resident: MemoryEstimate::default(),
            max_buffer: u64::MAX,
        },
    )
    .unwrap();
    let draw = &highlights.draws[0];
    assert_eq!(draw.first, mesh.face_range(0).unwrap().first * 3);
    assert_eq!(draw.count, mesh.face_range(0).unwrap().count * 3);
    let preview = frame::preview(&packed).unwrap();
    assert!(preview.tris.is_empty());
    assert_eq!(preview.trans.len(), bytes.len());
    assert_eq!(preview.trans[27], 170);
}
#[test]
fn preparation_caps_before_packing_and_captured_matrices_match_both_projections() {
    let scene = scene();
    assert!(matches!(
        PreparedSolidCraftScene::new(&scene, budget().limit_cpu_prepared_bytes(1)),
        Err(RenderError::Budget("CPU preparation"))
    ));
    for projection in [Projection::Perspective, Projection::Orthographic] {
        let camera = Camera {
            projection,
            ..Default::default()
        };
        let view = ViewSnapshot::new(
            scene.identity(),
            1,
            camera,
            Viewport {
                width: 301.25,
                height: 200.25,
                scale_factor: 1.5,
            },
        )
        .unwrap();
        let mut frame = frame::empty();
        frame::update(&mut frame, &view, [452, 300], &SolidCraftOptions::default()).unwrap();
        assert_eq!(
            frame.view_proj,
            view.view_projection().as_mat4().to_cols_array_2d()
        );
        assert_eq!(frame.size_px, [452.0, 300.0]);
        assert_eq!(
            frame.eye[3],
            if projection == Projection::Perspective {
                1.0
            } else {
                0.0
            }
        );
        assert!(matches!(
            frame::update(&mut frame, &view, [451, 300], &SolidCraftOptions::default()),
            Err(RenderError::CaptureMismatch)
        ));
    }
}

#[test]
#[ignore = "requires a real GPU; run explicitly with --ignored --nocapture"]
fn headless_full_solidcraft_viewport_shared_device() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: solidcraft_gpu::depth_features(&adapter),
        ..Default::default()
    }))
    .unwrap();
    eprintln!("SolidCraft viewport GPU: {:?}", adapter.get_info());
    let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let scene = scene();
    let mut renderer =
        SolidCraftViewport::new(&device, &queue, [320, 240], RenderLimits::default()).unwrap();
    renderer.replace_scene(&scene).unwrap();
    verify_options(&device, &queue, &scene, &mut renderer);
    verify_access_cache(&device, &queue, &scene, &mut renderer);
    verify_lifecycle(&device, &queue, &scene, &mut renderer);
    wait(&device);
    assert!(pollster::block_on(errors.pop()).is_none());
}
fn captured(scene: &SceneSnapshot, projection: Projection) -> ViewSnapshot {
    let bounds = scene.bounds().unwrap();
    ViewSnapshot::new(
        scene.identity(),
        3,
        Camera {
            target: (bounds.min + bounds.max) * 0.5,
            offset: DVec3::new(90.0, -120.0, 100.0),
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
fn verify_access_cache(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneSnapshot,
    renderer: &mut SolidCraftViewport,
) {
    let options = SolidCraftOptions {
        access: Some(view::AccessMap {
            version: 1,
            size: 1,
            depth: Arc::new(vec![0.0]),
            u: [0.0; 4],
            v: [0.0; 4],
            scale: 1.0,
        }),
        ..Default::default()
    };
    renderer.set_options(options).unwrap();
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer
        .encode(
            &mut encoder,
            &captured(scene, Projection::Perspective),
            RenderStyle::default(),
        )
        .unwrap();
    queue.submit([encoder.finish()]);
    renderer.after_submit(|| {}).unwrap();
    wait(device);
    assert!(renderer.poll_idle().unwrap());
    renderer.set_options(SolidCraftOptions::default()).unwrap();
    assert_eq!(renderer.access_highwater_bytes, 4);
    assert!(renderer.options_payload_bytes(&renderer.options).unwrap() >= 4);
}
fn wait(device: &wgpu::Device) {
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
}
fn verify_options(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneSnapshot,
    renderer: &mut SolidCraftViewport,
) {
    for (index, projection) in [Projection::Perspective, Projection::Orthographic]
        .into_iter()
        .enumerate()
    {
        let view = captured(scene, projection);
        for options in [
            SolidCraftOptions::default(),
            SolidCraftOptions {
                ssaa: 2.0,
                hidden_edges: true,
                section: [0.0, 0.0, 1.0, 8.0],
                ..Default::default()
            },
            SolidCraftOptions {
                opacity: 0.5,
                analysis: Some(solidcraft_render::AnalysisMode::Environment),
                ..Default::default()
            },
        ] {
            renderer.set_options(options).unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            renderer
                .encode(&mut encoder, &view, RenderStyle::default())
                .unwrap();
            queue.submit([encoder.finish()]);
            renderer.after_submit(|| {}).unwrap();
            wait(device);
            assert!(renderer.poll_idle().unwrap());
        }
        renderer.set_options(SolidCraftOptions::default()).unwrap();
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer
            .encode(&mut encoder, &view, RenderStyle::default())
            .unwrap();
        queue.submit([encoder.finish()]);
        renderer.after_submit(|| {}).unwrap();
        capture(device, queue, renderer, index);
        assert!(renderer.poll_idle().unwrap());
    }
}
fn verify_lifecycle(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneSnapshot,
    renderer: &mut SolidCraftViewport,
) {
    let view = captured(scene, Projection::Perspective);
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer
        .encode(&mut encoder, &view, RenderStyle::default())
        .unwrap();
    assert!(matches!(renderer.resize([64, 64]), Err(RenderError::Busy)));
    assert!(matches!(
        renderer.replace_scene(scene),
        Err(RenderError::Busy)
    ));
    assert!(matches!(renderer.clear_preview(), Err(RenderError::Busy)));
    drop(encoder);
    renderer.abandon_frame(|| {}).unwrap();
    wait(device);
    assert!(renderer.poll_idle().unwrap());
    let preview = PreviewIdentity {
        base: scene.identity(),
        generation: 1,
    };
    renderer.replace_preview(preview, scene).unwrap();
    assert_eq!(renderer.preview_identity(), Some(preview));
    renderer.clear_preview().unwrap();
    assert_eq!(
        renderer.memory_estimate().source_bytes,
        scene.resident_bytes() as u64
    );
    assert!(renderer.retired_preview_bytes > 0);
    assert!(matches!(
        renderer.replace_preview(preview, scene),
        Err(RenderError::CaptureMismatch)
    ));
    verify_highlight_lifecycle(device, queue, scene, renderer, &view);
    verify_stale_capture(device, queue, renderer, &view);
}
fn verify_highlight_lifecycle(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneSnapshot,
    renderer: &mut SolidCraftViewport,
    view: &ViewSnapshot,
) {
    let mesh = scene.bodies()[0].mesh().unwrap();
    let target = HighlightTarget {
        scene: scene.identity(),
        body_id: scene.bodies()[0].body_id(),
        mesh_revision: 7,
        kind: HighlightKind::Face,
        topology_id: mesh.face_id(0).unwrap(),
    };
    let highlights = PreparedHighlights::new(
        scene,
        &[target],
        None,
        renderer.highlight_preparation_budget(),
    )
    .unwrap();
    renderer.replace_highlights(highlights).unwrap();
    assert!(renderer.highlight_bytes > 0);
    verify_rejected_changes_preserve_state(renderer, scene);
    renderer.clear_highlights().unwrap();
    assert_eq!(renderer.highlight_bytes, 0);
    assert!(renderer.retired_highlight_bytes > 0);
    renderer.clear_highlights().unwrap();
    verify_unchanged_styles(renderer, scene);
    let mut clear = device.create_command_encoder(&Default::default());
    renderer
        .encode(&mut clear, view, RenderStyle::default())
        .unwrap();
    queue.submit([clear.finish()]);
    renderer.after_submit(|| {}).unwrap();
    wait(device);
    assert!(renderer.poll_idle().unwrap());
    assert_eq!(renderer.retired_preview_bytes, 0);
    assert_eq!(renderer.retired_highlight_bytes, 0);
}
fn verify_stale_capture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut SolidCraftViewport,
    view: &ViewSnapshot,
) {
    let mut stale = view.identity().scene;
    stale.scene += 1;
    let bad = ViewSnapshot::new(stale, 1, view.camera(), view.viewport()).unwrap();
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(matches!(
        renderer.encode(&mut encoder, &bad, RenderStyle::default()),
        Err(RenderError::CaptureMismatch)
    ));
    drop(encoder);
    renderer.resize([0, 0]).unwrap();
    assert!(renderer.output_view().is_none());
    assert!(renderer.resize([u32::MAX, 1]).is_err());
    queue.submit([]);
}
fn verify_rejected_changes_preserve_state(
    renderer: &mut SolidCraftViewport,
    scene: &SceneSnapshot,
) {
    let pending = PreparedSolidCraftScene::new(scene, renderer.preparation_budget()).unwrap();
    let original_limit = renderer.limits.max_cpu_prepared_bytes;
    let geometry = renderer.scene.as_ref().unwrap().bytes;
    let images = renderer.options_payload_bytes(&renderer.options).unwrap();
    let keys = (
        renderer.frame.key,
        renderer.frame.pv_key,
        renderer.frame.hl_key,
    );
    let cap = renderer.options.cap;
    let identity = renderer.preview_identity();
    renderer.limits.max_cpu_prepared_bytes = geometry * 4 + images;
    let options = SolidCraftOptions {
        cap: [1.0, 0.0, 0.0, 1.0],
        ..renderer.options.clone()
    };
    assert!(matches!(
        renderer.set_options(options),
        Err(RenderError::Budget("CPU preparation"))
    ));
    assert_eq!(renderer.options.cap, cap);
    renderer.limits.max_cpu_prepared_bytes = (geometry + pending.bytes) * 4 + images;
    let preview = PreviewIdentity {
        base: scene.identity(),
        generation: 2,
    };
    assert!(matches!(
        renderer.replace_prepared_preview(preview, pending),
        Err(RenderError::Budget("CPU preparation"))
    ));
    assert_eq!(renderer.preview_identity(), identity);
    assert_eq!(
        (
            renderer.frame.key,
            renderer.frame.pv_key,
            renderer.frame.hl_key
        ),
        keys
    );
    renderer.limits.max_cpu_prepared_bytes = original_limit;
}
fn verify_unchanged_styles(renderer: &mut SolidCraftViewport, scene: &SceneSnapshot) {
    let id = scene.bodies()[0].body_id();
    let key = renderer.frame.key;
    assert!(renderer.set_body_style(id, BodyStyle { selected: false }));
    assert_eq!(renderer.frame.key, key);
    assert!(renderer.set_body_style(id, BodyStyle { selected: true }));
    let changed = renderer.frame.key;
    assert_ne!(changed, key);
    assert!(renderer.set_body_style(id, BodyStyle { selected: true }));
    assert_eq!(renderer.frame.key, changed);
}
fn capture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &SolidCraftViewport,
    index: usize,
) {
    let (width, height, stride) = (320, 240, 1280);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("SolidCraft viewport test capture"),
        size: u64::from(stride * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &renderer.target.as_ref().unwrap()._color,
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
    wait(device);
    let data = buffer.slice(..).get_mapped_range().unwrap();
    let pixels: Vec<u8> = data.iter().copied().collect();
    assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
    assert!(pixels.chunks_exact(4).any(|p| p[..3] != pixels[..3]));
    write_capture(&pixels, width, height, index);
}
fn write_capture(pixels: &[u8], width: u32, height: u32, index: usize) {
    let directory = std::path::Path::new("/private/tmp/onecad-solidcraft-viewport-smoke");
    std::fs::create_dir_all(directory).unwrap();
    let file = std::fs::File::create(directory.join(format!("projection-{index}.png"))).unwrap();
    let mut png = png::Encoder::new(file, width, height);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()
        .unwrap()
        .write_image_data(pixels)
        .unwrap();
}
