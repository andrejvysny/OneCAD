//! CAD rendering on the host's shared wgpu device; no UI-toolkit dependency.
mod flight;
mod frame;
mod gpu;
mod highlight_prepare;
mod highlights;
mod ibl;
mod limits;
mod mesh;
mod passes;
mod pipelines;
mod prepared;
mod silhouette;
pub mod solidcraft_viewport;
mod targets;

pub use frame::{RenderStyle, SilhouetteMode, Theme};
pub use highlights::{HighlightKind, HighlightTarget, PreparedHighlights};
pub use limits::{MemoryEstimate, RenderLimits};
pub use mesh::{BodyStyle, PreviewIdentity};
pub use prepared::{PreparationBudget, PreparedScene};
pub use solidcraft_viewport::{
    PreparedSolidCraftScene, SolidCraftOptions, SolidCraftPreparationBudget, SolidCraftViewport,
};

use frame::FrameU;
use mesh::GpuBody;
use onecad_scene::{BodyId, SceneIdentity, SceneSnapshot, ViewSnapshot};
use pipelines::Pipelines;
use targets::Targets;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("renderer has an encoded or submitted frame in flight")]
    Busy,
    #[error("renderer submission registration does not match encoded state")]
    SubmissionState,
    #[error("GPU device polling failed: {0}")]
    Device(String),
    #[error("renderer resource budget exceeded: {0}")]
    Budget(&'static str),
    #[error("renderer capture does not match uploaded scene or viewport")]
    CaptureMismatch,
    #[error("checked mesh is not renderable: {0}")]
    Mesh(&'static str),
    #[error("invalid topology highlight: {0}")]
    Highlight(&'static str),
    #[error("scene: {0}")]
    Scene(#[from] onecad_scene::SceneError),
}

/// The host must discard outstanding encoders before replacing scene or targets.
/// Device and queue clones retain the SAME underlying host device, never a second device.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    limits: RenderLimits,
    pipelines: Pipelines,
    ibl: ibl::Ibl,
    targets: Option<Targets>,
    size: [u32; 2],
    committed: Vec<GpuBody>,
    preview: Vec<GpuBody>,
    scene_identity: Option<SceneIdentity>,
    preview_identity: Option<PreviewIdentity>,
    committed_bounds: Option<onecad_scene::Bounds>,
    committed_source_bytes: u64,
    preview_source_bytes: u64,
    estimate: MemoryEstimate,
    frame_buffer: wgpu::Buffer,
    frame_bind: wgpu::BindGroup,
    flight: flight::Flight,
    latest_preview_generation: Option<u64>,
    highlights: Option<highlights::GpuHighlights>,
}

impl Renderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        limits: RenderLimits,
    ) -> Result<Self, RenderError> {
        limits.validate_size(size, device)?;
        let estimate = limits.check(
            MemoryEstimate {
                target_bytes: limits::target_bytes(size)?,
                ibl_bytes: limits::ibl_bytes(),
                geometry_bytes: std::mem::size_of::<FrameU>() as u64,
                ..Default::default()
            },
            MemoryEstimate::default(),
        )?;
        let gpu = gpu::Gpu { device, queue };
        let pipelines = Pipelines::new(&gpu);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let ibl = ibl::Ibl::build(&gpu, &sampler);
        let targets = (!size.contains(&0)).then(|| Targets::new(&gpu, &pipelines, &ibl, size));
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("CAD frame"),
            size: std::mem::size_of::<FrameU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_bind = gpu.bind(
            "CAD frame",
            &pipelines.frame_layout,
            &[frame_buffer.as_entire_binding()],
        );
        Ok(Self {
            device: device.clone(),
            queue: queue.clone(),
            limits,
            pipelines,
            ibl,
            targets,
            size,
            committed: Vec::new(),
            preview: Vec::new(),
            scene_identity: None,
            preview_identity: None,
            committed_bounds: None,
            committed_source_bytes: 0,
            preview_source_bytes: 0,
            estimate,
            frame_buffer,
            frame_bind,
            flight: flight::Flight::submitted(queue, || {}),
            latest_preview_generation: None,
            highlights: None,
        })
    }
    pub fn output_view(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(|target| &target.output_view)
    }
    pub fn size(&self) -> [u32; 2] {
        self.size
    }
    pub fn memory_estimate(&self) -> MemoryEstimate {
        self.estimate
    }
    pub fn scene_identity(&self) -> Option<SceneIdentity> {
        self.scene_identity
    }
    pub fn preview_identity(&self) -> Option<PreviewIdentity> {
        self.preview_identity
    }
    pub fn set_body_style(&mut self, id: BodyId, style: BodyStyle) -> bool {
        if let Some(body) = self.committed.iter_mut().find(|body| body.id == id) {
            body.style = style;
            true
        } else {
            false
        }
    }
    pub fn resize(&mut self, size: [u32; 2]) -> Result<(), RenderError> {
        if self.size == size {
            return Ok(());
        }
        self.require_idle()?;
        self.limits.validate_size(size, &self.device)?;
        let next = MemoryEstimate {
            target_bytes: limits::target_bytes(size)?,
            cpu_prepared_bytes: 0,
            upload_bytes: 0,
            ..self.estimate
        };
        let estimate = self.limits.check(
            next,
            MemoryEstimate {
                target_bytes: self.estimate.target_bytes,
                ..Default::default()
            },
        )?;
        let gpu = gpu::Gpu {
            device: &self.device,
            queue: &self.queue,
        };
        self.targets =
            (!size.contains(&0)).then(|| Targets::new(&gpu, &self.pipelines, &self.ibl, size));
        self.size = size;
        self.estimate = estimate;
        Ok(())
    }
    /// Convenience CPU preparation for non-UI callers. Native hosts should prepare on a
    /// worker and use replace_prepared_scene to keep geometry packing off the UI thread.
    pub fn replace_scene(&mut self, scene: &SceneSnapshot) -> Result<(), RenderError> {
        self.require_idle()?;
        let prepared = PreparedScene::new(scene, self.preparation_budget())?;
        self.replace_prepared_scene(prepared)
    }
    pub fn replace_prepared_scene(&mut self, scene: PreparedScene) -> Result<(), RenderError> {
        self.require_idle()?;
        let identity = scene.identity;
        let bounds = scene.bounds;
        let source = scene.source_bytes;
        let (bodies, estimate) = self.upload_prepared(scene, false)?;
        self.committed = bodies;
        self.highlights = None;
        self.preview.clear();
        self.preview_identity = None;
        self.latest_preview_generation = None;
        self.scene_identity = Some(identity);
        self.committed_bounds = bounds;
        self.committed_source_bytes = source;
        self.preview_source_bytes = 0;
        self.estimate = estimate;
        Ok(())
    }
    pub fn replace_preview(
        &mut self,
        identity: PreviewIdentity,
        scene: &SceneSnapshot,
    ) -> Result<(), RenderError> {
        self.require_idle()?;
        let prepared = PreparedScene::new(scene, self.preparation_budget())?;
        self.replace_prepared_preview(identity, prepared)
    }
    pub fn replace_prepared_preview(
        &mut self,
        identity: PreviewIdentity,
        scene: PreparedScene,
    ) -> Result<(), RenderError> {
        self.require_idle()?;
        if Some(identity.base) != self.scene_identity
            || scene.identity != identity.base
            || self
                .latest_preview_generation
                .is_some_and(|last| identity.generation <= last)
        {
            return Err(RenderError::CaptureMismatch);
        }
        let source = scene.source_bytes;
        let (bodies, estimate) = self.upload_prepared(scene, true)?;
        self.preview = bodies;
        self.preview_source_bytes = source;
        self.preview_identity = Some(identity);
        self.latest_preview_generation = Some(identity.generation);
        self.estimate = estimate;
        Ok(())
    }
    pub fn clear_preview(&mut self) -> Result<(), RenderError> {
        self.require_idle()?;
        self.preview.clear();
        self.preview_identity = None;
        self.preview_source_bytes = 0;
        self.estimate.source_bytes = self.committed_source_bytes;
        self.estimate.resident_metadata_bytes = self.committed.len() as u64
            * std::mem::size_of::<GpuBody>() as u64
            + self.highlight_metadata_bytes();
        self.estimate.geometry_bytes = self.committed.iter().map(mesh::gpu_bytes).sum::<u64>()
            + std::mem::size_of::<FrameU>() as u64
            + self.highlight_gpu_bytes();
        self.estimate.cpu_prepared_bytes = 0;
        self.estimate.upload_bytes = 0;
        Ok(())
    }
    /// Record private CAD passes. The host submits this encoder and composites output_view.
    /// Zero-sized suspended targets produce no commands. No readback occurs.
    pub fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &ViewSnapshot,
        style: RenderStyle,
    ) -> Result<bool, RenderError> {
        self.require_idle()?;
        let Some(target) = &self.targets else {
            return Ok(false);
        };
        if Some(view.identity().scene) != self.scene_identity {
            return Err(RenderError::CaptureMismatch);
        }
        let viewport = view.viewport();
        if [
            (viewport.width * viewport.scale_factor).round() as u32,
            (viewport.height * viewport.scale_factor).round() as u32,
        ] != self.size
        {
            return Err(RenderError::CaptureMismatch);
        }
        let highlight_uniforms = if self.highlights.is_some() { 2 } else { 0 };
        if limits::uniform_bytes(self.committed.len() + self.preview.len() + highlight_uniforms)?
            > self.limits.max_upload_bytes
        {
            return Err(RenderError::Budget("frame upload"));
        }
        let staging =
            limits::uniform_bytes(self.committed.len() + self.preview.len() + highlight_uniforms)?;
        self.limits.check(
            MemoryEstimate {
                source_bytes: limits::add(self.committed_source_bytes, self.preview_source_bytes)?,
                cpu_prepared_bytes: (3 + highlight_uniforms) as u64
                    * std::mem::size_of::<FrameU>() as u64,
                upload_bytes: staging,
                ..self.estimate
            },
            MemoryEstimate::default(),
        )?;
        let uniform = frame::uniform(
            view,
            self.committed_bounds,
            style,
            BodyStyle::default(),
            false,
        )?;
        let selected = frame::uniform(
            view,
            self.committed_bounds,
            style,
            BodyStyle { selected: true },
            false,
        )?;
        let preview_frame = frame::uniform(
            view,
            self.committed_bounds,
            style,
            BodyStyle::default(),
            true,
        )?;
        let highlight_frames = if self.highlights.is_some() {
            Some([
                frame::highlight_uniform(
                    uniform,
                    view,
                    style,
                    highlights::HighlightTone::Selected,
                )?,
                frame::highlight_uniform(uniform, view, style, highlights::HighlightTone::Hover)?,
            ])
        } else {
            None
        };
        self.queue
            .write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&uniform));
        for (bodies, preview) in [(&self.committed, false), (&self.preview, true)] {
            for body in bodies {
                let uniform = if preview {
                    preview_frame
                } else if body.style.selected {
                    selected
                } else {
                    uniform
                };
                self.queue
                    .write_buffer(&body.uniform, 0, bytemuck::bytes_of(&uniform));
            }
        }
        if let (Some(highlights), Some(frames)) = (&self.highlights, highlight_frames) {
            for (buffer, frame) in highlights.uniforms.iter().zip(frames) {
                self.queue
                    .write_buffer(buffer, 0, bytemuck::bytes_of(&frame));
            }
        }
        self.record(encoder, target, uniform.canvas);
        self.flight = flight::Flight::Encoded;
        Ok(true)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod highlight_tests;
