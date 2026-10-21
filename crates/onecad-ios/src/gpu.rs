//! One Metal device for egui and the CAD renderer, and the swapchain over the
//! host's `CAMetalLayer`. The device exists before any surface so the editor and
//! its services start at create; any `CAMetalLayer` accepts this device's format.
use egui_wgpu::RenderState;
use std::ffi::c_void;
use std::sync::Arc;

/// egui paints in gamma space, as eframe does on macOS; the layer must not be sRGB.
pub const SURFACE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

pub fn create_render_state() -> Result<RenderState, String> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::METAL;
    let instance = wgpu::Instance::new(descriptor);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .map_err(|error| format!("Metal adapter: {error}"))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&device_descriptor(&adapter)))
        .map_err(|error| format!("Metal device: {error}"))?;
    let renderer = egui_wgpu::Renderer::new(&device, SURFACE_FORMAT, Default::default());
    Ok(RenderState {
        available_adapters: vec![adapter.clone()],
        adapter,
        instance,
        device,
        queue,
        target_format: SURFACE_FORMAT,
        renderer: Arc::new(egui::mutex::RwLock::new(renderer)),
    })
}

/// egui-wgpu's default device request plus the renderer's optional features,
/// matching the desktop host's `viewport_gpu_configuration`. Limits the adapter
/// cannot meet are lowered to its own: the iOS Simulator's Metal adapter reports
/// 15 inter-stage shader variables against the default 16.
fn device_descriptor(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    let desktop = wgpu::Limits {
        max_texture_dimension_2d: 8192,
        ..wgpu::Limits::default()
    };
    wgpu::DeviceDescriptor {
        label: Some("egui wgpu device"),
        required_features: onecad_render::required_device_features(adapter.features()),
        required_limits: desktop.or_worse_values_from(&adapter.limits()),
        ..Default::default()
    }
}

pub struct Surface {
    surface: wgpu::Surface<'static>,
    layer: *mut c_void,
    config: wgpu::SurfaceConfiguration,
}

/// What the frame should do after a failed acquire.
pub enum Acquired {
    Frame(wgpu::SurfaceTexture, bool),
    Skip,
}

impl Surface {
    /// # Safety
    /// `layer` must be a live `CAMetalLayer` the host keeps retained until
    /// [`Surface`] is dropped (`surface_detach` or destroy).
    pub unsafe fn new(
        gpu: &RenderState,
        layer: *mut c_void,
        size: [u32; 2],
    ) -> Result<Self, String> {
        // SAFETY: the caller guarantees `layer` stays a live CAMetalLayer for the
        // surface's lifetime.
        let surface = unsafe { create(&gpu.instance, layer)? };
        let caps = surface.get_capabilities(&gpu.adapter);
        if !caps.formats.contains(&SURFACE_FORMAT) {
            return Err(format!(
                "layer lacks {SURFACE_FORMAT:?}: {:?}",
                caps.formats
            ));
        }
        let (width, height) = (size[0].max(1), size[1].max(1));
        // As egui-wgpu does: the adapter's defaults (colour space included) with
        // the egui format and the latency eframe uses.
        let defaults = surface
            .get_default_config(&gpu.adapter, width, height)
            .ok_or("the Metal adapter cannot present to this layer")?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: SURFACE_FORMAT,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes.first().copied().unwrap_or_default(),
            view_formats: vec![SURFACE_FORMAT],
            ..defaults
        };
        surface.configure(&gpu.device, &config);
        Ok(Self {
            surface,
            layer,
            config,
        })
    }

    pub fn resize(&mut self, gpu: &RenderState, size: [u32; 2]) {
        self.config.width = size[0].max(1);
        self.config.height = size[1].max(1);
        self.surface.configure(&gpu.device, &self.config);
    }

    pub fn size(&self) -> [u32; 2] {
        [self.config.width, self.config.height]
    }

    /// Acquires the next drawable, reconfiguring or recreating the surface for
    /// the next frame when it is outdated or lost.
    pub fn acquire(&mut self, gpu: &RenderState) -> Acquired {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => Acquired::Frame(frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Acquired::Frame(frame, true),
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&gpu.device, &self.config);
                Acquired::Skip
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.recreate(gpu);
                Acquired::Skip
            }
            other => {
                tracing::debug!(?other, "surface frame skipped");
                Acquired::Skip
            }
        }
    }

    pub fn reconfigure(&self, gpu: &RenderState) {
        self.surface.configure(&gpu.device, &self.config);
    }

    fn recreate(&mut self, gpu: &RenderState) {
        // SAFETY: `self.layer` is the layer `new` was given, still retained by the
        // host while this surface exists (contract of `Surface::new`).
        match unsafe { create(&gpu.instance, self.layer) } {
            Ok(surface) => {
                surface.configure(&gpu.device, &self.config);
                self.surface = surface;
            }
            Err(error) => tracing::error!(%error, "surface recreation failed"),
        }
    }
}

/// # Safety
/// `layer` must be a live `CAMetalLayer` that outlives the returned surface.
unsafe fn create(
    instance: &wgpu::Instance,
    layer: *mut c_void,
) -> Result<wgpu::Surface<'static>, String> {
    // SAFETY: forwarded from the caller: `layer` is a live CAMetalLayer that
    // outlives the surface; wgpu retains it for the surface's lifetime.
    unsafe { instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer)) }
        .map_err(|error| format!("Metal surface: {error}"))
}
