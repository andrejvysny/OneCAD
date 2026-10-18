//! wgpu viewport drawn inside egui's render pass through an `egui_wgpu` paint callback.
//! The callback shape (resources in `CallbackResources`, `prepare` then `paint` with the
//! viewport rect) and the reversed-depth helpers follow SolidCraft
//! `crates/ui-egui/src/gpu.rs` and `crates/gpu/src/view.rs` (MIT OR Apache-2.0).

use crate::scene::Scene;
use bytemuck::{Pod, Zeroable};
use egui_wgpu::wgpu;
use wgpu::util::DeviceExt;

pub const MSAA: u32 = 4;
pub const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Faces are pushed back by a few depth steps and by their own slope, so edges lying on them
/// draw on top at any angle (SolidCraft `FACE_BIAS`, sign flipped for reversed depth).
const FACE_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: -8,
    slope_scale: -1.0,
    clamp: 0.0,
};
const BODY_STRIDE: u64 = 256;
pub const EDGE_WIDTH_PX: f32 = 1.25;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    vp: [[f32; 4]; 4],
    eye: [f32; 4],
    screen: [f32; 4],
    target: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct LineInstance {
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub color: [u8; 4],
    pub width: f32,
}

struct GpuBody {
    positions: wgpu::Buffer,
    normals: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    edges: std::ops::Range<u32>,
}

pub struct Renderer {
    tri: wgpu::RenderPipeline,
    line: wgpu::RenderPipeline,
    reset: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    body_uniform: wgpu::Buffer,
    body_bind: wgpu::BindGroup,
    bodies: Vec<GpuBody>,
    edges: wgpu::Buffer,
    grid: wgpu::Buffer,
    grid_count: u32,
    last_colors: Vec<[f32; 4]>,
    encode_srgb: bool,
    pub uploaded_bytes: u64,
}

/// Per-frame data handed from the UI to the GPU.
pub struct Frame {
    pub view_proj: glam::Mat4,
    pub eye: glam::Vec3,
    pub target: glam::Vec3,
    pub grid_fade: f32,
    pub visible: Vec<bool>,
    pub colors: Vec<[f32; 4]>,
    /// Latency probe: the newest pointer position consumed by this frame (see metrics).
    pub probe: Option<[f32; 2]>,
    /// Viewport size in physical pixels.
    pub size_px: [f32; 2],
}

pub struct ViewportCallback(pub Frame);

fn reversed(vp: glam::Mat4) -> [[f32; 4]; 4] {
    // z' = w - z: depth 1 at the near plane, 0 at the far one (SolidCraft `gpu_view_proj`).
    vp.to_cols_array_2d()
        .map(|c| [c[0], c[1], c[3] - c[2], c[3]])
}

#[allow(clippy::too_many_arguments)]
fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    entry: (&str, &str),
    buffers: &[Option<wgpu::VertexBufferLayout>],
    depth: wgpu::DepthStencilState,
    blend: Option<wgpu::BlendState>,
    write: wgpu::ColorWrites,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry.0),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(entry.0),
            buffers,
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(entry.1),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: write,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: Some(depth),
        multisample: wgpu::MultisampleState {
            count: MSAA,
            ..Default::default()
        },
        multiview_mask: None,
        cache: None,
    })
}

fn depth_state(
    write: bool,
    compare: wgpu::CompareFunction,
    bias: wgpu::DepthBiasState,
) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH,
        depth_write_enabled: Some(write),
        depth_compare: Some(compare),
        stencil: Default::default(),
        bias,
    }
}

