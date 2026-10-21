use crate::gpu::{self, Gpu, Pipe};
use crate::{targets::OUTPUT_FORMAT, Renderer};

/// Caller-owned GPU copy of a renderer output. Texture and view are independent of the renderer.
pub struct Thumbnail {
    pub texture: wgpu::Texture,
    /// Rgba8Unorm view of the gamma-encoded image, same convention as `Renderer::output_view`.
    pub view: wgpu::TextureView,
    pub size: [u32; 2],
}

/// Tests add COPY_SRC to read pixels back; the product texture is sample/render only.
#[cfg(not(test))]
const USAGE: wgpu::TextureUsages =
    wgpu::TextureUsages::TEXTURE_BINDING.union(wgpu::TextureUsages::RENDER_ATTACHMENT);
#[cfg(test)]
const USAGE: wgpu::TextureUsages = wgpu::TextureUsages::TEXTURE_BINDING
    .union(wgpu::TextureUsages::RENDER_ATTACHMENT)
    .union(wgpu::TextureUsages::COPY_SRC);

pub(crate) struct BlitResources {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

impl BlitResources {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let gpu = Gpu { device, queue };
        let module = gpu.shader("thumbnail blit", &[include_str!("shaders/blit.wgsl")]);
        let layout = gpu.layout(
            "thumbnail blit",
            &[
                gpu::tex_entry(0, gpu::FLOAT, gpu::D2),
                gpu::sampler_entry(1),
            ],
        );
        let pipeline_layout = gpu.pipeline_layout("thumbnail blit", &[&layout]);
        let pipeline = Pipe {
            label: "thumbnail blit",
            layout: &pipeline_layout,
            module: &module,
            vs: "vs_full",
            fs: Some("fs_blit"),
            buffers: &[],
            format: Some(wgpu::TextureFormat::Rgba8Unorm),
            blend: None,
            depth: None,
            samples: 1,
            cull: None,
            strip: false,
            a2c: false,
        }
        .build(device);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("thumbnail blit"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            layout,
            pipeline,
            sampler,
        }
    }
}

/// Fit `source` inside `max` keeping aspect, rounding down, never upscaling, at least 1x1.
pub(crate) fn fit(source: [u32; 2], max: [u32; 2]) -> [u32; 2] {
    let scale = (f64::from(max[0]) / f64::from(source[0]))
        .min(f64::from(max[1]) / f64::from(source[1]))
        .min(1.0);
    source.map(|v| ((f64::from(v) * scale).floor() as u32).max(1))
}

impl Renderer {
    /// Copy the current output into a small texture the caller owns.
    ///
    /// The host keeps one thumbnail per parked document for its tab switcher. The renderer
    /// draws only the active document into its single output, so a thumbnail must stay valid
    /// after the renderer draws another document or resizes; it shares no storage with targets.
    /// The blit is submitted on `queue` and ordered after earlier submissions, so call it after
    /// the frame to capture was submitted. Returns `None` without a drawable output.
    pub fn snapshot_thumbnail(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        max: [u32; 2],
    ) -> Option<Thumbnail> {
        let (source, source_size) = match (&self.targets, &self.retained_output) {
            (Some(targets), _) => (&targets.output_view, self.size),
            (None, Some(output)) => (&output.view, output.size),
            _ => return None,
        };
        if source_size.contains(&0) {
            return None;
        }
        let size = fit(source_size, max);
        let gpu = Gpu { device, queue };
        let texture = gpu.texture("thumbnail", size, OUTPUT_FORMAT, 1, USAGE);
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Rgba8Unorm),
            ..Default::default()
        });
        let blit = self
            .blit
            .get_or_insert_with(|| BlitResources::new(device, queue));
        let bind = gpu.bind(
            "thumbnail blit",
            &blit.layout,
            &[
                wgpu::BindingResource::TextureView(source),
                wgpu::BindingResource::Sampler(&blit.sampler),
            ],
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("thumbnail blit"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("thumbnail blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&blit.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
        Some(Thumbnail {
            texture,
            view,
            size,
        })
    }
}
