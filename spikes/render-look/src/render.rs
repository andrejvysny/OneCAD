//! The frame: pipelines, targets and the pass ladder.
//!
//! Quality: contact height → contact occlusion → contact blur ×2 → depth/normal prepass → GTAO →
//! AO blur ×2 → main MSAA pass (bodies, ground + grid, silhouettes, edges; resolved in-pass).
//! Baseline: the main MSAA pass only (bodies, grid lines, edges).

use wgpu::util::DeviceExt;

use crate::gpu::{self, depth_state, Gpu, Pipe};
use crate::ibl::Ibl;
use crate::look::{FrameU, HEIGHT_SIZE, OCC_SIZE};
use crate::scene::{Scene, Segment, SilEdge};

pub const SAMPLES: u32 = 4;
pub const OUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const HDR16: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
const OCC_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

/// Faces pushed back by a few depth steps plus their own slope so edges lying on them win at
/// any angle (SolidCraft `FACE_BIAS`, MIT OR Apache-2.0), negated for reversed depth.
const FACE_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState { constant: -8, slope_scale: -1.0, clamp: 0.0 };
/// Baseline: three's polygonOffset(factor 1, units 1) on the face material.
const BASE_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState { constant: -1, slope_scale: -1.0, clamp: 0.0 };

/// Which parts of the main pass to draw (for incremental cost measurement).
#[derive(Clone, Copy)]
pub struct Parts {
    pub bodies: bool,
    pub ground: bool,
    pub silhouettes: bool,
    /// Silhouettes culled in the line vertex shader over every candidate, instead of the
    /// compute compaction + indirect draw (kept for the cost comparison).
    pub sil_vertex_cull: bool,
    pub edges: bool,
    /// Lines through MSAA alpha-to-coverage (coverage becomes the sample mask, no blending)
    /// instead of alpha blending: feature edges and compacted silhouettes, and in the baseline
    /// the edges and grid (with hard coverage this is today's opaque LineMaterial).
    pub a2c: bool,
    /// Feature-edge timing experiments (bench only; the images never use them).
    pub edge_exp: EdgeExp,
}

/// Feature-edge pipeline variants that exist only to attribute the edge cost.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EdgeExp {
    None,
    /// Blending off, colour written over the whole quad (wrong image, cost of opaque quads).
    Opaque,
    /// Blended as the default, but uncovered fragments discarded.
    Discard,
    /// The full edge vertex work, every quad culled afterwards (no rasterisation).
    CulledAfterVs,
}

impl Parts {
    /// Everything except the geometric silhouette lines: the baseline, and the fragment-
    /// silhouette alternative (silhouettes drawn in the body shader).
    pub const DEFAULT: Parts =
        Parts { bodies: true, ground: true, silhouettes: false, sil_vertex_cull: false, edges: true, a2c: false, edge_exp: EdgeExp::None };
    pub const ALL: Parts = Parts { silhouettes: true, ..Parts::DEFAULT };
    /// Today's look: opaque edges and grid (hard coverage through alpha-to-coverage).
    pub const BASELINE: Parts = Parts { a2c: true, ..Parts::DEFAULT };
}

struct Pipes {
    prepass: wgpu::RenderPipeline,
    body: wgpu::RenderPipeline,
    body_base: wgpu::RenderPipeline,
    height: wgpu::RenderPipeline,
    ground: wgpu::RenderPipeline,
    edge: wgpu::RenderPipeline,
    edge_a2c: wgpu::RenderPipeline,
    edge_opaque: wgpu::RenderPipeline,
    edge_discard: wgpu::RenderPipeline,
    edge_culled: wgpu::RenderPipeline,
    grid_a2c: wgpu::RenderPipeline,
    sil_seg_a2c: wgpu::RenderPipeline,
    sil_a2c: wgpu::RenderPipeline,
    sil: wgpu::RenderPipeline,
    sil_seg: wgpu::RenderPipeline,
    sil_cull: wgpu::ComputePipeline,
    grid_base: wgpu::RenderPipeline,
    contact_occ: wgpu::RenderPipeline,
    contact_h: wgpu::RenderPipeline,
    contact_v: wgpu::RenderPipeline,
    gtao: wgpu::RenderPipeline,
    ao_h: wgpu::RenderPipeline,
    ao_v: wgpu::RenderPipeline,
}

