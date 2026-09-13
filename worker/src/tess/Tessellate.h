// Tessellate.h — BRepMesh triangulation → MESH1 blob for one body (W-WP5).
//
// SCHEMA §7.6 Tessellate. Faces/edges are labelled with snapshot-scoped TopoKeys
// ("f:N"/"e:N", the MapShapes ordinal — consistent with the ElementMap partition)
// and, where the partition already holds a minted ElementId for that TopoKey, the
// persistent ElementId (IDS_HAVE_ELEMENTIDS). Meshing parallelism never affects the
// ids or the ordinal (Invariant 5).
//
// LOD tiers: coarse/medium/fine. Deflection is both bbox-relative and bounded by
// tier-specific absolute limits. Fine is committed display/export quality (5 degree
// angular cap); coarse remains suitable for transient interaction. Planar
// prisms/booleans tessellate identically across tiers (2 triangles per rectangular
// face), so the W-WP5 corpus meshes are byte-stable.
#pragma once

#include <cstdint>
#include <string>
#include <vector>

#include <TopoDS_Shape.hxx>

#include "elementmap/ElementMapPartition.h"
#include "tess/EdgeClassification.h"

namespace onecad::tess {

// Producer-side completeness evidence for one body (VP-HARDENING WP09, spec §14,
// NUM §9.2). NOT on the wire yet: WP10 publishes it as MESH1 v2 QUALITY_INFO
// bit 0 (complete nondegenerate face coverage) and bit 3 (some normal fallback).
//
// `ok` on BodyMesh stays true when a face is missing — the body still publishes,
// because refusing to draw the rest of a solid helps nobody. What must never
// happen is calling that output COMPLETE, so the flag is explicit and the warning
// names the face.
struct TessellationCompleteness {
    bool allNondegenerateFacesCovered = true;
    std::vector<std::string> missingFaces;     // TopoKeys ("f:7") with no triangles
    std::vector<std::string> degenerateFaces;  // TopoKeys legitimately holding a zero range
    std::uint32_t normalFallbackNodes = 0;     // NUM §7.3 triangulation fallback
    std::uint32_t singularSplitNodes = 0;      // NUM §7.3 singular split
    std::uint32_t normalMissingNodes = 0;      // no surface AND no usable incident triangle
};

struct BodyMesh {
    std::string body_id;
    std::vector<std::uint8_t> blob;       // MESH1 bytes
    std::uint32_t triangle_count = 0;
    bool ok = false;                      // false if the body produced no triangulation
    TessellationCompleteness completeness;
    // Per-edge display semantics in `TopExp::MapShapes(shape, TopAbs_EDGE)` order,
    // empty when `include_edges` is false. WP10 publishes it (MESH1 v2 §16/§20/§21).
    std::vector<EdgeClass> edge_classes;
    // Per-face local solid-partition ordinal in face-map order (WP10 §19, WP13 caps).
    std::vector<std::uint32_t> face_solid_ordinals;
};

// Raw triangle geometry for one body — the SAME positions/normals/indices
// tessellate_body assembles for the MESH1 blob, minus id labelling + edges. Reused
// by the STL/OBJ mesh exporters (io/MeshExport) so an exported mesh is byte-for-byte
// the geometry the viewport meshes (identical BRepMesh params + winding ⇒ the STL
// triangle count equals the tessellation triangle count; determinism, Invariant 5).
struct RawMesh {
    std::vector<float> positions;         // 3·V (xyz per vertex)
    std::vector<float> normals;           // 3·V (per-vertex, area-weighted)
    std::vector<std::uint32_t> indices;   // 3·T (triangle vertex indices)
    std::uint32_t triangle_count = 0;
};

// Tessellate one body into a MESH1 blob. `lod` ∈ "coarse"|"medium"|"fine".
// `partition` (optional) supplies minted ElementIds by TopoKey for id labelling.
//
// `face_colors` (optional, WP-A W4.5) is the body's authored appearance in
// `session::BodyRecord::face_colors` form — packed sRGB RGBA, indexed by the SAME
// `TopExp::MapShapes(shape, TopAbs_FACE)` order this function walks. It is copied
// verbatim into the MESH1 FACE_COLORS section; a null pointer, an empty vector, or
// a length that is not the face count leaves the section out and the flag bit clear
// (mesh_format.md §2/§4), so a body without colors produces byte-identical output.
BodyMesh tessellate_body(const TopoDS_Shape& shape, const std::string& body_id,
                         const std::string& lod, bool include_edges,
                         const elementmap::ElementMapPartition* partition,
                         const std::vector<std::uint32_t>* face_colors = nullptr);

// Mesh one body into raw triangle arrays (no ids, no edges). `lod` selects the same
// deflection tier as tessellate_body, so the triangles match the viewport mesh.
RawMesh tessellate_raw(const TopoDS_Shape& shape, const std::string& lod);

}  // namespace onecad::tess
