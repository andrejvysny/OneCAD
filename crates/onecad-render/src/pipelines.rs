use crate::gpu::{self, depth_state, Gpu, Pipe};
use crate::targets::{AO_FORMAT, HDR_FORMAT, OCC_FORMAT, OUTPUT_FORMAT, SAMPLES};

pub(crate) struct Pipelines {
    pub frame_layout: wgpu::BindGroupLayout,
    pub body_layout: wgpu::BindGroupLayout,
    pub ground_layout: wgpu::BindGroupLayout,
    pub post_layout: wgpu::BindGroupLayout,
    pub prepass: wgpu::RenderPipeline,
    pub body: wgpu::RenderPipeline,
    pub preview: wgpu::RenderPipeline,
    pub highlight_face: wgpu::RenderPipeline,
    pub highlight_edge: wgpu::RenderPipeline,
    pub height: wgpu::RenderPipeline,
    pub ground: wgpu::RenderPipeline,
    pub edge: wgpu::RenderPipeline,
    pub silhouette: wgpu::RenderPipeline,
    pub contact_occ: wgpu::RenderPipeline,
    pub contact_h: wgpu::RenderPipeline,
    pub contact_v: wgpu::RenderPipeline,
    pub gtao: wgpu::RenderPipeline,
    pub ao_h: wgpu::RenderPipeline,
    pub ao_v: wgpu::RenderPipeline,
}

const POS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0=>Float32x3];
const NORMAL: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![1=>Float32x3];
const SEG: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3];
const SIL: [wgpu::VertexAttribute; 4] =
    wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x3,3=>Float32x3];
fn vertex(
    stride: u64,
    attributes: &'static [wgpu::VertexAttribute],
    step_mode: wgpu::VertexStepMode,
) -> Option<wgpu::VertexBufferLayout<'static>> {
    Some(wgpu::VertexBufferLayout {
        array_stride: stride,
        attributes,
        step_mode,
    })
}

