use crate::{
    gpu::Gpu,
    limits::{add, mul},
    RenderError,
};
use bytemuck::{Pod, Zeroable};
use onecad_scene::{BodyId, SceneBody, SceneIdentity};
use std::sync::Arc;
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
    pub(crate) mesh_revision: u64,
    pub(crate) geometry: Arc<PreparedGeometry>,
    pub(crate) model: [[f32; 4]; 4],
    pub(crate) is_instance: bool,
    pub(crate) visible: bool,
    pub(crate) source_body_id: BodyId,
    pub(crate) path: Option<onecad_scene::instances::OccurrencePath>,
    pub(crate) model64: [f64; 16],
}

pub(crate) struct PreparedGeometry {
    pub(crate) source: onecad_scene::SceneGeometry,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
    segments: Vec<Segment>,
    feature_segments: Vec<Segment>,
    silhouettes: Vec<crate::silhouette::SilEdge>,
    source_retention: Option<Arc<dyn Send + Sync>>,
    _preparation_retention: Option<Arc<dyn Send + Sync>>,
}

#[derive(Clone)]
pub(crate) struct GpuBody {
    pub(crate) source: onecad_scene::SceneGeometry,
    pub id: BodyId,
    pub mesh_revision: u64,
    pub style: BodyStyle,
    pub preview_suppressed: bool,
    pub positions: wgpu::Buffer,
    pub normals: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub segments: wgpu::Buffer,
    pub feature_segments: wgpu::Buffer,
    pub feature_segment_count: u32,
    pub silhouettes: wgpu::Buffer,
    pub silhouette_count: u32,
    pub index_count: u32,
    pub segment_count: u32,
    pub uniform: wgpu::Buffer,
    pub frame_bind: wgpu::BindGroup,
    pub model_buffer: wgpu::Buffer,
    pub model_bind: wgpu::BindGroup,
    pub model: [[f32; 4]; 4],
    pub is_instance: bool,
    pub visible: bool,
    pub source_body_id: BodyId,
    pub path: Option<onecad_scene::instances::OccurrencePath>,
    pub model64: [f64; 16],
    _source_retention: Option<Arc<dyn Send + Sync>>,
}

#[cfg(test)]
pub(crate) fn estimate(
    scene: &onecad_scene::SceneSnapshot,
    max_buffer: u64,
) -> Result<u64, RenderError> {
    estimate_inner(scene, max_buffer, true)
}

pub(crate) fn body_estimate(
    body: &SceneBody,
    max_buffer: u64,
    silhouettes: bool,
) -> Result<u64, RenderError> {
    let mesh = body.mesh()?;
    let h = mesh.header();
    if mesh.normals().is_none() {
        return Err(RenderError::Mesh("normal section required"));
    }
    let mut segments = 0;
    let mut features = 0;
    for i in 0..h.edge_count {
        let range = mesh
            .edge_range(i as usize)
            .ok_or(RenderError::Mesh("edge range"))?;
        segments = add(segments, u64::from(range.count.saturating_sub(1)))?;
        if clean_edge(mesh.edge_class(i as usize)) {
            features = add(features, u64::from(range.count.saturating_sub(1)))?;
        }
    }
    if u64::from(h.triangle_count) * 3 > u64::from(u32::MAX) || segments > u64::from(u32::MAX) {
        return Err(RenderError::Budget("draw count"));
    }
    let lengths = [
        mul(u64::from(h.vertex_count), 12)?,
        mul(u64::from(h.vertex_count), 12)?,
        mul(u64::from(h.triangle_count), 12)?,
        mul(segments, 24)?,
        mul(features, 24)?,
        mul(
            if silhouettes {
                crate::silhouette::candidate_count(&mesh)? as u64
            } else {
                0
            },
            48,
        )?,
    ];
    buffer_estimate(lengths, max_buffer)
}

fn buffer_estimate(lengths: [u64; 6], max_buffer: u64) -> Result<u64, RenderError> {
    lengths.into_iter().try_fold(0, |total, length| {
        if length.max(64) > max_buffer {
            return Err(RenderError::Budget("device buffer size"));
        }
        add(total, length.max(64))
    })
}

#[cfg(test)]
fn estimate_inner(
    scene: &onecad_scene::SceneSnapshot,
    max_buffer: u64,
    silhouettes: bool,
) -> Result<u64, RenderError> {
    scene.bodies().iter().try_fold(0, |total, body| {
        add(total, body_estimate(body, max_buffer, silhouettes)?)
    })
}

impl PreparedGeometry {
    pub(crate) fn retains(&self, requested: &crate::PreparationRetention) -> bool {
        (requested.source.is_none() || self.source_retention.is_some())
            && (requested.preparation.is_none() || self._preparation_retention.is_some())
    }
    pub(crate) fn gpu_bytes(&self, max_buffer: u64) -> Result<u64, RenderError> {
        buffer_estimate(
            [
                self.positions.len() as u64 * 12,
                self.normals.len() as u64 * 12,
                self.indices.len() as u64 * 4,
                self.segments.len() as u64 * 24,
                self.feature_segments.len() as u64 * 24,
                self.silhouettes.len() as u64 * 48,
            ],
            max_buffer,
        )
    }

