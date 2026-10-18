use crate::{
    gpu::{self, Gpu},
    ibl::Ibl,
    pipelines::Pipelines,
};
pub const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub(crate) fn oit_formats(device: &wgpu::Device) -> [wgpu::TextureFormat; 2] {
    if device
        .features()
        .contains(wgpu::Features::FLOAT32_BLENDABLE)
    {
        [
            wgpu::TextureFormat::Rgba32Float,
            wgpu::TextureFormat::R32Float,
        ]
    } else {
        [HDR_FORMAT, AO_FORMAT]
    }
}
pub(crate) const SAMPLES: u32 = 4;
pub(crate) const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub(crate) const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
pub(crate) const OCC_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

pub(crate) struct Targets {
    pub oit_accum: wgpu::TextureView,
    pub oit_reveal: wgpu::TextureView,
    pub oit_bind: wgpu::BindGroup,
    pub _output: wgpu::Texture,
    pub output_view: wgpu::TextureView,
    pub output_attachment: wgpu::TextureView,
    pub color: wgpu::TextureView,
    pub resolved: wgpu::TextureView,
    pub compose: wgpu::BindGroup,
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
    pub fn try_new(
        gpu: &Gpu,
        pipes: &Pipelines,
        ibl: &Ibl,
        size: [u32; 2],
        options: crate::RenderOptions,
    ) -> Result<Self, crate::RenderError> {
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let targets = Self::new(gpu, pipes, ibl, size, options);
        // Allocation scopes contain no submitted work; popping does not wait for GPU completion.
        let invalid = pollster::block_on(validation.pop());
        let allocation = pollster::block_on(scope.pop());
        if let Some(error) = invalid.or(allocation) {
            return Err(crate::RenderError::Device(error.to_string()));
        }
        Ok(targets)
    }
    fn new(
        gpu: &Gpu,
        pipes: &Pipelines,
        ibl: &Ibl,
        size: [u32; 2],
        options: crate::RenderOptions,
    ) -> Self {
        use wgpu::TextureUsages as U;
        let rt = U::RENDER_ATTACHMENT | U::TEXTURE_BINDING;
        let target = |label, size, format, samples, usage| {
            gpu::view(&gpu.texture(label, size, format, samples, usage))
        };
        let output = gpu.texture("CAD output", size, OUTPUT_FORMAT, 1, rt | U::COPY_SRC);
        let output_attachment = gpu::view(&output);
        // egui-wgpu expects gamma-encoded values from a non-sRGB sampling view.
        // Internal attachment writes and downsampling still operate in linear light.
        let output_view = output.create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Rgba8Unorm),
            ..Default::default()
        });
        let render_size = size.map(|v| v * options.ssaa as u32);
        let resolved = if options.ssaa == 1.0 {
            output_attachment.clone()
        } else {
            target("supersampled resolve", render_size, OUTPUT_FORMAT, 1, rt)
        };
        let size = render_size;
        let depth_format = crate::options::depth_format(gpu.device);
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
            depth_format,
            SAMPLES,
            U::RENDER_ATTACHMENT,
        );
        let pre_texture = gpu.texture("prepass depth", size, depth_format, 1, rt);
        let pre_depth = gpu::view(&pre_texture);
        let pre_sample = pre_texture.create_view(&wgpu::TextureViewDescriptor {
            aspect: wgpu::TextureAspect::DepthOnly,
            ..Default::default()
        });
        let pre_normal = target("prepass normal", size, HDR_FORMAT, 1, rt);
        let ao_raw = target("AO raw", size, AO_FORMAT, 1, rt);
        let ao_tmp = target("AO tmp", size, AO_FORMAT, 1, rt);
        let ao_final = target("AO final", size, AO_FORMAT, 1, rt);
        let height_texture = gpu.texture("contact height", [1024; 2], depth_format, 1, rt);
        let height = gpu::view(&height_texture);
        let height_sample = height_texture.create_view(&wgpu::TextureViewDescriptor {
            aspect: wgpu::TextureAspect::DepthOnly,
            ..Default::default()
        });
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
            post(&height_sample, &occ_tmp),
            post(&height_sample, &occ),
            post(&height_sample, &occ_tmp),
            post(&pre_sample, &pre_normal),
            post(&pre_sample, &ao_raw),
            post(&pre_sample, &ao_tmp),
        ];
        let compose = gpu.bind(
            "CAD composition",
            &pipes.compose_layout,
            &[Tv(&resolved), Sampler(&sampler)],
        );
        let formats = oit_formats(gpu.device);
        let oit_accum = target("Xray accumulation", size, formats[0], 1, rt);
        let oit_reveal = target("Xray revealage", size, formats[1], 1, rt);
        let oit_bind = gpu.bind(
            "Xray resolve",
            &pipes.oit_layout,
            &[Tv(&oit_accum), Tv(&oit_reveal)],
        );
        Self {
            oit_accum,
            oit_reveal,
            oit_bind,
            resolved,
            compose,
            _output: output,
            output_view,
            output_attachment,
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
