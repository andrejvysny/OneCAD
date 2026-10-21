use crate::gpu::{self, depth_state, Gpu, Pipe};
use crate::targets::{AO_FORMAT, HDR_FORMAT, OCC_FORMAT, OUTPUT_FORMAT, SAMPLES};
mod preview;
mod section;
mod transparency;

pub(crate) struct Pipelines {
    pub preview_cap_clear: wgpu::RenderPipeline,
    pub preview_cap: wgpu::RenderPipeline,
    pub oit_cap: wgpu::RenderPipeline,
    pub oit_mask: wgpu::RenderPipeline,
    pub cap_depth: wgpu::RenderPipeline,
    pub oit: wgpu::RenderPipeline,
    pub oit_compose: wgpu::RenderPipeline,
    pub oit_layout: wgpu::BindGroupLayout,
    pub compose_layout: wgpu::BindGroupLayout,
    pub compose: wgpu::RenderPipeline,
    pub cap: wgpu::RenderPipeline,
    pub pre_cap: wgpu::RenderPipeline,
    pub height_cap: wgpu::RenderPipeline,
    pub cap_mask: wgpu::RenderPipeline,
    pub pre_cap_mask: wgpu::RenderPipeline,
    pub height_cap_mask: wgpu::RenderPipeline,
    pub body_depth: wgpu::RenderPipeline,
    pub hidden_silhouette: wgpu::RenderPipeline,
    pub hidden: wgpu::RenderPipeline,
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
        let format = crate::options::depth_format(gpu.device);
        let depth = |write, bias| {
            let mut state = depth_state(wgpu::CompareFunction::GreaterEqual, write, bias);
            if let Some(state) = &mut state {
                state.format = format;
            }
            state
        };
        let cap_state = || section::state(format, wgpu::CompareFunction::GreaterEqual, false);
        let bias = wgpu::DepthBiasState {
            constant: -8,
            slope_scale: -1.0,
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
        let compose_module = gpu.shader("composition", &[include_str!("shaders/compose.wgsl")]);
        let compose_layout = gpu.layout(
            "composition",
            &[
                gpu::tex_entry(0, gpu::FLOAT, gpu::D2),
                gpu::sampler_entry(1),
            ],
        );
        let compose_pl = gpu.pipeline_layout("composition", &[&compose_layout]);
        let cap_pipe = |name, fs, format, samples| {
            Pipe {
                label: name,
                layout: &body_pl,
                module: &body,
                vs: "vs_cap",
                fs: Some(fs),
                buffers: &[],
                format: Some(format),
                blend: if samples == SAMPLES {
                    Some(wgpu::BlendState::ALPHA_BLENDING)
                } else {
                    None
                },
                depth: cap_state(),
                samples,
                cull: None,
                strip: false,
                a2c: false,
            }
            .build(gpu.device)
        };
        let mask_pipe = |name, vs, fs, buffers, color, samples| {
            pipe(
                name,
                &body_pl,
                &body,
                vs,
                Some(fs),
                buffers,
                color,
                color.map(|_| wgpu::BlendState::ALPHA_BLENDING),
                section::state(format, wgpu::CompareFunction::Always, true),
                samples,
                false,
            )
        };
        let (oit, oit_compose, oit_layout, oit_mask, oit_cap) =
            transparency::pipelines(gpu, &body_pl, &body, &body_v, &frame_layout, format);
        let (preview_cap_clear, preview_cap) = preview::pipelines(gpu, &body_pl, &body, format);
        Self {
            preview_cap_clear,
            preview_cap,
            oit_cap,
            oit_mask,
            cap_depth: cap_pipe("Xray cap depth", "fs_cap_depth", OUTPUT_FORMAT, SAMPLES),
            oit,
            oit_compose,
            oit_layout,
            compose: pipe(
                "composition",
                &compose_pl,
                &compose_module,
                "vs_full",
                Some("fs_compose"),
                &[],
                Some(OUTPUT_FORMAT),
                None,
                None,
                1,
                false,
            ),
            compose_layout,
            cap: cap_pipe("section cap", "fs_cap", OUTPUT_FORMAT, SAMPLES),
            pre_cap: cap_pipe("prepass section cap", "fs_pre_cap", HDR_FORMAT, 1),
            cap_mask: mask_pipe(
                "section winding",
                "vs_mask",
                "fs_depth",
                &body_v,
                Some(OUTPUT_FORMAT),
                SAMPLES,
            ),
            pre_cap_mask: mask_pipe(
                "prepass section winding",
                "vs_mask",
                "fs_depth",
                &body_v,
                Some(HDR_FORMAT),
                1,
            ),
            height_cap_mask: mask_pipe(
                "contact section winding",
                "vs_height",
                "fs_height",
                &positions,
                None,
                1,
            ),
            height_cap: pipe(
                "contact section cap",
                &body_pl,
                &body,
                "vs_cap",
                Some("fs_height_cap"),
                &[],
                None,
                None,
                section::state(format, wgpu::CompareFunction::LessEqual, false),
                1,
                false,
            ),
            body_depth: pipe(
                "transparent depth",
                &body_pl,
                &body,
                "vs_body",
                Some("fs_depth"),
                &body_v,
                Some(OUTPUT_FORMAT),
                Some(wgpu::BlendState::ALPHA_BLENDING),
                depth(true, bias),
                SAMPLES,
                false,
            ),
            hidden_silhouette: pipe(
                "hidden silhouettes",
                &frame_pl,
                &lines,
                "vs_sil",
                Some("fs_hidden"),
                &silhouettes,
                Some(OUTPUT_FORMAT),
                Some(wgpu::BlendState::ALPHA_BLENDING),
                {
                    let mut state = depth(false, none);
                    if let Some(state) = &mut state {
                        state.depth_compare = Some(wgpu::CompareFunction::Less);
                    }
                    state
                },
                SAMPLES,
                true,
            ),
            hidden: pipe(
                "hidden edges",
                &frame_pl,
                &lines,
                "vs_edge",
                Some("fs_hidden"),
                &segments,
                Some(OUTPUT_FORMAT),
                Some(wgpu::BlendState::ALPHA_BLENDING),
                {
                    let mut state = depth(false, none);
                    if let Some(state) = &mut state {
                        state.depth_compare = Some(wgpu::CompareFunction::Less);
                    }
                    state
                },
                SAMPLES,
                true,
            ),
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
                Some(wgpu::BlendState::ALPHA_BLENDING),
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
                {
                    let mut s = highlight_depth();
                    if let Some(s) = &mut s {
                        s.format = format;
                    }
                    s
                },
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
                {
                    let mut s = highlight_line_depth();
                    if let Some(s) = &mut s {
                        s.format = format;
                    }
                    s
                },
                SAMPLES,
                true,
            ),
            height: pipe(
                "height",
                &body_pl,
                &body,
                "vs_height",
                Some("fs_height"),
                &positions,
                None,
                None,
                section::height_depth(format),
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
                Some(wgpu::BlendState::ALPHA_BLENDING),
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
/// Reversed depth makes positive bias pull toward the eye. Shared geometry uses a small
/// one-step offset instead of the donor's larger separate-mesh offset.
pub(crate) fn highlight_depth() -> Option<wgpu::DepthStencilState> {
    depth_state(
        wgpu::CompareFunction::GreaterEqual,
        false,
        wgpu::DepthBiasState {
            constant: 1,
            slope_scale: 1.0,
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
