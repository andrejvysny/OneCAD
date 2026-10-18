//! MESH1 inspection (via the onecad-protocol validator) and fixture file output.

use std::path::Path;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use onecad_protocol::mesh::{u32_le, validate_mesh_blob, MeshHeaderView};

const SEC_POSITIONS: u32 = 1;
const SEC_NORMALS: u32 = 2;
const SEC_INDICES: u32 = 3;
const SEC_FACE_RANGES: u32 = 4;
const SEC_FACE_ID_OFFS: u32 = 5;
const SEC_EDGE_RANGES: u32 = 7;
const SEC_EDGE_POSITIONS: u32 = 8;
const SEC_FACE_COLORS: u32 = 12;

fn section_name(ty: u32) -> &'static str {
    match ty {
        1 => "POSITIONS",
        2 => "NORMALS",
        3 => "INDICES",
        4 => "FACE_RANGES",
        5 => "FACE_ID_OFFS",
        6 => "FACE_ID_CHARS",
        7 => "EDGE_RANGES",
        8 => "EDGE_POSITIONS",
        9 => "EDGE_ID_OFFS",
        10 => "EDGE_ID_CHARS",
        11 => "FACE_BBOXES",
        12 => "FACE_COLORS",
        _ => "UNKNOWN",
    }
}

/// Validates with `onecad_protocol::mesh::validate_mesh_blob`, then checks the
/// element-count/byteLen relations of mesh_format.md §4 that a renderer relies on.
pub fn validate(blob: &[u8]) -> Result<MeshHeaderView, String> {
    let view = validate_mesh_blob(blob).map_err(|e| format!("validate_mesh_blob: {e:?}"))?;
    let (v, t, f) = (view.vertex_count, view.triangle_count, view.face_count);
    let mut expect = vec![
        (SEC_POSITIONS, 12 * v),
        (SEC_INDICES, 12 * t),
        (SEC_FACE_RANGES, 8 * f),
        (SEC_FACE_ID_OFFS, 4 * (f + 1)),
    ];
    if view.has_normals() {
        expect.push((SEC_NORMALS, 12 * v));
    }
    if view.has_edges() {
        expect.push((SEC_EDGE_RANGES, 8 * view.edge_count));
        expect.push((SEC_EDGE_POSITIONS, 12 * view.edge_point_count));
    }
    for (ty, want) in expect {
        let sec = view
            .section(ty)
            .ok_or(format!("missing section {}", section_name(ty)))?;
        if sec.byte_len != want {
            return Err(format!(
                "{} byteLen {} != {want}",
                section_name(ty),
                sec.byte_len
            ));
        }
    }
    let idx = view.section(SEC_INDICES).expect("checked above");
    for i in 0..(3 * t as usize) {
        if u32_le(blob, idx.offset as usize + 4 * i) >= v {
            return Err(format!("index {i} out of range"));
        }
    }
    Ok(view)
}

/// Distinct authored per-face colors (alpha 0 = unset, skipped).
fn face_colors(view: &MeshHeaderView, blob: &[u8]) -> Vec<[u8; 4]> {
    let Some(sec) = view.section(SEC_FACE_COLORS) else {
        return Vec::new();
    };
    let mut out: Vec<[u8; 4]> = Vec::new();
    for f in 0..view.face_count as usize {
        let o = sec.offset as usize + 4 * f;
        let c = [blob[o], blob[o + 1], blob[o + 2], blob[o + 3]];
        if c[3] != 0 && !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

pub struct BodyMeta {
    pub body_id: String,
    pub wire_id: String,
    pub name: String,
    pub color: Option<[u8; 4]>,
}

/// Writes `<dir>/<bodyId>.mesh1` (bytes verbatim) and returns its manifest entry.
pub fn write_body(dir: &Path, meta: &BodyMeta, blob: &[u8]) -> Result<Value, String> {
    let view = validate(blob)?;
    let file = format!("{}.mesh1", meta.body_id);
    std::fs::write(dir.join(&file), blob).map_err(|e| format!("write {file}: {e}"))?;
    let sections: Vec<Value> = view
        .sections
        .iter()
        .map(|s| json!({"type": s.section_type, "name": section_name(s.section_type), "offset": s.offset, "byteLen": s.byte_len}))
        .collect();
    Ok(json!({
        "bodyId": meta.body_id,
        "wireId": meta.wire_id,
        "name": meta.name,
        "file": file,
        "byteLen": blob.len(),
        "sha256": format!("{:x}", Sha256::digest(blob)),
        "bodyColor": meta.color,
        "faceColors": face_colors(&view, blob),
        "mesh1Version": view.version,
        "flags": view.flags,
        "lod": view.lod,
        "vertexCount": view.vertex_count,
        "triangleCount": view.triangle_count,
        "faceCount": view.face_count,
        "edgeCount": view.edge_count,
        "edgePointCount": view.edge_point_count,
        "bbox": {"min": view.bbox_min, "max": view.bbox_max},
        "sections": sections,
    }))
}

/// Union of the per-body header bboxes.
pub fn scene_bbox(bodies: &[Value]) -> Value {
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for b in bodies {
        for k in 0..3 {
            lo[k] = lo[k].min(b["bbox"]["min"][k].as_f64().unwrap_or(f64::INFINITY));
            hi[k] = hi[k].max(b["bbox"]["max"][k].as_f64().unwrap_or(f64::NEG_INFINITY));
        }
    }
    json!({"min": lo, "max": hi})
}

/// Removes stale `*.mesh1` files (import body ids change on every regeneration).
pub fn reset_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {dir:?}: {e}"))?;
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|x| x == "mesh1") {
            std::fs::remove_file(&path).map_err(|e| format!("rm {path:?}: {e}"))?;
        }
    }
    Ok(())
}