impl Pipelines {
    pub fn new(gpu: &Gpu) -> Self {
        let common = include_str!("shaders/common.wgsl");
        let body = gpu.shader("body", &[common, include_str!("shaders/body.wgsl")]);
        let lines = gpu.shader("lines", &[common, include_str!("shaders/lines.wgsl")]);
        let ground = gpu.shader("ground", &[common, include_str!("shaders/ground.wgsl")]);
        let post = gpu.shader("post", &[common, include_str!("shaders/post.wgsl")]);
        let frame_layout = gpu.layout("frame", &[gpu::uniform_entry(0)]);
        let body_layout = gpu.layout(
            "body",
            &[
                gpu::tex_entry(0, gpu::FLOAT_NF, gpu::D2),
                gpu::tex_entry(1, gpu::FLOAT, gpu::CUBE),
                gpu::tex_entry(2, gpu::FLOAT, gpu::CUBE),
                gpu::tex_entry(3, gpu::FLOAT, gpu::D2),
                gpu::sampler_entry(4),
            ],
        );
        let ground_layout = gpu.layout(
            "ground",
            &[
                gpu::tex_entry(0, gpu::FLOAT, gpu::D2),
                gpu::sampler_entry(1),
            ],
        );
        let post_layout = gpu.layout(
            "post",
            &[
                gpu::tex_entry(0, wgpu::TextureSampleType::Depth, gpu::D2),
                gpu::tex_entry(1, gpu::FLOAT_NF, gpu::D2),
            ],
        );
        let body_pl = gpu.pipeline_layout("body", &[&frame_layout, &body_layout]);
        let frame_pl = gpu.pipeline_layout("frame", &[&frame_layout]);
        let ground_pl = gpu.pipeline_layout("ground", &[&frame_layout, &ground_layout]);
        let post_pl = gpu.pipeline_layout("post", &[&frame_layout, &post_layout]);
        let body_v = [
            vertex(12, &POS, wgpu::VertexStepMode::Vertex),
            vertex(12, &NORMAL, wgpu::VertexStepMode::Vertex),
        ];
        let positions = [vertex(12, &POS, wgpu::VertexStepMode::Vertex)];
        let segments = [vertex(24, &SEG, wgpu::VertexStepMode::Instance)];
        let silhouettes = [vertex(48, &SIL, wgpu::VertexStepMode::Instance)];
        let depth = |write, bias| depth_state(wgpu::CompareFunction::LessEqual, write, bias);
        let bias = wgpu::DepthBiasState {
            constant: 8,
            slope_scale: 1.0,
            clamp: 0.0,
        };
        let none = wgpu::DepthBiasState::default();
        let pipe = |label, layout, module, vs, fs, buf, format, blend, ds, samples, strip| {
            Pipe {
                label,
                layout,
                module,
                vs,
                fs,
                buffers: buf,
                format,
                blend,
                depth: ds,
                samples,
                cull: None,
                strip,
                a2c: false,
            }
            .build(gpu.device)
        };
        let full = |label, fs, format| {
            pipe(
                label,
                &post_pl,
                &post,
                "vs_full",
                Some(fs),
                &[],
                Some(format),
                None,
                None,
                1,
                false,
            )
        };
        Self {
            prepass: pipe(
                "prepass",
                &body_pl,
                &body,
                "vs_body",
                Some("fs_prepass"),
                &body_v,
                Some(HDR_FORMAT),
                None,
                depth(true, none),
                1,
                false,
            ),
            body: pipe(
                "body",
                &body_pl,
                &body,
                "vs_body",
                Some("fs_body"),
                &body_v,
                Some(OUTPUT_FORMAT),
                None,
                depth(true, bias),
                SAMPLES,
                false,
            ),
            preview: pipe(
                "preview",
                &body_pl,
                &body,
                "vs_body",
                Some("fs_preview"),
                &body_v,
                Some(OUTPUT_FORMAT),
                Some(wgpu::BlendState::ALPHA_BLENDING),
                depth(false, none),
                SAMPLES,
                false,
            ),
            highlight_face: pipe(
                "highlight faces",
                &body_pl,
                &body,
                "vs_body",
                Some("fs_highlight"),
                &body_v,
                Some(OUTPUT_FORMAT),
                Some(wgpu::BlendState::ALPHA_BLENDING),
                highlight_depth(),
                SAMPLES,
                false,
            ),
            highlight_edge: pipe(
                "highlight edges",
                &frame_pl,
                &lines,
                "vs_edge",
                Some("fs_line"),
                &segments,
                Some(OUTPUT_FORMAT),
                Some(wgpu::BlendState::ALPHA_BLENDING),
                highlight_line_depth(),
                SAMPLES,
                true,
            ),
            height: pipe(
                "height",
                &body_pl,
                &body,
                "vs_height",
                None,
                &positions,
                None,
                None,
                depth(true, none),
                1,
                false,
            ),
            ground: pipe(
                "ground",
                &ground_pl,
                &ground,
                "vs_ground",
                Some("fs_ground"),
                &[],
                Some(OUTPUT_FORMAT),
                None,
                depth(false, none),
                SAMPLES,
                false,
            ),
            edge: pipe(
                "edges",
                &frame_pl,
                &lines,
                "vs_edge",
                Some("fs_line"),
                &segments,
                Some(OUTPUT_FORMAT),
                Some(wgpu::BlendState::ALPHA_BLENDING),
                depth(false, none),
                SAMPLES,
                true,
            ),
            silhouette: pipe(
                "silhouettes",
                &frame_pl,
                &lines,
                "vs_sil",
                Some("fs_line"),
                &silhouettes,
                Some(OUTPUT_FORMAT),
                Some(wgpu::BlendState::ALPHA_BLENDING),
                depth(false, none),
                SAMPLES,
                true,
            ),
            contact_occ: full("contact occlusion", "fs_contact_occ", OCC_FORMAT),
            contact_h: full("contact h", "fs_contact_blur_h", OCC_FORMAT),
            contact_v: full("contact v", "fs_contact_blur_v", OCC_FORMAT),
            gtao: full("gtao", "fs_gtao", AO_FORMAT),
            ao_h: full("AO h", "fs_ao_blur_h", AO_FORMAT),
            ao_v: full("AO v", "fs_ao_blur_v", AO_FORMAT),
            frame_layout,
            body_layout,
            ground_layout,
            post_layout,
        }
    }
}

/// Adapted from SolidCraft view.rs::highlight_depth at 6bde126a5cabd1f5af0c68e6955bd0176fab805a.
/// Forward depth makes negative bias pull toward the eye. Shared geometry needs OneCAD's
/// existing -1/-1 offset, rather than the donor's -8 constant for separate GPU-plan meshes.
pub(crate) fn highlight_depth() -> Option<wgpu::DepthStencilState> {
    depth_state(
        wgpu::CompareFunction::LessEqual,
        false,
        wgpu::DepthBiasState {
            constant: -1,
            slope_scale: -1.0,
            clamp: 0.0,
        },
    )
}

pub(crate) fn highlight_line_depth() -> Option<wgpu::DepthStencilState> {
    depth_state(
        wgpu::CompareFunction::Always,
        false,
        wgpu::DepthBiasState::default(),
    )
}
