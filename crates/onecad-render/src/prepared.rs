use crate::{
    limits::{add, mul},
    mesh::{self, GpuBody, PreparedBody},
    MemoryEstimate, RenderError, RenderLimits, Renderer,
};
use onecad_scene::{BodyId, Bounds, SceneIdentity, SceneSnapshot};

/// Copy to a worker thread before CPU scene preparation. GPU upload rechecks current budgets.
#[derive(Clone, Copy)]
pub struct PreparationBudget {
    pub(crate) limits: RenderLimits,
    pub(crate) resident: MemoryEstimate,
    pub(crate) max_buffer: u64,
}
impl PreparationBudget {
    /// Tightens preparation limits before packing, for a host's shared CPU budget.
    pub fn limit_cpu_prepared_bytes(mut self, maximum: u64) -> Self {
        self.limits.max_cpu_prepared_bytes = self.limits.max_cpu_prepared_bytes.min(maximum);
        self
    }
}

/// Owned CPU arrays with no wgpu handles. Build on the same background worker that captures
/// the immutable CPU scene; retain only the latest pending preparation in the host.
pub struct PreparedScene {
    pub(crate) identity: SceneIdentity,
    pub(crate) bounds: Option<Bounds>,
    pub(crate) source_bytes: u64,
    pub(crate) gpu_bytes: u64,
    pub(crate) cpu_bytes: u64,
    pub(crate) bodies: Vec<(BodyId, PreparedBody)>,
}
impl PreparedScene {
    pub fn new(scene: &SceneSnapshot, budget: PreparationBudget) -> Result<Self, RenderError> {
        if scene.bodies().len() > budget.limits.max_bodies {
            return Err(RenderError::Budget("body count"));
        }
        let gpu_bytes = mesh::estimate(scene, budget.max_buffer)?;
        let cpu_bytes = add(gpu_bytes, mesh::cpu_scratch_bytes(scene)?)?;
        let incoming = MemoryEstimate {
            source_bytes: scene.resident_bytes() as u64,
            cpu_prepared_bytes: cpu_bytes,
            geometry_bytes: gpu_bytes,
            upload_bytes: gpu_bytes,
            target_bytes: budget.resident.target_bytes,
            ibl_bytes: budget.resident.ibl_bytes,
            resident_metadata_bytes: mul(
                scene.bodies().len() as u64,
                std::mem::size_of::<GpuBody>() as u64,
            )?,
            peak_total_bytes: 0,
        };
        budget.limits.check(
            incoming,
            MemoryEstimate {
                target_bytes: 0,
                ..budget.resident
            },
        )?;
        let mut bodies = Vec::new();
        bodies
            .try_reserve_exact(scene.bodies().len())
            .map_err(|_| RenderError::Budget("prepared body metadata"))?;
        for body in scene.bodies() {
            bodies.push((body.body_id(), PreparedBody::new(body)?));
        }
        Ok(Self {
            identity: scene.identity(),
            bounds: scene.bounds(),
            source_bytes: scene.resident_bytes() as u64,
            gpu_bytes,
            cpu_bytes,
            bodies,
        })
    }
    pub fn identity(&self) -> SceneIdentity {
        self.identity
    }
    /// Conservative CPU preparation payload, including the bounded silhouette scratch peak.
    /// Hosts can retain this charge across worker/display handoff instead of counting bodies.
    pub fn cpu_bytes(&self) -> u64 {
        self.cpu_bytes
    }
    pub fn body_count(&self) -> usize {
        self.bodies.len()
    }
}
impl Renderer {
    pub fn preparation_budget(&self) -> PreparationBudget {
        PreparationBudget {
            limits: self.limits,
            resident: self.estimate,
            max_buffer: self.device.limits().max_buffer_size,
        }
    }
    pub(crate) fn upload_prepared(
        &self,
        scene: PreparedScene,
        preview: bool,
    ) -> Result<(Vec<GpuBody>, MemoryEstimate), RenderError> {
        let count = scene.bodies.len() + if preview { self.committed.len() } else { 0 };
        if count > self.limits.max_bodies {
            return Err(RenderError::Budget("body count"));
        }
        let retained = if preview {
            add(
                self.committed.iter().map(mesh::gpu_bytes).sum::<u64>(),
                self.highlight_gpu_bytes(),
            )?
        } else {
            0
        };
        let next = MemoryEstimate {
            source_bytes: add(
                scene.source_bytes,
                if preview {
                    self.committed_source_bytes
                } else {
                    0
                },
            )?,
            cpu_prepared_bytes: scene.cpu_bytes,
            resident_metadata_bytes: add(
                mul(count as u64, std::mem::size_of::<GpuBody>() as u64)?,
                if preview {
                    self.highlight_metadata_bytes()
                } else {
                    0
                },
            )?,
            upload_bytes: scene.gpu_bytes,
            geometry_bytes: add(
                add(scene.gpu_bytes, retained)?,
                std::mem::size_of::<crate::FrameU>() as u64,
            )?,
            target_bytes: self.estimate.target_bytes,
            ibl_bytes: self.estimate.ibl_bytes,
            peak_total_bytes: 0,
        };
        let old = MemoryEstimate {
            geometry_bytes: self.estimate.geometry_bytes,
            source_bytes: self.estimate.source_bytes,
            resident_metadata_bytes: self.estimate.resident_metadata_bytes,
            ..Default::default()
        };
        let estimate = self.limits.check(next, old)?;
        let mut bodies = Vec::new();
        bodies
            .try_reserve_exact(scene.bodies.len())
            .map_err(|_| RenderError::Budget("GPU body metadata"))?;
        let gpu = crate::gpu::Gpu {
            device: &self.device,
            queue: &self.queue,
        };
        for (id, body) in scene.bodies {
            bodies.push(body.upload(&gpu, &self.pipelines.frame_layout, id));
        }
        Ok((bodies, estimate))
    }
}
