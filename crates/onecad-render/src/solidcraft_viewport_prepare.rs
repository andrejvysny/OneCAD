//! Bounded CPU packing of checked OneCAD meshes into SolidCraft draw batches.
use crate::{
    limits::{add, mul},
    MemoryEstimate, RenderError, RenderLimits,
};
use onecad_scene::{BodyId, SceneIdentity, SceneSnapshot};
use solidcraft_gpu::view::{GpuScene, LINE_SIZE, TRI_SIZE};
use std::collections::HashMap;

#[derive(Clone, Copy)]
pub struct SolidCraftPreparationBudget {
    pub(crate) limits: RenderLimits,
    pub(crate) resident: MemoryEstimate,
}
impl SolidCraftPreparationBudget {
    pub fn limit_cpu_prepared_bytes(mut self, maximum: u64) -> Self {
        self.limits.max_cpu_prepared_bytes = self.limits.max_cpu_prepared_bytes.min(maximum);
        self
    }
}

pub(crate) struct PackedBody {
    pub id: BodyId,
    pub revision: u64,
    pub scene: GpuScene,
    pub selected: bool,
}

/// CPU-only upload payload. Construct on a service thread, then fence replacement on the host.
pub struct PreparedSolidCraftScene {
    pub(crate) identity: SceneIdentity,
    pub(crate) source_bytes: u64,
    pub(crate) bytes: u64,
    pub(crate) bodies: Vec<PackedBody>,
    pub(crate) bounds: Option<onecad_scene::Bounds>,
    pub(crate) body_indices: HashMap<BodyId, usize>,
}
impl PreparedSolidCraftScene {
    pub fn new(
        scene: &SceneSnapshot,
        budget: SolidCraftPreparationBudget,
    ) -> Result<Self, RenderError> {
        if scene.bodies().len() > budget.limits.max_bodies {
            return Err(RenderError::Budget("body count"));
        }
        let bytes = estimate(scene)?;
        budget.limits.check(
            MemoryEstimate {
                source_bytes: scene.resident_bytes() as u64,
                cpu_prepared_bytes: mul(bytes, 3)?,
                geometry_bytes: bytes,
                upload_bytes: bytes,
                target_bytes: budget.resident.target_bytes,
                ..Default::default()
            },
            MemoryEstimate {
                target_bytes: 0,
                ..budget.resident
            },
        )?;
        let mut bodies = Vec::new();
        bodies
            .try_reserve_exact(scene.bodies().len())
            .map_err(|_| RenderError::Budget("donor body metadata"))?;
        for body in scene.bodies() {
            bodies.push(PackedBody {
                id: body.body_id(),
                revision: body.mesh_revision(),
                scene: pack(body)?,
                selected: false,
            });
        }
        let body_indices = index_bodies(&bodies)?;
        Ok(Self {
            identity: scene.identity(),
            source_bytes: scene.resident_bytes() as u64,
            bytes,
            bodies,
            bounds: scene.bounds(),
            body_indices,
        })
    }
    pub fn identity(&self) -> SceneIdentity {
        self.identity
    }
    pub fn cpu_bytes(&self) -> u64 {
        self.bytes
    }
    pub fn body_count(&self) -> usize {
        self.bodies.len()
    }
}

fn estimate(scene: &SceneSnapshot) -> Result<u64, RenderError> {
    let mut bytes = add(8192, mul(scene.bodies().len() as u64, 128)?)?;
    for body in scene.bodies() {
        let mesh = body.mesh()?;
        if mesh.normals().is_none() {
            return Err(RenderError::Mesh("normal section required"));
        }
        bytes = add(
            bytes,
            mul(
                u64::from(mesh.header().triangle_count),
                (3 * TRI_SIZE) as u64,
            )?,
        )?;
        for edge in 0..mesh.header().edge_count {
            let span = mesh
                .edge_range(edge as usize)
                .ok_or(RenderError::Mesh("edge range"))?;
            bytes = add(
                bytes,
                mul(u64::from(span.count.saturating_sub(1)), LINE_SIZE as u64)?,
            )?;
        }
    }
    Ok(bytes)
}

fn pack(body: &onecad_scene::SceneBody) -> Result<GpuScene, RenderError> {
    let mesh = body.mesh()?;
    let mut scene = GpuScene::default();
    let triangles = mesh.header().triangle_count as usize;
    scene
        .tris
        .try_reserve_exact(triangles * 3 * TRI_SIZE)
        .map_err(|_| RenderError::Budget("donor triangles"))?;
    for triangle in 0..triangles {
        for vertex in mesh
            .triangle(triangle)
            .ok_or(RenderError::Mesh("triangle"))?
        {
            scene.tri(
                mesh.position(vertex as usize)
                    .ok_or(RenderError::Mesh("position"))?,
                mesh.normal(vertex as usize)
                    .ok_or(RenderError::Mesh("normal"))?,
                [168, 179, 194, 255],
            );
        }
    }
    for edge in 0..mesh.header().edge_count {
        let span = mesh
            .edge_range(edge as usize)
            .ok_or(RenderError::Mesh("edge range"))?;
        scene
            .lines
            .try_reserve_exact(span.count.saturating_sub(1) as usize * LINE_SIZE)
            .map_err(|_| RenderError::Budget("donor lines"))?;
        for point in 1..span.count {
            scene.line(
                mesh.edge_point((span.first + point - 1) as usize)
                    .ok_or(RenderError::Mesh("edge point"))?,
                mesh.edge_point((span.first + point) as usize)
                    .ok_or(RenderError::Mesh("edge point"))?,
                [35, 42, 55, 255],
                1.0,
                false,
            );
        }
    }
    Ok(scene)
}

fn index_bodies(bodies: &[PackedBody]) -> Result<HashMap<BodyId, usize>, RenderError> {
    let mut body_indices = HashMap::new();
    body_indices
        .try_reserve(bodies.len())
        .map_err(|_| RenderError::Budget("donor body index"))?;
    for (index, body) in bodies.iter().enumerate() {
        body_indices.insert(body.id, index);
    }
    Ok(body_indices)
}