    pub(crate) fn cpu_bytes(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
            + 2 * std::mem::size_of::<usize>() as u64
            + self.positions.capacity() as u64 * 12
            + self.normals.capacity() as u64 * 12
            + self.indices.capacity() as u64 * 4
            + self.segments.capacity() as u64 * 24
            + self.feature_segments.capacity() as u64 * 24
            + self.silhouettes.capacity() as u64 * 48
    }
}

fn reserve<T>(count: usize) -> Result<Vec<T>, RenderError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| RenderError::Budget("CPU allocation"))?;
    Ok(values)
}

// Bound f32 model conversion and shader arithmetic to less than 1/1024 mm.
pub(crate) fn check_occurrence_precision(body: &SceneBody) -> Result<(), RenderError> {
    if body.occurrence_path().is_none() {
        return Ok(());
    }
    let matrix = body.model().to_cols_array();
    let bounds = body.local_bounds();
    let extent = bounds.min.abs().max(bounds.max.abs()).to_array();
    for row in 0..3 {
        let translation = matrix[12 + row];
        let mut error = (translation - f64::from(translation as f32)).abs();
        let mut magnitude = translation.abs();
        for col in 0..3 {
            let coefficient = matrix[col * 4 + row];
            error += (coefficient - f64::from(coefficient as f32)).abs() * extent[col];
            magnitude += coefficient.abs() * extent[col];
        }
        error += 8.0 * f64::from(f32::EPSILON) * magnitude;
        if !error.is_finite() || error > 1.0 / 1024.0 {
            return Err(RenderError::Mesh(
                "occurrence exceeds 1/1024 mm GPU precision bound",
            ));
        }
    }
    Ok(())
}

impl PreparedBody {
    pub(crate) fn from_geometry(
        body: &SceneBody,
        geometry: Arc<PreparedGeometry>,
    ) -> Result<Self, RenderError> {
        check_occurrence_precision(body)?;
        let bounds = body.bounds();
        if bounds
            .min
            .to_array()
            .into_iter()
            .chain(bounds.max.to_array())
            .any(|value| !(value as f32).is_finite())
        {
            return Err(RenderError::Mesh("occurrence bounds exceed GPU precision"));
        }
        let values = body.model().to_cols_array().map(|value| value as f32);
        if values.iter().any(|v| !v.is_finite()) {
            return Err(RenderError::Mesh("occurrence model exceeds GPU precision"));
        }
        Ok(Self {
            mesh_revision: body.mesh_revision(),
            geometry,
            model: [
                values[0..4]
                    .try_into()
                    .map_err(|_| RenderError::Mesh("model"))?,
                values[4..8]
                    .try_into()
                    .map_err(|_| RenderError::Mesh("model"))?,
                values[8..12]
                    .try_into()
                    .map_err(|_| RenderError::Mesh("model"))?,
                values[12..16]
                    .try_into()
                    .map_err(|_| RenderError::Mesh("model"))?,
            ],
            is_instance: body.occurrence_path().is_some(),
            visible: body.visible(),
            source_body_id: body.source_body_id(),
            path: body.occurrence_path().cloned(),
            model64: body.model().to_cols_array(),
        })
    }
    #[cfg(test)]
    pub(crate) fn new(body: &SceneBody) -> Result<Self, RenderError> {
        Self::new_retained(body, crate::PreparationRetention::default())
    }
    pub(crate) fn new_retained(
        body: &SceneBody,
        retention: crate::PreparationRetention,
    ) -> Result<Self, RenderError> {
        Self::new_allocated(body, retention, None)
    }
    pub(crate) fn new_allocated(
        body: &SceneBody,
        retention: crate::PreparationRetention,
        allocator: Option<&mut crate::prepared::GeometryAdmission<'_>>,
    ) -> Result<Self, RenderError> {
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
        let feature_count: usize = (0..h.edge_count as usize)
            .filter(|i| clean_edge(mesh.edge_class(*i)))
            .map(|i| {
                mesh.edge_range(i)
                    .map_or(0, |range| range.count.saturating_sub(1) as usize)
            })
            .sum();
        let mut feature_segments = reserve(feature_count)?;
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
                let segment = Segment {
                    a: mesh
                        .edge_point((range.first + j - 1) as usize)
                        .ok_or(RenderError::Mesh("edge point"))?,
                    b: mesh
                        .edge_point((range.first + j) as usize)
                        .ok_or(RenderError::Mesh("edge point"))?,
                };
                segments.push(segment);
                if clean_edge(mesh.edge_class(i as usize)) {
                    feature_segments.push(segment);
                }
            }
        }
        let silhouettes = crate::silhouette::candidates(&mesh)?;
        let mut geometry = PreparedGeometry {
            source: body.retained_geometry(),
            source_retention: body.geometry_retention().or(retention.source),
            _preparation_retention: retention.preparation,
            positions,
            normals,
            indices,
            segments,
            feature_segments,
            silhouettes,
        };
        if let Some(allocate) = allocator {
            geometry._preparation_retention = Some(allocate(geometry.cpu_bytes())?);
        }
        Self::from_geometry(body, Arc::new(geometry))
    }
    pub(crate) fn upload(
        self,
        gpu: &Gpu,
        frame: &crate::uniforms::FrameBinding,
        id: BodyId,
        model: &ModelBinding,
    ) -> GpuBody {
        let buffer = |label, data: &[u8], usage| {
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: if data.is_empty() { &[0; 64] } else { data },
                    usage,
                })
        };
        let vertex = wgpu::BufferUsages::VERTEX;
        GpuBody {
            preview_suppressed: false,
            source: self.geometry.source.clone(),
            _source_retention: self.geometry.source_retention.clone(),
            id,
            mesh_revision: self.mesh_revision,
            style: BodyStyle::default(),
            index_count: self.geometry.indices.len() as u32,
            segment_count: self.geometry.segments.len() as u32,
            feature_segment_count: self.geometry.feature_segments.len() as u32,
            feature_segments: buffer(
                "feature edges",
                bytemuck::cast_slice(&self.geometry.feature_segments),
                vertex,
            ),
            silhouette_count: self.geometry.silhouettes.len() as u32,
            silhouettes: buffer(
                "silhouettes",
                bytemuck::cast_slice(&self.geometry.silhouettes),
                vertex,
            ),
            positions: buffer(
                "positions",
                bytemuck::cast_slice(&self.geometry.positions),
                vertex,
            ),
            normals: buffer(
                "normals",
                bytemuck::cast_slice(&self.geometry.normals),
                vertex,
            ),
            indices: buffer(
                "indices",
                bytemuck::cast_slice(&self.geometry.indices),
                wgpu::BufferUsages::INDEX,
            ),
            segments: buffer(
                "edges",
                bytemuck::cast_slice(&self.geometry.segments),
                vertex,
            ),
            uniform: frame.buffer.clone(),
            frame_bind: frame.bind.clone(),
            model_buffer: model.buffer.clone(),
            model_bind: model.bind.clone(),
            model: self.model,
            is_instance: self.is_instance,
            visible: self.visible,
            source_body_id: self.source_body_id,
            path: self.path,
            model64: self.model64,
        }
    }
}

