//! MESH1 → 3MF adapter — the one place that parses mesh bytes for `export_3mf_file`.
//!
//! `onecad-core`'s [`threemf`](onecad_core::io::threemf) writer is deliberately
//! dumb (plain arrays, no MESH1 knowledge — see its module docs). This module
//! is the bridge: it pulls each body's fine-LOD MESH1 blob via [`MeshProvider`],
//! reads the sections the picking/colour idiom in
//! `src-tauri/tests/face_color_reopen.rs` already established, merges the two
//! colour sources per face, and hands the writer plain [`ThreeMfBody`] values.
//!
//! # Colour precedence (lowest first)
//!
//! 1. Mesh `FACE_COLORS` (import-derived; alpha `0` = unset, per
//!    `protocol/mesh_format.md` §4).
//! 2. Authored per-face colour (`BodyMeta.face_colors`), joined against the
//!    mesh's face id table:
//!    - id table carries `TopoKey`s (the common case): join through the
//!      already-resolved [`StepExportAttributes::face_colors`] (W2/W3's
//!      `ElementId → TopoKey` ladder — reused verbatim, no second resolution
//!      path).
//!    - id table carries `ElementId`s (`ids_have_element_ids()`): the
//!      TopoKey-keyed map does not apply (different key space), so this joins
//!      [`raw_face_colors_by_body`]'s `ElementId → color` map directly — no
//!      resolution needed at all, since the mesh already names faces by
//!      `ElementId`.
//!
//! Whole-body fallback (`BodyMeta.color`) becomes the object-level
//! `basematerials` default, so a triangle with no face colour at all still
//! picks up the body's own authored colour rather than rendering unstyled.
//!
//! An id that resolves to nothing in either lane is simply uncoloured — never
//! guessed (H5-B discipline, same as [`crate::export::resolve_face_colors`]).

use std::collections::{BTreeMap, HashMap};

use onecad_core::ids::{BodyId, SnapshotId};
use onecad_core::io::threemf::ThreeMfBody;
use onecad_core::regen::{EngineError, Lod};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

use crate::export::{PendingFaceColor, StepExportAttributes};
use crate::worker::{wire, MeshProvider};

const SEC_POSITIONS: u32 = 1;
const SEC_INDICES: u32 = 3;
const SEC_FACE_RANGES: u32 = 4;
const SEC_FACE_ID_OFFS: u32 = 5;
const SEC_FACE_ID_CHARS: u32 = 6;
const SEC_FACE_COLORS: u32 = 12;

/// Groups the raw (pre-resolution) authored face colours by body, keyed by the
/// `ElementId`'s own string form.
///
/// Captured from [`PendingFaceColor`] **before** [`crate::export::resolve_face_colors`]
/// consumes the list — that call turns each entry into a `TopoKey`-keyed map,
/// which is the right join key when the mesh id table carries `TopoKey`s but
/// the WRONG key space when it carries `ElementId`s (`ids_have_element_ids()`).
/// Cloning `pending` in the caller before resolution keeps this a read of data
/// already computed, not a second resolution path.
#[must_use]
pub fn raw_face_colors_by_body(
    pending: &[PendingFaceColor],
) -> HashMap<BodyId, BTreeMap<String, [u8; 4]>> {
    let mut out: HashMap<BodyId, BTreeMap<String, [u8; 4]>> = HashMap::new();
    for item in pending {
        out.entry(item.body)
            .or_default()
            .insert(item.element.as_str().to_string(), item.color);
    }
    out
}

/// Fetches every body's fine-LOD mesh and folds it into the writer's plain
/// input, in `bodies` order (the caller's order is the export's determinism —
/// see `onecad_core::io::threemf`'s module docs).
///
/// # Errors
/// [`EngineError`] on a disconnected worker, a worker-side tessellation
/// failure, or a MESH1 blob that fails header/section validation (the mesh
/// pipeline forwards blobs verbatim per Invariant 5, so a bad blob here means
/// the worker itself sent something wrong — surfaced, never guessed past).
pub async fn build_bodies(
    bodies: &[BodyId],
    meshes: &dyn MeshProvider,
    snapshot: Option<SnapshotId>,
    attributes: &StepExportAttributes,
    raw_face_colors: &HashMap<BodyId, BTreeMap<String, [u8; 4]>>,
) -> Result<Vec<ThreeMfBody>, EngineError> {
    let Some(snapshot) = snapshot else {
        return Err(EngineError::Protocol {
            message: "export3mf: no published snapshot to mesh bodies against".into(),
        });
    };
    let mut out = Vec::with_capacity(bodies.len());
    for &body in bodies {
        let blob = meshes.fetch_mesh(body, Lod::Fine, snapshot).await?;
        let wire_body_id = wire::body_id_wire(body);
        let name = attributes.body_names.get(&wire_body_id).cloned();
        let body_color = attributes.body_colors.get(&wire_body_id).copied();
        let authored_topo = attributes.face_colors.get(&wire_body_id);
        let authored_element = raw_face_colors.get(&body);
        out.push(mesh_to_threemf_body(
            &blob,
            name,
            body_color,
            authored_topo,
            authored_element,
        )?);
    }
    Ok(out)
}

fn missing_section(name: &str) -> EngineError {
    EngineError::Protocol {
        message: format!("export3mf: MESH1 blob missing required section {name}"),
    }
}

