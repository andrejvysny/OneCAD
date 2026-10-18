//! MESH1 reader (protocol/mesh_format.md). Only the sections this spike draws are decoded:
//! POSITIONS, NORMALS, INDICES, FACE_RANGES, EDGE_RANGES and EDGE_POSITIONS. Sections are
//! looked up by type, never by position; unknown types are skipped.

use std::path::Path;

const MAGIC: u32 = 0x4D45_5348;
const HEADER_LEN: usize = 0x40;
const SEC_POSITIONS: u32 = 1;
const SEC_NORMALS: u32 = 2;
const SEC_INDICES: u32 = 3;
const SEC_FACE_RANGES: u32 = 4;
const SEC_EDGE_RANGES: u32 = 7;
const SEC_EDGE_POSITIONS: u32 = 8;

pub struct Mesh1 {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<[u32; 3]>,
    /// (first triangle, triangle count) per face.
    pub face_ranges: Vec<(u32, u32)>,
    /// One polyline per topological edge.
    pub edges: Vec<Vec<[f32; 3]>>,
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
}

fn u32_at(b: &[u8], off: usize) -> Result<u32, String> {
    b.get(off..off + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| format!("read past end at {off}"))
}

fn f32_at(b: &[u8], off: usize) -> Result<f32, String> {
    u32_at(b, off).map(f32::from_bits)
}

fn section(b: &[u8], count: usize, ty: u32) -> Result<Option<&[u8]>, String> {
    for i in 0..count {
        let e = HEADER_LEN + 16 * i;
        if u32_at(b, e)? == ty {
            let off = u32_at(b, e + 8)? as usize;
            let len = u32_at(b, e + 12)? as usize;
            return b.get(off..off + len).map(Some).ok_or_else(|| format!("section {ty} out of bounds"));
        }
    }
    Ok(None)
}

fn vec3s(s: &[u8]) -> Vec<[f32; 3]> {
    s.chunks_exact(12)
        .map(|c| {
            let f = |k: usize| f32::from_le_bytes([c[k], c[k + 1], c[k + 2], c[k + 3]]);
            [f(0), f(4), f(8)]
        })
        .collect()
}

fn u32s(s: &[u8]) -> Vec<u32> {
    s.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

impl Mesh1 {
    pub fn load(path: &Path) -> Result<Mesh1, String> {
        let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&b).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn parse(b: &[u8]) -> Result<Mesh1, String> {
        if u32_at(b, 0)? != MAGIC {
            return Err("bad magic".into());
        }
        let v = u32_at(b, 0x08)? as usize;
        let t = u32_at(b, 0x0C)? as usize;
        let sections = (u32_at(b, 0x1C)? >> 16) as usize;
        let mut bbox = [0f32; 6];
        for (k, slot) in bbox.iter_mut().enumerate() {
            *slot = f32_at(b, 0x20 + 4 * k)?;
        }
        let need = |ty: u32| section(b, sections, ty)?.ok_or_else(|| format!("missing section {ty}"));
        let positions = vec3s(need(SEC_POSITIONS)?);
        let normals = vec3s(need(SEC_NORMALS)?);
        let flat = u32s(need(SEC_INDICES)?);
        let indices: Vec<[u32; 3]> = flat.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
        if positions.len() != v || normals.len() != v || indices.len() != t {
            return Err(format!("count mismatch V={v} T={t}"));
        }
        if indices.iter().flatten().any(|&i| i as usize >= v) {
            return Err("index out of range".into());
        }
        let fr = u32s(need(SEC_FACE_RANGES)?);
        let face_ranges = fr.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        let mut edges = Vec::new();
        if let (Some(er), Some(ep)) = (section(b, sections, SEC_EDGE_RANGES)?, section(b, sections, SEC_EDGE_POSITIONS)?) {
            let pts = vec3s(ep);
            for c in u32s(er).chunks_exact(2) {
                let (first, n) = (c[0] as usize, c[1] as usize);
                if let Some(run) = pts.get(first..first + n) {
                    edges.push(run.to_vec());
                }
            }
        }
        Ok(Mesh1 {
            positions,
            normals,
            indices,
            face_ranges,
            edges,
            bbox_min: [bbox[0], bbox[1], bbox[2]],
            bbox_max: [bbox[3], bbox[4], bbox[5]],
        })
    }
}
