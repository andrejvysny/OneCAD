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
#include "tess/CurveSampler.h"
#include "tess/EdgeClassification.h"

class BRepAdaptor_Curve;

namespace onecad::tess {

// Producer-side completeness evidence for one body (VP-HARDENING WP09, spec §14,
// NUM §9.2). NOT on the wire yet: WP10 publishes it as MESH1 v2 QUALITY_INFO
// bit 0 (complete nondegenerate face coverage) and bit 3 (some normal fallback).
//
// D11 / Astra F1: `ok` on BodyMesh is now exactly `allNondegenerateFacesCovered`.
// An incomplete display tessellation is a FAILED tessellation until WP10 can put
// the completeness record on the wire — publishing a solid with a face silently
// missing is the one outcome spec §14 forbids ("Partial output must not be
// silently labeled complete"), and the retired code published it while recording
// the contradiction in a field nothing read. EDGES never affect `ok`: they are
// display-only, not pickable topology, so a failed nondegenerate edge keeps a
// warned zero-point range and is recorded in `missingEdges` for WP10.
struct TessellationCompleteness {
    bool allNondegenerateFacesCovered = true;
    std::vector<std::string> missingFaces;     // TopoKeys ("f:7") with no triangles
    std::vector<std::string> degenerateFaces;  // TopoKeys legitimately holding a zero range
    // PR-07 / D9: TopoKeys ("e:12") of nondegenerate edges that reached the body's
    // 2 000 000-segment cap and therefore carry a ZERO-POINT range. The declared
    // cap is a cap: a post-budget edge keeps its id and its range slot and gets
    // no geometry at all, rather than the two-endpoint chord that used to push
    // the total past the cap one uncounted segment at a time.
    std::vector<std::string> budgetLimitedEdges;
    // D11 / Astra F1: TopoKeys ("e:12") of NONDEGENERATE edges that produced no
    // drawable polyline for a reason other than the body budget — a failed or
    // too-short sample, a non-finite point, a sampler exception. They keep a
    // warned zero-point range and their id; they never affect `ok`.
    std::vector<std::string> missingEdges;
    std::uint32_t normalFallbackNodes = 0;     // NUM §7.3 triangulation fallback
    std::uint32_t singularSplitNodes = 0;      // NUM §7.3 singular split
    std::uint32_t normalUnresolvedNodes = 0;   // Astra F3/F5: a normal with no error bound
    std::uint32_t normalMissingNodes = 0;      // no surface AND no incident triangle direction
    // Astra §4's representation-collapse class: triangles with positive
    // binary64 area whose EMITTED float32 positions collapse to exactly zero.
    // A representation failure, recorded and warned — never a degeneracy escape.
    std::uint32_t float32CollapsedTriangles = 0;
};

struct BodyMesh {
    std::string body_id;
    std::vector<std::uint8_t> blob;       // MESH1 bytes
    std::uint32_t triangle_count = 0;
    // D11: false when the body produced no triangulation at all, OR when a
    // NONDEGENERATE face of it carries no triangles. `diagnostic` then names the
    // faces so the caller's per-body failure path can say WHICH face failed.
    bool ok = false;
    std::string diagnostic;
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

namespace detail {

// The WP08 edge policy ladder: sample, and on a WORK cap re-sample against a
// deliberately relaxed budget, up to four doublings, before falling back to the
// two analytic endpoints. A relaxed result is returned as QualityLimited with
// `requestedChordToleranceMm` and `achievedChordToleranceMm` separated (PR-07 /
// D9) — the retry's own `Certified` never leaks out. Declared here so the policy
// can be tested directly instead of only through a whole body.
CurveSampleResult sample_edge_polyline(const BRepAdaptor_Curve& curve, const std::string& label,
                                       const CurveSampleRequest& request,
                                       const CurveSampleLimits& limits);

// TEST SEAM ONLY for the NUM §2.5 per-body edge segment budget (2 000 000).
// Production never calls these, so the shipped budget is a constant and the
// output stays deterministic; a test needs them because exhausting two million
// segments honestly is not a unit test. Not thread-safe: the worker is
// single-threaded and only the sampler tests touch this.
void set_body_edge_segment_budget_for_test(std::size_t budget);
void clear_body_edge_segment_budget_for_test();
std::size_t body_edge_segment_budget();

}  // namespace detail

}  // namespace onecad::tess
