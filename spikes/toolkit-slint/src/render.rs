//! wgpu viewport renderer: draws the scene into a texture that Slint shows as an `Image`.
//! One indexed draw for every body's triangles, one instanced draw for grid + edges + axes.

use crate::mesh1::Body;
use bytemuck::{Pod, Zeroable};
use slint::wgpu_30::wgpu;
use std::time::{Duration, Instant};

const SAMPLES: u32 = 4;
const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Slint's femtovg-wgpu composites into a non-sRGB surface and samples an sRGB texture as linear
/// (too dark), so the shared texture is `Rgba8Unorm` holding sRGB-encoded bytes, written through
/// an sRGB view.
const SHARED: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    p: [f32; 3],
    n: [f32; 3],
    body: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LineInst {
    a: [f32; 3],
    b: [f32; 3],
    body: u32,
    kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
pub struct Uniforms {
    pub vp: [[f32; 4]; 4],
    pub eye: [f32; 4],
    pub back: [f32; 4],
    pub screen: [f32; 4],
    pub grid: [f32; 4],
    pub pal: [[f32; 4]; 8],
}

struct Targets {
    size: (u32, u32),
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    out: wgpu::Texture,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    tri: wgpu::RenderPipeline,
    line: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    ubuf: wgpu::Buffer,
    body_buf: wgpu::Buffer,
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    index_count: u32,
    lbuf: wgpu::Buffer,
    line_count: u32,
    targets: Option<Targets>,
    pub upload_time: Duration,
}

fn line_instances(bodies: &[Body], grid_half: f32, minor: f32, major_every: i32) -> Vec<LineInst> {
    let mut out = Vec::new();
    let n = (grid_half / minor) as i32;
    for i in -n..=n {
        let t = i as f32 * minor;
        let kind = if i % major_every == 0 { 2 } else { 1 };
        out.push(LineInst {
            a: [t, -grid_half, 0.0],
            b: [t, grid_half, 0.0],
            body: 0,
            kind,
        });
        out.push(LineInst {
            a: [-grid_half, t, 0.0],
            b: [grid_half, t, 0.0],
            body: 0,
            kind,
        });
    }
    for (bi, body) in bodies.iter().enumerate() {
        let m = &body.mesh;
        for &(first, count) in &m.edge_ranges {
            for k in first..first + count.saturating_sub(1) {
                let (a, b) = (m.edge_points[k as usize], m.edge_points[k as usize + 1]);
                out.push(LineInst {
                    a,
                    b,
                    body: bi as u32,
                    kind: 0,
                });
            }
        }
    }
    let axis = minor * 2.0;
    out.push(LineInst {
        a: [0.0; 3],
        b: [axis, 0.0, 0.0],
        body: 0,
        kind: 3,
    });
    out.push(LineInst {
        a: [0.0; 3],
        b: [0.0, axis, 0.0],
        body: 0,
        kind: 4,
    });
    out
}

impl Renderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, bodies: &[Body]) -> Self {
        let t0 = Instant::now();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                bgl_entry(0, wgpu::BufferBindingType::Uniform),
                bgl_entry(1, wgpu::BufferBindingType::Storage { read_only: true }),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let tri = pipeline(device, &pl, &shader, true);
        let line = pipeline(device, &pl, &shader, false);

