use crate::{
    limits::{add, mul},
    mesh::{self, GpuBody, PreparedBody, PreparedGeometry},
    MemoryEstimate, RenderError, RenderLimits, Renderer,
};
use onecad_scene::{BodyId, Bounds, SceneBody, SceneIdentity, SceneSnapshot};
use std::sync::Arc;

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

/// A bounded cache of the last successful preparation. Keep its CPU budget lease alive until
/// this cache and all prepared results are dropped. Source bytes remain immutable scene data.
#[derive(Default)]
pub struct PreparationCache {
    geometries: Vec<Arc<PreparedGeometry>>,
}
impl PreparationCache {
    /// Retained capacities and Arc allocations, excluding separately charged source blobs.
    pub fn cpu_bytes(&self) -> u64 {
        self.geometries.capacity() as u64 * std::mem::size_of::<Arc<PreparedGeometry>>() as u64
            + self
                .geometries
                .iter()
                .map(|geometry| geometry.cpu_bytes())
                .sum::<u64>()
    }
    /// Source blobs retained by the cache; hosts must preserve their source-residency lease.
    pub fn source_bytes(&self) -> u64 {
        self.geometries
            .iter()
            .map(|geometry| geometry.source.len() as u64)
            .sum()
    }
    fn find(&self, body: &SceneBody) -> Option<&Arc<PreparedGeometry>> {
        self.geometries
            .iter()
            .find(|geometry| Arc::ptr_eq(&geometry.source, body.bytes()))
    }
    fn estimates(
        &self,
        scene: &SceneSnapshot,
        max_buffer: u64,
        exact: bool,
    ) -> Result<(u64, u64), RenderError> {
        let mut gpu = 0;
        let mut new_cpu = 0;
        for body in scene.bodies() {
            let bytes = if let Some(geometry) = self.find(body) {
                geometry.gpu_bytes(max_buffer)?
            } else {
                let bytes = mesh::body_estimate(body, max_buffer, exact)?;
                new_cpu = add(
                    new_cpu,
                    add(
                        bytes,
                        std::mem::size_of::<PreparedGeometry>() as u64
                            + 2 * std::mem::size_of::<usize>() as u64,
                    )?,
                )?;
                bytes
            };
            gpu = add(gpu, bytes)?;
        }
        Ok((gpu, new_cpu))
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
        Self::new_cached(scene, budget, &mut PreparationCache::default())
    }
    /// Reuse immutable geometry despite changed publication revisions. Failure leaves the
    /// previous cache intact; cache and incoming preparation share one bounded CPU budget.
    pub fn new_cached(
        scene: &SceneSnapshot,
        budget: PreparationBudget,
        cache: &mut PreparationCache,
    ) -> Result<Self, RenderError> {
        if scene.bodies().len() > budget.limits.max_bodies {
            return Err(RenderError::Budget("body count"));
        }
        let metadata = mul(
            scene.bodies().len() as u64,
            (std::mem::size_of::<(BodyId, PreparedBody)>()
                + std::mem::size_of::<Arc<PreparedGeometry>>()) as u64,
        )?;
        let mut scratch = 0;
        for body in scene.bodies() {
            if cache.find(body).is_none() {
                scratch = scratch.max(crate::silhouette::scratch_bytes(&body.mesh()?)?);
            }
        }
        let retained = add(cache.cpu_bytes(), add(metadata, scratch)?)?;
        let (_, minimum) = cache.estimates(scene, budget.max_buffer, false)?;
        if add(retained, minimum)? > budget.limits.max_cpu_prepared_bytes {
            return Err(RenderError::Budget("CPU preparation"));
        }
        let (gpu_bytes, new_cpu) = cache.estimates(scene, budget.max_buffer, true)?;
        let cpu_bytes = add(retained, new_cpu)?;
        check_preparation_budget(scene, budget, cache, gpu_bytes, cpu_bytes)?;
        let mut bodies = Vec::new();
        let mut geometries = Vec::new();
        bodies
            .try_reserve_exact(scene.bodies().len())
            .map_err(|_| RenderError::Budget("prepared body metadata"))?;
        geometries
            .try_reserve_exact(scene.bodies().len())
            .map_err(|_| RenderError::Budget("prepared cache metadata"))?;
        for body in scene.bodies() {
            let prepared = if let Some(geometry) = cache.find(body) {
                PreparedBody {
                    mesh_revision: body.mesh_revision(),
                    geometry: Arc::clone(geometry),
                }
            } else {
                PreparedBody::new(body)?
            };
            geometries.push(Arc::clone(&prepared.geometry));
            bodies.push((body.body_id(), prepared));
        }
        cache.geometries = geometries;
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
fn check_preparation_budget(
    scene: &SceneSnapshot,
    budget: PreparationBudget,
    cache: &PreparationCache,
    gpu_bytes: u64,
    cpu_bytes: u64,
) -> Result<(), RenderError> {
    let old_sources = cache
        .geometries
        .iter()
        .filter(|geometry| {
            !scene
                .bodies()
                .iter()
                .any(|body| Arc::ptr_eq(&geometry.source, body.bytes()))
        })
        .try_fold(0, |total, geometry| {
            add(total, geometry.source.len() as u64)
        })?;
    let incoming = MemoryEstimate {
        source_bytes: add(scene.resident_bytes() as u64, old_sources)?,
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
    Ok(())
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
        let previous = if preview {
            &self.preview
        } else {
            &self.committed
        };
        let reused_bytes = scene.bodies.iter().try_fold(0, |total, (id, body)| {
            let reused = previous.iter().find(|existing| {
                existing.id == *id
                    && std::sync::Arc::ptr_eq(&existing.source, &body.geometry.source)
            });
            add(total, reused.map(mesh::gpu_bytes).unwrap_or(0))
        })?;
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
            upload_bytes: scene.gpu_bytes.saturating_sub(reused_bytes),
            geometry_bytes: add(
                add(scene.gpu_bytes, retained)?,
                std::mem::size_of::<crate::FrameU>() as u64,
            )?,
            target_bytes: self.estimate.target_bytes,
            ibl_bytes: self.estimate.ibl_bytes,
            peak_total_bytes: 0,
        };
        let old = MemoryEstimate {
            geometry_bytes: self.estimate.geometry_bytes.saturating_sub(reused_bytes),
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
            // Preview and committed draws need distinct frame uniforms even for shared geometry.
            let previous = if preview {
                &self.preview
            } else {
                &self.committed
            };
            let reusable = previous.iter().find(|existing| {
                existing.id == id && std::sync::Arc::ptr_eq(&existing.source, &body.geometry.source)
            });
            let next = if let Some(existing) = reusable {
                let mut next = existing.clone();
                next.mesh_revision = body.mesh_revision;
                next.style = crate::BodyStyle::default();
                next
            } else {
                body.upload(&gpu, &self.pipelines.frame_layout, id)
            };
            bodies.push(next);
        }
        Ok((bodies, estimate))
    }
}

#[cfg(test)]
#[path = "preparation_tests.rs"]
mod tests;
