use std::path::Path;

pub(super) struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    pub async fn new() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: None,
                power_preference: wgpu::PowerPreference::LowPower,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .expect("a GPU or software Vulkan adapter is required for this explicit capture");
        eprintln!("Extension UI capture adapter: {:?}", adapter.get_info());
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .unwrap();
        Self { device, queue }
    }

    pub fn painter(&self) -> egui_wgpu::Renderer {
        egui_wgpu::Renderer::new(
            &self.device,
            wgpu::TextureFormat::Rgba8Unorm,
            egui_wgpu::RendererOptions::PREDICTABLE,
        )
    }

    pub fn save(
        &self,
        painter: &mut egui_wgpu::Renderer,
        jobs: &[egui::ClippedPrimitive],
        size: [u32; 2],
        path: &Path,
    ) {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("extension UI review"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: size,
            pixels_per_point: 1.0,
        };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let mut commands =
            painter.update_buffers(&self.device, &self.queue, &mut encoder, jobs, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("production extension UI"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            painter.render(&mut pass, jobs, &screen);
        }
        commands.push(encoder.finish());
        self.queue.submit(commands);
        self.write_png(&texture, size, path);
    }

    fn write_png(&self, texture: &wgpu::Texture, size: [u32; 2], path: &Path) {
        let stride = (size[0] * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("UI review readback"),
            size: u64::from(stride) * u64::from(size[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(size[1]),
                },
            },
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |result| result.unwrap());
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .unwrap();
        let data = buffer.slice(..).get_mapped_range().unwrap();
        let pixels: Vec<_> = data
            .chunks(stride as usize)
            .flat_map(|row| row[..size[0] as usize * 4].iter().copied())
            .collect();
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[0] != 255 || p[1] != 255 || p[2] != 255),
            "empty UI capture"
        );
        let mut encoder = png::Encoder::new(std::fs::File::create(path).unwrap(), size[0], size[1]);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
    }
}
