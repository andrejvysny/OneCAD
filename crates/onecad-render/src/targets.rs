use crate::{
    gpu::{self, Gpu},
    ibl::Ibl,
    pipelines::Pipelines,
};
pub const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub(crate) const SAMPLES: u32 = 4;
pub(crate) const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub(crate) const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
pub(crate) const OCC_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

pub(crate) struct Targets {
    pub _output: wgpu::Texture,
    pub output_view: wgpu::TextureView,
    pub color: wgpu::TextureView,
    pub depth: wgpu::TextureView,
    pub pre_depth: wgpu::TextureView,
    pub pre_normal: wgpu::TextureView,
    pub ao_raw: wgpu::TextureView,
    pub ao_tmp: wgpu::TextureView,
    pub ao_final: wgpu::TextureView,
    pub height: wgpu::TextureView,
    pub occ: wgpu::TextureView,
    pub occ_tmp: wgpu::TextureView,
    pub occ_final: wgpu::TextureView,
    pub body_bind: wgpu::BindGroup,
    pub ground_bind: wgpu::BindGroup,
    pub post_binds: [wgpu::BindGroup; 6],
}
impl Targets {
    pub fn new(gpu: &Gpu, pipes: &Pipelines, ibl: &Ibl, size: [u32; 2]) -> Self {
        use wgpu::TextureUsages as U;
        let rt = U::RENDER_ATTACHMENT | U::TEXTURE_BINDING;
        let target = |label, size, format, samples, usage| {
            gpu::view(&gpu.texture(label, size, format, samples, usage))
        };
        let output = gpu.texture("CAD output", size, OUTPUT_FORMAT, 1, rt | U::COPY_SRC);
        let output_view = gpu::view(&output);
        let color = target(
            "MSAA color",
            size,
            OUTPUT_FORMAT,
            SAMPLES,
            U::RENDER_ATTACHMENT,
        );
        let depth = target(
            "MSAA depth",
            size,
            wgpu::TextureFormat::Depth32Float,
            SAMPLES,
            U::RENDER_ATTACHMENT,
        );
        let pre_depth = target(
            "prepass depth",
            size,
            wgpu::TextureFormat::Depth32Float,
            1,
            rt,
        );
        let pre_normal = target("prepass normal", size, HDR_FORMAT, 1, rt);
        let ao_raw = target("AO raw", size, AO_FORMAT, 1, rt);
        let ao_tmp = target("AO tmp", size, AO_FORMAT, 1, rt);
        let ao_final = target("AO final", size, AO_FORMAT, 1, rt);
        let height = target(
            "contact height",
            [1024; 2],
            wgpu::TextureFormat::Depth32Float,
            1,
            rt,
        );
        let occ = target("contact occlusion", [256; 2], OCC_FORMAT, 1, rt);
        let occ_tmp = target("contact tmp", [256; 2], OCC_FORMAT, 1, rt);
        let occ_final = target("contact final", [256; 2], OCC_FORMAT, 1, rt);
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("CAD linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        use wgpu::BindingResource::{Sampler, TextureView as Tv};
        let body_bind = gpu.bind(
            "body",
            &pipes.body_layout,
            &[
                Tv(&ao_final),
                Tv(&ibl.irradiance),
                Tv(&ibl.specular),
                Tv(&ibl.lut),
                Sampler(&sampler),
            ],
        );
        let ground_bind = gpu.bind(
            "ground",
            &pipes.ground_layout,
            &[Tv(&occ_final), Sampler(&sampler)],
        );
        let post = |depth: &wgpu::TextureView, tex: &wgpu::TextureView| {
            gpu.bind("post", &pipes.post_layout, &[Tv(depth), Tv(tex)])
        };
        let post_binds = [
            post(&height, &occ_tmp),
            post(&height, &occ),
            post(&height, &occ_tmp),
            post(&pre_depth, &pre_normal),
            post(&pre_depth, &ao_raw),
            post(&pre_depth, &ao_tmp),
        ];
        Self {
            _output: output,
            output_view,
            color,
            depth,
            pre_depth,
            pre_normal,
            ao_raw,
            ao_tmp,
            ao_final,
            height,
            occ,
            occ_tmp,
            occ_final,
            body_bind,
            ground_bind,
            post_binds,
        }
    }
}
