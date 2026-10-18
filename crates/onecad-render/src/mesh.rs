use crate::{
    frame::FrameU,
    gpu::Gpu,
    limits::{add, mul},
    RenderError,
};
use bytemuck::{Pod, Zeroable};
use onecad_scene::{BodyId, SceneBody, SceneIdentity, SceneSnapshot};
use wgpu::util::DeviceExt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreviewIdentity {
    pub base: SceneIdentity,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BodyStyle {
    pub selected: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Segment {
    a: [f32; 3],
    b: [f32; 3],
}

pub(crate) struct PreparedBody {
    mesh_revision: u64,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
    segments: Vec<Segment>,
    silhouettes: Vec<crate::silhouette::SilEdge>,
}

pub(crate) struct GpuBody {
    pub id: BodyId,
    pub mesh_revision: u64,
    pub style: BodyStyle,
    pub positions: wgpu::Buffer,
    pub normals: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub segments: wgpu::Buffer,
    pub silhouettes: wgpu::Buffer,
    pub silhouette_count: u32,
    pub index_count: u32,
    pub segment_count: u32,
    pub uniform: wgpu::Buffer,
    pub frame_bind: wgpu::BindGroup,
}

pub(crate) fn estimate(scene: &SceneSnapshot, max_buffer: u64) -> Result<u64, RenderError> {
    let mut total = 0;
    for body in scene.bodies() {
        let mesh = body.mesh()?;
        let h = mesh.header();
        if mesh.normals().is_none() {
            return Err(RenderError::Mesh("normal section required"));
        }
        let mut segments = 0u64;
        for i in 0..h.edge_count {
            let range = mesh
                .edge_range(i as usize)
                .ok_or(RenderError::Mesh("edge range"))?;
            segments = add(segments, u64::from(range.count.saturating_sub(1)))?;
        }
        let lengths = [
            mul(u64::from(h.vertex_count), 12)?,
            mul(u64::from(h.vertex_count), 12)?,
            mul(u64::from(h.triangle_count), 12)?,
            mul(segments, 24)?,
            mul(u64::from(h.triangle_count), 144)?,
        ];
        if u64::from(h.triangle_count) * 3 > u64::from(u32::MAX) || segments > u64::from(u32::MAX) {
            return Err(RenderError::Budget("draw count"));
        }
        for length in lengths {
            if length.max(64) > max_buffer {
                return Err(RenderError::Budget("device buffer size"));
            }
            total = add(total, length.max(64))?;
        }
        total = add(total, std::mem::size_of::<FrameU>() as u64)?;
    }
    Ok(total)
}

fn reserve<T>(count: usize) -> Result<Vec<T>, RenderError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| RenderError::Budget("CPU allocation"))?;
    Ok(values)
}

impl PreparedBody {
    pub(crate) fn new(body: &SceneBody) -> Result<Self, RenderError> {
        let mesh = body.mesh()?;
        let h = mesh.header();
        let mut positions = reserve(h.vertex_count as usize)?;
        let mut normals = reserve(h.vertex_count as usize)?;
        let mut indices = reserve(h.triangle_count as usize * 3)?;
        let mut count = 0usize;
        for i in 0..h.edge_count {
            count += mesh
                .edge_range(i as usize)
                .ok_or(RenderError::Mesh("edge range"))?
                .count
                .saturating_sub(1) as usize;
        }
        let mut segments = reserve(count)?;
        for i in 0..h.vertex_count {
            positions.push(
                mesh.position(i as usize)
                    .ok_or(RenderError::Mesh("position"))?,
            );
            normals.push(mesh.normal(i as usize).ok_or(RenderError::Mesh("normal"))?);
        }
        for i in 0..h.triangle_count {
            indices.extend(
                mesh.triangle(i as usize)
                    .ok_or(RenderError::Mesh("triangle"))?,
            );
        }
        for i in 0..h.edge_count {
            let range = mesh
                .edge_range(i as usize)
                .ok_or(RenderError::Mesh("edge range"))?;
            for j in 1..range.count {
                segments.push(Segment {
                    a: mesh
                        .edge_point((range.first + j - 1) as usize)
                        .ok_or(RenderError::Mesh("edge point"))?,
                    b: mesh
                        .edge_point((range.first + j) as usize)
                        .ok_or(RenderError::Mesh("edge point"))?,
                });
            }
        }
        Ok(Self {
            mesh_revision: body.mesh_revision(),
            positions,
            normals,
            indices,
            segments,
            silhouettes: crate::silhouette::candidates(&mesh)?,
        })
    }
    pub(crate) fn upload(self, gpu: &Gpu, layout: &wgpu::BindGroupLayout, id: BodyId) -> GpuBody {
        let buffer = |label, data: &[u8], usage| {
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: if data.is_empty() { &[0; 64] } else { data },
                    usage,
                })
        };
        let vertex = wgpu::BufferUsages::VERTEX;
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("body frame"),
            size: std::mem::size_of::<FrameU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_bind = gpu.bind("body frame", layout, &[uniform.as_entire_binding()]);
        GpuBody {
            id,
            mesh_revision: self.mesh_revision,
            style: BodyStyle::default(),
            index_count: self.indices.len() as u32,
            segment_count: self.segments.len() as u32,
            silhouette_count: self.silhouettes.len() as u32,
            silhouettes: buffer(
                "silhouettes",
                bytemuck::cast_slice(&self.silhouettes),
                vertex,
            ),
            positions: buffer("positions", bytemuck::cast_slice(&self.positions), vertex),
            normals: buffer("normals", bytemuck::cast_slice(&self.normals), vertex),
            indices: buffer(
                "indices",
                bytemuck::cast_slice(&self.indices),
                wgpu::BufferUsages::INDEX,
            ),
            segments: buffer("edges", bytemuck::cast_slice(&self.segments), vertex),
            uniform,
            frame_bind,
        }
    }
}

pub(crate) fn gpu_bytes(body: &GpuBody) -> u64 {
    body.positions.size()
        + body.normals.size()
        + body.indices.size()
        + body.segments.size()
        + body.uniform.size()
        + body.silhouettes.size()
}

#[cfg(test)]
impl PreparedBody {
    pub(crate) fn first_position(&self) -> Option<[f32; 3]> {
        self.positions.first().copied()
    }
}

pub(crate) fn cpu_scratch_bytes(scene: &SceneSnapshot) -> Result<u64, RenderError> {
    let mut scratch = 0;
    for body in scene.bodies() {
        scratch = add(
            scratch,
            crate::silhouette::scratch_bytes(u64::from(body.mesh()?.header().triangle_count))?,
        )?;
    }
    add(
        scratch,
        mul(
            scene.bodies().len() as u64,
            std::mem::size_of::<PreparedBody>() as u64,
        )?,
    )
}