pub struct Renderer {
    pub size: [u32; 2],
    pipes: Pipes,
    uniform: wgpu::Buffer,
    frame_bind: wgpu::BindGroup,
    body_bind: wgpu::BindGroup,
    ground_bind: wgpu::BindGroup,
    post_binds: [wgpu::BindGroup; 6],
    positions: wgpu::Buffer,
    normals: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    segments: wgpu::Buffer,
    segment_count: u32,
    sils: wgpu::Buffer,
    sil_count: u32,
    sil_out: wgpu::Buffer,
    sil_args: wgpu::Buffer,
    sil_bind: wgpu::BindGroup,
    grid: Option<(wgpu::Buffer, u32)>,
    pub output: wgpu::Texture,
    output_view: wgpu::TextureView,
    msaa_color: wgpu::TextureView,
    msaa_depth: wgpu::TextureView,
    pre_depth: wgpu::TextureView,
    pre_normal: wgpu::TextureView,
    ao_raw: wgpu::TextureView,
    ao_tmp: wgpu::TextureView,
    ao_final: wgpu::TextureView,
    height: wgpu::TextureView,
    occ: wgpu::TextureView,
    occ_tmp: wgpu::TextureView,
    occ_final: wgpu::TextureView,
}

fn buf(gpu: &Gpu, label: &str, data: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    // A zero-sized vertex buffer cannot be bound; keep at least one element's worth.
    let data = if data.is_empty() { &[0u8; 64][..] } else { data };
    gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: data, usage })
}

const VEC3: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x3];
const NRM3: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![1 => Float32x3];
const SEG: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
const SIL: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x3];
const GRIDL: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4];

