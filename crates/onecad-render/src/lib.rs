//! CAD rendering on the host's shared wgpu device; no UI-toolkit dependency.
#![cfg_attr(
    not(test),
    deny(
        clippy::panic,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::todo,
        clippy::unimplemented,
        clippy::unreachable
    )
)]
mod flight;
mod frame;
mod gpu;
mod highlight_prepare;
mod highlights;
mod ibl;
mod limits;
mod mesh;
mod options;
mod passes;
mod pipelines;
mod prepared;
mod profile;
mod resize;
mod section_contours;
mod section_contours_gpu;
mod section_contours_worker;
mod silhouette;
mod targets;
mod thumbnail;
mod uniforms;

pub use frame::{grid_step, GridFrame, RenderStyle, SilhouetteMode, Theme};
pub use highlights::{HighlightKind, HighlightTarget, PreparedHighlights};
pub use limits::{MemoryEstimate, RenderLimits};
pub use mesh::{BodyStyle, PreviewIdentity};
pub use options::{required_device_features, DisplayMode, RenderOptions};
pub use prepared::{PreparationBudget, PreparationCache, PreparationRetention, PreparedScene};
pub use profile::{RenderProfile, MIN_SCENE_SCALE, MOBILE_MAX_SCENE_PIXELS, SCENE_SCALE_STEP};
pub use section_contours::{PreparedSectionContours, SectionContourKey};
pub use section_contours_worker::SectionContourWorker;
pub use thumbnail::Thumbnail;
pub use uniforms::FrameStats;

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
    #[error("GPU target allocation refused: {0}")]
    Allocation(String),
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
    retained_output: Option<resize::RetainedOutput>,
    size: [u32; 2],
    committed: Vec<GpuBody>,
    preview: Vec<GpuBody>,
    scene_identity: Option<SceneIdentity>,
    preview_identity: Option<PreviewIdentity>,
    committed_bounds: Option<onecad_scene::Bounds>,
    committed_source_bytes: u64,
    preview_source_bytes: u64,
    estimate: MemoryEstimate,
    frame_bindings: uniforms::FrameBindings,
    frame_stats: FrameStats,
    flight: flight::Flight,
    latest_preview_generation: Option<u64>,
    highlights: Option<highlights::GpuHighlights>,
    section_contours: [Option<section_contours::GpuSectionContours>; 2],
    options: RenderOptions,
    profile: RenderProfile,
    contact_dirty: bool,
    blit: Option<thumbnail::BlitResources>,
}

