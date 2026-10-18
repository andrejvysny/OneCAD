use super::*;
use onecad_scene::{Camera, DVec3, Projection, SceneBody, SceneLimits, Viewport};
use std::sync::Arc;

fn fixture_scene(identity: SceneIdentity) -> SceneSnapshot {
    let bytes = include_bytes!(
        "../../../spikes/fixtures/fillet-block/00000000-0000-0000-0000-00000000f101.mesh1"
    );
    let body = SceneBody::new(
        BodyId::new(),
        1,
        Arc::from(bytes.as_slice()),
        onecad_scene::MeshLimits::default(),
    )
    .unwrap();
    SceneSnapshot::new(identity, vec![body], SceneLimits::default()).unwrap()
}

#[test]
fn checked_fixture_preparation_preserves_z_up_positions_and_count() {
    let scene = fixture_scene(SceneIdentity {
        session: 1,
        scene: 1,
    });
    let body = &scene.bodies()[0];
    let expected = body.mesh().unwrap().position(0).unwrap();
    let bytes = mesh::estimate(&scene, u64::MAX).unwrap();
    assert!(bytes > 0);
    let prepared = mesh::PreparedBody::new(body).unwrap();
    assert_eq!(prepared.first_position(), Some(expected));
    assert!(mesh::estimate(&scene, 1).is_err());
}

#[test]
fn frame_uses_captured_projection_and_scale_for_both_projections() {
    let identity = SceneIdentity {
        session: 2,
        scene: 3,
    };
    for projection in [Projection::Perspective, Projection::Orthographic] {
        let camera = Camera {
            projection,
            near: 0.25,
            far: 5000.0,
            ..Default::default()
        };
        let view = ViewSnapshot::new(
            identity,
            7,
            camera,
            Viewport {
                width: 300.0,
                height: 200.0,
                scale_factor: 2.0,
            },
        )
        .unwrap();
        let frame = frame::uniform(
            &view,
            None,
            RenderStyle::default(),
            BodyStyle::default(),
            false,
        )
        .unwrap();
        assert_eq!(
            frame.view_proj,
            view.view_projection().as_mat4().to_cols_array_2d()
        );
        assert_eq!(
            frame.inverse_projection,
            view.projection_matrix()
                .inverse()
                .as_mat4()
                .to_cols_array_2d()
        );
        assert_eq!(frame.viewport, [600.0, 400.0, 1.0 / 600.0, 1.0 / 400.0]);
        assert_eq!(
            frame.eye[3],
            if projection == Projection::Perspective {
                1.0
            } else {
                0.0
            }
        );
    }
}

#[test]
#[ignore = "requires a real headless GPU; run explicitly with --ignored --nocapture"]
fn headless_quality_shared_device_capture() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: None,
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("onecad-render test host"),
        ..Default::default()
    }))
    .unwrap();
    eprintln!("headless adapter: {:?}", adapter.get_info());
    let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let scene = fixture_scene(SceneIdentity {
        session: 1,
        scene: 2,
    });
    let mut renderer = Renderer::new(&device, &queue, [320, 240], RenderLimits::default()).unwrap();
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    assert!(renderer.poll_idle().unwrap());
    renderer.replace_scene(&scene).unwrap();
    let bounds = scene.bounds().unwrap();
    let center = (bounds.min + bounds.max) * 0.5;
    for (index, projection) in [Projection::Perspective, Projection::Orthographic]
        .into_iter()
        .enumerate()
    {
        let camera = Camera {
            target: center,
            offset: DVec3::new(90.0, -120.0, 100.0),
            projection,
            ..Default::default()
        };
        let view = ViewSnapshot::new(
            scene.identity(),
            index as u64,
            camera,
            Viewport {
                width: 320.0,
                height: 240.0,
                scale_factor: 1.0,
            },
        )
        .unwrap();
        for theme in [Theme::Dark, Theme::Light] {
            let mut encoder = device.create_command_encoder(&Default::default());
            assert!(renderer
                .encode(
                    &mut encoder,
                    &view,
                    RenderStyle {
                        theme,
                        ..Default::default()
                    }
                )
                .unwrap());
            queue.submit([encoder.finish()]);
            renderer.after_submit(|| {}).unwrap();
            capture(
                &device,
                &queue,
                &renderer,
                &format!("{projection:?}-{theme:?}"),
            );
        }
    }
    verify_flight_guards(&mut renderer, &device, &queue, &scene);
    verify_replacement_and_stale_capture(&mut renderer, &device, &queue, &scene);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    let validation = pollster::block_on(errors.pop());
    assert!(validation.is_none(), "{validation:?}");
}

fn verify_replacement_and_stale_capture(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneSnapshot,
) {
    let view = ViewSnapshot::new(
        SceneIdentity {
            session: 9,
            scene: 9,
        },
        1,
        Camera::default(),
        Viewport {
            width: 320.0,
            height: 240.0,
            scale_factor: 1.0,
        },
    )
    .unwrap();
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(matches!(
        renderer.encode(&mut encoder, &view, RenderStyle::default()),
        Err(RenderError::CaptureMismatch)
    ));
    drop(encoder);
    let identity = PreviewIdentity {
        base: scene.identity(),
        generation: 1,
    };
    renderer.replace_preview(identity, scene).unwrap();
    assert_eq!(renderer.preview_identity(), Some(identity));
    assert_eq!(
        renderer.memory_estimate().source_bytes,
        2 * scene.resident_bytes() as u64
    );
    renderer.clear_preview().unwrap();
    assert_eq!(
        renderer.memory_estimate().source_bytes,
        scene.resident_bytes() as u64
    );
    assert!(matches!(
        renderer.replace_preview(identity, scene),
        Err(RenderError::CaptureMismatch)
    ));
    assert_eq!(renderer.scene_identity(), Some(scene.identity()));
    renderer.resize([0, 0]).unwrap();
    assert!(renderer.output_view().is_none());
    renderer.resize([64, 64]).unwrap();
    assert!(renderer.output_view().is_some());
    assert!(renderer.resize([u32::MAX, 1]).is_err());
    assert_eq!(renderer.size(), [64, 64]);
    queue.submit([]);
}

