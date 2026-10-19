//! Structural validation for immutable, borrowed MESH1 views.
use super::{f32_le, u16_le, u32_le, validate_mesh_blob, MeshError, MeshHeaderView};
use super::{FLAG_HAS_EDGES, FLAG_HAS_FACE_BBOXES, FLAG_HAS_FACE_COLORS, FLAG_HAS_NORMALS};

/// Resource policy checked before header/table allocation or body traversal.
#[derive(Debug, Clone, Copy)]
pub struct MeshLimits {
    pub max_blob_bytes: usize,
    pub max_sections: u16,
    pub max_vertices: u32,
    pub max_triangles: u32,
    pub max_faces: u32,
    pub max_edges: u32,
    pub max_edge_points: u32,
    /// Combined FACE_ID_CHARS and EDGE_ID_CHARS bytes.
    pub max_id_bytes: usize,
}

impl Default for MeshLimits {
    fn default() -> Self {
        Self {
            max_blob_bytes: 1 << 30,
            max_sections: 128,
            max_vertices: 32_000_000,
            max_triangles: 32_000_000,
            max_faces: 4_000_000,
            max_edges: 4_000_000,
            max_edge_points: 32_000_000,
            max_id_bytes: 64 << 20,
        }
    }
}

/// Failures from checked decoding; the legacy header validator is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MeshDecodeError {
    #[error(transparent)]
    Header(#[from] MeshError),
    #[error("MESH1 resource limit exceeded: {0}")]
    Limit(&'static str),
    #[error("unsupported MESH1 lod: {0}")]
    UnsupportedLod(u16),
    #[error("MESH1 section {0} missing")]
    MissingSection(u32),
    #[error("MESH1 section {0} conflicts with header flags/counts")]
    FlagMismatch(u32),
    #[error("MESH1 section {0} has invalid byte length")]
    Length(u32),
    #[error("MESH1 count arithmetic overflow in section {0}")]
    CountOverflow(u32),
    #[error("MESH1 padding must be zero")]
    Padding,
    #[error("MESH1 section {0} contains a non-finite float")]
    NonFinite(u32),
    #[error("MESH1 bounding box in section {0} has min > max")]
    BoundingBox(u32),
    #[error("MESH1 vertex index out of bounds")]
    VertexIndex,
    #[error("MESH1 edge class is unknown")]
    EdgeClass,
    #[error("MESH1 incident face ordinal out of bounds")]
    IncidentFace,
    #[error("MESH1 incident faces must be unique ascending ordinals")]
    IncidentFaceOrder,
    #[error("MESH1 section {0} ranges are not contiguous bounded full coverage")]
    Ranges(u32),
    #[error("MESH1 section {0} ID offsets are not bounded prefix sums")]
    IdOffsets(u32),
    #[error("MESH1 section {0} ID is not valid UTF-8")]
    IdUtf8(u32),
}

pub(super) fn preflight(blob: &[u8], limits: MeshLimits) -> Result<(), MeshDecodeError> {
    if blob.len() > limits.max_blob_bytes {
        return Err(MeshDecodeError::Limit("blob bytes"));
    }
    if blob.len() < 64 {
        return Err(MeshError::TooSmall(blob.len()).into());
    }
    for (off, max, name) in [
        (8, limits.max_vertices, "vertices"),
        (12, limits.max_triangles, "triangles"),
        (16, limits.max_faces, "faces"),
        (20, limits.max_edges, "edges"),
        (24, limits.max_edge_points, "edge points"),
    ] {
        if u32_le(blob, off) > max {
            return Err(MeshDecodeError::Limit(name));
        }
    }
    if u16_le(blob, 30) > limits.max_sections {
        return Err(MeshDecodeError::Limit("sections"));
    }
    preflight_ids(blob, limits)
}

fn preflight_ids(blob: &[u8], limits: MeshLimits) -> Result<(), MeshDecodeError> {
    let table_end = 64 + usize::from(u16_le(blob, 30)) * 16;
    if table_end > blob.len() {
        return Err(MeshError::TruncatedTable {
            need: table_end,
            have: blob.len(),
        }
        .into());
    }
    let mut bytes = 0usize;
    for entry in blob[64..table_end].as_chunks::<16>().0.iter() {
        if matches!(u32_le(entry, 0), 6 | 10) {
            bytes = bytes
                .checked_add(u32_le(entry, 12) as usize)
                .ok_or(MeshDecodeError::Limit("ID bytes"))?;
            if bytes > limits.max_id_bytes {
                return Err(MeshDecodeError::Limit("ID bytes"));
            }
        }
    }
    Ok(())
}

pub(super) fn validate(blob: &[u8], limits: MeshLimits) -> Result<MeshHeaderView, MeshDecodeError> {
    preflight(blob, limits)?;
    let header = validate_mesh_blob(blob)?;
    if super::Lod::from_u16(header.lod).is_none() {
        return Err(MeshDecodeError::UnsupportedLod(header.lod));
    }
    validate_padding(blob, &header)?;
    bbox(&blob[32..56], 0)?;
    validate_lengths(&header, limits)?;
    validate_geometry(blob, &header)?;
    Ok(header)
}

fn validate_padding(blob: &[u8], h: &MeshHeaderView) -> Result<(), MeshDecodeError> {
    let mut end = 64 + h.sections.len() * 16;
    for s in &h.sections {
        if blob[end..s.offset as usize].iter().any(|&b| b != 0) {
            return Err(MeshDecodeError::Padding);
        }
        end = s.offset as usize + s.byte_len as usize;
    }
    if blob[end..].iter().any(|&b| b != 0) {
        return Err(MeshDecodeError::Padding);
    }
    Ok(())
}

fn length(h: &MeshHeaderView, ty: u32, count: u32, width: u32) -> Result<(), MeshDecodeError> {
    let expected = count
        .checked_mul(width)
        .ok_or(MeshDecodeError::CountOverflow(ty))?;
    let s = h.section(ty).ok_or(MeshDecodeError::MissingSection(ty))?;
    if s.byte_len != expected {
        return Err(MeshDecodeError::Length(ty));
    }
    Ok(())
}

fn validate_lengths(h: &MeshHeaderView, limits: MeshLimits) -> Result<(), MeshDecodeError> {
    for (ty, count, width) in [
        (1, h.vertex_count, 12),
        (3, h.triangle_count, 12),
        (4, h.face_count, 8),
    ] {
        length(h, ty, count, width)?;
    }
    let faces = h
        .face_count
        .checked_add(1)
        .ok_or(MeshDecodeError::CountOverflow(5))?;
    length(h, 5, faces, 4)?;
    h.section(6).ok_or(MeshDecodeError::MissingSection(6))?;
    for (flag, ty, count, width) in [
        (FLAG_HAS_NORMALS, 2, h.vertex_count, 12),
        (FLAG_HAS_EDGES, 7, h.edge_count, 8),
        (FLAG_HAS_EDGES, 8, h.edge_point_count, 12),
        (FLAG_HAS_FACE_BBOXES, 11, h.face_count, 24),
        (FLAG_HAS_FACE_COLORS, 12, h.face_count, 4),
    ] {
        if h.flags & flag != 0 {
            length(h, ty, count, width)?;
        } else if h.section(ty).is_some() {
            return Err(MeshDecodeError::FlagMismatch(ty));
        }
    }
    validate_edge_lengths(h)?;
    validate_metadata_lengths(h)?;
    let bytes = [6, 10]
        .iter()
        .filter_map(|&ty| h.section(ty))
        .try_fold(0usize, |sum, s| {
            sum.checked_add(s.byte_len as usize)
                .ok_or(MeshDecodeError::CountOverflow(s.section_type))
        })?;
    if bytes > limits.max_id_bytes {
        return Err(MeshDecodeError::Limit("ID bytes"));
    }
    Ok(())
}

fn validate_metadata_lengths(h: &MeshHeaderView) -> Result<(), MeshDecodeError> {
    for ty in [14, 15, 16] {
        if h.section(ty).is_some() && !h.has_edges() {
            return Err(MeshDecodeError::FlagMismatch(ty));
        }
    }
    if h.section(14).is_some() {
        length(h, 14, h.edge_count, 4)?;
    }
    if h.section(15).is_some() || h.section(16).is_some() {
        length(h, 15, h.edge_count, 8)?;
        let faces = h.section(16).ok_or(MeshDecodeError::MissingSection(16))?;
        if faces.byte_len % 4 != 0 {
            return Err(MeshDecodeError::Length(16));
        }
    }
    Ok(())
}

fn validate_edge_lengths(h: &MeshHeaderView) -> Result<(), MeshDecodeError> {
    if h.has_edges() {
        let edges = h
            .edge_count
            .checked_add(1)
            .ok_or(MeshDecodeError::CountOverflow(9))?;
        length(h, 9, edges, 4)?;
        h.section(10).ok_or(MeshDecodeError::MissingSection(10))?;
    } else if h.edge_count != 0
        || h.edge_point_count != 0
        || h.section(9).is_some()
        || h.section(10).is_some()
    {
        return Err(MeshDecodeError::FlagMismatch(7));
    }
    Ok(())
}

pub(super) fn section<'a>(blob: &'a [u8], h: &MeshHeaderView, ty: u32) -> Option<&'a [u8]> {
    h.section(ty)
        .map(|s| &blob[s.offset as usize..s.offset as usize + s.byte_len as usize])
}

