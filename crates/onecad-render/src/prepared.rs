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
    /// Conservative CPU-only preparation budget; GPU publication rechecks actual device limits.
    pub fn cpu_validation() -> Self {
        let limits = RenderLimits::default();
        Self {
            max_buffer: limits.max_upload_bytes,
            limits,
            resident: MemoryEstimate::default(),
        }
    }

    /// Tightens preparation limits before packing, for a host's shared CPU budget.
    pub fn limit_cpu_prepared_bytes(mut self, maximum: u64) -> Self {
        self.limits.max_cpu_prepared_bytes = self.limits.max_cpu_prepared_bytes.min(maximum);
        self
    }
}

/// Opaque host admission guards, retained by the allocations they cover.
#[derive(Clone, Default)]
pub struct PreparationRetention {
    pub source: Option<Arc<dyn Send + Sync>>,
    pub preparation: Option<Arc<dyn Send + Sync>>,
}

/// A bounded cache of the last successful preparation. Retained constructors preserve host
/// admission guards with geometry and vector metadata until their final owner drops.
#[derive(Default)]
pub struct PreparationCache {
    geometries: Vec<Arc<PreparedGeometry>>,
    _preparation_retention: Option<Arc<dyn Send + Sync>>,
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
    /// Source blobs retained by the cache, excluding checked scene metadata and BVHs.
    pub fn source_bytes(&self) -> u64 {
        self.geometries
            .iter()
            .map(|geometry| geometry.source.len() as u64)
            .sum()
    }
    fn find(
        &self,
        body: &SceneBody,
        retention: &PreparationRetention,
    ) -> Option<&Arc<PreparedGeometry>> {
        self.geometries.iter().find(|geometry| {
            Arc::ptr_eq(&geometry.source, body.bytes()) && geometry.retains(retention)
        })
    }
    fn estimates(
        &self,
        scene: &SceneSnapshot,
        max_buffer: u64,
        exact: bool,
        retention: &PreparationRetention,
    ) -> Result<(u64, u64), RenderError> {
        let mut gpu = 0;
        let mut new_cpu = 0;
        for (index, body) in scene.bodies().iter().enumerate() {
            if scene.bodies()[..index]
                .iter()
                .any(|other| Arc::ptr_eq(other.bytes(), body.bytes()))
            {
                continue;
            }
            let bytes = if let Some(geometry) = self.find(body, retention) {
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

fn visible_work(scene: &SceneSnapshot) -> Result<(u64, u64), RenderError> {
    let (mut triangles, mut segments) = (0, 0);
    for body in scene.bodies().iter().filter(|body| body.visible()) {
        triangles = add(triangles, body.triangle_count() as u64)?;
        segments = add(segments, body.edge_segment_count() as u64)?;
        if triangles > 500_000 || segments > 500_000 {
            return Err(RenderError::Budget("visible draw work"));
        }
    }
    Ok((triangles, segments))
}

/// Owned CPU arrays with no wgpu handles. Build on the same background worker that captures
/// the immutable CPU scene; retain only the latest pending preparation in the host.
pub struct PreparedScene {
    pub(crate) identity: SceneIdentity,
    pub(crate) bounds: Option<Bounds>,
    pub(crate) has_instances: bool,
    visible_triangles: u64,
    visible_segments: u64,
    pub(crate) source_bytes: u64,
    pub(crate) gpu_bytes: u64,
    pub(crate) cpu_bytes: u64,
    pub(crate) bodies: Vec<(BodyId, PreparedBody)>,
    _preparation_retention: Option<Arc<dyn Send + Sync>>,
}
impl PreparedScene {
    pub fn new(scene: &SceneSnapshot, budget: PreparationBudget) -> Result<Self, RenderError> {
        Self::new_retained(scene, budget, PreparationRetention::default())
    }
    pub fn new_retained(
        scene: &SceneSnapshot,
        budget: PreparationBudget,
        retention: PreparationRetention,
    ) -> Result<Self, RenderError> {
        Self::new_cached_retained(scene, budget, &mut PreparationCache::default(), retention)
    }
    /// Reuse immutable geometry despite changed publication revisions. Failure leaves the
    /// previous cache intact; cache and incoming preparation share one bounded CPU budget.
    pub fn new_cached(
        scene: &SceneSnapshot,
        budget: PreparationBudget,
        cache: &mut PreparationCache,
    ) -> Result<Self, RenderError> {
        Self::new_cached_retained(scene, budget, cache, PreparationRetention::default())
    }
    /// Reused geometry preserves its original guards; new metadata retains this build's guard.
    pub fn new_cached_retained(
        scene: &SceneSnapshot,
        budget: PreparationBudget,
        cache: &mut PreparationCache,
        retention: PreparationRetention,
    ) -> Result<Self, RenderError> {
        for body in scene.bodies() {
            mesh::check_occurrence_precision(body)?;
        }
        let (visible_triangles, visible_segments) = visible_work(scene)?;
        let triangles =
            scene
                .bodies()
                .iter()
                .enumerate()
                .try_fold(0u64, |total, (index, body)| {
                    if scene.bodies()[..index]
                        .iter()
                        .any(|other| other.shares_geometry(body))
                    {
                        Ok(total)
                    } else {
                        add(total, u64::from(body.mesh()?.header().triangle_count))
                    }
                })?;
        if triangles > 500_000 {
            return Err(RenderError::Budget("triangle admission"));
        }
        if scene.bodies().len() > budget.limits.max_bodies {
            return Err(RenderError::Budget("body count"));
        }
        let path_bytes = scene.bodies().iter().try_fold(0u64, |total, body| {
            add(
                total,
                mul(
                    body.occurrence_path().map_or(0, |path| path.0.len()) as u64,
                    16,
                )?,
            )
        })?;
        let metadata = add(
            path_bytes,
            mul(
                scene.bodies().len() as u64,
                (std::mem::size_of::<(BodyId, PreparedBody)>()
                    + std::mem::size_of::<Arc<PreparedGeometry>>()) as u64,
            )?,
        )?;
        let mut scratch = 0;
        for body in scene.bodies() {
            if cache.find(body, &retention).is_none() {
                scratch = scratch.max(crate::silhouette::scratch_bytes(&body.mesh()?)?);
            }
        }
        let retained = add(cache.cpu_bytes(), add(metadata, scratch)?)?;
        let (_, minimum) = cache.estimates(scene, budget.max_buffer, false, &retention)?;
        if add(retained, minimum)? > budget.limits.max_cpu_prepared_bytes {
            return Err(RenderError::Budget("CPU preparation"));
        }
        let (geometry_bytes, new_cpu) =
            cache.estimates(scene, budget.max_buffer, true, &retention)?;
        let models = scene
            .bodies()
            .iter()
            .filter(|body| body.occurrence_path().is_some())
            .count() as u64;
        let gpu_bytes = add(geometry_bytes, mul(models, 64)?)?;
        let cpu_bytes = add(retained, new_cpu)?;
        check_preparation_budget(scene, budget, cache, gpu_bytes, cpu_bytes)?;
        let mut bodies = Vec::new();
        let mut geometries: Vec<Arc<PreparedGeometry>> = Vec::new();
        bodies
            .try_reserve_exact(scene.bodies().len())
            .map_err(|_| RenderError::Budget("prepared body metadata"))?;
        geometries
            .try_reserve_exact(scene.bodies().len())
            .map_err(|_| RenderError::Budget("prepared cache metadata"))?;
        for body in scene.bodies() {
            let geometry = geometries
                .iter()
                .find(|geometry| Arc::ptr_eq(&geometry.source, body.bytes()))
                .or_else(|| cache.find(body, &retention));
            let prepared = if let Some(geometry) = geometry {
                PreparedBody::from_geometry(body, Arc::clone(geometry))?
            } else {
                PreparedBody::new_retained(body, retention.clone())?
            };
            if !geometries
                .iter()
                .any(|geometry| Arc::ptr_eq(geometry, &prepared.geometry))
            {
                geometries.push(Arc::clone(&prepared.geometry));
            }
            bodies.push((body.body_id(), prepared));
        }
        cache.geometries = geometries;
        cache._preparation_retention = retention.preparation.clone();
        Ok(Self {
            identity: scene.identity(),
            bounds: scene.bounds(),
            visible_triangles,
            visible_segments,
            has_instances: scene
                .bodies()
                .iter()
                .any(|body| body.occurrence_path().is_some()),
            source_bytes: scene.resident_bytes() as u64,
            gpu_bytes,
            cpu_bytes,
            bodies,
            _preparation_retention: retention.preparation,
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
    pub fn has_instances(&self) -> bool {
        self.has_instances
    }
    pub fn body_count(&self) -> usize {
        self.bodies.len()
    }
    /// Resident CPU bytes this scene alone keeps alive: its body metadata and the geometry
    /// `cache` does not also hold. Unlike [`Self::cpu_bytes`] (a preparation peak), this is
    /// what dropping the scene frees while `cache` lives.
    pub fn cpu_bytes_beyond(&self, cache: &PreparationCache) -> u64 {
        let metadata = self.bodies.capacity() as u64
            * std::mem::size_of::<(BodyId, PreparedBody)>() as u64
            + self
                .bodies
                .iter()
                .map(|(_, body)| {
                    body.path
                        .as_ref()
                        .map_or(0, |path| path.0.capacity() as u64 * 16)
                })
                .sum::<u64>();
        let unshared = self
            .bodies
            .iter()
            .map(|(_, body)| &body.geometry)
            .filter(|geometry| !cache.geometries.iter().any(|g| Arc::ptr_eq(g, geometry)))
            .map(|geometry| geometry.cpu_bytes())
            .sum::<u64>();
        metadata + unshared
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
        geometry_bytes: add(gpu_bytes, crate::limits::shared_uniform_bytes())?,
        upload_bytes: gpu_bytes,
        target_bytes: budget.resident.target_bytes,
        ibl_bytes: budget.resident.ibl_bytes,
        resident_metadata_bytes: add(
            mul(
                scene.bodies().len() as u64,
                std::mem::size_of::<GpuBody>() as u64,
            )?,
            scene.bodies().iter().try_fold(0u64, |total, body| {
                add(
                    total,
                    mul(
                        body.occurrence_path().map_or(0, |path| path.0.len()) as u64,
                        16,
                    )?,
                )
            })?,
        )?,
        peak_gpu_bytes: 0,
        peak_total_bytes: 0,
    };
    budget.limits.check(
        incoming,
        MemoryEstimate {
            target_bytes: 0,
            // The renderer retains its style bindings across scene replacement.
            geometry_bytes: budget
                .resident
                .geometry_bytes
                .saturating_sub(crate::limits::shared_uniform_bytes()),
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
        crate::options::validate_section_bodies(self.options, scene.bodies.len())?;
        if !preview
            && self.scene_identity == Some(scene.identity)
            && (scene.has_instances || self.committed.iter().any(|body| body.is_instance))
        {
            let same = self.committed.len() == scene.bodies.len()
                && self
                    .committed
                    .iter()
                    .zip(&scene.bodies)
                    .all(|(old, (id, new))| {
                        old.id == *id
                            && old.source_body_id == new.source_body_id
                            && old.path == new.path
                            && old.model64 == new.model64
                            && old.visible == new.visible
                            && old.mesh_revision == new.mesh_revision
                            && Arc::ptr_eq(&old.source, &new.geometry.source)
                    });
            if !same {
                return Err(RenderError::CaptureMismatch);
            }
        }
        if preview {
            let (triangles, segments) = self.committed.iter().filter(|body| body.visible).fold(
                (scene.visible_triangles, scene.visible_segments),
                |(triangles, segments), body| {
                    (
                        triangles + u64::from(body.index_count) / 3,
                        segments + u64::from(body.segment_count),
                    )
                },
            );
            if triangles > 500_000 || segments > 500_000 {
                return Err(RenderError::Budget("visible draw work"));
            }
        }
        let count = scene.bodies.len() + if preview { self.committed.len() } else { 0 };
        if count > self.limits.max_bodies {
            return Err(RenderError::Budget("body count"));
        }
        let retained = if preview {
            add(
                mesh::unique_gpu_bytes(&self.committed),
                add(
                    self.highlight_gpu_bytes(),
                    self.section_contour_gpu_bytes(false),
                )?,
            )?
        } else {
            0
        };
        let previous = if preview {
            &self.preview
        } else {
            &self.committed
        };
        let reused_bytes =
            scene
                .bodies
                .iter()
                .enumerate()
                .try_fold(0, |total, (index, (_, body))| {
                    if scene.bodies[..index].iter().any(|(_, other)| {
                        Arc::ptr_eq(&other.geometry.source, &body.geometry.source)
                    }) {
                        return Ok(total);
                    }
                    let reused = previous
                        .iter()
                        .find(|existing| Arc::ptr_eq(&existing.source, &body.geometry.source));
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
                add(
                    mul(count as u64, std::mem::size_of::<GpuBody>() as u64)?,
                    scene.bodies.iter().try_fold(0u64, |total, (_, body)| {
                        add(
                            total,
                            mul(body.path.as_ref().map_or(0, |path| path.0.len()) as u64, 16)?,
                        )
                    })?,
                )?,
                if preview {
                    add(
                        mesh::path_metadata_bytes(&self.committed),
                        add(
                            self.highlight_metadata_bytes(),
                            self.section_contour_metadata_bytes(false),
                        )?,
                    )?
                } else {
                    0
                },
            )?,
            upload_bytes: scene.gpu_bytes.saturating_sub(reused_bytes),
            geometry_bytes: add(
                add(scene.gpu_bytes, retained)?,
                crate::limits::shared_uniform_bytes(),
            )?,
            target_bytes: self.estimate.target_bytes,
            ibl_bytes: self.estimate.ibl_bytes,
            peak_gpu_bytes: 0,
            peak_total_bytes: 0,
        };
        let old = MemoryEstimate {
            geometry_bytes: self
                .estimate
                .geometry_bytes
                .saturating_sub(reused_bytes)
                .saturating_sub(crate::limits::shared_uniform_bytes()),
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
            let reusable = previous
                .iter()
                .chain(bodies.iter())
                .find(|existing| Arc::ptr_eq(&existing.source, &body.geometry.source));
            let frame = self
                .frame_bindings
                .body(crate::BodyStyle::default(), preview);
            let model = if body.is_instance {
                mesh::ModelBinding::new(&gpu, &self.pipelines.model_layout, &body.model)
            } else {
                mesh::ModelBinding {
                    buffer: self.identity_model.buffer.clone(),
                    bind: self.identity_model.bind.clone(),
                }
            };
            let next = if let Some(existing) = reusable {
                let mut next: GpuBody = existing.clone();
                next.retain_source(&body);
                next.id = id;
                next.mesh_revision = body.mesh_revision;
                next.style = crate::BodyStyle::default();
                next.preview_suppressed = false;
                next.uniform = frame.buffer.clone();
                next.frame_bind = frame.bind.clone();
                next.model_buffer = model.buffer;
                next.model_bind = model.bind;
                next.model = body.model;
                next.is_instance = body.is_instance;
                next.visible = body.visible;
                next.source_body_id = body.source_body_id;
                next.path = body.path;
                next.model64 = body.model64;
                next
            } else {
                body.upload(&gpu, frame, id, &model)
            };
            bodies.push(next);
        }
        Ok((bodies, estimate))
    }
}

#[cfg(test)]
#[path = "preparation_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "retention_tests.rs"]
mod retention_tests;
