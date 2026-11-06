use crate::index::PickIndex;
use crate::{BodyId, DVec3, MeshLimits, MeshView, SceneError};
use onecad_protocol::mesh::CheckedMesh;
use std::{
    mem::size_of,
    sync::{Arc, OnceLock},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneIdentity {
    pub session: u64,
    pub scene: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: DVec3,
    pub max: DVec3,
}

impl Bounds {
    pub fn new(min: DVec3, max: DVec3) -> Result<Self, SceneError> {
        if !min.is_finite() || !max.is_finite() || !min.cmple(max).all() {
            return Err(SceneError::InvalidView("invalid bounds"));
        }
        Ok(Self { min, max })
    }
    pub fn corners(self) -> impl Iterator<Item = DVec3> {
        (0..8).map(move |i| {
            DVec3::new(
                if i & 1 == 0 { self.min.x } else { self.max.x },
                if i & 2 == 0 { self.min.y } else { self.max.y },
                if i & 4 == 0 { self.min.z } else { self.max.z },
            )
        })
    }
}

#[derive(Debug, Clone)]
pub struct SceneBody {
    body_id: BodyId,
    mesh_revision: u64,
    source_body_id: BodyId,
    model: crate::DMat4,
    occurrence_path: Option<crate::instances::OccurrencePath>,
    visible: bool,
    geometry: Arc<BodyGeometry>,
}

#[derive(Debug)]
struct BodyGeometry {
    checked: CheckedMesh,
    indices: OnceLock<BodyIndices>,
    triangles: usize,
    decode_scratch_bytes: usize,
    segments: usize,
    bounds: Bounds,
    retention: Option<RetainedAllocation>,
}

/// Shared checked geometry and its host admission, retained as one unit.
#[derive(Debug, Clone)]
pub struct SceneGeometry(Arc<BodyGeometry>);
impl SceneGeometry {
    pub fn len(&self) -> usize {
        self.0.checked.bytes().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn shares_source(&self, other: &Self) -> bool {
        Arc::ptr_eq(self.0.checked.bytes(), other.0.checked.bytes())
    }
}

#[derive(Clone)]
struct RetainedAllocation(Arc<dyn Send + Sync>);
impl std::fmt::Debug for RetainedAllocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RetainedAllocation")
    }
}

#[derive(Debug)]
struct BodyIndices {
    faces: PickIndex,
    edges: PickIndex,
}

impl SceneBody {
    /// Bounds checked geometry and pick indices before copying a mesh for handoff.
    pub fn preflight_geometry_bytes(bytes: &[u8], limits: MeshLimits) -> Result<usize, SceneError> {
        let mesh = MeshView::decode(bytes, limits)?;
        let header = mesh.header();
        let mut segments = 0usize;
        for edge in 0..header.edge_count as usize {
            let range = mesh
                .edge_range(edge)
                .ok_or(SceneError::BudgetExceeded("invalid edge range"))?;
            segments = segments
                .checked_add(range.count.saturating_sub(1) as usize)
                .ok_or(SceneError::BudgetExceeded("segment count overflow"))?;
        }
        let metadata = size_of::<BodyGeometry>()
            + 6 * size_of::<usize>()
            + size_of::<onecad_protocol::mesh::MeshHeaderView>()
            + header.sections.capacity() * size_of::<onecad_protocol::mesh::MeshSectionEntry>();
        [
            bytes.len(),
            metadata,
            PickIndex::allocation_bytes(header.triangle_count as usize)
                .ok_or(SceneError::BudgetExceeded("index overflow"))?,
            PickIndex::allocation_bytes(segments)
                .ok_or(SceneError::BudgetExceeded("index overflow"))?,
        ]
        .into_iter()
        .try_fold(0usize, |sum, n| {
            sum.checked_add(n)
                .ok_or(SceneError::BudgetExceeded("body geometry overflow"))
        })
    }
    pub fn new(
        body_id: BodyId,
        mesh_revision: u64,
        bytes: Arc<[u8]>,
        mesh_limits: MeshLimits,
    ) -> Result<Self, SceneError> {
        let checked = CheckedMesh::decode(bytes, mesh_limits)?;
        let mesh = checked.view();
        let h = mesh.header();
        let bounds = Bounds::new(
            DVec3::from_array(h.bbox_min.map(f64::from)),
            DVec3::from_array(h.bbox_max.map(f64::from)),
        )?;
        let triangles = h.triangle_count as usize;
        let decode_scratch_bytes = usize::from(h.section_count) * 16
            + 2 * size_of::<MeshView<'_>>()
            + size_of::<Vec<u32>>();
        let mut segments = 0usize;
        for index in 0..h.edge_count {
            let range = mesh
                .edge_range(index as usize)
                .ok_or(SceneError::BudgetExceeded("invalid checked edge range"))?;
            segments = segments
                .checked_add(range.count.saturating_sub(1) as usize)
                .ok_or(SceneError::BudgetExceeded("segment count overflow"))?;
        }
        Ok(Self {
            body_id,
            mesh_revision,
            source_body_id: body_id,
            model: crate::DMat4::IDENTITY,
            occurrence_path: None,
            visible: true,
            geometry: Arc::new(BodyGeometry {
                checked,
                indices: OnceLock::new(),
                triangles,
                decode_scratch_bytes,
                segments,
                bounds,
                retention: None,
            }),
        })
    }