pub(crate) struct ModelBinding {
    pub buffer: wgpu::Buffer,
    pub bind: wgpu::BindGroup,
}
impl ModelBinding {
    pub fn new(gpu: &Gpu, layout: &wgpu::BindGroupLayout, model: &[[f32; 4]; 4]) -> Self {
        let buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("occurrence model"),
                contents: bytemuck::bytes_of(model),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind = gpu.bind("occurrence model", layout, &[buffer.as_entire_binding()]);
        Self { buffer, bind }
    }
}

impl GpuBody {
    pub(crate) fn retain_source(&mut self, body: &PreparedBody) {
        if let Some(retention) = &body.geometry.source_retention {
            self._source_retention = Some(retention.clone());
        }
    }
}

pub(crate) fn gpu_bytes(body: &GpuBody) -> u64 {
    body.positions.size()
        + body.normals.size()
        + body.indices.size()
        + body.segments.size()
        + body.feature_segments.size()
        + body.silhouettes.size()
}

pub(crate) fn path_metadata_bytes(bodies: &[GpuBody]) -> u64 {
    bodies
        .iter()
        .map(|body| {
            body.path
                .as_ref()
                .map_or(0, |path| path.0.capacity() as u64 * 16)
        })
        .sum()
}

pub(crate) fn unique_gpu_bytes(bodies: &[GpuBody]) -> u64 {
    bodies
        .iter()
        .enumerate()
        .filter(|(index, body)| {
            !bodies[..*index]
                .iter()
                .any(|other| other.source.shares_source(&body.source))
        })
        .map(|(_, body)| gpu_bytes(body))
        .sum::<u64>()
        + bodies.iter().filter(|body| body.is_instance).count() as u64 * 64
}

#[cfg(test)]
impl PreparedBody {
    pub(crate) fn first_position(&self) -> Option<[f32; 3]> {
        self.geometry.positions.first().copied()
    }
}

#[cfg(test)]
impl PreparedBody {
    pub(crate) fn silhouette_len_and_capacity(&self) -> (usize, usize) {
        (
            self.geometry.silhouettes.len(),
            self.geometry.silhouettes.capacity(),
        )
    }
}

/// Feature edges include tangent (smooth) joins such as fillet boundaries, as CAD
/// viewers show them; only a surface's own seams and collapsed edges are hidden.
fn clean_edge(class: onecad_protocol::mesh::EdgeClass) -> bool {
    use onecad_protocol::mesh::EdgeClass;
    !matches!(class, EdgeClass::Seam | EdgeClass::Degenerate)
}
