//! Offscreen GPU frame-time benchmark: the viewport's own pipelines drawing NEMA into a 4x
//! MSAA target while orbiting, each frame submitted and waited on. On macOS winit paces
//! redraws to the display (60 Hz here) even without vsync, so the on-screen loop cannot show
//! the renderer's real cost; this does. `SPIKE_GPU_BENCH=<frames>[x<scale>]`.

use crate::camera::Camera;
use crate::render::{Frame, MSAA, Renderer};
use crate::scene::Scene;
use egui_wgpu::wgpu;

pub fn run(
    scene: &Scene,
    theme: &crate::theme::Theme,
    frames: u32,
    size: [u32; 2],
) -> Result<(), String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .map_err(|e| e.to_string())?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))
        .map_err(|e| e.to_string())?;
    let format = wgpu::TextureFormat::Bgra8Unorm;
    let grid = crate::viewport::grid_lines(theme, scene);
    let mut r = Renderer::new(&device, format, scene, &grid);
    r.set_edge_color(&queue, scene, theme.body_edge.to_array());
    let tex = |format, samples| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bench"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
    };
    let msaa = tex(format, MSAA).create_view(&Default::default());
    let resolve = tex(format, 1).create_view(&Default::default());
    let depth = tex(crate::render::DEPTH, MSAA).create_view(&Default::default());
    let aspect = size[0] as f32 / size[1] as f32;
    let mut cam = Camera::fit(scene.bbox_min, scene.bbox_max, aspect);
    let mut times = Vec::with_capacity(frames as usize);
    for i in 0..frames + 20 {
        cam.orbit(1.5, 0.0);
        let f = Frame {
            view_proj: cam.view_proj(aspect),
            eye: cam.eye(),
            target: cam.target,
            grid_fade: scene.radius() * 6.0,
            visible: vec![true; scene.bodies.len()],
            colors: vec![theme.body_fill.to_normalized_gamma_f32(); scene.bodies.len()],
            probe: None,
            size_px: [size[0] as f32, size[1] as f32],
        };
        let t0 = crate::metrics::now_ns();
        r.prepare_frame(&queue, &f);
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("bench"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &msaa,
                        depth_slice: None,
                        resolve_target: Some(&resolve),
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Discard,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            pass.set_viewport(0.0, 0.0, size[0] as f32, size[1] as f32, 0.0, 1.0);
            r.paint_frame(&f, &mut pass);
        }
        queue.submit([enc.finish()]);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| e.to_string())?;
        if i >= 20 {
            times.push((crate::metrics::now_ns() - t0) as f64 / 1e6);
        }
    }
    times.sort_by(f64::total_cmp);
    let q = |p: f64| times[((times.len() - 1) as f64 * p).round() as usize];
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    eprintln!(
        "[metric] gpu_bench size={}x{} msaa={MSAA} frames={} submit_to_idle_ms mean={mean:.3} p50={:.3} p95={:.3} max={:.3} adapter={:?}",
        size[0],
        size[1],
        times.len(),
        q(0.5),
        q(0.95),
        times[times.len() - 1],
        adapter.get_info().name
    );
    crate::metrics::rss("gpu_bench_end");
    Ok(())
}