fn capture(device: &wgpu::Device, queue: &wgpu::Queue, renderer: &Renderer, name: &str) {
    let [width, height] = renderer.size();
    let stride = (width * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test capture only"),
        size: u64::from(stride) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &renderer.targets.as_ref().unwrap()._output,
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
    let pixels: Vec<u8> = data
        .chunks(stride as usize)
        .flat_map(|row| row[..width as usize * 4].iter().copied())
        .collect();
    assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
    assert!(pixels
        .chunks_exact(4)
        .any(|pixel| pixel[..3] != pixels[..3]));
    let directory = std::path::Path::new("/private/tmp/onecad-render-quality-smoke");
    std::fs::create_dir_all(directory).unwrap();
    let file = std::fs::File::create(directory.join(format!("{name}.png"))).unwrap();
    let mut png = png::Encoder::new(file, width, height);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()
        .unwrap()
        .write_image_data(&pixels)
        .unwrap();
}

#[test]
fn fractional_viewport_uses_exact_rounded_target_texels() {
    let view = ViewSnapshot::new(
        SceneIdentity {
            session: 1,
            scene: 2,
        },
        3,
        Camera::default(),
        Viewport {
            width: 301.25,
            height: 200.25,
            scale_factor: 1.5,
        },
    )
    .unwrap();
    let frame = frame::uniform(
        &view,
        None,
        RenderStyle::default(),
        BodyStyle::default(),
        false,
    )
    .unwrap();
    assert_eq!(frame.viewport, [452.0, 300.0, 1.0 / 452.0, 1.0 / 300.0]);
}

#[test]
fn preparation_budget_rejects_before_packing_and_silhouettes_are_deterministic() {
    let scene = fixture_scene(SceneIdentity {
        session: 1,
        scene: 1,
    });
    let budget = PreparationBudget {
        limits: RenderLimits {
            max_cpu_prepared_bytes: 1,
            ..Default::default()
        },
        resident: MemoryEstimate::default(),
        max_buffer: u64::MAX,
    };
    assert!(matches!(
        PreparedScene::new(&scene, budget),
        Err(RenderError::Budget("CPU preparation"))
    ));
    let mesh = scene.bodies()[0].mesh().unwrap();
    let first = crate::silhouette::candidates(&mesh).unwrap();
    let second = crate::silhouette::candidates(&mesh).unwrap();
    assert!(!first.is_empty());
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&first),
        bytemuck::cast_slice::<_, u8>(&second)
    );
}

fn verify_flight_guards(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &SceneSnapshot,
) {
    assert!(renderer.poll_idle().unwrap());
    assert!(matches!(
        renderer.after_submit(|| {}),
        Err(RenderError::SubmissionState)
    ));
    let view = ViewSnapshot::new(
        scene.identity(),
        99,
        Camera::default(),
        Viewport {
            width: 320.0,
            height: 240.0,
            scale_factor: 1.0,
        },
    )
    .unwrap();
    let mut first = device.create_command_encoder(&Default::default());
    renderer
        .encode(&mut first, &view, RenderStyle::default())
        .unwrap();
    let mut second = device.create_command_encoder(&Default::default());
    assert!(matches!(
        renderer.encode(&mut second, &view, RenderStyle::default()),
        Err(RenderError::Busy)
    ));
    assert!(matches!(renderer.resize([64, 64]), Err(RenderError::Busy)));
    assert!(matches!(
        renderer.replace_scene(scene),
        Err(RenderError::Busy)
    ));
    drop(second);
    drop(first);
    renderer.abandon_frame(|| {}).unwrap();
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    assert!(renderer.poll_idle().unwrap());
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer
        .encode(&mut encoder, &view, RenderStyle::default())
        .unwrap();
    queue.submit([encoder.finish()]);
    renderer.after_submit(|| {}).unwrap();
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    assert!(renderer.poll_idle().unwrap());
}

#[test]
fn selection_and_preview_styles_stay_distinct_from_committed_shading() {
    let view = ViewSnapshot::new(
        SceneIdentity {
            session: 1,
            scene: 2,
        },
        3,
        Camera::default(),
        Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 2.0,
        },
    )
    .unwrap();
    for theme in [Theme::Dark, Theme::Light] {
        let style = RenderStyle {
            theme,
            ..Default::default()
        };
        let committed = frame::uniform(&view, None, style, BodyStyle::default(), false).unwrap();
        let selected =
            frame::uniform(&view, None, style, BodyStyle { selected: true }, false).unwrap();
        let preview = frame::uniform(&view, None, style, BodyStyle::default(), true).unwrap();
        assert_eq!(committed.overlay, [0.0; 4]);
        assert_eq!(selected.overlay[3], 0.45);
        assert_eq!(preview.overlay[3], 0.3);
        assert_eq!(selected.lines[0], 6.0);
        assert_ne!(selected.edge, committed.edge);
        assert_eq!(committed.material[2], 0.0);
        assert_eq!(committed.view_proj, preview.view_proj);
        assert_eq!(committed.shadow_vp, preview.shadow_vp);
    }
}
