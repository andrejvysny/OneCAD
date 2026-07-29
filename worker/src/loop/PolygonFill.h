// PolygonFill.h — 2D triangulation of a detected region (outer loop + holes).
//
// The preview fill served by the solver lane (SCHEMA §7.4 `SketchRegions`
// `previewTriangles`) must agree with the solid the KERNEL builds from the same
// region: `FaceBuilder` adds every `innerLoop` as a hole wire to the profile
// face, so a region with a hole extrudes to a tube. Filling only the outer loop
// made the drag preview lie about the result and made the frontend's region
// hit-test treat the inside of a hole as part of the region.
//
// Holes are merged into the outer loop with BRIDGES, then the single resulting
// loop is ear-clipped. Bridge segments deliberately REUSE the existing vertex
// indices rather than duplicating vertices: each bridge is then traversed once
// in each direction, lands in exactly two triangles, and therefore reads as an
// INTERIOR edge to a boundary-edge counter.
//
// That property is LOAD-BEARING, not an implementation detail. The frontend
// derives its profile rings from triangulation topology (edges used by exactly
// one triangle — `src/tools/preview/prismPreview.ts profileFromRegion`), so a
// duplicated bridge vertex would fabricate a spurious boundary loop and corrupt
// the extruded side walls. `bridges_are_interior()` pins it.
//
// Pure 2D — deliberately NO OCCT. This runs on the solver lane, which must stay
// responsive while the kernel lane is busy (`test_concurrent_lanes`).

#pragma once

#include <cstdint>
#include <vector>

#include "../sketch/SketchTypes.h"

namespace onecad::core::loop {

namespace sk = onecad::core::sketch;

/// A region's outer loop and its holes merged into ONE weakly-simple CCW loop.
struct MergedLoop {
    /// Outer-loop vertices, followed by the vertices of each MERGED hole.
    std::vector<sk::Vec2d> verts;
    /// CCW traversal as indices into `verts`. Bridge indices appear twice.
    std::vector<std::uint32_t> loop;
    /// How many holes got a bridge. `< holes.size()` means some hole could not
    /// be bridged and remains filled — degraded, but never wrong-by-corruption.
    std::size_t holes_merged = 0;
};

/// Merge `holes` into `outer`, shortest valid bridge first. `outer` is
/// normalized to CCW and each hole to CW; a closing point coincident with the
/// first is dropped. A hole with no valid bridge is SKIPPED rather than
/// corrupting the whole region's triangulation.
MergedLoop merge_holes(const std::vector<sk::Vec2d>& outer,
                       const std::vector<std::vector<sk::Vec2d>>& holes);

/// Ear-clip a CCW index loop, emitting triangle index triples that address
/// `verts` directly (bridge vertices stay shared, never duplicated).
std::vector<std::uint32_t> ear_clip_indices(
    const std::vector<sk::Vec2d>& verts, std::vector<std::uint32_t> loop);

/// Convenience: `merge_holes` + `ear_clip_indices` in one step.
struct RegionFill {
    std::vector<sk::Vec2d> verts;
    std::vector<std::uint32_t> indices;
    std::size_t holes_subtracted = 0;
    /// True iff ear clipping consumed the whole merged loop (exactly loop-2
    /// triangles). A degenerate loop that stalls the clipper yields a PARTIAL
    /// triangle list — publishing it would hand the frontend an incomplete
    /// boundary to recover extrusion rings from, so callers must fail closed.
    bool complete = false;
};
RegionFill fill_region(const std::vector<sk::Vec2d>& outer,
                       const std::vector<std::vector<sk::Vec2d>>& holes);

// --- introspection helpers (used by tests to pin the invariants) ------------

/// Signed area of a polygon (CCW positive).
double polygon_signed_area(const std::vector<sk::Vec2d>& poly);

/// Summed area of every emitted triangle. For a correct fill this equals
/// `|outer area| - Σ|hole area|`.
double triangulated_area(const std::vector<sk::Vec2d>& verts,
                         const std::vector<std::uint32_t>& indices);

/// True when every bridge segment is INTERIOR, i.e. no undirected edge of the
/// triangulation is used exactly once except the real outer/hole boundary
/// edges. Concretely: the set of single-use edges equals the set of loop edges
/// that are NOT bridges. This is the invariant the frontend ring derivation
/// depends on.
bool bridges_are_interior(const MergedLoop& merged, const std::vector<std::uint32_t>& indices);

}  // namespace onecad::core::loop