fn vb(stride: usize, attrs: &'static [wgpu::VertexAttribute], instance: bool) -> Option<wgpu::VertexBufferLayout<'static>> {
    let step_mode = if instance { wgpu::VertexStepMode::Instance } else { wgpu::VertexStepMode::Vertex };
    Some(wgpu::VertexBufferLayout { array_stride: stride as u64, step_mode, attributes: attrs })
}

impl Renderer {
    pub fn new(gpu: &Gpu, scene: &Scene, ibl: &Ibl, size: [u32; 2], grid: Option<&[[f32; 10]]>) -> Renderer {
        let d = &gpu.device;
        let common = include_str!("shaders/common.wgsl");
        let m_body = gpu.shader("body", &[common, include_str!("shaders/body.wgsl")]);
        let m_lines = gpu.shader("lines", &[common, include_str!("shaders/lines.wgsl")]);
        let m_ground = gpu.shader("ground", &[common, include_str!("shaders/ground.wgsl")]);
        let m_post = gpu.shader("post", &[common, include_str!("shaders/post.wgsl")]);

        let frame_l = gpu.layout("frame", &[gpu::uniform_entry(0)]);
        let body_l = gpu.layout(
            "body_tex",
            &[
                gpu::tex_entry(0, gpu::FLOAT_NF, gpu::D2),
                gpu::tex_entry(1, gpu::FLOAT, gpu::CUBE),
                gpu::tex_entry(2, gpu::FLOAT, gpu::CUBE),
                gpu::tex_entry(3, gpu::FLOAT, gpu::D2),
                gpu::sampler_entry(4),
            ],
        );
        let ground_l = gpu.layout("ground_tex", &[gpu::tex_entry(0, gpu::FLOAT, gpu::D2), gpu::sampler_entry(1)]);
        let post_l = gpu
            .layout("post_tex", &[gpu::tex_entry(0, wgpu::TextureSampleType::Depth, gpu::D2), gpu::tex_entry(1, gpu::FLOAT_NF, gpu::D2)]);
        let pl_body = gpu.pipeline_layout("body", &[&frame_l, &body_l]);
        let pl_frame = gpu.pipeline_layout("frame", &[&frame_l]);
        let pl_ground = gpu.pipeline_layout("ground", &[&frame_l, &ground_l]);
        let pl_post = gpu.pipeline_layout("post", &[&frame_l, &post_l]);

        use wgpu::CompareFunction::{Greater, GreaterEqual, Less};
        let alpha = Some(wgpu::BlendState::ALPHA_BLENDING);
        let none = wgpu::DepthBiasState::default();
        let body_vb = [vb(12, &VEC3, false), vb(12, &NRM3, false)];
        let pos_vb = [vb(12, &VEC3, false)];
        let seg_vb = [vb(std::mem::size_of::<Segment>(), &SEG, true)];
        let sil_vb = [vb(std::mem::size_of::<SilEdge>(), &SIL, true)];
        let grid_vb = [vb(40, &GRIDL, true)];
        let p = |label, layout, module, vs, fs: Option<&'static str>, buffers, format, blend, depth, samples| {
            Pipe {
                label,
                layout,
                module,
                vs,
                fs,
                buffers,
                format,
                blend,
                depth,
                samples,
                cull: None,
                // Line quads are 4-vertex strips (one instance per segment).
                strip: matches!(vs, "vs_edge" | "vs_edge_culled" | "vs_sil" | "vs_silseg" | "vs_grid"),
                a2c: false,
            }
            .build(d)
        };
        let main_depth = |bias| depth_state(Greater, true, bias);
        let line_depth = depth_state(GreaterEqual, false, none);
        let a2c_line = |label, vs, buffers| {
            Pipe {
                label,
                layout: &pl_frame,
                module: &m_lines,
                vs,
                fs: Some("fs_line"),
                buffers,
                format: Some(OUT_FORMAT),
                blend: None,
                depth: line_depth.clone(),
                samples: SAMPLES,
                cull: None,
                strip: true,
                a2c: true,
            }
            .build(d)
        };
        let m_sil = gpu.shader("sil_cull", &[common, include_str!("shaders/sil_cull.wgsl")]);
        let sil_l = gpu.layout("sil_cull", &[gpu::storage_entry(0, true), gpu::storage_entry(1, false), gpu::storage_entry(2, false)]);
        let pl_sil = gpu.pipeline_layout("sil_cull", &[&frame_l, &sil_l]);
        let pipes = Pipes {
            prepass: p("prepass", &pl_body, &m_body, "vs_body", Some("fs_prepass"), &body_vb, Some(HDR16), None, main_depth(none), 1),
            body: p(
                "body",
                &pl_body,
                &m_body,
                "vs_body",
                Some("fs_body"),
                &body_vb,
                Some(OUT_FORMAT),
                None,
                main_depth(FACE_BIAS),
                SAMPLES,
            ),
            body_base: p(
                "body_base",
                &pl_body,
                &m_body,
                "vs_body",
                Some("fs_body_baseline"),
                &body_vb,
                Some(OUT_FORMAT),
                None,
                main_depth(BASE_BIAS),
                SAMPLES,
            ),
            height: p("height", &pl_body, &m_body, "vs_height", None, &pos_vb, None, None, depth_state(Less, true, none), 1),
            ground: p(
                "ground",
                &pl_ground,
                &m_ground,
                "vs_ground",
                Some("fs_ground"),
                &[],
                Some(OUT_FORMAT),
                None,
                depth_state(GreaterEqual, false, none),
                SAMPLES,
            ),
            edge: p("edge", &pl_frame, &m_lines, "vs_edge", Some("fs_line"), &seg_vb, Some(OUT_FORMAT), alpha, line_depth.clone(), SAMPLES),
            edge_a2c: a2c_line("edge_a2c", "vs_edge", &seg_vb),
            sil_seg_a2c: a2c_line("sil_seg_a2c", "vs_silseg", &seg_vb),
            grid_a2c: a2c_line("grid_a2c", "vs_grid", &grid_vb),
            sil_a2c: a2c_line("sil_a2c", "vs_sil", &sil_vb),
            edge_opaque: p(
                "edge_opaque",
                &pl_frame,
                &m_lines,
                "vs_edge",
                Some("fs_line"),
                &seg_vb,
                Some(OUT_FORMAT),
                None,
                line_depth.clone(),
                SAMPLES,
            ),
            edge_discard: p(
                "edge_discard",
                &pl_frame,
                &m_lines,
                "vs_edge",
                Some("fs_line_discard"),
                &seg_vb,
                Some(OUT_FORMAT),
                alpha,
                line_depth.clone(),
                SAMPLES,
            ),
            edge_culled: p(
                "edge_culled",
                &pl_frame,
                &m_lines,
                "vs_edge_culled",
                Some("fs_line"),
                &seg_vb,
                Some(OUT_FORMAT),
                alpha,
                line_depth.clone(),
                SAMPLES,
            ),
            sil: p("sil", &pl_frame, &m_lines, "vs_sil", Some("fs_line"), &sil_vb, Some(OUT_FORMAT), alpha, line_depth.clone(), SAMPLES),
            sil_seg: p(
                "sil_seg",
                &pl_frame,
                &m_lines,
                "vs_silseg",
                Some("fs_line"),
                &seg_vb,
                Some(OUT_FORMAT),
                alpha,
                line_depth.clone(),
                SAMPLES,
            ),
            sil_cull: d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("sil_cull"),
                layout: Some(&pl_sil),
                module: &m_sil,
                entry_point: Some("cs_sil"),
                compilation_options: Default::default(),
                cache: None,
            }),
            grid_base: p(
                "grid_base",
                &pl_frame,
                &m_lines,
                "vs_grid",
                Some("fs_line"),
                &grid_vb,
                Some(OUT_FORMAT),
                alpha,
                line_depth,
                SAMPLES,
            ),
            contact_occ: p("contact_occ", &pl_post, &m_post, "vs_full", Some("fs_contact_occ"), &[], Some(OCC_FORMAT), None, None, 1),
            contact_h: p("contact_h", &pl_post, &m_post, "vs_full", Some("fs_contact_blur_h"), &[], Some(OCC_FORMAT), None, None, 1),
            contact_v: p("contact_v", &pl_post, &m_post, "vs_full", Some("fs_contact_blur_v"), &[], Some(OCC_FORMAT), None, None, 1),
            gtao: p("gtao", &pl_post, &m_post, "vs_full", Some("fs_gtao"), &[], Some(AO_FORMAT), None, None, 1),
            ao_h: p("ao_h", &pl_post, &m_post, "vs_full", Some("fs_ao_blur_h"), &[], Some(AO_FORMAT), None, None, 1),
            ao_v: p("ao_v", &pl_post, &m_post, "vs_full", Some("fs_ao_blur_v"), &[], Some(AO_FORMAT), None, None, 1),
        };

        use wgpu::TextureUsages as U;
        let rt = U::RENDER_ATTACHMENT | U::TEXTURE_BINDING;
        let t = |label, s, f, n, usage| gpu::view(&gpu.texture(label, s, f, n, usage));
        let depth = wgpu::TextureFormat::Depth32Float;
        let output = gpu.texture("output", size, OUT_FORMAT, 1, U::RENDER_ATTACHMENT | U::COPY_SRC);
        let output_view = gpu::view(&output);
        let msaa_color = t("msaa_color", size, OUT_FORMAT, SAMPLES, U::RENDER_ATTACHMENT);
        let msaa_depth = t("msaa_depth", size, depth, SAMPLES, U::RENDER_ATTACHMENT);
        let pre_depth = t("pre_depth", size, depth, 1, rt);
        let pre_normal = t("pre_normal", size, HDR16, 1, rt);
        let ao_raw = t("ao_raw", size, AO_FORMAT, 1, rt);
        let ao_tmp = t("ao_tmp", size, AO_FORMAT, 1, rt);
        let ao_final = t("ao_final", size, AO_FORMAT, 1, rt);
        let height = t("height", [HEIGHT_SIZE; 2], depth, 1, rt);
        let occ = t("occ", [OCC_SIZE; 2], OCC_FORMAT, 1, rt);
        let occ_tmp = t("occ_tmp", [OCC_SIZE; 2], OCC_FORMAT, 1, rt);
        let occ_final = t("occ_final", [OCC_SIZE; 2], OCC_FORMAT, 1, rt);

        let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let uniform = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame"),
            size: std::mem::size_of::<FrameU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        use wgpu::BindingResource::{Sampler, TextureView as Tv};
        let frame_bind = gpu.bind("frame", &frame_l, &[uniform.as_entire_binding()]);
        let body_bind =
            gpu.bind("body_tex", &body_l, &[Tv(&ao_final), Tv(&ibl.irradiance), Tv(&ibl.specular), Tv(&ibl.lut), Sampler(&sampler)]);
        let ground_bind = gpu.bind("ground_tex", &ground_l, &[Tv(&occ_final), Sampler(&sampler)]);
        let pb = |dv: &wgpu::TextureView, tv: &wgpu::TextureView| gpu.bind("post", &post_l, &[Tv(dv), Tv(tv)]);
        let post_binds = [
            pb(&height, &occ_tmp),       // contact occlusion (tex unused)
            pb(&height, &occ),           // contact blur h
            pb(&height, &occ_tmp),       // contact blur v
            pb(&pre_depth, &pre_normal), // gtao
            pb(&pre_depth, &ao_raw),     // ao blur h
            pb(&pre_depth, &ao_tmp),     // ao blur v
        ];
        let vtx = wgpu::BufferUsages::VERTEX;
        let storage = wgpu::BufferUsages::STORAGE;
        let sils = buf(gpu, "silhouettes", bytemuck::cast_slice(&scene.silhouettes), vtx | storage);
        let sil_out = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("silhouettes_visible"),
            size: (scene.silhouettes.len().max(1) * std::mem::size_of::<Segment>()) as u64,
            usage: vtx | storage,
            mapped_at_creation: false,
        });
        // draw_indirect args: 4 strip vertices, instance count written by the compaction.
        let sil_args = buf(
            gpu,
            "silhouette_args",
            bytemuck::cast_slice(&[4u32, 0, 0, 0]),
            storage | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        );
        let sil_bind = gpu.bind("sil_cull", &sil_l, &[sils.as_entire_binding(), sil_out.as_entire_binding(), sil_args.as_entire_binding()]);
        Renderer {
            sil_out,
            sil_args,
            sil_bind,
            size,
            pipes,
            frame_bind,
            body_bind,
            ground_bind,
            post_binds,
            uniform,
            positions: buf(gpu, "positions", bytemuck::cast_slice(&scene.positions), vtx),
            normals: buf(gpu, "normals", bytemuck::cast_slice(&scene.normals), vtx),
            indices: buf(gpu, "indices", bytemuck::cast_slice(&scene.indices), wgpu::BufferUsages::INDEX),
            index_count: scene.indices.len() as u32,
            segments: buf(gpu, "segments", bytemuck::cast_slice(&scene.segments), vtx),
            segment_count: scene.segments.len() as u32,
            sils,
            sil_count: scene.silhouettes.len() as u32,
            grid: grid.map(|g| (buf(gpu, "grid", bytemuck::cast_slice(g), vtx), g.len() as u32)),
            output,
            output_view,
            msaa_color,
            msaa_depth,
            pre_depth,
            pre_normal,
            ao_raw,
            ao_tmp,
            ao_final,
            height,
            occ,
            occ_tmp,
            occ_final,
        }
    }

    /// The compaction's instance count (reads the indirect args back; diagnostics only).
    pub fn visible_silhouettes(&self, gpu: &Gpu) -> u32 {
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sil_args_read"),
            size: 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.sil_args, 0, &read, 0, 16);
        gpu.queue.submit([enc.finish()]);
        let slice = read.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        gpu.wait();
        slice.get_mapped_range().map(|v| u32::from_le_bytes([v[4], v[5], v[6], v[7]])).unwrap_or(0)
    }

    pub fn set_frame(&self, gpu: &Gpu, u: &FrameU) {
        gpu.queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(u));
    }
}

mod passes;
pub use passes::{encode, frame_passes, Pass};