impl Renderer {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        scene: &Scene,
        grid: &[LineInstance],
    ) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewport"),
            source: wgpu::ShaderSource::Wgsl(include_str!("viewport.wgsl").into()),
        });
        let uniform_entry = |dynamic: bool| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: None,
            },
            count: None,
        };
        let bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("u"),
            entries: &[uniform_entry(false)],
        });
        let bgl1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("body"),
            entries: &[uniform_entry(true)],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport"),
            bind_group_layouts: &[Some(&bgl0), Some(&bgl1)],
            immediate_size: 0,
        });
        let empty = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("reset"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let f3 = |loc| wgpu::VertexBufferLayout {
            array_stride: 12,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: match loc {
                0 => &[wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                }],
                _ => &[wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 1,
                }],
            },
        };
        const LINE_ATTRS: [wgpu::VertexAttribute; 4] =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Unorm8x4, 3 => Float32];
        let line_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &LINE_ATTRS,
        };
        use wgpu::CompareFunction::{Always, GreaterEqual};
        let tri = pipeline(
            device,
            &layout,
            &module,
            format,
            ("vs_tri", "fs_tri"),
            &[Some(f3(0)), Some(f3(1))],
            depth_state(true, GreaterEqual, FACE_BIAS),
            None,
            wgpu::ColorWrites::ALL,
        );
        let line = pipeline(
            device,
            &layout,
            &module,
            format,
            ("vs_line", "fs_line"),
            &[Some(line_layout)],
            depth_state(false, GreaterEqual, Default::default()),
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::ColorWrites::ALL,
        );
        let reset = pipeline(
            device,
            &empty,
            &module,
            format,
            ("vs_reset", "fs_reset"),
            &[],
            depth_state(true, Always, Default::default()),
            None,
            wgpu::ColorWrites::empty(),
        );

        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("u"),
            size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("u"),
            layout: &bgl0,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let n = scene.bodies.len() as u64;
        let body_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("body"),
            size: BODY_STRIDE * n.max(1),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let body_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("body"),
            layout: &bgl1,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &body_uniform,
                    offset: 0,
                    size: wgpu::BufferSize::new(16),
                }),
            }],
        });

        let mut uploaded = 0u64;
        let mut buf = |label: &str, bytes: &[u8], usage: wgpu::BufferUsages| {
            uploaded += bytes.len() as u64;
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytes,
                usage,
            })
        };
        let mut edge_instances: Vec<LineInstance> = Vec::with_capacity(scene.segment_count());
        let mut bodies = Vec::with_capacity(scene.bodies.len());
        for b in &scene.bodies {
            let start = edge_instances.len() as u32;
            for s in &b.mesh.edge_segments {
                edge_instances.push(LineInstance {
                    a: [s[0], s[1], s[2]],
                    b: [s[3], s[4], s[5]],
                    color: [0; 4],
                    width: EDGE_WIDTH_PX,
                });
            }
            bodies.push(GpuBody {
                positions: buf("positions", &b.mesh.positions, wgpu::BufferUsages::VERTEX),
                normals: buf("normals", &b.mesh.normals, wgpu::BufferUsages::VERTEX),
                indices: buf("indices", &b.mesh.indices, wgpu::BufferUsages::INDEX),
                index_count: 3 * b.mesh.triangle_count,
                edges: start..edge_instances.len() as u32,
            });
        }
        let edges = buf(
            "edges",
            bytemuck::cast_slice(&edge_instances),
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        );
        let grid_buf = buf(
            "grid",
            bytemuck::cast_slice(grid),
            wgpu::BufferUsages::VERTEX,
        );
        Renderer {
            tri,
            line,
            reset,
            uniform,
            bind,
            body_uniform,
            body_bind,
            bodies,
            edges,
            grid: grid_buf,
            grid_count: grid.len() as u32,
            last_colors: Vec::new(),
            encode_srgb: !format.is_srgb(),
            uploaded_bytes: uploaded,
        }
    }

    /// Recolor every edge instance (theme edge token) in place.
    pub fn set_edge_color(&self, queue: &wgpu::Queue, scene: &Scene, color: [u8; 4]) {
        let mut all = Vec::with_capacity(scene.segment_count());
        for b in &scene.bodies {
            for s in &b.mesh.edge_segments {
                all.push(LineInstance {
                    a: [s[0], s[1], s[2]],
                    b: [s[3], s[4], s[5]],
                    color,
                    width: EDGE_WIDTH_PX,
                });
            }
        }
        queue.write_buffer(&self.edges, 0, bytemuck::cast_slice(&all));
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn prepare_frame(&mut self, queue: &wgpu::Queue, f: &Frame) {
        self.prepare(queue, f);
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn paint_frame(&self, f: &Frame, pass: &mut wgpu::RenderPass<'static>) {
        self.paint(f, pass);
    }

    fn prepare(&mut self, queue: &wgpu::Queue, f: &Frame) {
        let (size_px, encode_srgb) = (f.size_px, self.encode_srgb);
        let u = Uniform {
            vp: reversed(f.view_proj),
            eye: f.eye.extend(1.0).to_array(),
            screen: [
                size_px[0],
                size_px[1],
                f.grid_fade,
                if encode_srgb { 1.0 } else { 0.0 },
            ],
            target: f.target.extend(0.0).to_array(),
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        if f.colors != self.last_colors {
            let mut bytes = vec![0u8; BODY_STRIDE as usize * f.colors.len()];
            for (i, c) in f.colors.iter().enumerate() {
                let at = i * BODY_STRIDE as usize;
                bytes[at..at + 16].copy_from_slice(bytemuck::bytes_of(c));
            }
            queue.write_buffer(&self.body_uniform, 0, &bytes);
            self.last_colors = f.colors.clone();
        }
    }

    fn paint(&self, f: &Frame, pass: &mut wgpu::RenderPass<'static>) {
        pass.set_pipeline(&self.reset);
        pass.draw(0..3, 0..1);
        pass.set_bind_group(0, &self.bind, &[]);
        pass.set_pipeline(&self.tri);
        for (i, b) in self.bodies.iter().enumerate() {
            if !f.visible.get(i).copied().unwrap_or(true) {
                continue;
            }
            pass.set_bind_group(1, &self.body_bind, &[(i as u64 * BODY_STRIDE) as u32]);
            pass.set_vertex_buffer(0, b.positions.slice(..));
            pass.set_vertex_buffer(1, b.normals.slice(..));
            pass.set_index_buffer(b.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..b.index_count, 0, 0..1);
        }
        pass.set_pipeline(&self.line);
        pass.set_bind_group(1, &self.body_bind, &[0]);
        pass.set_vertex_buffer(0, self.grid.slice(..));
        pass.draw(0..6, 0..self.grid_count);
        pass.set_vertex_buffer(0, self.edges.slice(..));
        for (i, b) in self.bodies.iter().enumerate() {
            if f.visible.get(i).copied().unwrap_or(true) && !b.edges.is_empty() {
                pass.draw(0..6, b.edges.clone());
            }
        }
    }
}

impl egui_wgpu::CallbackTrait for ViewportCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        crate::metrics::on_prepare(self.0.probe);
        let _ = screen;
        if let Some(r) = resources.get_mut::<Renderer>() {
            r.prepare(queue, &self.0);
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(r) = resources.get::<Renderer>() else {
            return;
        };
        let vp = info.viewport_in_pixels();
        if vp.width_px <= 0 || vp.height_px <= 0 {
            return;
        }
        pass.set_viewport(
            vp.left_px as f32,
            vp.top_px as f32,
            vp.width_px as f32,
            vp.height_px as f32,
            0.0,
            1.0,
        );
        r.paint(&self.0, pass);
    }
}
