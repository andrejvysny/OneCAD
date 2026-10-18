use crate::{
    gpu::{Gpu, Pipe},
    targets::{OUTPUT_FORMAT, SAMPLES},
};

pub(super) fn pipelines(
    gpu: &Gpu,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    depth_format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let build = |label, fs, depth| {
        Pipe {
            label,
            layout,
            module,
            vs: "vs_cap",
            fs: Some(fs),
            buffers: &[],
            format: Some(OUTPUT_FORMAT),
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            depth,
            samples: SAMPLES,
            cull: None,
            strip: false,
            a2c: false,
        }
        .build(gpu.device)
    };
    (
        build(
            "preview section reset",
            "fs_clear_mask",
            super::section::clear(depth_format),
        ),
        build(
            "preview section cap",
            "fs_preview_cap",
            super::section::preview(depth_format),
        ),
    )
}
