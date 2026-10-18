//! 3MF (3D Manufacturing Format) writer — a pure, dumb ZIP+XML encoder (W4).
//!
//! Core spec (2015/02) only, no extensions. This module never parses MESH1 or
//! knows about `ElementId`/`TopoKey`/`BodyId`: the app crate resolves everything
//! down to plain arrays before calling [`write_3mf`], so `onecad-core` (which the
//! protocol crate and worker never depend on) stays wire-format-agnostic — the
//! same reason [`super::container`] never parses a mesh blob either.
//!
//! # Container shape
//!
//! Three parts, exactly:
//!
//! * `[Content_Types].xml` — declares the `rels` and `model` part types.
//! * `_rels/.rels` — one relationship pointing at the model part.
//! * `3D/3dmodel.model` — `<model>` → `<resources>` (one `<basematerials>` per
//!   coloured body, one `<object>` per body) → `<build>` (one `<item>` per
//!   object).
//!
//! # Colour encoding
//!
//! A body with any colour gets **one** `<basematerials>` resource holding its
//! deduplicated palette. A coloured triangle carries its own `pid`+`p1`
//! attributes rather than relying on the object's default `pid`/`pindex` to be
//! inherited — always self-contained, so a triangle's colour is correct whether
//! or not the object declares a default (H5-B discipline: never lean on an
//! inference that could silently mis-colour a face). The object-level
//! `pid`/`pindex` pair is written only when [`ThreeMfBody::default_color_index`]
//! is `Some` (a genuine authored whole-body colour) — a body with only some
//! faces coloured gets NO object default, so an uncoloured face never inherits
//! an arbitrary palette entry.
//!
//! # Determinism
//!
//! No timestamps: zip entries carry the fixed epoch `zip::DateTime::default()`
//! (same convention as [`super::container`]'s `.onecad` writer). Given the same
//! input, [`write_3mf`] produces byte-identical output every call.

use std::fmt::Write as _;
use std::io::{Cursor, Write as _};

use super::{IoError, IoResult};

/// One body to write as a 3MF `<object>` (+ optional `<basematerials>`).
///
/// Deliberately dumb: plain arrays only, resolved by the caller (MESH1 parsing
/// + colour-precedence merge live in the app crate's `export_threemf` module).
#[derive(Debug, Clone, PartialEq)]
pub struct ThreeMfBody {
    /// Tree display name, written as the object's `name` attribute. `None`
    /// omits the attribute (3MF does not require one).
    pub name: Option<String>,
    /// Vertex positions, written VERBATIM — 3MF's convention is Z-up
    /// millimetres, the same as the MESH1 wire invariant. No axis swap, ever.
    pub vertices: Vec<[f32; 3]>,
    /// Triangle vertex indices (0-based, into `vertices`).
    pub triangles: Vec<[u32; 3]>,
    /// Per-triangle colour, parallel to `triangles`: an index into `palette`,
    /// or `None` for "no authored/imported colour on this triangle" (renders
    /// at the consumer's own default appearance — never guessed).
    pub triangle_colors: Vec<Option<u32>>,
    /// Deduplicated sRGBA colours referenced by `triangle_colors` and
    /// `default_color_index`. Empty ⇒ the body has no colour at all and no
    /// `<basematerials>` resource is written for it.
    pub palette: Vec<[u8; 4]>,
    /// The object-level fallback colour (an index into `palette`), used only
    /// when there is a genuine authored whole-body colour. `None` leaves
    /// triangles without their own `triangle_colors` entry uncoloured, rather
    /// than inheriting an arbitrary palette entry.
    pub default_color_index: Option<u32>,
}

/// Writes `bodies` as a 3MF core-spec (2015/02) package. See the module docs
/// for the exact container shape.
///
/// # Errors
/// [`IoError::Corrupt`] if a body's arrays are internally inconsistent (a
/// `triangle_colors` length mismatch, an out-of-range palette index, or a
/// coloured triangle on a body with an empty palette) — defensive: this is an
/// app-crate-internal contract, but a malformed call must never silently emit
/// a 3MF file with mismatched geometry.
pub fn write_3mf(bodies: &[ThreeMfBody]) -> IoResult<Vec<u8>> {
    let model_xml = build_model_xml(bodies)?;

    let cursor = Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(cursor);
    write_entry(
        &mut zip,
        "[Content_Types].xml",
        CONTENT_TYPES_XML.as_bytes(),
    )?;
    write_entry(&mut zip, "_rels/.rels", RELS_XML.as_bytes())?;
    write_entry(&mut zip, "3D/3dmodel.model", model_xml.as_bytes())?;
    let cursor = zip
        .finish()
        .map_err(|e| IoError::Io(format!("3mf zip finish: {e}")))?;
    Ok(cursor.into_inner())
}