impl Renderer {
    /// A [`RenderProfile::Desktop`] renderer.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        limits: RenderLimits,
    ) -> Result<Self, RenderError> {
        Self::with_profile(device, queue, size, limits, RenderProfile::Desktop)
    }
    pub fn with_profile(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        limits: RenderLimits,
        profile: RenderProfile,
    ) -> Result<Self, RenderError> {
        limits.validate_size(size, device)?;
        let estimate = limits.check(
            MemoryEstimate {
                target_bytes: limits::quality_target_bytes(
                    size,
                    RenderOptions::default(),
                    profile,
                )?,
                ibl_bytes: limits::ibl_bytes(profile),
                geometry_bytes: limits::shared_uniform_bytes(),
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
        let ibl = ibl::Ibl::build(&gpu, &sampler, profile.ibl());
        let targets = if size.contains(&0) {
            None
        } else {
            Some(Targets::try_new(
                &gpu,
                &pipelines,
                &ibl,
                size,
                RenderOptions::default(),
                profile,
            )?)
        };
        let frame_bindings = uniforms::FrameBindings::new(&gpu, &pipelines.frame_layout);
        Ok(Self {
            device: device.clone(),
            queue: queue.clone(),
            limits,
            pipelines,
            ibl,
            targets,
            retained_output: None,
            size,
            committed: Vec::new(),
            preview: Vec::new(),
            scene_identity: None,
            preview_identity: None,
            committed_bounds: None,
            committed_source_bytes: 0,
            preview_source_bytes: 0,
            estimate,
            frame_bindings,
            frame_stats: FrameStats::default(),
            flight: flight::Flight::submitted(queue, || {}),
            latest_preview_generation: None,
            highlights: None,
            section_contours: [None, None],
            options: RenderOptions::default(),
            profile,
            contact_dirty: true,
            blit: None,
        })
    }
    pub fn output_view(&self) -> Option<&wgpu::TextureView> {
        self.targets
            .as_ref()
            .map(|target| &target.output_view)
            .or_else(|| self.retained_output.as_ref().map(|output| &output.view))
    }
    pub fn size(&self) -> [u32; 2] {
        self.size
    }
    pub fn has_targets(&self) -> bool {
        self.targets.is_some()
    }
    pub fn memory_estimate(&self) -> MemoryEstimate {
        self.estimate
    }
    pub fn profile(&self) -> RenderProfile {
        self.profile
    }
    /// Bytes of the targets this renderer would allocate for `size` with its current options.
    pub fn target_bytes(&self, size: [u32; 2]) -> Result<u64, RenderError> {
        limits::quality_target_bytes(size, self.options, self.profile)
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
            let frame = self.frame_bindings.body(style, false);
            body.uniform = frame.buffer.clone();
            body.frame_bind = frame.bind.clone();
            true
        } else {
            false
        }
    }
    pub fn resize(&mut self, size: [u32; 2]) -> Result<(), RenderError> {
        if self.size == size && (self.targets.is_some() || size.contains(&0)) {
            return Ok(());
        }
        self.require_idle()?;
        self.replace_targets(size, self.options)
    }
    /// Convenience CPU preparation for non-UI callers. Native hosts should prepare on a
    /// worker and use replace_prepared_scene to keep geometry packing off the UI thread.
    pub fn replace_scene(&mut self, scene: &SceneSnapshot) -> Result<(), RenderError> {
        self.require_idle()?;
        let prepared = PreparedScene::new(scene, self.preparation_budget())?;
        if self.options.section[..3].iter().any(|v| *v != 0.0) {
            let contours = PreparedSectionContours::new(
                scene,
                self.preparation_budget(),
                self.options.section,
            )?;
            return self.replace_prepared_scene_with_section_contours(prepared, contours);
        }
        self.replace_prepared_scene(prepared)
    }
    pub fn replace_prepared_scene(&mut self, scene: PreparedScene) -> Result<(), RenderError> {
        options::validate_section_bodies(self.options, scene.body_count())?;
        self.require_idle()?;
        if self.options.section[..3].iter().any(|v| *v != 0.0) && scene.body_count() != 0 {
            return Err(RenderError::CaptureMismatch);
        }
        let identity = scene.identity;
        let bounds = scene.bounds;
        let source = scene.source_bytes;
        let (bodies, estimate) = self.upload_prepared(scene, false)?;
        self.publish_scene(bodies, estimate, identity, bounds, source);
        Ok(())
    }
    fn publish_scene(
        &mut self,
        bodies: Vec<GpuBody>,
        estimate: MemoryEstimate,
        identity: SceneIdentity,
        bounds: Option<onecad_scene::Bounds>,
        source: u64,
    ) {
        self.reset_preview_suppression();
        self.contact_dirty |= self.committed.len() != bodies.len()
            || self.committed.iter().zip(&bodies).any(|(old, new)| {
                old.id != new.id || !std::sync::Arc::ptr_eq(&old.source, &new.source)
            });
        self.committed = bodies;
        self.clear_section_contours();
        self.highlights = None;
        self.preview.clear();
        self.preview_identity = None;
        self.latest_preview_generation = None;
        self.scene_identity = Some(identity);
        self.committed_bounds = bounds;
        self.committed_source_bytes = source;
        self.preview_source_bytes = 0;
        self.estimate = estimate;
    }
    pub fn replace_preview(
        &mut self,
        identity: PreviewIdentity,
        scene: &SceneSnapshot,
    ) -> Result<(), RenderError> {
        self.require_idle()?;
        let prepared = PreparedScene::new(scene, self.preparation_budget())?;
        if self.options.section[..3].iter().any(|v| *v != 0.0) {
            let contours = PreparedSectionContours::new(
                scene,
                self.preparation_budget(),
                self.options.section,
            )?
            .with_preview(identity)?;
            return self
                .replace_prepared_preview_with_section_contours(identity, prepared, contours);
        }
        self.replace_prepared_preview(identity, prepared)
    }
    pub fn replace_prepared_preview(
        &mut self,
        identity: PreviewIdentity,
        scene: PreparedScene,
    ) -> Result<(), RenderError> {
        self.require_idle()?;
        if self.options.section[..3].iter().any(|v| *v != 0.0) && scene.body_count() != 0 {
            return Err(RenderError::CaptureMismatch);
        }
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
        self.publish_preview(bodies, estimate, identity, source);
        Ok(())
    }
    fn publish_preview(
        &mut self,
        bodies: Vec<GpuBody>,
        estimate: MemoryEstimate,
        identity: PreviewIdentity,
        source: u64,
    ) {
        self.reset_preview_suppression();
        self.preview = bodies;
        self.clear_preview_section_contours();
        self.preview_source_bytes = source;
        self.preview_identity = Some(identity);
        self.latest_preview_generation = Some(identity.generation);
        self.estimate = estimate;
    }
    pub fn clear_preview(&mut self) -> Result<(), RenderError> {
        self.require_idle()?;
        self.reset_preview_suppression();
        self.preview.clear();
        self.clear_preview_section_contours();
        self.preview_identity = None;
        self.preview_source_bytes = 0;
        self.estimate.source_bytes = self.committed_source_bytes;
        self.estimate.resident_metadata_bytes = self.committed.len() as u64
            * std::mem::size_of::<GpuBody>() as u64
            + self.highlight_metadata_bytes();
        self.estimate.resident_metadata_bytes += self.section_contour_metadata_bytes(false);
        self.estimate.geometry_bytes = self.committed.iter().map(mesh::gpu_bytes).sum::<u64>()
            + limits::shared_uniform_bytes()
            + self.highlight_gpu_bytes();
        self.estimate.geometry_bytes += self.section_contour_gpu_bytes(false);
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
        let stats = self.frame_upload_stats();
        if stats.uniform_upload_bytes > self.limits.max_upload_bytes {
            return Err(RenderError::Budget("frame upload"));
        }
        self.limits.check(
            MemoryEstimate {
                source_bytes: limits::add(self.committed_source_bytes, self.preview_source_bytes)?,
                cpu_prepared_bytes: (3 + if self.highlights.is_some() { 2 } else { 0 })
                    * std::mem::size_of::<FrameU>() as u64,
                upload_bytes: stats.uniform_upload_bytes,
                ..self.estimate
            },
            MemoryEstimate::default(),
        )?;
        let frames = self.frame_uniforms(view, style)?;
        let uploaded = self.upload_frame_uniforms(&frames);
        debug_assert_eq!(uploaded, stats);
        self.record(encoder, target, frames.normal.canvas);
        self.frame_stats = uploaded;
        self.contact_dirty = false;
        self.flight = flight::Flight::Encoded;
        Ok(true)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod highlight_tests;
mod preview_visibility;
