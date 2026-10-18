//! Minimal MESH1 reader (protocol/mesh_format.md): positions, normals, indices, edge polylines.
//! Sections are looked up by type, never by position; unknown types are skipped.

use std::path::{Path, PathBuf};

const MAGIC: u32 = 0x4D45_5348;
const POSITIONS: u32 = 1;
const NORMALS: u32 = 2;
const INDICES: u32 = 3;
const FACE_RANGES: u32 = 4;
const EDGE_RANGES: u32 = 7;
const EDGE_POSITIONS: u32 = 8;

pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub face_count: u32,
    /// Edge polylines, each a contiguous run in `edge_points`.
    pub edge_ranges: Vec<(u32, u32)>,
    pub edge_points: Vec<[f32; 3]>,
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
}

pub struct Body {
    pub name: String,
    pub file: String,
    pub mesh: Mesh,
}

fn u32_at(b: &[u8], off: usize) -> Result<u32, String> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| format!("read past end at {off}"))
}

fn f32s(b: &[u8], off: usize, len: usize) -> Result<Vec<f32>, String> {
    let s = b.get(off..off + len).ok_or("section out of bounds")?;
    Ok(s.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

fn u32s(b: &[u8], off: usize, len: usize) -> Result<Vec<u32>, String> {
    let s = b.get(off..off + len).ok_or("section out of bounds")?;
    Ok(s.chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

fn triples(v: Vec<f32>) -> Vec<[f32; 3]> {
    v.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect()
}

pub fn parse(b: &[u8]) -> Result<Mesh, String> {
    if u32_at(b, 0)? != MAGIC {
        return Err("bad MESH1 magic".into());
    }
    let face_count = u32_at(b, 0x10)?;
    let section_count = u16::from_le_bytes([b[0x1E], b[0x1F]]) as usize;
    let bbox = f32s(b, 0x20, 24)?;
    let mut sec = std::collections::HashMap::new();
    for i in 0..section_count {
        let e = 0x40 + 16 * i;
        sec.insert(
            u32_at(b, e)?,
            (u32_at(b, e + 8)? as usize, u32_at(b, e + 12)? as usize),
        );
    }
    let get = |t: u32| {
        sec.get(&t)
            .copied()
            .ok_or_else(|| format!("missing section {t}"))
    };
    let (po, pl) = get(POSITIONS)?;
    let (no, nl) = get(NORMALS)?;
    let (io, il) = get(INDICES)?;
    let _ = get(FACE_RANGES)?;
    let (eo, el) = sec.get(&EDGE_RANGES).copied().unwrap_or((0, 0));
    let (pp, ppl) = sec.get(&EDGE_POSITIONS).copied().unwrap_or((0, 0));
    let er = u32s(b, eo, el)?;
    Ok(Mesh {
        positions: triples(f32s(b, po, pl)?),
        normals: triples(f32s(b, no, nl)?),
        indices: u32s(b, io, il)?,
        face_count,
        edge_ranges: er.chunks_exact(2).map(|c| (c[0], c[1])).collect(),
        edge_points: triples(f32s(b, pp, ppl)?),
        bbox_min: [bbox[0], bbox[1], bbox[2]],
        bbox_max: [bbox[3], bbox[4], bbox[5]],
    })
}

/// Reads every body listed in `<dir>/manifest.json`, in manifest order.
/// The manifest's keys are sorted, so each body's `"file"` is followed by its `"name"`; a tiny
/// scan for that pair avoids a JSON dependency.
pub fn load_fixture(dir: &Path) -> Result<Vec<Body>, String> {
    let manifest = std::fs::read_to_string(dir.join("manifest.json"))
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut out = Vec::new();
    let mut rest = manifest.as_str();
    while let Some((file, after)) = string_value(rest, "\"file\"") {
        let name = string_value(after, "\"name\"")
            .map(|(n, _)| n)
            .unwrap_or_default();
        rest = after;
        let bytes = std::fs::read(dir.join(&file)).map_err(|e| format!("{file}: {e}"))?;
        let mesh = parse(&bytes).map_err(|e| format!("{file}: {e}"))?;
        out.push(Body { name, file, mesh });
    }
    Ok(out)
}

/// The string value of the next `key` in `json`, and the text after it.
fn string_value<'a>(json: &'a str, key: &str) -> Option<(String, &'a str)> {
    let k = json.find(key)?;
    let rest = &json[k + key.len()..];
    let colon = rest.find(':')?;
    let s = rest[colon + 1..].trim_start().strip_prefix('"')?;
    let end = s.find('"')?;
    Some((s[..end].to_string(), &s[end + 1..]))
}

/// Fixture directory: `ONECAD_FIXTURE_DIR`, else the repo copy next to this crate.
pub fn fixture_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("ONECAD_FIXTURE_DIR") {
        return PathBuf::from(d);
    }
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/nema17"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fillet_block_counts() {
        let dir = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../fixtures/fillet-block"
        ));
        let bodies = load_fixture(&dir).unwrap();
        assert_eq!(bodies.len(), 1);
        let m = &bodies[0].mesh;
        assert_eq!(m.positions.len(), 170);
        assert_eq!(m.indices.len(), 156 * 3);
        assert_eq!(m.face_count, 7);
        assert_eq!(m.edge_ranges.len(), 15);
        assert_eq!(m.edge_points.len(), 92);
    }
}