const CONTENT_TYPES_XML: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
    "<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>",
    "<Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>",
    "</Types>",
);

const RELS_XML: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    "<Relationship Id=\"rel0\" ",
    "Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\" ",
    "Target=\"/3D/3dmodel.model\"/>",
    "</Relationships>",
);

/// One body's assigned resource ids (shared xml `id` namespace: 3MF requires
/// every `<basematerials>`/`<object>` id to be unique across the WHOLE model,
/// not just within its own resource type).
struct AssignedIds {
    basematerials_id: Option<u32>,
    object_id: u32,
}

fn build_model_xml(bodies: &[ThreeMfBody]) -> IoResult<String> {
    for body in bodies {
        validate_body(body)?;
    }

    // Ids are handed out in one deterministic pass, all `basematerials` before
    // any `object` — 3MF resources must be defined before anything references
    // them, and this keeps that true regardless of iteration order below.
    let mut next_id: u32 = 1;
    let assigned: Vec<AssignedIds> = bodies
        .iter()
        .map(|body| {
            let basematerials_id = if body.palette.is_empty() {
                None
            } else {
                let id = next_id;
                next_id += 1;
                Some(id)
            };
            AssignedIds {
                basematerials_id,
                object_id: 0, // filled in the second pass, once every basematerials id exists
            }
        })
        .collect();
    let assigned: Vec<AssignedIds> = assigned
        .into_iter()
        .map(|a| AssignedIds {
            basematerials_id: a.basematerials_id,
            object_id: {
                let id = next_id;
                next_id += 1;
                id
            },
        })
        .collect();

    let mut resources = String::new();
    for (body, ids) in bodies.iter().zip(&assigned) {
        let Some(bm_id) = ids.basematerials_id else {
            continue;
        };
        let _ = write!(resources, "<basematerials id=\"{bm_id}\">");
        for (i, color) in body.palette.iter().enumerate() {
            let name = xml_escape(&format!("Color {i}"));
            let hex = color_hex(*color);
            let _ = write!(resources, "<base name=\"{name}\" displaycolor=\"{hex}\"/>");
        }
        resources.push_str("</basematerials>");
    }

    for (body, ids) in bodies.iter().zip(&assigned) {
        write_object(&mut resources, body, ids);
    }

    let mut build = String::new();
    for ids in &assigned {
        let _ = write!(build, "<item objectid=\"{}\"/>", ids.object_id);
    }

    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <model unit=\"millimeter\" xml:lang=\"en-US\" \
         xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\
         <resources>{resources}</resources><build>{build}</build></model>"
    ))
}

fn write_object(out: &mut String, body: &ThreeMfBody, ids: &AssignedIds) {
    let _ = write!(out, "<object id=\"{}\" type=\"model\"", ids.object_id);
    if let Some(name) = &body.name {
        let _ = write!(out, " name=\"{}\"", xml_escape(name));
    }
    if let (Some(bm_id), Some(default_idx)) = (ids.basematerials_id, body.default_color_index) {
        let _ = write!(out, " pid=\"{bm_id}\" pindex=\"{default_idx}\"");
    }
    out.push_str("><mesh><vertices>");
    for v in &body.vertices {
        let _ = write!(
            out,
            "<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>",
            v[0], v[1], v[2]
        );
    }
    out.push_str("</vertices><triangles>");
    for (tri, color) in body.triangles.iter().zip(&body.triangle_colors) {
        let _ = write!(
            out,
            "<triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"",
            tri[0], tri[1], tri[2]
        );
        // Every coloured triangle carries its OWN pid+p1 — see module docs on
        // why this never relies on inheriting the object's default.
        if let Some(idx) = color {
            let bm_id = ids
                .basematerials_id
                .expect("validate_body guarantees a palette when any triangle is coloured");
            let _ = write!(out, " pid=\"{bm_id}\" p1=\"{idx}\"");
        }
        out.push_str("/>");
    }
    out.push_str("</triangles></mesh></object>");
}

