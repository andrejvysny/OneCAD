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
}

#[derive(Debug)]
struct BodyIndices {
    faces: PickIndex,
    edges: PickIndex,
}

impl SceneBody {
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
            geometry: Arc::new(BodyGeometry {
                checked,
                indices: OnceLock::new(),
                triangles,
                decode_scratch_bytes,
                segments,
                bounds,
            }),
        })
    }

    /// Compare immutable content on the preparation thread, never during picks.
    pub fn same_content(&self, bytes: &Arc<[u8]>) -> bool {
        Arc::ptr_eq(self.bytes(), bytes) || self.bytes().as_ref() == bytes.as_ref()
    }

    pub fn shares_geometry(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.geometry, &other.geometry)
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
    pub fn bytes(&self) -> &Arc<[u8]> {
        self.geometry.checked.bytes()
    }
    pub fn mesh(&self) -> Result<MeshView<'_>, SceneError> {
        Ok(self.geometry.checked.view())
    }
    pub fn bounds(&self) -> Bounds {
        self.geometry.bounds
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
    let mut bounds: Option<Bounds> = None;
    for body in bodies {
        let geometry = &body.geometry;
        let metadata = size_of::<SceneBody>() + size_of::<BodyGeometry>() + 4 * size_of::<usize>();
        let retained = budget_add(
            body.bytes().len(),
            metadata,
            limits.max_resident_bytes,
            "body bytes",
        )?;
        bytes = budget_add(bytes, retained, limits.max_resident_bytes, "mesh bytes")?;
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
        bounds = Some(match bounds {
            Some(b) => Bounds {
                min: b.min.min(geometry.bounds.min),
                max: b.max.max(geometry.bounds.max),
            },
            None => geometry.bounds,
        });
    }
    let bytes = budget_add(bytes, scratch, limits.max_resident_bytes, "decoder scratch")?;
    Ok((bytes, bounds))
}

fn unique_bodies(bodies: &[SceneBody]) -> Result<(), SceneError> {
    for (i, body) in bodies.iter().enumerate() {
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
                "../../../spikes/fixtures/fillet-block/00000000-0000-0000-0000-00000000f101.mesh1"
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