/// Parses one body's MESH1 blob into the writer's plain-array input, merging
/// the two colour lanes per module docs.
fn mesh_to_threemf_body(
    blob: &[u8],
    name: Option<String>,
    body_color: Option<[u8; 4]>,
    authored_topo: Option<&BTreeMap<String, [u8; 4]>>,
    authored_element: Option<&BTreeMap<String, [u8; 4]>>,
) -> Result<ThreeMfBody, EngineError> {
    let view = validate_mesh_blob(blob).map_err(|e| EngineError::Protocol {
        message: format!("export3mf: invalid MESH1 blob: {e}"),
    })?;
    let pos = view
        .section(SEC_POSITIONS)
        .ok_or_else(|| missing_section("POSITIONS"))?;
    let idx = view
        .section(SEC_INDICES)
        .ok_or_else(|| missing_section("INDICES"))?;
    let fr = view
        .section(SEC_FACE_RANGES)
        .ok_or_else(|| missing_section("FACE_RANGES"))?;

    let pbase = pos.offset as usize;
    let vertices: Vec<[f32; 3]> = (0..view.vertex_count as usize)
        .map(|i| {
            let o = pbase + i * 12;
            [f32_le(blob, o), f32_le(blob, o + 4), f32_le(blob, o + 8)]
        })
        .collect();

    let ibase = idx.offset as usize;
    let triangles: Vec<[u32; 3]> = (0..view.triangle_count as usize)
        .map(|t| {
            let o = ibase + t * 12;
            [u32_le(blob, o), u32_le(blob, o + 4), u32_le(blob, o + 8)]
        })
        .collect();

    let face_ids = read_id_table(&view, blob)?;
    let mesh_face_colors = read_face_colors_section(&view, blob);
    let uses_element_ids = view.ids_have_element_ids();

    let face_count = view.face_count as usize;
    let mut face_color: Vec<Option<[u8; 4]>> = vec![None; face_count];
    for f in 0..face_count {
        // Lane 1: import-derived, positional (alpha 0 = unset — mesh_format.md §4).
        let mut resolved = mesh_face_colors.as_ref().and_then(|colors| {
            let c = colors[f];
            (c[3] != 0).then_some(c)
        });
        // Lane 2: authored override, joined by the id table's own key space.
        let fid = &face_ids[f];
        let authored = if uses_element_ids {
            authored_element.and_then(|m| m.get(fid))
        } else {
            authored_topo.and_then(|m| m.get(fid))
        };
        if let Some(color) = authored {
            resolved = Some(*color);
        }
        face_color[f] = resolved;
    }

    let mut palette: Vec<[u8; 4]> = Vec::new();
    let mut index_of: HashMap<[u8; 4], u32> = HashMap::new();
    let default_color_index = body_color.map(|c| palette_index(c, &mut palette, &mut index_of));

    let mut triangle_colors: Vec<Option<u32>> = vec![None; triangles.len()];
    let frbase = fr.offset as usize;
    for (f, color) in face_color.iter().enumerate() {
        let Some(color) = *color else {
            continue;
        };
        let idx_val = palette_index(color, &mut palette, &mut index_of);
        let base = frbase + f * 8;
        let first = u32_le(blob, base) as usize;
        let count = u32_le(blob, base + 4) as usize;
        for slot in triangle_colors.iter_mut().skip(first).take(count) {
            *slot = Some(idx_val);
        }
    }

    Ok(ThreeMfBody {
        name,
        vertices,
        triangles,
        triangle_colors,
        palette,
        default_color_index,
    })
}

/// Dedupes `color` into `palette`, returning its (possibly newly-assigned) index.
fn palette_index(
    color: [u8; 4],
    palette: &mut Vec<[u8; 4]>,
    index_of: &mut HashMap<[u8; 4], u32>,
) -> u32 {
    if let Some(&i) = index_of.get(&color) {
        return i;
    }
    let i = palette.len() as u32;
    palette.push(color);
    index_of.insert(color, i);
    i
}

/// Reads FACE_ID_OFFS/FACE_ID_CHARS (always present — `mesh_format.md` §4 marks
/// both `required`), the offset+chars idiom `face_color_reopen.rs` already uses.
fn read_id_table(view: &MeshHeaderView, blob: &[u8]) -> Result<Vec<String>, EngineError> {
    let offs = view
        .section(SEC_FACE_ID_OFFS)
        .ok_or_else(|| missing_section("FACE_ID_OFFS"))?;
    let chars = view
        .section(SEC_FACE_ID_CHARS)
        .ok_or_else(|| missing_section("FACE_ID_CHARS"))?;
    let (obase, cbase) = (offs.offset as usize, chars.offset as usize);
    let n = view.face_count as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let lo = u32_le(blob, obase + i * 4) as usize;
        let hi = u32_le(blob, obase + (i + 1) * 4) as usize;
        out.push(String::from_utf8_lossy(&blob[cbase + lo..cbase + hi]).into_owned());
    }
    Ok(out)
}

/// Reads the optional FACE_COLORS section (type 12), one `[r,g,b,a]` per face
/// in face order — a straight positional array, no id-table lookup needed.
fn read_face_colors_section(view: &MeshHeaderView, blob: &[u8]) -> Option<Vec<[u8; 4]>> {
    let sec = view.section(SEC_FACE_COLORS)?;
    let base = sec.offset as usize;
    Some(
        (0..view.face_count as usize)
            .map(|f| {
                let o = base + f * 4;
                [blob[o], blob[o + 1], blob[o + 2], blob[o + 3]]
            })
            .collect(),
    )
}
