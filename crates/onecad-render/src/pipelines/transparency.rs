use crate::{
    gpu::{self, Gpu, Pipe},
    targets::{OUTPUT_FORMAT, SAMPLES},
};

pub(super) fn pipelines(
    gpu: &Gpu,
    body_layout: &wgpu::PipelineLayout,
    body: &wgpu::ShaderModule,
    vertices: &[Option<wgpu::VertexBufferLayout<'_>>],
    frame: &wgpu::BindGroupLayout,
    depth_format: wgpu::TextureFormat,
) -> (
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::BindGroupLayout,
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
) {
    let additive = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    };
    let reveal = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Zero,
        dst_factor: wgpu::BlendFactor::OneMinusSrc,
        operation: wgpu::BlendOperation::Add,
    };
    let formats = crate::targets::oit_formats(gpu.device);
    let targets = [
        Some(wgpu::ColorTargetState {
            format: formats[0],
            blend: Some(wgpu::BlendState {
                color: additive,
                alpha: additive,
            }),
            write_mask: wgpu::ColorWrites::ALL,
        }),
        Some(wgpu::ColorTargetState {
            format: formats[1],
            blend: Some(wgpu::BlendState {
                color: reveal,
                alpha: reveal,
            }),
            write_mask: wgpu::ColorWrites::ALL,
        }),
    ];
    let accumulation = Pipe {
        label: "Xray weighted accumulation",
        layout: body_layout,
        module: body,
        vs: "vs_body",
        fs: Some("fs_xray"),
        buffers: vertices,
        format: Some(formats[0]),
        blend: None,
        depth: {
            let mut d = gpu::depth_state(wgpu::CompareFunction::Always, false, Default::default());
            if let Some(d) = &mut d {
                d.format = depth_format;
            }
            d
        },
        samples: 1,
        cull: None,
        strip: false,
        a2c: false,
    }
    .build_targets(gpu.device, &targets);
    let layout = gpu.layout(
        "Xray textures",
        &[
            gpu::tex_entry(0, gpu::FLOAT_NF, gpu::D2),
            gpu::tex_entry(1, gpu::FLOAT_NF, gpu::D2),
        ],
    );
    let pipeline_layout = gpu.pipeline_layout("Xray composite", &[frame, &layout]);
    let module = gpu.shader("Xray composite", &[include_str!("../shaders/oit.wgsl")]);
    let mut depth = gpu::depth_state(wgpu::CompareFunction::Always, false, Default::default());
    if let Some(depth) = &mut depth {
        depth.format = depth_format;
    }
    let composite = Pipe {
        label: "Xray linear composite",
        layout: &pipeline_layout,
        module: &module,
        vs: "vs_full",
        fs: Some("fs_composite"),
        buffers: &[],
        format: Some(OUTPUT_FORMAT),
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        depth,
        samples: SAMPLES,
        cull: None,
        strip: false,
        a2c: false,
    }
    .build(gpu.device);
    let mask = Pipe {
        label: "Xray section winding",
        layout: body_layout,
        module: body,
        vs: "vs_mask",
        fs: Some("fs_xray_mask"),
        buffers: vertices,
        format: Some(formats[0]),
        blend: None,
        depth: super::section::state(depth_format, wgpu::CompareFunction::Always, true),
        samples: 1,
        cull: None,
        strip: false,
        a2c: false,
    }
    .build_targets(gpu.device, &targets);
    let cap = Pipe {
        label: "Xray section contribution",
        layout: body_layout,
        module: body,
        vs: "vs_cap",
        fs: Some("fs_xray_cap"),
        buffers: &[],
        format: Some(formats[0]),
        blend: None,
        depth: super::section::state(depth_format, wgpu::CompareFunction::Always, false),
        samples: 1,
        cull: None,
        strip: false,
        a2c: false,
    }
    .build_targets(gpu.device, &targets);
    (accumulation, composite, layout, mask, cap)
}
