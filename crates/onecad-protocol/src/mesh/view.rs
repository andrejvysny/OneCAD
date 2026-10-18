//! Borrowed accessors are independent of host alignment and endianness.
use super::checked::{section, validate};
use super::{f32_le, u32_le, MeshDecodeError, MeshHeaderView, MeshLimits};
use std::sync::Arc;

/// Immutable ownership of bytes validated once at the trust boundary.
#[derive(Debug, Clone)]
pub struct CheckedMesh {
    blob: Arc<[u8]>,
    header: Arc<MeshHeaderView>,
}

impl CheckedMesh {
    pub fn decode(blob: Arc<[u8]>, limits: MeshLimits) -> Result<Self, MeshDecodeError> {
        let header = Arc::new(validate(&blob, limits)?);
        Ok(Self { blob, header })
    }

    /// Constant-time borrowed view; shares the already checked section table.
    pub fn view(&self) -> MeshView<'_> {
        MeshView {
            blob: &self.blob,
            header: self.header.clone(),
        }
    }

    pub fn bytes(&self) -> &Arc<[u8]> {
        &self.blob
    }

    /// Recheck resource policy without rescanning validated geometry.
    pub fn ensure_limits(&self, limits: MeshLimits) -> Result<(), MeshDecodeError> {
        super::checked::preflight(&self.blob, limits)
    }

    pub fn metadata_bytes(&self) -> usize {
        std::mem::size_of::<MeshHeaderView>()
            + 2 * std::mem::size_of::<usize>()
            + self.header.sections.capacity() * std::mem::size_of::<super::MeshSectionEntry>()
    }
}

/// Fully checked MESH1 snapshot. Holds the original bytes without repacking.
///
/// Header and body are immutable. All indexed access returns `None` outside the
/// validated count; scalar loads support unaligned input and all host endiannesses.
#[derive(Debug)]
pub struct MeshView<'a> {
    blob: &'a [u8],
    header: Arc<MeshHeaderView>,
}

impl<'a> MeshView<'a> {
    pub fn decode(blob: &'a [u8], limits: MeshLimits) -> Result<Self, MeshDecodeError> {
        Ok(Self {
            blob,
            header: Arc::new(validate(blob, limits)?),
        })
    }