    /// Create one placed view of this checked definition geometry.
    /// `display_id` must uniquely identify this body occurrence in the scene.
    pub fn instance(
        &self,
        display_id: BodyId,
        path: crate::instances::OccurrencePath,
        model: crate::DMat4,
        visible: bool,
    ) -> Result<Self, SceneError> {
        if path.0.is_empty() || path.0.len() > crate::instances::MAX_DEPTH {
            return Err(SceneError::InvalidInstance("occurrence path depth"));
        }
        crate::instances::check_rigid(&model)
            .map_err(|_| SceneError::InvalidInstance("model is not rigid"))?;
        let (min, max) = crate::instances::transformed_aabb(
            self.geometry.bounds.min,
            self.geometry.bounds.max,
            &model,
        );
        Bounds::new(min, max)?;
        Ok(Self {
            body_id: display_id,
            mesh_revision: self.mesh_revision,
            source_body_id: self.source_body_id,
            model,
            occurrence_path: Some(path),
            visible,
            geometry: Arc::clone(&self.geometry),
        })
    }

    pub fn source_body_id(&self) -> BodyId {
        self.source_body_id
    }
    pub fn model(&self) -> crate::DMat4 {
        self.model
    }
    pub fn occurrence_path(&self) -> Option<&crate::instances::OccurrencePath> {
        self.occurrence_path.as_ref()
    }
    pub fn visible(&self) -> bool {
        self.visible
    }
    pub fn triangle_count(&self) -> usize {
        self.geometry.triangles
    }
    pub fn edge_segment_count(&self) -> usize {
        self.geometry.segments
    }
    pub fn local_bounds(&self) -> Bounds {
        self.geometry.bounds
    }

    /// Compare immutable content on the preparation thread, never during picks.
    pub fn same_content(&self, bytes: &Arc<[u8]>) -> bool {
        Arc::ptr_eq(self.geometry.checked.bytes(), bytes) || self.bytes() == bytes.as_ref()
    }

