// PolygonFill.cpp — see PolygonFill.h.
#include "loop/PolygonFill.h"

#include <algorithm>
#include <cmath>
#include <map>
#include <utility>

namespace onecad::core::loop {

namespace {

constexpr double kOrientEps = 1e-9;   // orientation sign threshold
constexpr double kCoincEps = 1e-12;   // point-coincidence threshold

// Drop a trailing point coincident with the first (loops arrive both ways).
void drop_closing_point(std::vector<sk::Vec2d>& poly) {
    if (poly.size() < 2) return;
    const auto& f = poly.front();
    const auto& l = poly.back();
    if (std::abs(f.x - l.x) < kCoincEps && std::abs(f.y - l.y) < kCoincEps) poly.pop_back();
}

double cross2(const sk::Vec2d& o, const sk::Vec2d& a, const sk::Vec2d& b) {
    return (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
}

// True when `p` lies strictly between `a` and `b` on the segment ab.
bool point_on_segment_interior(const sk::Vec2d& p, const sk::Vec2d& a, const sk::Vec2d& b) {
    if (std::abs(cross2(a, b, p)) > kOrientEps) return false;
    const double dot = (p.x - a.x) * (b.x - a.x) + (p.y - a.y) * (b.y - a.y);
    const double len2 = (b.x - a.x) * (b.x - a.x) + (b.y - a.y) * (b.y - a.y);
    return dot > kOrientEps && dot < len2 - kOrientEps;
}

// Proper crossing (interior of both segments) OR a collinear overlap. Segments
// that merely share an endpoint are NOT a crossing — every bridge is anchored on
// two existing polygon vertices, so shared endpoints are the normal case.
bool segments_cross(const sk::Vec2d& p1, const sk::Vec2d& p2, const sk::Vec2d& p3,
                    const sk::Vec2d& p4) {
    const double d1 = cross2(p3, p4, p1);
    const double d2 = cross2(p3, p4, p2);
    const double d3 = cross2(p1, p2, p3);
    const double d4 = cross2(p1, p2, p4);
    const bool straddle_a =
        (d1 > kOrientEps && d2 < -kOrientEps) || (d1 < -kOrientEps && d2 > kOrientEps);
    const bool straddle_b =
        (d3 > kOrientEps && d4 < -kOrientEps) || (d3 < -kOrientEps && d4 > kOrientEps);
    if (straddle_a && straddle_b) return true;
    // Collinear overlap: a bridge lying ALONG an edge would break ear clipping.
    if (std::abs(d1) <= kOrientEps && std::abs(d2) <= kOrientEps) {
        return point_on_segment_interior(p3, p1, p2) || point_on_segment_interior(p4, p1, p2) ||
               point_on_segment_interior(p1, p3, p4) || point_on_segment_interior(p2, p3, p4);
    }
    return false;
}

// Ray-cast point-in-polygon over an index loop. Bridge edges appear twice (once
// per direction) and contribute two crossings, so they cancel — a merged loop
// tests exactly like the material region it bounds.
bool point_in_index_loop(const sk::Vec2d& pt, const std::vector<sk::Vec2d>& verts,
                         const std::vector<std::uint32_t>& loop) {
    bool inside = false;
    for (std::size_t i = 0, n = loop.size(); i < n; ++i) {
        const sk::Vec2d& a = verts[loop[i]];
        const sk::Vec2d& b = verts[loop[(i + 1) % n]];
        if ((a.y > pt.y) != (b.y > pt.y)) {
            const double t = (pt.y - a.y) / (b.y - a.y);
            if (pt.x < a.x + t * (b.x - a.x)) inside = !inside;
        }
    }
    return inside;
}

bool point_in_poly(const sk::Vec2d& pt, const std::vector<sk::Vec2d>& poly) {
    bool inside = false;
    for (std::size_t i = 0, n = poly.size(); i < n; ++i) {
        const sk::Vec2d& a = poly[i];
        const sk::Vec2d& b = poly[(i + 1) % n];
        if ((a.y > pt.y) != (b.y > pt.y)) {
            const double t = (pt.y - a.y) / (b.y - a.y);
            if (pt.x < a.x + t * (b.x - a.x)) inside = !inside;
        }
    }
    return inside;
}

bool point_in_triangle(double px, double py, const sk::Vec2d& a, const sk::Vec2d& b,
                       const sk::Vec2d& c) {
    const double d1 = (px - b.x) * (a.y - b.y) - (a.x - b.x) * (py - b.y);
    const double d2 = (px - c.x) * (b.y - c.y) - (b.x - c.x) * (py - c.y);
    const double d3 = (px - a.x) * (c.y - a.y) - (c.x - a.x) * (py - a.y);
    const bool has_neg = (d1 < 0) || (d2 < 0) || (d3 < 0);
    const bool has_pos = (d1 > 0) || (d2 > 0) || (d3 > 0);
    return !(has_neg && has_pos);
}

std::pair<std::uint32_t, std::uint32_t> edge_key(std::uint32_t a, std::uint32_t b) {
    return a < b ? std::make_pair(a, b) : std::make_pair(b, a);
}

}  // namespace

double polygon_signed_area(const std::vector<sk::Vec2d>& poly) {
    double a = 0.0;
    for (std::size_t i = 0, n = poly.size(); i < n; ++i) {
        const auto& p = poly[i];
        const auto& q = poly[(i + 1) % n];
        a += p.x * q.y - q.x * p.y;
    }
    return 0.5 * a;
}

MergedLoop merge_holes(const std::vector<sk::Vec2d>& outer_in,
                       const std::vector<std::vector<sk::Vec2d>>& holes_in) {
    MergedLoop out;

    std::vector<sk::Vec2d> outer = outer_in;
    drop_closing_point(outer);
    if (outer.size() < 3) return out;

    out.verts = outer;
    out.loop.resize(outer.size());
    for (std::size_t i = 0; i < outer.size(); ++i) out.loop[i] = static_cast<std::uint32_t>(i);
    if (polygon_signed_area(outer) < 0.0) {
        std::reverse(out.loop.begin(), out.loop.end());  // force CCW
    }

    // Normalize holes to CW (opposite the outer loop) and drop degenerates.
    std::vector<std::vector<sk::Vec2d>> pending;
    pending.reserve(holes_in.size());
    for (const auto& h_in : holes_in) {
        std::vector<sk::Vec2d> h = h_in;
        drop_closing_point(h);
        if (h.size() < 3) continue;
        if (polygon_signed_area(h) > 0.0) std::reverse(h.begin(), h.end());
        pending.push_back(std::move(h));
    }

    for (std::size_t hi = 0; hi < pending.size(); ++hi) {
        const std::vector<sk::Vec2d>& hole = pending[hi];

        // Shortest bridge (loop position li → hole vertex hj) that stays inside
        // the material: crosses no edge, touches no vertex, and whose midpoint is
        // inside the merged loop and outside every not-yet-merged hole.
        std::size_t best_li = 0;
        std::size_t best_hj = 0;
        double best_d2 = 0.0;
        bool found = false;

        for (std::size_t li = 0; li < out.loop.size(); ++li) {
            const sk::Vec2d& p = out.verts[out.loop[li]];
            for (std::size_t hj = 0; hj < hole.size(); ++hj) {
                const sk::Vec2d& q = hole[hj];
                const double d2 = (p.x - q.x) * (p.x - q.x) + (p.y - q.y) * (p.y - q.y);
                if (found && d2 >= best_d2) continue;
                if (d2 < kCoincEps) continue;  // degenerate bridge

                bool valid = true;
                for (std::size_t e = 0; e < out.loop.size() && valid; ++e) {
                    const sk::Vec2d& a = out.verts[out.loop[e]];
                    const sk::Vec2d& b = out.verts[out.loop[(e + 1) % out.loop.size()]];
                    if (segments_cross(p, q, a, b)) valid = false;
                }
                for (std::size_t k = hi; k < pending.size() && valid; ++k) {
                    const std::vector<sk::Vec2d>& other = pending[k];
                    for (std::size_t e = 0; e < other.size() && valid; ++e) {
                        if (segments_cross(p, q, other[e], other[(e + 1) % other.size()])) {
                            valid = false;
                        }
                    }
                }
                // No third vertex may sit ON the bridge (a touch breaks clipping).
                for (std::size_t e = 0; e < out.loop.size() && valid; ++e) {
                    if (point_on_segment_interior(out.verts[out.loop[e]], p, q)) valid = false;
                }
                for (std::size_t k = hi; k < pending.size() && valid; ++k) {
                    for (const auto& v : pending[k]) {
                        if (point_on_segment_interior(v, p, q)) {
                            valid = false;
                            break;
                        }
                    }
                }
                if (!valid) continue;

                const sk::Vec2d mid{0.5 * (p.x + q.x), 0.5 * (p.y + q.y)};
                if (!point_in_index_loop(mid, out.verts, out.loop)) continue;
                bool in_hole = false;
                for (std::size_t k = hi; k < pending.size() && !in_hole; ++k) {
                    if (point_in_poly(mid, pending[k])) in_hole = true;
                }
                if (in_hole) continue;

                best_li = li;
                best_hj = hj;
                best_d2 = d2;
                found = true;
            }
        }
        if (!found) continue;  // hole stays filled; reported via holes_merged

        // Append the hole's vertices, then splice after `best_li`:
        //   … L[li], H[hj], H[hj+1], …, H[hj-1], H[hj], L[li], …
        // Both bridge segments reuse L[li] / H[hj], so each ends up interior.
        const std::uint32_t base = static_cast<std::uint32_t>(out.verts.size());
        out.verts.insert(out.verts.end(), hole.begin(), hole.end());

        std::vector<std::uint32_t> spliced;
        spliced.reserve(out.loop.size() + hole.size() + 2);
        for (std::size_t i = 0; i <= best_li; ++i) spliced.push_back(out.loop[i]);
        for (std::size_t i = 0; i < hole.size(); ++i) {
            spliced.push_back(base + static_cast<std::uint32_t>((best_hj + i) % hole.size()));
        }
        spliced.push_back(base + static_cast<std::uint32_t>(best_hj));
        for (std::size_t i = best_li; i < out.loop.size(); ++i) spliced.push_back(out.loop[i]);
        out.loop = std::move(spliced);
        ++out.holes_merged;
    }
    return out;
}

std::vector<std::uint32_t> ear_clip_indices(const std::vector<sk::Vec2d>& verts,
                                            std::vector<std::uint32_t> v) {
    std::vector<std::uint32_t> tris;
    if (v.size() < 3) return tris;

    std::size_t guard = 0;
    const std::size_t guard_max = v.size() * v.size() + 8;
    while (v.size() > 2 && guard++ < guard_max) {
        bool clipped = false;
        const std::size_t m = v.size();
        for (std::size_t i = 0; i < m; ++i) {
            const std::uint32_t ia = v[(i + m - 1) % m];
            const std::uint32_t ib = v[i];
            const std::uint32_t ic = v[(i + 1) % m];
            if (ia == ib || ib == ic || ia == ic) continue;  // bridge-collapsed ear
            const sk::Vec2d& a = verts[ia];
            const sk::Vec2d& b = verts[ib];
            const sk::Vec2d& c = verts[ic];
            const double cross = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
            if (cross <= 0.0) continue;  // reflex or degenerate (CCW convex ear needs >0)
            bool contains = false;
            for (std::size_t k = 0; k < m; ++k) {
                const std::uint32_t iv = v[k];
                if (iv == ia || iv == ib || iv == ic) continue;
                if (point_in_triangle(verts[iv].x, verts[iv].y, a, b, c)) {
                    contains = true;
                    break;
                }
            }
            if (contains) continue;
            tris.push_back(ia);
            tris.push_back(ib);
            tris.push_back(ic);
            v.erase(v.begin() + static_cast<long>(i));
            clipped = true;
            break;
        }
        if (!clipped) break;  // no ear found (degenerate) — stop
    }
    return tris;
}

RegionFill fill_region(const std::vector<sk::Vec2d>& outer,
                       const std::vector<std::vector<sk::Vec2d>>& holes) {
    MergedLoop merged = merge_holes(outer, holes);
    RegionFill out;
    out.indices = ear_clip_indices(merged.verts, merged.loop);
    out.verts = std::move(merged.verts);
    out.holes_subtracted = merged.holes_merged;
    // Every clipped ear removes exactly one loop entry and the clipper stops at
    // two, so a COMPLETE fill has exactly loop-2 triangles (duplicated bridge
    // indices included). Anything less means the clipper stalled on degeneracy.
    out.complete =
        merged.loop.size() >= 3 && out.indices.size() == 3 * (merged.loop.size() - 2);
    return out;
}

double triangulated_area(const std::vector<sk::Vec2d>& verts,
                         const std::vector<std::uint32_t>& indices) {
    double area = 0.0;
    for (std::size_t i = 0; i + 2 < indices.size(); i += 3) {
        const sk::Vec2d& a = verts[indices[i]];
        const sk::Vec2d& b = verts[indices[i + 1]];
        const sk::Vec2d& c = verts[indices[i + 2]];
        area += std::abs(0.5 * ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)));
    }
    return area;
}

