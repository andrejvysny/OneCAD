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
    triangle: u32,
    edge: u32,
}

pub(crate) fn scratch_bytes(mesh: &MeshView<'_>) -> Result<u64, RenderError> {
    let mut largest = 0;
    for face in 0..mesh.header().face_count {
        let span = mesh
            .face_range(face as usize)
            .ok_or(RenderError::Mesh("silhouette face"))?;
        largest = largest.max(u64::from(span.count));
    }
    crate::limits::mul(
        crate::limits::mul(largest, 3)?,
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

fn edge_uses(
    mesh: &MeshView<'_>,
    first: u32,
    count: u32,
    uses: &mut Vec<EdgeUse>,
) -> Result<(), RenderError> {
    uses.clear();
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
        for index in 0..3 {
            let a = points[index];
            let b = points[(index + 1) % 3];
            let ka = key(a)?;
            let kb = key(b)?;
            if ka == kb {
                continue;
            }
            let key = if ka < kb { [ka, kb] } else { [kb, ka] };
            uses.push(EdgeUse {
                key,
                triangle,
                edge: index as u32,
            });
        }
    }
    uses.sort_unstable_by_key(|edge| edge.key);
    Ok(())
}

fn geometry(mesh: &MeshView<'_>, edge: EdgeUse) -> Result<[[f32; 3]; 3], RenderError> {
    let indices = mesh
        .triangle(edge.triangle as usize)
        .ok_or(RenderError::Mesh("silhouette triangle"))?;
    let mut points = [[0.0; 3]; 3];
    for (point, index) in points.iter_mut().zip(indices) {
        *point = mesh
            .position(index as usize)
            .ok_or(RenderError::Mesh("silhouette position"))?;
    }
    let normal = (glam::Vec3::from(points[1]) - glam::Vec3::from(points[0]))
        .cross(glam::Vec3::from(points[2]) - glam::Vec3::from(points[0]))
        .normalize_or_zero()
        .to_array();
    Ok([
        points[edge.edge as usize],
        points[(edge.edge as usize + 1) % 3],
        normal,
    ])
}

fn visit(mesh: &MeshView<'_>, mut emit: impl FnMut(SilEdge)) -> Result<usize, RenderError> {
    let mut uses = Vec::new();
    let capacity = scratch_bytes(mesh)? / std::mem::size_of::<EdgeUse>() as u64;
    uses.try_reserve_exact(capacity as usize)
        .map_err(|_| RenderError::Budget("silhouette scratch"))?;
    let mut count = 0;
    for face in 0..mesh.header().face_count {
        let span = mesh
            .face_range(face as usize)
            .ok_or(RenderError::Mesh("silhouette face"))?;
        edge_uses(mesh, span.first, span.count, &mut uses)?;
        let mut first = 0;
        while first < uses.len() {
            let mut end = first + 1;
            while end < uses.len() && uses[end].key == uses[first].key {
                end += 1;
            }
            if end - first == 2 {
                let [a, b, n0] = geometry(mesh, uses[first])?;
                let [_, _, n1] = geometry(mesh, uses[first + 1])?;
                if glam::Vec3::from(n0).dot(glam::Vec3::from(n1)) <= 0.99999 {
                    emit(SilEdge { a, b, n0, n1 });
                    count += 1;
                }
            }
            first = end;
        }
    }
    Ok(count)
}

pub(crate) fn candidate_count(mesh: &MeshView<'_>) -> Result<usize, RenderError> {
    visit(mesh, |_| {})
}

pub(crate) fn candidates(mesh: &MeshView<'_>) -> Result<Vec<SilEdge>, RenderError> {
    let count = candidate_count(mesh)?;
    let mut out = Vec::new();
    out.try_reserve_exact(count)
        .map_err(|_| RenderError::Budget("silhouette candidates"))?;
    visit(mesh, |edge| out.push(edge))?;
    Ok(out)
}