    pub fn header(&self) -> &MeshHeaderView {
        &self.header
    }
    pub fn bytes(&self) -> &'a [u8] {
        self.blob
    }
    pub fn positions(&self) -> MeshFloats<'a> {
        self.floats(1).expect("validated section")
    }
    pub fn normals(&self) -> Option<MeshFloats<'a>> {
        self.floats(2)
    }
    pub fn indices(&self) -> MeshWords<'a> {
        self.words(3).expect("validated section")
    }
    pub fn edge_positions(&self) -> Option<MeshFloats<'a>> {
        self.floats(8)
    }
    pub fn face_bboxes(&self) -> Option<MeshFloats<'a>> {
        self.floats(11)
    }
    pub fn face_colors(&self) -> Option<&'a [u8]> {
        self.data(12)
    }

    pub fn position(&self, vertex: usize) -> Option<[f32; 3]> {
        self.positions().triple(vertex)
    }
    pub fn normal(&self, vertex: usize) -> Option<[f32; 3]> {
        self.normals()?.triple(vertex)
    }
    pub fn triangle(&self, triangle: usize) -> Option<[u32; 3]> {
        self.indices().triple(triangle)
    }
    pub fn edge_point(&self, point: usize) -> Option<[f32; 3]> {
        self.edge_positions()?.triple(point)
    }

    /// Triangle units, not scalar index units.
    pub fn face_range(&self, face: usize) -> Option<MeshSpan> {
        self.span(4, face)
    }
    /// Polyline point units, not scalar coordinate units.
    pub fn edge_range(&self, edge: usize) -> Option<MeshSpan> {
        self.span(7, edge)
    }
    pub fn face_id(&self, face: usize) -> Option<&'a str> {
        self.id(5, 6, face)
    }
    pub fn edge_id(&self, edge: usize) -> Option<&'a str> {
        self.id(9, 10, edge)
    }

    pub fn edge_class(&self, edge: usize) -> super::EdgeClass {
        self.words(14)
            .and_then(|classes| classes.get(edge))
            .and_then(super::EdgeClass::from_u32)
            .unwrap_or(super::EdgeClass::Unknown)
    }

    /// Zero-based incident face ordinals; legacy meshes return an empty sequence.
    pub fn edge_incident_faces(&self, edge: usize) -> MeshWords<'a> {
        let Some(span) = self.span(15, edge) else {
            return MeshWords(&[]);
        };
        let start = span.first as usize * 4;
        let end = start + span.count as usize * 4;
        MeshWords(&self.data(16).expect("validated adjacency")[start..end])
    }

    pub fn face_bbox(&self, face: usize) -> Option<([f32; 3], [f32; 3])> {
        let start = face.checked_mul(2)?;
        let values = self.face_bboxes()?;
        Some((values.triple(start)?, values.triple(start.checked_add(1)?)?))
    }

    pub fn face_color(&self, face: usize) -> Option<[u8; 4]> {
        let start = face.checked_mul(4)?;
        self.face_colors()?
            .get(start..start.checked_add(4)?)?
            .try_into()
            .ok()
    }

    fn data(&self, ty: u32) -> Option<&'a [u8]> {
        section(self.blob, &self.header, ty)
    }
    fn floats(&self, ty: u32) -> Option<MeshFloats<'a>> {
        Some(MeshFloats(self.data(ty)?))
    }
    fn words(&self, ty: u32) -> Option<MeshWords<'a>> {
        Some(MeshWords(self.data(ty)?))
    }

    fn span(&self, ty: u32, index: usize) -> Option<MeshSpan> {
        let start = index.checked_mul(8)?;
        let pair = self.data(ty)?.get(start..start.checked_add(8)?)?;
        Some(MeshSpan {
            first: u32_le(pair, 0),
            count: u32_le(pair, 4),
        })
    }

    fn id(&self, offs: u32, chars: u32, index: usize) -> Option<&'a str> {
        let start = index.checked_mul(4)?;
        let pair = self.data(offs)?.get(start..start.checked_add(8)?)?;
        let bytes = self
            .data(chars)?
            .get(u32_le(pair, 0) as usize..u32_le(pair, 4) as usize)?;
        std::str::from_utf8(bytes).ok()
    }
}

/// Validated contiguous face-triangle or edge-point span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshSpan {
    pub first: u32,
    pub count: u32,
}

/// Borrowed little-endian f32 sequence. No alignment-dependent casts.
#[derive(Debug, Clone, Copy)]
pub struct MeshFloats<'a>(&'a [u8]);

impl<'a> MeshFloats<'a> {
    pub fn len(self) -> usize {
        self.0.len() / 4
    }
    pub fn is_empty(self) -> bool {
        self.0.is_empty()
    }
    pub fn get(self, index: usize) -> Option<f32> {
        let start = index.checked_mul(4)?;
        Some(f32_le(self.0.get(start..start.checked_add(4)?)?, 0))
    }
    pub fn triple(self, index: usize) -> Option<[f32; 3]> {
        let start = index.checked_mul(3)?;
        Some([
            self.get(start)?,
            self.get(start.checked_add(1)?)?,
            self.get(start.checked_add(2)?)?,
        ])
    }
    pub fn iter(self) -> impl ExactSizeIterator<Item = f32> + 'a {
        self.0.chunks_exact(4).map(|word| f32_le(word, 0))
    }
}

/// Borrowed little-endian u32 sequence. No alignment-dependent casts.
#[derive(Debug, Clone, Copy)]
pub struct MeshWords<'a>(&'a [u8]);

impl<'a> MeshWords<'a> {
    pub fn len(self) -> usize {
        self.0.len() / 4
    }
    pub fn is_empty(self) -> bool {
        self.0.is_empty()
    }
    pub fn get(self, index: usize) -> Option<u32> {
        let start = index.checked_mul(4)?;
        Some(u32_le(self.0.get(start..start.checked_add(4)?)?, 0))
    }
    pub fn triple(self, index: usize) -> Option<[u32; 3]> {
        let start = index.checked_mul(3)?;
        Some([
            self.get(start)?,
            self.get(start.checked_add(1)?)?,
            self.get(start.checked_add(2)?)?,
        ])
    }
    pub fn iter(self) -> impl ExactSizeIterator<Item = u32> + 'a {
        self.0.chunks_exact(4).map(|word| u32_le(word, 0))
    }
}
