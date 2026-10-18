//! The complete vendored SolidCraft GPU viewport hosted on OneCAD's shared device.
use crate::{
    flight::Flight,
    limits::{add, mul},
    BodyStyle, MemoryEstimate, PreviewIdentity, RenderError, RenderLimits, RenderStyle,
};
use onecad_scene::{BodyId, SceneIdentity, SceneSnapshot, ViewSnapshot};
use solidcraft_gpu::view::{self, Frame, GpuScene};
use std::sync::{Arc, Mutex};
#[path = "solidcraft_viewport_accounting.rs"]
mod accounting;
#[path = "solidcraft_viewport_frame.rs"]
mod frame;
#[path = "solidcraft_viewport_highlight.rs"]
mod highlight;
#[path = "solidcraft_viewport_prepare.rs"]
mod prepare;
pub use frame::SolidCraftOptions;
pub use prepare::{PreparedSolidCraftScene, SolidCraftPreparationBudget};

pub struct SolidCraftViewport {
    device: wgpu::Device,
    queue: wgpu::Queue,
    limits: RenderLimits,
    renderer: view::Renderer,
    target: Option<Target>,
    size: [u32; 2],
    flight: Flight,
    scene: Option<PreparedSolidCraftScene>,
    preview: Option<PreparedSolidCraftScene>,
    preview_id: Option<PreviewIdentity>,
    latest_preview: Option<u64>,
    estimate: MemoryEstimate,
    frame: Frame,
    options: SolidCraftOptions,
    sequence: u64,
    highlight_bytes: u64,
    retired_model_bytes: u64,
    retired_preview_bytes: u64,
    retired_highlight_bytes: u64,
    access_highwater_bytes: u64,
}
struct Target {
    _color: wgpu::Texture,
    color: wgpu::TextureView,
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
}
impl SolidCraftViewport {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        limits: RenderLimits,
    ) -> Result<Self, RenderError> {
        limits.validate_size(size, device)?;
        let estimate = limits.check(
            MemoryEstimate {
                target_bytes: target_bytes(size)?,
                ..Default::default()
            },
            MemoryEstimate::default(),
        )?;
        let renderer = view::Renderer::new(
            device,
            view::GpuTarget {
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                depth: Some(solidcraft_gpu::depth_format(device, true)),
                samples: 4,
                tess: false,
            },
            None,
        );
        let target = (!size.contains(&0)).then(|| Target::new(device, size));
        Ok(Self {
            device: device.clone(),
            queue: queue.clone(),
            limits,
            renderer,
            target,
            size,
            flight: Flight::Idle,
            scene: None,
            preview: None,
            preview_id: None,
            latest_preview: None,
            estimate,
            frame: frame::empty(),
            options: SolidCraftOptions::default(),
            sequence: 1,
            highlight_bytes: 0,
            retired_model_bytes: 0,
            retired_preview_bytes: 0,
            retired_highlight_bytes: 0,
            access_highwater_bytes: 0,
        })
    }
    pub fn size(&self) -> [u32; 2] {
        self.size
    }
    pub fn scene_identity(&self) -> Option<SceneIdentity> {
        self.scene.as_ref().map(|s| s.identity)
    }
    pub fn preview_identity(&self) -> Option<PreviewIdentity> {
        self.preview_id
    }
    pub fn output_view(&self) -> Option<&wgpu::TextureView> {
        self.target.as_ref().map(|t| &t.color)
    }
    pub fn memory_estimate(&self) -> MemoryEstimate {
        self.estimate
    }
    pub fn preparation_budget(&self) -> SolidCraftPreparationBudget {
        SolidCraftPreparationBudget {
            limits: self.limits,
            resident: self.estimate,
        }
    }
    pub fn replace_scene(&mut self, scene: &SceneSnapshot) -> Result<(), RenderError> {
        self.require_idle()?;
        self.replace_prepared_scene(PreparedSolidCraftScene::new(
            scene,
            self.preparation_budget(),
        )?)
    }
    pub fn replace_preview(
        &mut self,
        id: PreviewIdentity,
        scene: &SceneSnapshot,
    ) -> Result<(), RenderError> {
        self.require_idle()?;
        self.replace_prepared_preview(
            id,
            PreparedSolidCraftScene::new(scene, self.preparation_budget())?,
        )
    }
    pub fn replace_prepared_scene(
        &mut self,
        scene: PreparedSolidCraftScene,
    ) -> Result<(), RenderError> {
        self.require_idle()?;
        let next = self.check_scene(&scene, false)?;
        let model = frame::model(&scene, &self.options)?;
        self.put_model(model)?;
        self.retired_model_bytes = self
            .retired_model_bytes
            .max(self.scene.as_ref().map_or(0, |s| s.bytes));
        self.retired_preview_bytes = self
            .retired_preview_bytes
            .max(self.preview.as_ref().map_or(0, |s| s.bytes));
        self.retired_highlight_bytes = self.retired_highlight_bytes.max(self.highlight_bytes);
        self.scene = Some(scene);
        self.preview = None;
        self.preview_id = None;
        self.latest_preview = None;
        self.highlight_bytes = 0;
        self.set_slot(false, GpuScene::default())?;
        self.set_slot(true, GpuScene::default())?;
        self.estimate = next;
        self.refresh_accounting()
    }
    pub fn replace_prepared_preview(
        &mut self,
        id: PreviewIdentity,
        scene: PreparedSolidCraftScene,
    ) -> Result<(), RenderError> {
        self.require_idle()?;
        if Some(id.base) != self.scene_identity()
            || scene.identity != id.base
            || self.latest_preview.is_some_and(|g| id.generation <= g)
        {
            return Err(RenderError::CaptureMismatch);
        }
        let next = self.check_scene(&scene, true)?;
        let gpu = frame::preview(&scene)?;
        self.set_slot(false, gpu)?;
        self.retired_preview_bytes = self
            .retired_preview_bytes
            .max(self.preview.as_ref().map_or(0, |s| s.bytes));
        self.preview = Some(scene);
        self.preview_id = Some(id);
        self.latest_preview = Some(id.generation);
        self.estimate = next;
        self.refresh_accounting()
    }
    pub fn clear_preview(&mut self) -> Result<(), RenderError> {
        self.require_idle()?;
        self.set_slot(false, GpuScene::default())?;
        self.retired_preview_bytes = self
            .retired_preview_bytes
            .max(self.preview.as_ref().map_or(0, |s| s.bytes));
        self.preview = None;
        self.preview_id = None;
        self.refresh_accounting()
    }
    fn check_scene(
        &self,
        scene: &PreparedSolidCraftScene,
        preview: bool,
    ) -> Result<MemoryEstimate, RenderError> {
        let retained = if preview {
            self.scene.as_ref().map_or(0, |s| s.bytes)
        } else {
            0
        };
        let geometry = add(scene.bytes, retained)?;
        let images = self.options_payload_bytes(&self.options)?;
        self.limits.check(
            MemoryEstimate {
                source_bytes: add(
                    scene.source_bytes,
                    if preview {
                        self.scene.as_ref().map_or(0, |s| s.source_bytes)
                    } else {
                        0
                    },
                )?,
                cpu_prepared_bytes: add(
                    add(mul(geometry, 4)?, images)?,
                    add(self.highlight_bytes, self.retired_highlight_bytes)?,
                )?,
                resident_metadata_bytes: add(
                    add(mul(geometry, 2)?, images)?,
                    if preview { self.highlight_bytes } else { 0 },
                )?,
                geometry_bytes: add(
                    add(mul(geometry, 3)?, images)?,
                    if preview { self.highlight_bytes } else { 0 },
                )?,
                upload_bytes: add(geometry, images)?,
                target_bytes: self.estimate.target_bytes,
                ..Default::default()
            },
            MemoryEstimate {
                target_bytes: 0,
                ..self.estimate
            },
        )
    }
    pub fn resize(&mut self, size: [u32; 2]) -> Result<(), RenderError> {
        if self.size == size {
            return Ok(());
        }
        self.require_idle()?;
        self.limits.validate_size(size, &self.device)?;
        self.limits.validate_size(
            size.map(|v| (v as f64 * f64::from(self.options.ssaa)).ceil() as u32),
            &self.device,
        )?;
        let estimate = self.limits.check(
            MemoryEstimate {
                target_bytes: frame::supersample_bytes(size, self.options.ssaa)?
                    .max(self.estimate.target_bytes),
                ..self.estimate
            },
            MemoryEstimate {
                target_bytes: self.estimate.target_bytes,
                ..Default::default()
            },
        )?;
        self.target = (!size.contains(&0)).then(|| Target::new(&self.device, size));
        self.size = size;
        self.estimate = estimate;
        Ok(())
    }
    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &ViewSnapshot,
        style: RenderStyle,
    ) -> Result<bool, RenderError> {
        self.require_idle()?;
        if self.target.is_none() {
            return Ok(false);
        }
        if self.scene_identity() != Some(view.identity().scene) {
            return Err(RenderError::CaptureMismatch);
        }
        frame::update(&mut self.frame, view, self.size, &self.options)?;
        self.renderer
            .prepare(&self.device, &self.queue, encoder, &self.frame);
        self.paint(encoder, style);
        self.flight = Flight::Encoded;
        Ok(true)
    }
    fn paint(&self, encoder: &mut wgpu::CommandEncoder, style: RenderStyle) {
        let Some(target) = &self.target else { return };
        let background = match style.theme {
            crate::Theme::Dark => [0.025, 0.032, 0.045],
            crate::Theme::Light => [0.82, 0.84, 0.87],
        };
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("OneCAD SolidCraft viewport"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.msaa,
                    depth_slice: None,
                    resolve_target: Some(&target.color),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: background[0],
                            g: background[1],
                            b: background[2],
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(view::far_depth(
                            self.renderer.target().reversed_depth(),
                        )),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Discard,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        self.renderer.paint(
            &self.frame,
            &mut pass,
            [0.0, 0.0, self.size[0] as f32, self.size[1] as f32],
        );
    }
    pub fn poll_idle(&mut self) -> Result<bool, RenderError> {
        self.device
            .poll(wgpu::PollType::Poll)
            .map_err(|e| RenderError::Device(e.to_string()))?;
        if let Flight::Submitted(done) = &self.flight {
            if done.load(std::sync::atomic::Ordering::Acquire) {
                self.flight = Flight::Idle;
                self.retired_model_bytes = 0;
                self.retired_preview_bytes = 0;
                self.retired_highlight_bytes = 0;
                self.refresh_accounting()?;
            }
        }
        Ok(matches!(self.flight, Flight::Idle))
    }
    fn require_idle(&mut self) -> Result<(), RenderError> {
        if self.poll_idle()? {
            Ok(())
        } else {
            Err(RenderError::Busy)
        }
    }
    pub fn after_submit(
        &mut self,
        wake: impl FnOnce() + Send + 'static,
    ) -> Result<(), RenderError> {
        if !matches!(self.flight, Flight::Encoded) {
            return Err(RenderError::SubmissionState);
        }
        self.flight = Flight::submitted(&self.queue, wake);
        Ok(())
    }
    /// The host must discard its outstanding encoder before calling this method.
    pub fn abandon_frame(
        &mut self,
        wake: impl FnOnce() + Send + 'static,
    ) -> Result<(), RenderError> {
        if !matches!(self.flight, Flight::Encoded) {
            return Err(RenderError::SubmissionState);
        }
        self.queue.submit([]);
        self.flight = Flight::submitted(&self.queue, wake);
        Ok(())
    }
    fn put_model(&mut self, scene: GpuScene) -> Result<(), RenderError> {
        let key = self.next_key()?;
        self.frame.key = key;
        self.frame.slot = Arc::new(Mutex::new(Some(scene)));
        Ok(())
    }
    fn set_slot(&mut self, highlight: bool, scene: GpuScene) -> Result<(), RenderError> {
        let key = self.next_key()?;
        let slot = Arc::new(Mutex::new(Some(scene)));
        if highlight {
            self.frame.hl_key = key;
            self.frame.hl_slot = slot;
        } else {
            self.frame.pv_key = key;
            self.frame.pv_slot = slot;
        }
        Ok(())
    }
    fn next_key(&mut self) -> Result<u64, RenderError> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or(RenderError::Budget("donor frame sequence exhausted"))?;
        Ok(self.sequence)
    }
}
fn target_bytes(size: [u32; 2]) -> Result<u64, RenderError> {
    mul(mul(u64::from(size[0]), u64::from(size[1]))?, 52)
}
impl Target {
    fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {
        let texture = |format, usage, sample_count| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("OneCAD SolidCraft target"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = texture(
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            1,
        );
        Self {
            color: color.create_view(&Default::default()),
            _color: color,
            msaa: texture(
                wgpu::TextureFormat::Rgba8UnormSrgb,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
                4,
            )
            .create_view(&Default::default()),
            depth: texture(
                solidcraft_gpu::depth_format(device, true),
                wgpu::TextureUsages::RENDER_ATTACHMENT,
                4,
            )
            .create_view(&Default::default()),
        }
    }
}

#[cfg(test)]
#[path = "solidcraft_viewport_tests.rs"]
mod tests;
