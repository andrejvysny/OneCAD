//! Snapshot-scoped overlays adapted from SolidCraft's separate highlight batches.
use crate::{
    frame::FrameU,
    gpu::Gpu,
    limits::{add, mul},
    mesh::GpuBody,
    MemoryEstimate, PreparationBudget, RenderError, Renderer,
};
use onecad_scene::{BodyId, PickHit, SceneIdentity};

pub use onecad_scene::PickKind as HighlightKind;

#[derive(Clone, Copy, Debug)]
pub struct HighlightTarget<'a> {
    pub scene: SceneIdentity,
    pub body_id: BodyId,
    pub mesh_revision: u64,
    pub kind: HighlightKind,
    pub topology_id: &'a str,
}
impl<'a> HighlightTarget<'a> {
    pub fn from_pick(hit: &PickHit<'a>) -> Self {
        Self {
            scene: hit.capture.scene,
            body_id: hit.body_id,
            mesh_revision: hit.mesh_revision,
            kind: hit.kind,
            topology_id: hit.topology_id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HighlightTone {
    Selected,
    Hover,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HighlightDraw {
    pub body_id: BodyId,
    pub body_index: usize,
    pub mesh_revision: u64,
    pub kind: HighlightKind,
    pub first: u32,
    pub count: u32,
    pub tone: HighlightTone,
}
impl HighlightDraw {
    pub(crate) fn validate(
        &self,
        revision: u64,
        indices: u32,
        segments: u32,
    ) -> Result<(), RenderError> {
        if self.mesh_revision != revision {
            return Err(RenderError::CaptureMismatch);
        }
        let end = self
            .first
            .checked_add(self.count)
            .ok_or(RenderError::Highlight("range overflow"))?;
        let limit = match self.kind {
            HighlightKind::Face => indices,
            HighlightKind::Edge => segments,
        };
        if self.count == 0 || end > limit {
            return Err(RenderError::Highlight("draw range"));
        }
        if self.kind == HighlightKind::Face
            && (!self.first.is_multiple_of(3) || !self.count.is_multiple_of(3))
        {
            return Err(RenderError::Highlight("face index units"));
        }
        Ok(())
    }
    pub(crate) fn same_target(&self, other: &Self) -> bool {
        self.body_id == other.body_id
            && self.mesh_revision == other.mesh_revision
            && self.kind == other.kind
            && self.first == other.first
            && self.count == other.count
    }
}

/// CPU-only resolved ranges. IDs never become persistent triangle identities.
pub struct PreparedHighlights {
    pub(crate) scene: SceneIdentity,
    pub(crate) draws: Vec<HighlightDraw>,
}
impl PreparedHighlights {
    pub(crate) fn validate_scene(
        &self,
        expected: Option<SceneIdentity>,
    ) -> Result<(), RenderError> {
        if expected == Some(self.scene) {
            Ok(())
        } else {
            Err(RenderError::CaptureMismatch)
        }
    }
    pub fn scene_identity(&self) -> SceneIdentity {
        self.scene
    }
    pub fn len(&self) -> usize {
        self.draws.len()
    }
    pub fn is_empty(&self) -> bool {
        self.draws.is_empty()
    }
    pub(crate) fn metadata_bytes(&self) -> Result<u64, RenderError> {
        mul(
            self.draws.capacity() as u64,
            std::mem::size_of::<HighlightDraw>() as u64,
        )
    }
    pub(crate) fn validate_bodies(&self, bodies: &[GpuBody]) -> Result<(), RenderError> {
        for draw in &self.draws {
            let body = bodies
                .get(draw.body_index)
                .filter(|body| body.id == draw.body_id)
                .ok_or(RenderError::Highlight("unknown body"))?;
            draw.validate(body.mesh_revision, body.index_count, body.segment_count)?;
        }
        Ok(())
    }
}

pub(crate) struct GpuHighlights {
    pub draws: Vec<HighlightDraw>,
    pub uniforms: [wgpu::Buffer; 2],
    pub binds: [wgpu::BindGroup; 2],
}
impl GpuHighlights {
    pub(crate) fn new(
        gpu: &Gpu,
        layout: &wgpu::BindGroupLayout,
        draws: Vec<HighlightDraw>,
    ) -> Self {
        let uniforms = std::array::from_fn(|_| {
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("topology highlight frame"),
                size: std::mem::size_of::<FrameU>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let binds = std::array::from_fn(|index| {
            gpu.bind(
                "highlight frame",
                layout,
                &[uniforms[index].as_entire_binding()],
            )
        });
        Self {
            draws,
            uniforms,
            binds,
        }
    }
    pub(crate) fn gpu_bytes(&self) -> u64 {
        self.uniforms.iter().map(wgpu::Buffer::size).sum()
    }
    pub(crate) fn metadata_bytes(&self) -> u64 {
        self.draws.capacity() as u64 * std::mem::size_of::<HighlightDraw>() as u64
    }
}

pub(crate) fn highlight_budget(
    budget: PreparationBudget,
    metadata: u64,
    gpu_bytes: u64,
) -> Result<MemoryEstimate, RenderError> {
    budget.limits.check(
        MemoryEstimate {
            cpu_prepared_bytes: metadata,
            upload_bytes: gpu_bytes,
            geometry_bytes: add(budget.resident.geometry_bytes, gpu_bytes)?,
            resident_metadata_bytes: add(budget.resident.resident_metadata_bytes, metadata)?,
            ..budget.resident
        },
        MemoryEstimate::default(),
    )
}

impl Renderer {
    pub fn replace_highlights(&mut self, prepared: PreparedHighlights) -> Result<(), RenderError> {
        self.require_idle()?;
        prepared.validate_scene(self.scene_identity)?;
        prepared.validate_bodies(&self.committed)?;
        if prepared.len() > self.limits.max_highlights {
            return Err(RenderError::Budget("highlight count"));
        }
        let gpu_bytes = if prepared.is_empty() {
            0
        } else {
            2 * std::mem::size_of::<FrameU>() as u64
        };
        let mut estimate = highlight_budget(
            self.preparation_budget(),
            prepared.metadata_bytes()?,
            gpu_bytes,
        )?;
        estimate.geometry_bytes -= self.highlight_gpu_bytes();
        estimate.resident_metadata_bytes -= self.highlight_metadata_bytes();
        let gpu = Gpu {
            device: &self.device,
            queue: &self.queue,
        };
        self.highlights = (!prepared.is_empty())
            .then(|| GpuHighlights::new(&gpu, &self.pipelines.frame_layout, prepared.draws));
        self.estimate = estimate;
        Ok(())
    }
    pub fn clear_highlights(&mut self) -> Result<(), RenderError> {
        self.require_idle()?;
        self.estimate.geometry_bytes -= self.highlight_gpu_bytes();
        self.estimate.resident_metadata_bytes -= self.highlight_metadata_bytes();
        self.highlights = None;
        Ok(())
    }
    pub(crate) fn highlight_gpu_bytes(&self) -> u64 {
        self.highlights.as_ref().map_or(0, GpuHighlights::gpu_bytes)
    }
    pub(crate) fn highlight_metadata_bytes(&self) -> u64 {
        self.highlights
            .as_ref()
            .map_or(0, GpuHighlights::metadata_bytes)
    }
}
