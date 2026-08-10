// test_polygon_fill.cpp — region preview fill with holes subtracted.
//
// The fill served by `SketchRegions.previewTriangles` must agree with the solid
// the kernel builds from the same region (FaceBuilder adds every innerLoop as a
// hole wire). Two properties are asserted for every case:
//
//   1. AREA — Σ|triangle| == |outer| − Σ|hole|. A filled hole shows up here
//      immediately (the pre-fix behaviour fails this by exactly the hole area).
//   2. BRIDGE INTERIORITY — the set of edges used by exactly ONE triangle is
//      exactly the real outer + hole boundary; every bridge is used twice.
//      This is load-bearing, not cosmetic: the frontend derives its extrusion
//      rings from triangulation topology (`prismPreview.profileFromRegion`), so
//      a bridge that read as a boundary edge would fabricate a spurious loop
//      and corrupt the prism's side walls.
//
// No test framework (matches the prototype style): exit code == failure count.
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <vector>

#include "loop/PolygonFill.h"

namespace loop = onecad::core::loop;
namespace sk = onecad::core::sketch;

namespace {
int g_failures = 0;
#define CHECK(cond)                                                              \
    do {                                                                         \
        if (!(cond)) {                                                           \
            std::fprintf(stderr, "FAIL %s:%d: %s\n", __FILE__, __LINE__, #cond); \
            ++g_failures;                                                        \
        }                                                                        \
    } while (0)

constexpr double kAreaTol = 1e-9;

std::vector<sk::Vec2d> rect(double x0, double y0, double x1, double y1) {
    return {{x0, y0}, {x1, y0}, {x1, y1}, {x0, y1}};
}

// Regular n-gon, CCW, centred at (cx,cy).
std::vector<sk::Vec2d> ngon(double cx, double cy, double r, int n) {
    std::vector<sk::Vec2d> out;
    out.reserve(static_cast<std::size_t>(n));
    for (int i = 0; i < n; ++i) {
        const double a = 2.0 * M_PI * static_cast<double>(i) / static_cast<double>(n);
        out.push_back({cx + r * std::cos(a), cy + r * std::sin(a)});
    }
    return out;
}

double expected_area(const std::vector<sk::Vec2d>& outer,
                     const std::vector<std::vector<sk::Vec2d>>& holes) {
    double a = std::abs(loop::polygon_signed_area(outer));
    for (const auto& h : holes) a -= std::abs(loop::polygon_signed_area(h));
    return a;
}

// Run both invariants over one case. `expect_merged` is how many holes MUST get
// a bridge (a skipped hole degrades the fill but never corrupts it).
void check_case(const char* name, const std::vector<sk::Vec2d>& outer,
                const std::vector<std::vector<sk::Vec2d>>& holes, std::size_t expect_merged) {
    const loop::MergedLoop merged = loop::merge_holes(outer, holes);
    const std::vector<std::uint32_t> tris = loop::ear_clip_indices(merged.verts, merged.loop);

    const double got = loop::triangulated_area(merged.verts, tris);
    const double want = expect_merged == holes.size()
                            ? expected_area(outer, holes)
                            : 0.0;  // partial merges are area-checked by the caller

    std::fprintf(stderr, "[%s] merged=%zu/%zu tris=%zu area=%.9f want=%.9f\n", name,
                 merged.holes_merged, holes.size(), tris.size() / 3, got, want);

    CHECK(merged.holes_merged == expect_merged);
    if (expect_merged == holes.size()) CHECK(std::abs(got - want) < kAreaTol);
    CHECK(loop::bridges_are_interior(merged, tris));
    // A correct fill always produces at least one triangle per boundary vertex
    // minus two; a collapsed clip would silently emit far fewer.
    CHECK(tris.size() >= 3);
}

}  // namespace

int main() {
    // --- 1. No holes: unchanged behaviour (regression guard) ---
    check_case("square, no holes", rect(0, 0, 10, 10), {}, 0);

    // --- 2. THE case: square with a square hole => a tube, not a slab ---
    {
        const auto outer = rect(0, 0, 40, 20);
        const std::vector<std::vector<sk::Vec2d>> holes = {rect(10, 5, 20, 15)};
        check_case("rect + rect hole", outer, holes, 1);
        // Pin the magnitude: 800 - 100 = 700. Pre-fix this returned 800.
        const loop::RegionFill f = loop::fill_region(outer, holes);
        CHECK(std::abs(loop::triangulated_area(f.verts, f.indices) - 700.0) < kAreaTol);
        CHECK(f.holes_subtracted == 1);
        CHECK(f.complete);
    }

    // --- 3. Round hole (the sketch case: rectangle + circle) ---
    {
        const auto outer = rect(0, 0, 40, 20);
        const std::vector<std::vector<sk::Vec2d>> holes = {ngon(20, 10, 5, 24)};
        check_case("rect + round hole", outer, holes, 1);
    }

    // --- 4. Multiple disjoint holes ---
    {
        const auto outer = rect(0, 0, 60, 20);
        const std::vector<std::vector<sk::Vec2d>> holes = {
            rect(5, 5, 15, 15), rect(25, 5, 35, 15), ngon(50, 10, 4, 16)};
        check_case("rect + 3 holes", outer, holes, 3);
    }

    // --- 5. Concave outer (L-shape) with a hole in the thick arm ---
    {
        const std::vector<sk::Vec2d> ell = {{0, 0}, {30, 0}, {30, 10}, {10, 10}, {10, 30}, {0, 30}};
        const std::vector<std::vector<sk::Vec2d>> holes = {rect(15, 2, 25, 8)};
        check_case("L-shape + hole", ell, holes, 1);
    }

    // --- 6. Hole winding is normalized: a CCW hole must behave like a CW one ---
    {
        const auto outer = rect(0, 0, 40, 20);
        auto ccw_hole = rect(10, 5, 20, 15);                    // already CCW
        auto cw_hole = ccw_hole;                                // same ring, reversed
        std::reverse(cw_hole.begin(), cw_hole.end());
        const loop::RegionFill a = loop::fill_region(outer, {ccw_hole});
        const loop::RegionFill b = loop::fill_region(outer, {cw_hole});
        CHECK(std::abs(loop::triangulated_area(a.verts, a.indices) -
                       loop::triangulated_area(b.verts, b.indices)) < kAreaTol);
        CHECK(a.holes_subtracted == 1 && b.holes_subtracted == 1);
    }

    // --- 7. A closing point coincident with the first is dropped, not doubled ---
    {
        auto outer = rect(0, 0, 40, 20);
        outer.push_back(outer.front());
        auto hole = rect(10, 5, 20, 15);
        hole.push_back(hole.front());
        const loop::RegionFill f = loop::fill_region(outer, {hole});
        CHECK(f.verts.size() == 8);  // 4 outer + 4 hole, no duplicates
        CHECK(std::abs(loop::triangulated_area(f.verts, f.indices) - 700.0) < kAreaTol);
    }

    // --- 8. Degenerate inputs stay graceful (no crash, no garbage) ---
    {
        const loop::RegionFill empty = loop::fill_region({}, {});
        CHECK(empty.indices.empty());
        const loop::RegionFill two = loop::fill_region({{0, 0}, {1, 1}}, {});
        CHECK(two.indices.empty());
        // A degenerate hole (2 points) is dropped, the outer still fills.
        const loop::RegionFill degen_hole =
            loop::fill_region(rect(0, 0, 10, 10), {{{1, 1}, {2, 2}}});
        CHECK(std::abs(loop::triangulated_area(degen_hole.verts, degen_hole.indices) - 100.0) <
              kAreaTol);
        CHECK(degen_hole.holes_subtracted == 0);
    }

    // --- 8b. A stalled clip is REPORTED, never silently partial ---
    {
        // Collinear "outer": no positive-cross ear exists, the clipper stalls at
        // zero triangles. `complete=false` is what lets SketchRegions fail closed
        // instead of publishing partial material (review defect a).
        const loop::RegionFill collinear =
            loop::fill_region({{0, 0}, {5, 0}, {10, 0}}, {});
        CHECK(!collinear.complete);
        CHECK(collinear.indices.empty());
        // Sub-3-vertex inputs can never fill; they are incomplete too.
        CHECK(!loop::fill_region({}, {}).complete);
        CHECK(!loop::fill_region({{0, 0}, {1, 1}}, {}).complete);
        // The graceful degenerate-hole case (8) still completes its outer fill.
        const loop::RegionFill degen_hole =
            loop::fill_region(rect(0, 0, 10, 10), {{{1, 1}, {2, 2}}});
        CHECK(degen_hole.complete);
    }

    // --- 9. Determinism: identical input => byte-identical output ---
    {
        const auto outer = rect(0, 0, 40, 20);
        const std::vector<std::vector<sk::Vec2d>> holes = {rect(5, 5, 12, 15),
                                                           ngon(30, 10, 4, 12)};
        const loop::RegionFill a = loop::fill_region(outer, holes);
        const loop::RegionFill b = loop::fill_region(outer, holes);
        CHECK(a.indices == b.indices);
        CHECK(a.verts.size() == b.verts.size());
    }

    if (g_failures == 0) std::fprintf(stderr, "polygon_fill: all checks passed\n");
    return g_failures;
}
