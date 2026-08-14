// GearTypes.h — the 2D vocabulary shared by every gear sampler under
// worker/src/kernel/gear/ (Gear Generator implementation plan, phase G0).
//
// Deliberately NO OCCT: these samplers run as pure arithmetic so they can be
// unit-tested standalone against oracles captured from the reference
// implementation, and so neither worker lane pays a kernel dependency to
// evaluate a gear profile. The OCCT-facing conversion (Point2d -> gp_Pnt,
// Arc3 -> GC_MakeArcOfCircle, sampled polyline -> GeomAPI_Interpolate) lives
// in the op/geometry layer, not here.
//
// The transform helpers mirror `pygears._functions` (freecad.gears v1.3.0,
// GPL-3.0) so a ported formula reads the same as its source; the names are
// spelled out because upstream's `reflection(0)` vs its `mir = [-1, 1]`
// idiom are two DIFFERENT mirrors and conflating them silently flips a tooth
// flank.

#pragma once

#include <cmath>
#include <vector>

namespace onecad::kernel::gear {

/// A 2D point in a gear profile's local frame (millimetres). Profiles are
/// built concentric about the origin with the tooth centred on +Y or +X
/// depending on the recipe — each sampler's header states which.
struct Point2d {
  double x = 0.0;
  double y = 0.0;
};

inline Point2d add(const Point2d& a, const Point2d& b) { return {a.x + b.x, a.y + b.y}; }

inline Point2d sub(const Point2d& a, const Point2d& b) { return {a.x - b.x, a.y - b.y}; }

inline Point2d scaled(const Point2d& p, double s) { return {p.x * s, p.y * s}; }

inline double norm(const Point2d& p) { return std::sqrt(p.x * p.x + p.y * p.y); }

inline double distance(const Point2d& a, const Point2d& b) { return norm(sub(a, b)); }

/// CCW rotation by `angle` radians about the origin. Ported from
/// `_functions.rotation(angle)` with its default centre: upstream builds
/// `mat = [[cos, sin], [-sin, cos]]` and applies it as `dot(points, mat)`
/// (row-vector × matrix), which is the standard CCW rotation.
inline Point2d rotate(const Point2d& p, double angle) {
  const double c = std::cos(angle);
  const double s = std::sin(angle);
  return {p.x * c - p.y * s, p.x * s + p.y * c};
}

/// Mirror across the X axis (negates y). This is upstream's `reflection(0)`.
inline Point2d mirror_about_x_axis(const Point2d& p) { return {p.x, -p.y}; }

/// Mirror across the Y axis (negates x). This is upstream's elementwise
/// `mir = np.array([-1.0, 1.0])` multiply — a DIFFERENT mirror from
/// `reflection(0)` above.
inline Point2d mirror_about_y_axis(const Point2d& p) { return {-p.x, p.y}; }

/// Mirror across the line through the origin at `angle` radians. Ported from
/// `_functions.reflection(angle)`: `mat = [[cos2a, -sin2a], [-sin2a, -cos2a]]`
/// applied as `dot(x, mat)`, i.e.
///   x' =  x·cos2a - y·sin2a
///   y' = -x·sin2a - y·cos2a
inline Point2d mirror_about_line(const Point2d& p, double angle) {
  const double c2 = std::cos(2.0 * angle);
  const double s2 = std::sin(2.0 * angle);
  return {p.x * c2 - p.y * s2, -p.x * s2 - p.y * c2};
}

/// A circular arc in the canonical three-point form (start, an interior
/// point, end) — directly consumable by OCCT's `GC_MakeArcOfCircle(p1,p2,p3)`
/// and by upstream's `Part.Arc`, which is why every arc-based sampler here
/// emits this rather than a centre/radius/angle triple.
struct Arc3 {
  Point2d start;
  Point2d mid;
  Point2d end;
};

/// A profile built from a MIX of edge kinds — the shared vocabulary the G2
/// wire builder consumes. Each kind maps to exactly one OCCT edge builder:
///   Line         -> GC_MakeSegment(start, end)
///   Arc          -> GC_MakeArcOfCircle(start, mid, end)
///   Interpolated -> GeomAPI_Interpolate over `points`
/// Recipes whose every edge is an arc (GT/HTD) emit `Arc3` directly instead;
/// recipes that mix (timing-T: straight flanks + arcs; lantern: sampled
/// parallel curves + arcs) emit this.
///
/// Fields are kind-specific and must not be read otherwise: `mid` is Arc
/// only, `points` is Interpolated only. Keeping one struct rather than a
/// variant keeps that builder a single loop with one switch.
enum class SegmentKind {
  Line,
  Arc,
  Interpolated,
};

struct ProfileSegment {
  SegmentKind kind = SegmentKind::Line;
  Point2d start;
  Point2d end;
  Point2d mid;                  ///< interior point; Arc only.
  std::vector<Point2d> points;  ///< sample run INCLUDING start and end; Interpolated only.
};

inline ProfileSegment line_segment(const Point2d& a, const Point2d& b) {
  ProfileSegment s;
  s.kind = SegmentKind::Line;
  s.start = a;
  s.end = b;
  return s;
}

inline ProfileSegment arc_segment(const Arc3& a) {
  ProfileSegment s;
  s.kind = SegmentKind::Arc;
  s.start = a.start;
  s.mid = a.mid;
  s.end = a.end;
  return s;
}

/// `pts` must hold at least two samples; `start`/`end` are taken from its
/// ends so the chain-continuity contract reads the same for every kind.
inline ProfileSegment interpolated_segment(std::vector<Point2d> pts) {
  ProfileSegment s;
  s.kind = SegmentKind::Interpolated;
  s.start = pts.front();
  s.end = pts.back();
  s.points = std::move(pts);
  return s;
}

}  // namespace onecad::kernel::gear
