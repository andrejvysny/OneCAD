//! Deterministic geometric silhouette candidates, adapted from render-look/scene.rs.
//! Sorting edge uses replaces hash maps and atomic GPU append with bounded scratch space.
use crate::RenderError;
use bytemuck::{Pod, Zeroable};
use onecad_protocol::mesh::MeshView;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct SilEdge {
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub n0: [f32; 3],
    pub n1: [f32; 3],
}

#[derive(Clone, Copy)]
struct EdgeUse {
    key: [[i64; 3]; 2],
    a: [f32; 3],
    b: [f32; 3],
    normal: [f32; 3],
}

pub(crate) fn scratch_bytes(triangles: u64) -> Result<u64, RenderError> {
    crate::limits::mul(
        crate::limits::mul(triangles, 3)?,
        std::mem::size_of::<EdgeUse>() as u64,
    )
}

fn key(point: [f32; 3]) -> Result<[i64; 3], RenderError> {
    let scaled = point.map(|v| f64::from(v) * 1e4);
    if scaled
        .iter()
        .any(|v| *v < i64::MIN as f64 || *v >= i64::MAX as f64)
    {
        return Err(RenderError::Mesh("silhouette weld coordinate range"));
    }
    Ok(scaled.map(|v| v.round() as i64))
}

fn edge_uses(mesh: &MeshView<'_>, first: u32, count: u32) -> Result<Vec<EdgeUse>, RenderError> {
    let mut uses = Vec::new();
    uses.try_reserve_exact(count as usize * 3)
        .map_err(|_| RenderError::Budget("silhouette scratch"))?;
    for triangle in first..first + count {
        let indices = mesh
            .triangle(triangle as usize)
            .ok_or(RenderError::Mesh("silhouette triangle"))?;
        let a = mesh
            .position(indices[0] as usize)
            .ok_or(RenderError::Mesh("silhouette position"))?;
        let b = mesh
            .position(indices[1] as usize)
            .ok_or(RenderError::Mesh("silhouette position"))?;
        let c = mesh
            .position(indices[2] as usize)
            .ok_or(RenderError::Mesh("silhouette position"))?;
        let points = [a, b, c];
        let normal = (glam::Vec3::from(b) - glam::Vec3::from(a))
            .cross(glam::Vec3::from(c) - glam::Vec3::from(a))
            .normalize_or_zero()
            .to_array();
        for index in 0..3 {
            let a = points[index];
            let b = points[(index + 1) % 3];
            let ka = key(a)?;
            let kb = key(b)?;
            if ka == kb {
                continue;
            }
            let key = if ka < kb { [ka, kb] } else { [kb, ka] };
            uses.push(EdgeUse { key, a, b, normal });
        }
    }
    uses.sort_unstable_by_key(|edge| edge.key);
    Ok(uses)
}

pub(crate) fn candidates(mesh: &MeshView<'_>) -> Result<Vec<SilEdge>, RenderError> {
    let mut out = Vec::new();
    out.try_reserve_exact(mesh.header().triangle_count as usize * 3)
        .map_err(|_| RenderError::Budget("silhouette candidates"))?;
    for face in 0..mesh.header().face_count {
        let span = mesh
            .face_range(face as usize)
            .ok_or(RenderError::Mesh("silhouette face"))?;
        let uses = edge_uses(mesh, span.first, span.count)?;
        let mut first = 0;
        while first < uses.len() {
            let mut end = first + 1;
            while end < uses.len() && uses[end].key == uses[first].key {
                end += 1;
            }
            if end - first == 2 {
                let a = uses[first];
                let b = uses[first + 1];
                if glam::Vec3::from(a.normal).dot(glam::Vec3::from(b.normal)) <= 0.99999 {
                    out.push(SilEdge {
                        a: a.a,
                        b: a.b,
                        n0: a.normal,
                        n1: b.normal,
                    });
                }
            }
            first = end;
        }
    }
    Ok(out)
}
