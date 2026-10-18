//! Headless device and small wgpu helpers shared by the passes.

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter: String,
    pub timestamps: bool,
}

impl Gpu {
    pub fn new() -> Result<Gpu, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .map_err(|e| format!("no adapter: {e}"))?;
        let info = adapter.get_info();
        let timestamps = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let mut required_features = wgpu::Features::empty();
        if timestamps {
            required_features |= wgpu::Features::TIMESTAMP_QUERY;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("render-look"),
            required_features,
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| format!("no device: {e}"))?;
        Ok(Gpu { device, queue, adapter: format!("{} ({:?})", info.name, info.backend), timestamps })
    }

    pub fn wait(&self) {
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    }

    pub fn shader(&self, label: &str, sources: &[&str]) -> wgpu::ShaderModule {
        self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(sources.concat().into()),
        })
    }

    pub fn texture(
        &self,
        label: &str,
        size: [u32; 2],
        format: wgpu::TextureFormat,
        samples: u32,
        usage: wgpu::TextureUsages,
    ) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    }

    pub fn layout(&self, label: &str, entries: &[wgpu::BindGroupLayoutEntry]) -> wgpu::BindGroupLayout {
        self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries })
    }

    pub fn bind(&self, label: &str, layout: &wgpu::BindGroupLayout, resources: &[wgpu::BindingResource<'_>]) -> wgpu::BindGroup {
        let entries: Vec<wgpu::BindGroupEntry<'_>> =
            resources.iter().enumerate().map(|(i, r)| wgpu::BindGroupEntry { binding: i as u32, resource: r.clone() }).collect();
        self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout, entries: &entries })
    }

    pub fn pipeline_layout(&self, label: &str, groups: &[&wgpu::BindGroupLayout]) -> wgpu::PipelineLayout {
        let groups: Vec<Option<&wgpu::BindGroupLayout>> = groups.iter().map(|g| Some(*g)).collect();
        self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &groups,
            immediate_size: 0,
        })
    }
}

pub fn view(t: &wgpu::Texture) -> wgpu::TextureView {
    t.create_view(&wgpu::TextureViewDescriptor::default())
}

pub fn tex_entry(binding: u32, sample_type: wgpu::TextureSampleType, dim: wgpu::TextureViewDimension) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture { sample_type, view_dimension: dim, multisampled: false },
        count: None,
    }
}

pub fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

pub fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT.union(wgpu::ShaderStages::COMPUTE),
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

pub const FLOAT: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: true };
pub const FLOAT_NF: wgpu::TextureSampleType = wgpu::TextureSampleType::Float { filterable: false };
pub const D2: wgpu::TextureViewDimension = wgpu::TextureViewDimension::D2;
pub const CUBE: wgpu::TextureViewDimension = wgpu::TextureViewDimension::Cube;

/// One render pipeline description, filled in by the caller's defaults.
pub struct Pipe<'a> {
    pub label: &'a str,
    pub layout: &'a wgpu::PipelineLayout,
    pub module: &'a wgpu::ShaderModule,
    pub vs: &'a str,
    pub fs: Option<&'a str>,
    pub buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    pub format: Option<wgpu::TextureFormat>,
    pub blend: Option<wgpu::BlendState>,
    pub depth: Option<wgpu::DepthStencilState>,
    pub samples: u32,
    pub cull: Option<wgpu::Face>,
    pub strip: bool,
    /// MSAA alpha-to-coverage: the fragment alpha becomes the sample mask (no blending).
    pub a2c: bool,
}

impl Pipe<'_> {
    pub fn build(&self, device: &wgpu::Device) -> wgpu::RenderPipeline {
        // Alpha-to-coverage outputs coverage in alpha; it must drive the sample mask only, not land
        // in the target (the resolved image would come out partly transparent).
        let write_mask = if self.a2c { wgpu::ColorWrites::COLOR } else { wgpu::ColorWrites::ALL };
        let targets = [self.format.map(|format| wgpu::ColorTargetState { format, blend: self.blend, write_mask })];
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(self.label),
            layout: Some(self.layout),
            vertex: wgpu::VertexState {
                module: self.module,
                entry_point: Some(self.vs),
                buffers: self.buffers,
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: if self.strip { wgpu::PrimitiveTopology::TriangleStrip } else { wgpu::PrimitiveTopology::TriangleList },
                cull_mode: self.cull,
                ..Default::default()
            },
            depth_stencil: self.depth.clone(),
            multisample: wgpu::MultisampleState { count: self.samples, mask: !0, alpha_to_coverage_enabled: self.a2c },
            fragment: self.fs.map(|fs| wgpu::FragmentState {
                module: self.module,
                entry_point: Some(fs),
                targets: if self.format.is_some() { &targets } else { &[] },
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        })
    }
}

pub fn depth_state(compare: wgpu::CompareFunction, write: bool, bias: wgpu::DepthBiasState) -> Option<wgpu::DepthStencilState> {
    Some(wgpu::DepthStencilState {
        format: wgpu::TextureFormat::Depth32Float,
        depth_write_enabled: Some(write),
        depth_compare: Some(compare),
        stencil: Default::default(),
        bias,
    })
}

pub fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
