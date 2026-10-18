//! Minimal MESH1 reader (protocol/mesh_format.md): header, section table, typed views.
//! Only what the spike draws: positions, normals, indices, edge polylines.

const MAGIC: u32 = 0x4D45_5348;
const HEADER_LEN: usize = 64;
const POSITIONS: u32 = 1;
const NORMALS: u32 = 2;
const INDICES: u32 = 3;
const EDGE_RANGES: u32 = 7;
const EDGE_POSITIONS: u32 = 8;

pub struct Mesh1 {
    pub vertex_count: u32,
    pub triangle_count: u32,
    pub edge_count: u32,
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
    /// Raw section bytes, uploaded to the GPU verbatim (Z-up, no axis swap).
    pub positions: Vec<u8>,
    pub normals: Vec<u8>,
    pub indices: Vec<u8>,
    /// Edge polylines expanded to independent segments: [ax, ay, az, bx, by, bz] per segment.
    pub edge_segments: Vec<[f32; 6]>,
}

fn u32_at(b: &[u8], off: usize) -> Result<u32, String> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| format!("read past end at {off}"))
}

fn f32_at(b: &[u8], off: usize) -> Result<f32, String> {
    u32_at(b, off).map(f32::from_bits)
}

fn section(b: &[u8], kind: u32) -> Result<Option<&[u8]>, String> {
    let count = u32_at(b, 0x1C)? >> 16;
    for i in 0..count as usize {
        let e = HEADER_LEN + 16 * i;
        if u32_at(b, e)? == kind {
            let off = u32_at(b, e + 8)? as usize;
            let len = u32_at(b, e + 12)? as usize;
            return b
                .get(off..off + len)
                .map(Some)
                .ok_or_else(|| format!("section {kind} out of bounds"));
        }
    }
    Ok(None)
}

pub fn parse(b: &[u8]) -> Result<Mesh1, String> {
    if b.len() < HEADER_LEN || u32_at(b, 0)? != MAGIC {
        return Err("not a MESH1 blob".into());
    }
    let vertex_count = u32_at(b, 0x08)?;
    let triangle_count = u32_at(b, 0x0C)?;
    let edge_count = u32_at(b, 0x14)?;
    let mut bbox = [0f32; 6];
    for (i, v) in bbox.iter_mut().enumerate() {
        *v = f32_at(b, 0x20 + 4 * i)?;
    }
    let need = |kind: u32, len: usize| -> Result<Vec<u8>, String> {
        let s = section(b, kind)?.ok_or_else(|| format!("missing section {kind}"))?;
        if s.len() != len {
            return Err(format!("section {kind}: {} bytes, expected {len}", s.len()));
        }
        Ok(s.to_vec())
    };
    let positions = need(POSITIONS, 12 * vertex_count as usize)?;
    let normals = need(NORMALS, 12 * vertex_count as usize)?;
    let indices = need(INDICES, 12 * triangle_count as usize)?;
    let mut edge_segments = Vec::new();
    if let (Some(ranges), Some(points)) = (section(b, EDGE_RANGES)?, section(b, EDGE_POSITIONS)?) {
        let point = |i: usize| -> Result<[f32; 3], String> {
            Ok([
                f32_at(points, 12 * i)?,
                f32_at(points, 12 * i + 4)?,
                f32_at(points, 12 * i + 8)?,
            ])
        };
        for e in 0..edge_count as usize {
            let first = u32_at(ranges, 8 * e)? as usize;
            let n = u32_at(ranges, 8 * e + 4)? as usize;
            for k in 1..n {
                let (a, c) = (point(first + k - 1)?, point(first + k)?);
                edge_segments.push([a[0], a[1], a[2], c[0], c[1], c[2]]);
            }
        }
    }
    Ok(Mesh1 {
        vertex_count,
        triangle_count,
        edge_count,
        bbox_min: [bbox[0], bbox[1], bbox[2]],
        bbox_max: [bbox[3], bbox[4], bbox[5]],
        positions,
        normals,
        indices,
        edge_segments,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn fillet_block_fixture_counts() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../fixtures/fillet-block/00000000-0000-0000-0000-00000000f101.mesh1"
        );
        let bytes = std::fs::read(path).expect("fixture");
        let m = super::parse(&bytes).expect("parse");
        assert_eq!(
            (m.vertex_count, m.triangle_count, m.edge_count),
            (170, 156, 15)
        );
        // 92 points over 15 polylines -> 92 - 15 segments.
        assert_eq!(m.edge_segments.len(), 92 - 15);
        let want = [-30.0, 0.0, 0.0];
        assert!(
            m.bbox_min
                .iter()
                .zip(want)
                .all(|(a, b)| (a - b).abs() < 1e-5),
            "{:?}",
            m.bbox_min
        );
    }
}