    pub fn shares_geometry(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.geometry, &other.geometry)
    }

    /// Conservative allocation retained by this unique checked geometry.
    pub fn geometry_bytes(&self) -> Result<usize, SceneError> {
        let geometry = &self.geometry;
        let mut bytes = self
            .bytes()
            .len()
            .checked_add(size_of::<BodyGeometry>() + 4 * size_of::<usize>())
            .and_then(|n| n.checked_add(geometry.checked.metadata_bytes()))
            .ok_or(SceneError::BudgetExceeded("body geometry overflow"))?;
        for count in [geometry.triangles, geometry.segments] {
            bytes = bytes
                .checked_add(
                    PickIndex::allocation_bytes(count)
                        .ok_or(SceneError::BudgetExceeded("index overflow"))?,
                )
                .ok_or(SceneError::BudgetExceeded("body geometry overflow"))?;
        }
        Ok(bytes)
    }

    pub fn geometry_retention(&self) -> Option<Arc<dyn Send + Sync>> {
        self.geometry
            .retention
            .as_ref()
            .map(|guard| guard.0.clone())
    }

    /// Install an admission before sharing newly decoded geometry.
    pub fn retain_geometry(&mut self, guard: Arc<dyn Send + Sync>) -> Result<(), SceneError> {
        let geometry = Arc::get_mut(&mut self.geometry)
            .ok_or(SceneError::BudgetExceeded("geometry already shared"))?;
        if geometry.retention.is_some() {
            return Err(SceneError::BudgetExceeded("geometry already retained"));
        }
        geometry.retention = Some(RetainedAllocation(guard));
        Ok(())
    }

    /// Publication stamps do not change checked geometry or its spatial indices.
    pub fn with_revision(&self, mesh_revision: u64) -> Self {
        Self {
            mesh_revision,
            ..self.clone()
        }
    }

    pub fn new_reusing(
        body_id: BodyId,
        mesh_revision: u64,
        bytes: Arc<[u8]>,
        mesh_limits: MeshLimits,
        previous: Option<&Self>,
    ) -> Result<Self, SceneError> {
        if let Some(previous) =
            previous.filter(|body| body.body_id == body_id && body.same_content(&bytes))
        {
            previous.geometry.checked.ensure_limits(mesh_limits)?;
            return Ok(previous.with_revision(mesh_revision));
        }
        Self::new(body_id, mesh_revision, bytes, mesh_limits)
    }

    pub(crate) fn face_index(&self) -> Option<&PickIndex> {
        self.geometry.indices.get().map(|indices| &indices.faces)
    }

    pub(crate) fn edge_index(&self) -> Option<&PickIndex> {
        self.geometry.indices.get().map(|indices| &indices.edges)
    }
    pub fn body_id(&self) -> BodyId {
        self.body_id
    }
    pub fn mesh_revision(&self) -> u64 {
        self.mesh_revision
    }
    pub fn bytes(&self) -> &[u8] {
        self.geometry.checked.bytes().as_ref()
    }
    pub fn retained_geometry(&self) -> SceneGeometry {
        SceneGeometry(self.geometry.clone())
    }
    pub fn mesh(&self) -> Result<MeshView<'_>, SceneError> {
        Ok(self.geometry.checked.view())
    }
    pub fn bounds(&self) -> Bounds {
        let (min, max) = crate::instances::transformed_aabb(
            self.geometry.bounds.min,
            self.geometry.bounds.max,
            &self.model,
        );
        Bounds { min, max }
    }
}

/// Defaults bound the first brute-force implementation; these are not latency claims.
#[derive(Clone, Copy, Debug)]
pub struct SceneLimits {
    pub max_resident_bytes: usize,
    pub max_bodies: usize,
    pub max_triangles: usize,
    pub max_edge_segments: usize,
}

impl Default for SceneLimits {
    fn default() -> Self {
        Self {
            max_resident_bytes: 256 * 1024 * 1024,
            max_bodies: 4096,
            max_triangles: 500_000,
            max_edge_segments: 500_000,
        }
    }
}

#[derive(Debug)]
pub struct SceneSnapshot {
    identity: SceneIdentity,
    bodies: Box<[SceneBody]>,
    resident_bytes: usize,
    bounds: Option<Bounds>,
}

impl SceneSnapshot {
    pub fn new(
        identity: SceneIdentity,
        bodies: Vec<SceneBody>,
        limits: SceneLimits,
    ) -> Result<Self, SceneError> {
        if bodies.len() > limits.max_bodies {
            return Err(SceneError::BudgetExceeded("body count"));
        }
        if bodies
            .iter()
            .filter(|body| body.occurrence_path.is_some())
            .count()
            > crate::instances::MAX_INSTANCES
        {
            return Err(SceneError::BudgetExceeded("instance count"));
        }
        unique_bodies(&bodies)?;
        let (bytes, bounds) = bounded_usage(&bodies, limits)?;
        for body in &bodies {
            let geometry = &body.geometry;
            geometry.indices.get_or_init(|| {
                let mesh = geometry.checked.view();
                BodyIndices {
                    faces: PickIndex::build(&mesh, false, geometry.triangles),
                    edges: PickIndex::build(&mesh, true, geometry.segments),
                }
            });
        }
        Ok(Self {
            identity,
            bodies: bodies.into_boxed_slice(),
            resident_bytes: bytes,
            bounds,
        })
    }
    pub fn identity(&self) -> SceneIdentity {
        self.identity
    }
    pub fn bodies(&self) -> &[SceneBody] {
        &self.bodies
    }
    /// Conservative peak bytes: blobs, checked metadata, BVHs and decode scratch.
    pub fn resident_bytes(&self) -> usize {
        self.resident_bytes
    }
    pub fn bounds(&self) -> Option<Bounds> {
        self.bounds
    }
}