        let mut verts = Vec::new();
        let mut idx = Vec::new();
        for (bi, b) in bodies.iter().enumerate() {
            let base = verts.len() as u32;
            for (p, n) in b.mesh.positions.iter().zip(&b.mesh.normals) {
                verts.push(Vertex {
                    p: *p,
                    n: *n,
                    body: bi as u32,
                });
            }
            idx.extend(b.mesh.indices.iter().map(|i| i + base));
        }
        let lines = line_instances(bodies, 300.0, 10.0, 5);
        let init = |label: &str, contents: &[u8], usage: wgpu::BufferUsages| {
            use wgpu::util::DeviceExt;
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            })
        };
        let vbuf = init(
            "verts",
            bytemuck::cast_slice(&verts),
            wgpu::BufferUsages::VERTEX,
        );
        let ibuf = init("idx", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
        let lbuf = init(
            "lines",
            bytemuck::cast_slice(&lines),
            wgpu::BufferUsages::VERTEX,
        );
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let body_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bodies"),
            size: (bodies.len().max(1) * 16) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ubuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: body_buf.as_entire_binding(),
                },
            ],
        });
        Renderer {
            device: device.clone(),
            queue: queue.clone(),
            tri,
            line,
            bind,
            ubuf,
            body_buf,
            vbuf,
            ibuf,
            index_count: idx.len() as u32,
            lbuf,
            line_count: lines.len() as u32,
            targets: None,
            upload_time: t0.elapsed(),
        }
    }

    pub fn set_bodies(&self, rows: &[[f32; 4]]) {
        self.queue
            .write_buffer(&self.body_buf, 0, bytemuck::cast_slice(rows));
    }

    /// Renders one frame at `size` physical px. Returns the output texture and whether it is new
    /// (the caller then hands it to Slint again).
    pub fn render(
        &mut self,
        size: (u32, u32),
        u: &Uniforms,
        clear: [f64; 3],
    ) -> (wgpu::Texture, bool) {
        let size = (size.0.max(1), size.1.max(1));
        let fresh = self.targets.as_ref().is_none_or(|t| t.size != size);
        if fresh {
            self.targets = Some(self.make_targets(size));
        }
        let Some(t) = self.targets.as_ref() else {
            unreachable!()
        };
        self.queue
            .write_buffer(&self.ubuf, 0, bytemuck::bytes_of(u));
        let out_view = t.out.create_view(&wgpu::TextureViewDescriptor {
            format: Some(COLOR),
            ..Default::default()
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.color,
                    resolve_target: Some(&out_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear[0],
                            g: clear[1],
                            b: clear[2],
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Discard,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind, &[]);
            pass.set_pipeline(&self.tri);
            pass.set_vertex_buffer(0, self.vbuf.slice(..));
            pass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
            pass.set_pipeline(&self.line);
            pass.set_vertex_buffer(0, self.lbuf.slice(..));
            pass.draw(0..6, 0..self.line_count);
        }
        self.queue.submit(Some(enc.finish()));
        // Nothing else polls this device (Slint's renderers do not), and without a poll wgpu never
        // retires staging buffers or encoder memory: the footprint grew ~430 KB per frame.
        let _ = self.device.poll(wgpu::PollType::Poll);
        (t.out.clone(), fresh)
    }

    pub fn wait_idle(&self) {
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    }

    pub fn counts(&self) -> (u32, u32) {
        (self.index_count / 3, self.line_count)
    }

    fn make_targets(&self, size: (u32, u32)) -> Targets {
        let tex = |format, samples, usage, view_formats: &[wgpu::TextureFormat]| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats,
            })
        };
        let att = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let color = tex(COLOR, SAMPLES, att, &[]).create_view(&Default::default());
        let depth = tex(DEPTH, SAMPLES, att, &[]).create_view(&Default::default());
        let out = tex(
            SHARED,
            1,
            att | wgpu::TextureUsages::TEXTURE_BINDING,
            &[COLOR],
        );
        Targets {
            size,
            color,
            depth,
            out,
        }
    }
}

fn bgl_entry(binding: u32, ty: wgpu::BufferBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    triangles: bool,
) -> wgpu::RenderPipeline {
    let tri_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Uint32];
    let line_attrs =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Uint32, 3 => Uint32];
    let buffers = if triangles {
        [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &tri_attrs,
        })]
    } else {
        [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineInst>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &line_attrs,
        })]
    };
    let (vs, fs) = if triangles {
        ("vs_tri", "fs_tri")
    } else {
        ("vs_line", "fs_line")
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(vs),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vs),
            buffers: &buffers,
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fs),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR,
                blend: (!triangles).then_some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: Some(triangles),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: Default::default(),
            // Reversed depth: a negative bias pushes faces away so edges on them stay visible.
            bias: if triangles {
                wgpu::DepthBiasState {
                    constant: 0,
                    slope_scale: -1.0,
                    clamp: 0.0,
                }
            } else {
                Default::default()
            },
        }),
        multisample: wgpu::MultisampleState {
            count: SAMPLES,
            ..Default::default()
        },
        multiview_mask: None,
        cache: None,
    })
}