bool bridges_are_interior(const MergedLoop& merged, const std::vector<std::uint32_t>& indices) {
    // Loop edges traversed exactly once are the real boundary; a bridge is
    // traversed once per direction, so it counts twice.
    std::map<std::pair<std::uint32_t, std::uint32_t>, int> loop_uses;
    for (std::size_t i = 0, n = merged.loop.size(); i < n; ++i) {
        ++loop_uses[edge_key(merged.loop[i], merged.loop[(i + 1) % n])];
    }
    std::map<std::pair<std::uint32_t, std::uint32_t>, int> tri_uses;
    for (std::size_t i = 0; i + 2 < indices.size(); i += 3) {
        ++tri_uses[edge_key(indices[i], indices[i + 1])];
        ++tri_uses[edge_key(indices[i + 1], indices[i + 2])];
        ++tri_uses[edge_key(indices[i + 2], indices[i])];
    }
    // Every single-use triangulation edge must be a single-use loop edge, and
    // every single-use loop edge must survive as one. Bridges (loop count 2)
    // must NOT appear as single-use triangulation edges.
    for (const auto& [key, uses] : tri_uses) {
        const auto it = loop_uses.find(key);
        const bool boundary = it != loop_uses.end() && it->second == 1;
        if (uses == 1 && !boundary) return false;
        if (uses != 1 && boundary) return false;
    }
    for (const auto& [key, uses] : loop_uses) {
        if (uses != 1) continue;
        const auto it = tri_uses.find(key);
        if (it == tri_uses.end() || it->second != 1) return false;
    }
    return true;
}

}  // namespace onecad::core::loop