// Admission precedes index allocation; count all retained index capacities.
fn bounded_usage(
    bodies: &[SceneBody],
    limits: SceneLimits,
) -> Result<(usize, Option<Bounds>), SceneError> {
    let mut bytes = size_of::<SceneSnapshot>();
    let (mut triangles, mut segments, mut scratch) = (0usize, 0usize, 0usize);
    let (mut visible_triangles, mut visible_segments) = (0usize, 0usize);
    let mut bounds: Option<Bounds> = None;
    for (index, body) in bodies.iter().enumerate() {
        let path_bytes = body.occurrence_path.as_ref().map_or(0, |path| {
            path.0.capacity() * size_of::<crate::instances::OccurrenceKey>()
        });
        bytes = budget_add(
            bytes,
            size_of::<SceneBody>() + path_bytes,
            limits.max_resident_bytes,
            "instance metadata",
        )?;
        if body.visible {
            visible_triangles = budget_add(
                visible_triangles,
                body.geometry.triangles,
                limits.max_triangles,
                "visible triangles",
            )?;
            visible_segments = budget_add(
                visible_segments,
                body.geometry.segments,
                limits.max_edge_segments,
                "visible edge segments",
            )?;
            let b = body.bounds();
            bounds = Some(match bounds {
                Some(old) => Bounds {
                    min: old.min.min(b.min),
                    max: old.max.max(b.max),
                },
                None => b,
            });
        }
        if bodies[..index]
            .iter()
            .any(|previous| previous.shares_geometry(body))
        {
            continue;
        }
        let geometry = &body.geometry;
        let metadata = size_of::<BodyGeometry>() + 4 * size_of::<usize>();
        bytes = budget_add(
            bytes,
            body.bytes().len(),
            limits.max_resident_bytes,
            "mesh bytes",
        )?;
        bytes = budget_add(bytes, metadata, limits.max_resident_bytes, "body metadata")?;
        bytes = budget_add(
            bytes,
            geometry.checked.metadata_bytes(),
            limits.max_resident_bytes,
            "checked metadata",
        )?;
        triangles = budget_add(
            triangles,
            geometry.triangles,
            limits.max_triangles,
            "triangles",
        )?;
        segments = budget_add(
            segments,
            geometry.segments,
            limits.max_edge_segments,
            "edge segments",
        )?;
        for count in [geometry.triangles, geometry.segments] {
            let index = PickIndex::allocation_bytes(count)
                .ok_or(SceneError::BudgetExceeded("index overflow"))?;
            bytes = budget_add(bytes, index, limits.max_resident_bytes, "pick index")?;
        }
        scratch = scratch.max(geometry.decode_scratch_bytes);
    }
    let bytes = budget_add(bytes, scratch, limits.max_resident_bytes, "decoder scratch")?;
    Ok((bytes, bounds))
}

fn unique_bodies(bodies: &[SceneBody]) -> Result<(), SceneError> {
    for (i, body) in bodies.iter().enumerate() {
        if body.occurrence_path.is_some()
            && bodies[..i].iter().any(|other| {
                other.source_body_id == body.source_body_id
                    && other.occurrence_path == body.occurrence_path
            })
        {
            return Err(SceneError::InvalidInstance("duplicate occurrence body"));
        }
        if bodies[..i]
            .iter()
            .any(|other| other.body_id == body.body_id)
        {
            return Err(SceneError::DuplicateBody);
        }
    }
    Ok(())
}

fn budget_add(
    current: usize,
    amount: usize,
    limit: usize,
    name: &'static str,
) -> Result<usize, SceneError> {
    current
        .checked_add(amount)
        .filter(|total| *total <= limit)
        .ok_or(SceneError::BudgetExceeded(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recapture_preserves_both_bvh_allocations() {
        let bytes = Arc::from(
            include_bytes!(
                "../../../fixtures/render/fillet-block/00000000-0000-0000-0000-00000000f101.mesh1"
            )
            .as_slice(),
        );
        let body = SceneBody::new(BodyId::new(), 1, bytes, MeshLimits::default()).unwrap();
        let identity = SceneIdentity {
            session: 1,
            scene: 1,
        };
        let first = SceneSnapshot::new(identity, vec![body], SceneLimits::default()).unwrap();
        let previous = &first.bodies()[0];
        let next = SceneSnapshot::new(
            SceneIdentity {
                scene: 2,
                ..identity
            },
            vec![previous.with_revision(2)],
            SceneLimits::default(),
        )
        .unwrap();
        let current = &next.bodies()[0];
        assert!(std::ptr::eq(
            previous.face_index().unwrap(),
            current.face_index().unwrap()
        ));
        assert!(std::ptr::eq(
            previous.edge_index().unwrap(),
            current.edge_index().unwrap()
        ));
        assert_eq!(current.mesh_revision(), 2);
    }
}
