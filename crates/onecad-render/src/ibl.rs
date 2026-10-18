//! Procedural studio IBL, prefiltered once at start-up (no HDRI file): a 512² radiance cube with
//! a full mip chain, a 256² GGX-prefiltered specular cube (6 roughness mips), a 32² irradiance
//! cube and a 128² split-sum BRDF LUT.

use crate::gpu::{self, Gpu, Pipe};

pub const ENV_SIZE: u32 = 512;
pub const SPEC_SIZE: u32 = 256;
pub const SPEC_MIPS: u32 = 6;
pub const IRR_SIZE: u32 = 32;
pub const LUT_SIZE: u32 = 128;

pub struct Ibl {
    pub irradiance: wgpu::TextureView,
    pub specular: wgpu::TextureView,
    pub lut: wgpu::TextureView,
}

fn cube(gpu: &Gpu, label: &str, size: u32, mips: u32) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 6,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn face_view(t: &wgpu::Texture, face: u32, mip: u32) -> wgpu::TextureView {
    t.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2),
        base_mip_level: mip,
        mip_level_count: Some(1),
        base_array_layer: face,
        array_layer_count: Some(1),
        ..Default::default()
    })
}

fn cube_view(t: &wgpu::Texture, base_mip: u32, mips: Option<u32>) -> wgpu::TextureView {
    t.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::Cube),
        base_mip_level: base_mip,
        mip_level_count: mips,
        ..Default::default()
    })
}

fn draw(
    enc: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    pipe: &wgpu::RenderPipeline,
    bind: Option<&wgpu::BindGroup>,
    k: u32,
) {
    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("ibl"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
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
    pass.set_pipeline(pipe);
    if let Some(b) = bind {
        pass.set_bind_group(0, b, &[]);
    }
    pass.draw(0..3, k..k + 1);
}

impl Ibl {
    pub fn build(gpu: &Gpu, sampler: &wgpu::Sampler) -> Ibl {
        let module = gpu.shader("ibl", &[include_str!("shaders/ibl.wgsl")]);
        let bgl = gpu.layout(
            "ibl_src",
            &[
                gpu::tex_entry(0, gpu::FLOAT, gpu::CUBE),
                gpu::sampler_entry(1),
            ],
        );
        let with_src = gpu.pipeline_layout("ibl_src", &[&bgl]);
        let no_src = gpu.pipeline_layout("ibl_none", &[]);
        let hdr = wgpu::TextureFormat::Rgba16Float;
        let pipe =
            |label: &str, fs: &str, layout: &wgpu::PipelineLayout, format: wgpu::TextureFormat| {
                Pipe {
                    label,
                    layout,
                    module: &module,
                    vs: "vs_face",
                    fs: Some(fs),
                    buffers: &[],
                    format: Some(format),
                    blend: None,
                    depth: None,
                    samples: 1,
                    cull: None,
                    strip: false,
                    a2c: false,
                }
                .build(gpu.device)
            };
        let env_pipe = pipe("ibl_env", "fs_env", &no_src, hdr);
        let down_pipe = pipe("ibl_down", "fs_down", &with_src, hdr);
        let pre_pipe = pipe("ibl_prefilter", "fs_prefilter", &with_src, hdr);
        let irr_pipe = pipe("ibl_irradiance", "fs_irradiance", &with_src, hdr);
        let lut_pipe = pipe("ibl_lut", "fs_lut", &no_src, wgpu::TextureFormat::Rg16Float);

        let env_mips = ENV_SIZE.ilog2() + 1;
        let env = cube(gpu, "env", ENV_SIZE, env_mips);
        let spec = cube(gpu, "spec", SPEC_SIZE, SPEC_MIPS);
        let irr = cube(gpu, "irr", IRR_SIZE, 1);
        let lut = gpu.texture(
            "brdf_lut",
            [LUT_SIZE, LUT_SIZE],
            wgpu::TextureFormat::Rg16Float,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        for f in 0..6 {
            draw(&mut enc, &face_view(&env, f, 0), &env_pipe, None, f);
        }
        for m in 1..env_mips {
            let src = gpu.bind(
                "ibl_down",
                &bgl,
                &[
                    wgpu::BindingResource::TextureView(&cube_view(&env, m - 1, Some(1))),
                    wgpu::BindingResource::Sampler(sampler),
                ],
            );
            for f in 0..6 {
                draw(&mut enc, &face_view(&env, f, m), &down_pipe, Some(&src), f);
            }
        }
        let full = gpu.bind(
            "ibl_env",
            &bgl,
            &[
                wgpu::BindingResource::TextureView(&cube_view(&env, 0, None)),
                wgpu::BindingResource::Sampler(sampler),
            ],
        );
        for m in 0..SPEC_MIPS {
            for f in 0..6 {
                draw(
                    &mut enc,
                    &face_view(&spec, f, m),
                    &pre_pipe,
                    Some(&full),
                    m * 6 + f,
                );
            }
        }
        for f in 0..6 {
            draw(&mut enc, &face_view(&irr, f, 0), &irr_pipe, Some(&full), f);
        }
        draw(&mut enc, &gpu::view(&lut), &lut_pipe, None, 0);
        gpu.queue.submit([enc.finish()]);
        Ibl {
            irradiance: cube_view(&irr, 0, None),
            specular: cube_view(&spec, 0, None),
            lut: gpu::view(&lut),
        }
    }
}