fn validate_geometry(blob: &[u8], h: &MeshHeaderView) -> Result<(), MeshDecodeError> {
    for ty in [1, 2, 8, 11] {
        if let Some(data) = section(blob, h, ty) {
            if data
                .as_chunks::<4>()
                .0
                .iter()
                .any(|word| !f32_le(word, 0).is_finite())
            {
                return Err(MeshDecodeError::NonFinite(ty));
            }
            if ty == 11 {
                for data in data.as_chunks::<24>().0.iter() {
                    bbox(data, ty)?;
                }
            }
        }
    }
    if section(blob, h, 3)
        .expect("validated section")
        .as_chunks::<4>()
        .0
        .iter()
        .any(|word| u32_le(word, 0) >= h.vertex_count)
    {
        return Err(MeshDecodeError::VertexIndex);
    }
    ranges(
        section(blob, h, 4).expect("validated section"),
        h.triangle_count,
        4,
    )?;
    ids(blob, h, 5, 6)?;
    if h.has_edges() {
        ranges(
            section(blob, h, 7).expect("validated section"),
            h.edge_point_count,
            7,
        )?;
        ids(blob, h, 9, 10)?;
    }
    validate_metadata(blob, h)?;
    Ok(())
}

fn validate_metadata(blob: &[u8], h: &MeshHeaderView) -> Result<(), MeshDecodeError> {
    if let Some(classes) = section(blob, h, 14) {
        if classes
            .as_chunks::<4>()
            .0
            .iter()
            .any(|v| super::EdgeClass::from_u32(u32_le(v, 0)).is_none())
        {
            return Err(MeshDecodeError::EdgeClass);
        }
    }
    if let Some(adjacency) = section(blob, h, 16) {
        let spans = section(blob, h, 15).expect("validated ranges");
        ranges(spans, adjacency.len() as u32 / 4, 15)?;
        if adjacency
            .as_chunks::<4>()
            .0
            .iter()
            .any(|v| u32_le(v, 0) >= h.face_count)
        {
            return Err(MeshDecodeError::IncidentFace);
        }
        for pair in spans.as_chunks::<8>().0.iter() {
            let start = u32_le(pair, 0) as usize * 4;
            let end = start + u32_le(pair, 4) as usize * 4;
            if adjacency[start..end]
                .windows(8)
                .step_by(4)
                .any(|v| u32_le(v, 0) >= u32_le(v, 4))
            {
                return Err(MeshDecodeError::IncidentFaceOrder);
            }
        }
    }
    Ok(())
}