fn validate_body(body: &ThreeMfBody) -> IoResult<()> {
    if body.triangles.len() != body.triangle_colors.len() {
        return Err(IoError::Corrupt(format!(
            "3mf: {} triangles but {} triangle_colors entries",
            body.triangles.len(),
            body.triangle_colors.len()
        )));
    }
    let vcount = body.vertices.len() as u32;
    for tri in &body.triangles {
        for &idx in tri {
            if idx >= vcount {
                return Err(IoError::Corrupt(format!(
                    "3mf: triangle vertex index {idx} out of range ({vcount} vertices)"
                )));
            }
        }
    }
    let pcount = body.palette.len() as u32;
    for color in body.triangle_colors.iter().flatten() {
        if *color >= pcount {
            return Err(IoError::Corrupt(format!(
                "3mf: triangle color index {color} out of range (palette len {pcount})"
            )));
        }
    }
    if let Some(idx) = body.default_color_index {
        if idx >= pcount {
            return Err(IoError::Corrupt(format!(
                "3mf: default_color_index {idx} out of range (palette len {pcount})"
            )));
        }
    }
    Ok(())
}

fn color_hex(c: [u8; 4]) -> String {
    format!("#{:02X}{:02X}{:02X}{:02X}", c[0], c[1], c[2], c[3])
}

