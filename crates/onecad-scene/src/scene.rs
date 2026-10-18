use crate::{BodyId, DVec3, MeshLimits, MeshView, SceneError};
use std::{mem::size_of, sync::Arc};

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

#[derive(Debug)]
pub struct SceneBody {
    body_id: BodyId,
    mesh_revision: u64,
    bytes: Arc<[u8]>,
    mesh_limits: MeshLimits,
    triangles: usize,
    decode_scratch_bytes: usize,
    segments: usize,
    bounds: Bounds,
}

impl SceneBody {
    pub fn new(
        body_id: BodyId,
        mesh_revision: u64,
        bytes: Arc<[u8]>,
        mesh_limits: MeshLimits,
    ) -> Result<Self, SceneError> {
        let mesh = MeshView::decode(&bytes, mesh_limits)?;
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
            bytes,
            mesh_limits,
            triangles,
            decode_scratch_bytes,
            segments,
            bounds,
        })
    }
    pub fn body_id(&self) -> BodyId {
        self.body_id
    }
    pub fn mesh_revision(&self) -> u64 {
        self.mesh_revision
    }
    pub fn bytes(&self) -> &Arc<[u8]> {
        &self.bytes
    }
    pub fn mesh(&self) -> Result<MeshView<'_>, SceneError> {
        Ok(MeshView::decode(&self.bytes, self.mesh_limits)?)
    }
    pub fn bounds(&self) -> Bounds {
        self.bounds
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
    /// Conservative peak bytes: retained scene plus one checked decoder scratch.
    pub fn resident_bytes(&self) -> usize {
        self.resident_bytes
    }
    pub fn bounds(&self) -> Option<Bounds> {
        self.bounds
    }
}

// Include peak decoder scratch as well as retained blobs and body/Arc metadata.
// Geometry stays borrowed: no extra vertex/index/acceleration allocations exist.
fn bounded_usage(
    bodies: &[SceneBody],
    limits: SceneLimits,
) -> Result<(usize, Option<Bounds>), SceneError> {
    let mut bytes = size_of::<SceneSnapshot>();
    let (mut triangles, mut segments, mut scratch) = (0usize, 0usize, 0usize);
    let mut bounds: Option<Bounds> = None;
    for body in bodies {
        let metadata = size_of::<SceneBody>() + 2 * size_of::<usize>();
        let retained = budget_add(
            body.bytes.len(),
            metadata,
            limits.max_resident_bytes,
            "body bytes",
        )?;
        bytes = budget_add(bytes, retained, limits.max_resident_bytes, "mesh bytes")?;
        scratch = scratch.max(body.decode_scratch_bytes);
        triangles = budget_add(triangles, body.triangles, limits.max_triangles, "triangles")?;
        segments = budget_add(
            segments,
            body.segments,
            limits.max_edge_segments,
            "edge segments",
        )?;
        bounds = Some(match bounds {
            Some(b) => Bounds {
                min: b.min.min(body.bounds.min),
                max: b.max.max(body.bounds.max),
            },
            None => body.bounds,
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