fn bbox(data: &[u8], ty: u32) -> Result<(), MeshDecodeError> {
    if data
        .as_chunks::<4>()
        .0
        .iter()
        .any(|word| !f32_le(word, 0).is_finite())
    {
        return Err(MeshDecodeError::NonFinite(ty));
    }
    if (0..3).any(|i| f32_le(data, i * 4) > f32_le(data, 12 + i * 4)) {
        return Err(MeshDecodeError::BoundingBox(ty));
    }
    Ok(())
}

fn ranges(data: &[u8], total: u32, ty: u32) -> Result<(), MeshDecodeError> {
    let mut end = 0u32;
    for pair in data.as_chunks::<8>().0.iter() {
        let first = u32_le(pair, 0);
        let count = u32_le(pair, 4);
        if first != end {
            return Err(MeshDecodeError::Ranges(ty));
        }
        end = first
            .checked_add(count)
            .filter(|&end| end <= total)
            .ok_or(MeshDecodeError::Ranges(ty))?;
    }
    if end != total {
        return Err(MeshDecodeError::Ranges(ty));
    }
    Ok(())
}

fn ids(
    blob: &[u8],
    h: &MeshHeaderView,
    offs_ty: u32,
    chars_ty: u32,
) -> Result<(), MeshDecodeError> {
    let offsets = section(blob, h, offs_ty).expect("validated section");
    let chars = section(blob, h, chars_ty).expect("validated section");
    let mut start = 0usize;
    if u32_le(offsets, 0) != 0 {
        return Err(MeshDecodeError::IdOffsets(offs_ty));
    }
    for word in offsets[4..].as_chunks::<4>().0.iter() {
        let end = u32_le(word, 0) as usize;
        if end < start || end > chars.len() {
            return Err(MeshDecodeError::IdOffsets(offs_ty));
        }
        std::str::from_utf8(&chars[start..end]).map_err(|_| MeshDecodeError::IdUtf8(chars_ty))?;
        start = end;
    }
    if start != chars.len() {
        return Err(MeshDecodeError::IdOffsets(offs_ty));
    }
    Ok(())
}