/// Escapes the five XML predefined entities. 3MF names are otherwise free text
/// (user-authored body names), so this is the only thing standing between an
/// authored `"`/`&`/`<` and a malformed model part.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Writes one zip entry, deflated, with the fixed epoch timestamp (determinism
/// — same convention as [`super::container::write_zip_entry`]).
fn write_entry<W: std::io::Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    name: &str,
    bytes: &[u8],
) -> IoResult<()> {
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    zip.start_file(name, options)
        .map_err(|e| IoError::Io(format!("3mf zip start_file {name}: {e}")))?;
    zip.write_all(bytes)
        .map_err(|e| IoError::Io(format!("3mf zip write {name}: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn cube_body(name: Option<&str>) -> ThreeMfBody {
        ThreeMfBody {
            name: name.map(str::to_string),
            vertices: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            triangle_colors: vec![None, None],
            palette: Vec::new(),
            default_color_index: None,
        }
    }

    fn unzip(bytes: &[u8]) -> zip::ZipArchive<Cursor<&[u8]>> {
        zip::ZipArchive::new(Cursor::new(bytes)).expect("valid zip")
    }

    fn read_entry(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> String {
        let mut f = archive
            .by_name(name)
            .unwrap_or_else(|_| panic!("{name} present"));
        let mut s = String::new();
        f.read_to_string(&mut s).expect("valid utf8");
        s
    }

    #[test]
    fn writes_the_three_required_parts() {
        let bytes = write_3mf(&[cube_body(Some("Cube"))]).expect("writes");
        let mut archive = unzip(&bytes);
        assert!(archive.by_name("[Content_Types].xml").is_ok());
        assert!(archive.by_name("_rels/.rels").is_ok());
        assert!(archive.by_name("3D/3dmodel.model").is_ok());
    }

    #[test]
    fn content_types_and_rels_are_exact() {
        let bytes = write_3mf(&[cube_body(None)]).expect("writes");
        let mut archive = unzip(&bytes);
        let ct = read_entry(&mut archive, "[Content_Types].xml");
        assert!(ct.contains(
            r#"Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml""#
        ));
        assert!(ct.contains(
            r#"Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml""#
        ));
        let rels = read_entry(&mut archive, "_rels/.rels");
        assert!(rels.contains(r#"Id="rel0""#));
        assert!(rels.contains("http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"));
        assert!(rels.contains(r#"Target="/3D/3dmodel.model""#));
    }

    #[test]
    fn model_xml_has_the_right_namespace_and_unit() {
        let bytes = write_3mf(&[cube_body(Some("Cube"))]).expect("writes");
        let mut archive = unzip(&bytes);
        let model = read_entry(&mut archive, "3D/3dmodel.model");
        assert!(model.contains(r#"<model unit="millimeter" xml:lang="en-US""#));
        assert!(
            model.contains(r#"xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02""#)
        );
        assert!(model.contains(r#"name="Cube""#));
    }

    #[test]
    fn vertices_round_trip_verbatim() {
        let body = cube_body(None);
        let bytes = write_3mf(std::slice::from_ref(&body)).expect("writes");
        let mut archive = unzip(&bytes);
        let model = read_entry(&mut archive, "3D/3dmodel.model");
        for v in &body.vertices {
            let needle = format!(r#"<vertex x="{}" y="{}" z="{}"/>"#, v[0], v[1], v[2]);
            assert!(model.contains(&needle), "missing {needle} in {model}");
        }
    }

    #[test]
    fn colored_face_carries_the_right_p1_and_displaycolor() {
        let mut body = cube_body(Some("Coin"));
        // Whole-body colour (gold) at palette index 0, one face overridden red.
        body.palette = vec![[0xFF, 0xD7, 0x00, 0xFF], [0xFF, 0x00, 0x00, 0x80]];
        body.default_color_index = Some(0);
        body.triangle_colors = vec![None, Some(1)];

        let bytes = write_3mf(&[body]).expect("writes");
        let mut archive = unzip(&bytes);
        let model = read_entry(&mut archive, "3D/3dmodel.model");

        assert!(model.contains(r##"<base name="Color 0" displaycolor="#FFD700FF"/>"##));
        assert!(model.contains(r##"<base name="Color 1" displaycolor="#FF000080"/>"##));
        assert!(model.contains(r#"pid="1" pindex="0""#), "{model}");
        assert!(model.contains(r#"p1="1""#), "{model}");
        // The uncoloured triangle carries no p1 of its own (falls back to the
        // object default rather than an unrelated explicit index).
        assert!(model.contains(r#"<triangle v1="0" v2="1" v3="2"/>"#));
    }

    #[test]
    fn body_with_only_partial_face_colors_gets_no_object_default() {
        // No whole-body colour authored — only one face is coloured. The object
        // must NOT declare a pid/pindex default, or the uncoloured face would
        // silently inherit an arbitrary palette entry.
        let mut body = cube_body(Some("Partial"));
        body.palette = vec![[0, 255, 0, 255]];
        body.triangle_colors = vec![Some(0), None];
        body.default_color_index = None;

        let bytes = write_3mf(&[body]).expect("writes");
        let mut archive = unzip(&bytes);
        let model = read_entry(&mut archive, "3D/3dmodel.model");
        assert!(!model.contains("pindex"));
        assert!(model.contains(r#"pid="1" p1="0""#));
    }

    #[test]
    fn names_are_xml_escaped() {
        let body = cube_body(Some(r#"A & B <"weird">"#));
        let bytes = write_3mf(&[body]).expect("writes");
        let mut archive = unzip(&bytes);
        let model = read_entry(&mut archive, "3D/3dmodel.model");
        assert!(model.contains("A &amp; B &lt;&quot;weird&quot;&gt;"));
        assert!(!model.contains("A & B <"));
    }

    #[test]
    fn multiple_bodies_get_a_unique_id_per_resource_and_one_build_item_each() {
        let a = cube_body(Some("A"));
        let mut b = cube_body(Some("B"));
        b.palette = vec![[10, 20, 30, 255]];
        b.triangle_colors = vec![Some(0), Some(0)];
        let bodies = [a, b];

        let bytes = write_3mf(&bodies).expect("writes");
        let mut archive = unzip(&bytes);
        let model = read_entry(&mut archive, "3D/3dmodel.model");
        // Basematerials ids are assigned first, in body order, only for bodies
        // that have a palette: B is the only one, so it gets id 1. Object ids
        // are then assigned in body order for every body: A -> 2, B -> 3.
        assert!(model.contains(r#"<basematerials id="1">"#));
        assert!(model.contains(r#"<object id="2" type="model" name="A">"#));
        assert!(model.contains(r#"<object id="3" type="model" name="B""#));
        assert!(model.contains(r#"<item objectid="2"/>"#));
        assert!(model.contains(r#"<item objectid="3"/>"#));
    }

    #[test]
    fn deterministic_across_two_runs() {
        let bodies = [cube_body(Some("Cube"))];
        let a = write_3mf(&bodies).expect("writes");
        let b = write_3mf(&bodies).expect("writes");
        assert_eq!(a, b, "identical input must yield byte-identical output");
    }

    #[test]
    fn mismatched_triangle_colors_length_is_rejected() {
        let mut body = cube_body(None);
        body.triangle_colors = vec![None]; // one short
        let err = write_3mf(&[body]).unwrap_err();
        assert!(matches!(err, IoError::Corrupt(_)));
    }

    #[test]
    fn out_of_range_vertex_index_is_rejected() {
        let mut body = cube_body(None);
        body.triangles[0][0] = 99;
        let err = write_3mf(&[body]).unwrap_err();
        assert!(matches!(err, IoError::Corrupt(_)));
    }

    #[test]
    fn out_of_range_palette_index_is_rejected() {
        let mut body = cube_body(None);
        body.triangle_colors[0] = Some(0); // palette is empty
        let err = write_3mf(&[body]).unwrap_err();
        assert!(matches!(err, IoError::Corrupt(_)));
    }
}
