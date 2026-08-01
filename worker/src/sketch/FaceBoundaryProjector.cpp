// FaceBoundaryProjector.cpp — see FaceBoundaryProjector.h for the oracle
// provenance (OneCAD-CPP src/core/sketch/FaceBoundaryProjector.cpp @ b4ddcccc),
// the verbatim-parity list, and the deliberate deviations.
#include "sketch/FaceBoundaryProjector.h"

#include "kernel/topology/CoplanarFacePatch.h"

#include <BRepAdaptor_Curve.hxx>
#include <BRepTools.hxx>
#include <BRepTools_WireExplorer.hxx>
#include <BRep_Tool.hxx>
#include <GeomAbs_CurveType.hxx>
#include <TopAbs.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_MapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Wire.hxx>
#include <gp_Circ.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>

#include <algorithm>
#include <cmath>
#include <cstddef>
#include <numbers>
#include <utility>

namespace onecad::core::sketch {
namespace {

using Options = FaceBoundaryProjector::Options;
using Result = FaceBoundaryProjector::Result;
using Entity = FaceBoundaryProjector::Entity;
using EntityKind = FaceBoundaryProjector::EntityKind;

constexpr double kPointEpsilon = 1e-9;   // oracle :31
constexpr double kAngleTolerance = 1e-4;  // oracle :32

constexpr double twoPi() { return 2.0 * std::numbers::pi_v<double>; }

double distanceSquared(const Vec2d& a, const Vec2d& b) {
    const double dx = a.x - b.x;
    const double dy = a.y - b.y;
    return dx * dx + dy * dy;
}

bool nearlyEqual(double a, double b, double tolerance) {
    return std::abs(a - b) <= tolerance;
}

bool samePoint(const Vec2d& a, const Vec2d& b, double tolerance) {
    return distanceSquared(a, b) <= tolerance * tolerance;
}

double normalizeAngle(double angle) {
    angle = std::fmod(angle, twoPi());
    if (angle <= -std::numbers::pi_v<double>) {
        angle += twoPi();
    } else if (angle > std::numbers::pi_v<double>) {
        angle -= twoPi();
    }
    return angle;
}

double normalizeAnglePositive(double angle) {
    angle = std::fmod(angle, twoPi());
    if (angle < 0.0) {
        angle += twoPi();
    }
    return angle;
}

bool sameAngle(double a, double b, double tolerance) {
    return std::abs(normalizeAngle(a - b)) <= tolerance;
}

Vec2d pointAt(const Result& result, int index) {
    return Vec2d{result.points[static_cast<std::size_t>(index)].u,
                 result.points[static_cast<std::size_t>(index)].v};
}

// Oracle findOrCreatePoint (:85-110): linear merge scan, then append. The oracle
// scans the SKETCH's existing points; the buffer here starts empty, so every
// returned slot is response-local and every one of them is new.
int findOrCreatePoint(Result& result, const Vec2d& position, double tolerance) {
    for (std::size_t i = 0; i < result.points.size(); ++i) {
        if (samePoint(pointAt(result, static_cast<int>(i)), position, tolerance)) {
            return static_cast<int>(i);
        }
    }
    result.points.push_back(FaceBoundaryProjector::Point{position.x, position.y});
    return static_cast<int>(result.points.size()) - 1;
}

// Oracle lineExists (:112-147): POSITION comparison in both directions.
bool lineExists(const Result& result, int startIndex, int endIndex, double tolerance) {
    const Vec2d start = pointAt(result, startIndex);
    const Vec2d end = pointAt(result, endIndex);
    for (const Entity& entity : result.entities) {
        if (entity.kind != EntityKind::Line) {
            continue;
        }
        const Vec2d a = pointAt(result, entity.p0);
        const Vec2d b = pointAt(result, entity.p1);
        const bool direct = samePoint(start, a, tolerance) && samePoint(end, b, tolerance);
        const bool reversed = samePoint(start, b, tolerance) && samePoint(end, a, tolerance);
        if (direct || reversed) {
            return true;
        }
    }
    return false;
}

// Oracle circleExists (:149-172).
bool circleExists(const Result& result, const Vec2d& center, double radius,
                  double pointTolerance, double radiusTolerance) {
    for (const Entity& entity : result.entities) {
        if (entity.kind != EntityKind::Circle) {
            continue;
        }
        if (samePoint(pointAt(result, entity.center), center, pointTolerance) &&
            nearlyEqual(entity.radius, radius, radiusTolerance)) {
            return true;
        }
    }
    return false;
}

// Oracle arcExists (:174-205).
bool arcExists(const Result& result, const Vec2d& center, double radius, double startAngle,
               double endAngle, double pointTolerance, double radiusTolerance) {
    for (const Entity& entity : result.entities) {
        if (entity.kind != EntityKind::Arc) {
            continue;
        }
        if (!samePoint(pointAt(result, entity.center), center, pointTolerance)) {
            continue;
        }
        if (!nearlyEqual(entity.radius, radius, radiusTolerance)) {
            continue;
        }
        if (sameAngle(entity.startAngle, startAngle, kAngleTolerance) &&
            sameAngle(entity.endAngle, endAngle, kAngleTolerance)) {
            return true;
        }
    }
    return false;
}

struct SegmentResult {
    bool valid = false;
    int startIndex = -1;
    int endIndex = -1;
};

// Oracle addLineSegment (:213-245), minus the reference-lock bookkeeping.
SegmentResult addLineSegment(Result& result, const Vec2d& start, const Vec2d& end,
                             const Options& options) {
    if (distanceSquared(start, end) <= kPointEpsilon * kPointEpsilon) {
        return {};
    }
    const int startIndex = findOrCreatePoint(result, start, options.pointMergeTolerance);
    const int endIndex = findOrCreatePoint(result, end, options.pointMergeTolerance);
    if (!lineExists(result, startIndex, endIndex, options.pointMergeTolerance)) {
        Entity line;
        line.kind = EntityKind::Line;
        line.p0 = startIndex;
        line.p1 = endIndex;
        result.entities.push_back(line);
    }
    return SegmentResult{true, startIndex, endIndex};
}

// Oracle sampleEdgePolyline (:247-263).
std::vector<Vec2d> sampleEdgePolyline(const BRepAdaptor_Curve& curve, const SketchPlane& plane,
                                      int segments) {
    std::vector<Vec2d> points;
    const int segmentCount = std::max(2, segments);
    points.reserve(static_cast<std::size_t>(segmentCount) + 1);

    const double first = curve.FirstParameter();
    const double last = curve.LastParameter();
    for (int i = 0; i <= segmentCount; ++i) {
        const double t = first + (last - first) * (static_cast<double>(i) / segmentCount);
        const gp_Pnt point3d = curve.Value(t);
        points.push_back(plane.toSketch({point3d.X(), point3d.Y(), point3d.Z()}));
    }
    return points;
}

// Per-wire walk state (the oracle's firstPointId/lastPointId/wireCurveCount).
struct WireWalk {
    int firstPoint = -1;
    int lastPoint = -1;
    int curveCount = 0;
    bool closedByCircle = false;

    void note(const SegmentResult& segment) {
        if (!segment.valid) {
            return;
        }
        if (firstPoint < 0) {
            firstPoint = segment.startIndex;
        }
        lastPoint = segment.endIndex;
        ++curveCount;
    }
};

void emitPolyline(Result& result, const BRepAdaptor_Curve& curve, const SketchPlane& plane,
                  const Options& options, WireWalk& walk) {
    const std::vector<Vec2d> sampled =
        sampleEdgePolyline(curve, plane, options.fallbackSegmentsPerCurve);
    for (std::size_t i = 0; i + 1 < sampled.size(); ++i) {
        walk.note(addLineSegment(result, sampled[i], sampled[i + 1], options));
    }
}

// Oracle :319-443, minus `preferExactPrimitives` (exactness is unconditional here).
void emitCircularEdge(Result& result, const TopoDS_Edge& edge, const BRepAdaptor_Curve& curve,
                      const SketchPlane& plane, const Options& options, WireWalk& walk) {
    const gp_Circ circle = curve.Circle();
    const double radius = circle.Radius();
    if (radius <= kPointEpsilon) {
        return;
    }

    const double first = curve.FirstParameter();
    const double last = curve.LastParameter();
    const double span = std::abs(last - first);
    const bool isFullCircle = edge.Closed() || nearlyEqual(span, twoPi(), 1e-3);

    const gp_Pnt center3d = circle.Location();
    const Vec2d center = plane.toSketch({center3d.X(), center3d.Y(), center3d.Z()});
    const int centerIndex = findOrCreatePoint(result, center, options.pointMergeTolerance);

    if (isFullCircle) {
        if (!circleExists(result, center, radius, options.pointMergeTolerance,
                          options.radiusTolerance)) {
            Entity full;
            full.kind = EntityKind::Circle;
            full.center = centerIndex;
            full.radius = radius;
            result.entities.push_back(full);
        }
        ++walk.curveCount;
        walk.closedByCircle = true;
        return;
    }

    const gp_Pnt start3d = curve.Value(first);
    const gp_Pnt end3d = curve.Value(last);
    const Vec2d start = plane.toSketch({start3d.X(), start3d.Y(), start3d.Z()});
    const Vec2d end = plane.toSketch({end3d.X(), end3d.Y(), end3d.Z()});
    const double startAngle = std::atan2(start.y - center.y, start.x - center.x);
    const double endAngle = std::atan2(end.y - center.y, end.x - center.x);

    // CCW from the D1 tangent at mid-parameter, crossed with the radial vector —
    // both in the PLANE's basis, so the answer follows the caller's handedness.
    bool useFallback = false;
    bool ccw = true;
    gp_Pnt mid3d;
    gp_Vec tangent3d;
    try {
        curve.D1(first + 0.5 * (last - first), mid3d, tangent3d);
    } catch (...) {
        useFallback = true;
    }

    if (!useFallback) {
        const Vec2d mid = plane.toSketch({mid3d.X(), mid3d.Y(), mid3d.Z()});
        const Vec2d radial{mid.x - center.x, mid.y - center.y};
        const double tangentU = tangent3d.X() * plane.xAxis.x + tangent3d.Y() * plane.xAxis.y +
                                tangent3d.Z() * plane.xAxis.z;
        const double tangentV = tangent3d.X() * plane.yAxis.x + tangent3d.Y() * plane.yAxis.y +
                                tangent3d.Z() * plane.yAxis.z;
        const double cross = radial.x * tangentV - radial.y * tangentU;
        if (std::abs(cross) <= 1e-9) {
            useFallback = true;
        } else {
            ccw = cross > 0.0;
        }
    }

    if (!useFallback) {
        double arcStart = startAngle;
        double arcEnd = endAngle;
        if (!ccw) {
            std::swap(arcStart, arcEnd);
        }
        const double sweep = normalizeAnglePositive(arcEnd - arcStart);
        if (sweep <= kPointEpsilon || nearlyEqual(sweep, twoPi(), 1e-3)) {
            useFallback = true;
        } else {
            if (!arcExists(result, center, radius, arcStart, arcEnd, options.pointMergeTolerance,
                           options.radiusTolerance)) {
                Entity arc;
                arc.kind = EntityKind::Arc;
                arc.center = centerIndex;
                arc.radius = radius;
                arc.startAngle = arcStart;
                arc.endAngle = arcEnd;
                arc.ccw = ccw;
                result.entities.push_back(arc);
            }
            ++walk.curveCount;
            return;
        }
    }

    emitPolyline(result, curve, plane, options, walk);
}

// Normative wire order: the face's OUTER wire first, then its holes in
// TopExp_Explorer order. (The oracle used raw explorer order; determinism of the
// wire response requires the outer wire be pinned first.)
std::vector<TopoDS_Wire> orderedWires(const TopoDS_Face& face) {
    std::vector<TopoDS_Wire> wires;
    for (TopExp_Explorer exp(face, TopAbs_WIRE); exp.More(); exp.Next()) {
        const TopoDS_Wire wire = TopoDS::Wire(exp.Current());
        if (!wire.IsNull()) {
            wires.push_back(wire);
        }
    }

    TopoDS_Wire outer;
    try {
        outer = BRepTools::OuterWire(face);
    } catch (...) {
        // Explorer order stands.
    }
    if (outer.IsNull()) {
        return wires;
    }
    for (std::size_t i = 0; i < wires.size(); ++i) {
        if (!wires[i].IsSame(outer)) {
            continue;
        }
        if (i != 0) {
            std::rotate(wires.begin(), wires.begin() + static_cast<std::ptrdiff_t>(i),
                        wires.begin() + static_cast<std::ptrdiff_t>(i) + 1);
        }
        break;
    }
    return wires;
}

void projectFace(Result& result, const TopoDS_Face& face, const SketchPlane& plane,
                 const Options& options, TopTools_MapOfShape& emittedEdges) {
    for (const TopoDS_Wire& wire : orderedWires(face)) {
        const bool wireIsClosed = wire.Closed();
        WireWalk walk;

        for (BRepTools_WireExplorer edgeExp(wire, face); edgeExp.More(); edgeExp.Next()) {
            const TopoDS_Edge edge = edgeExp.Current();
            if (edge.IsNull() || BRep_Tool::Degenerated(edge)) {
                continue;
            }
            // A TopoDS_Edge shared by two coplanar faces is emitted ONCE (IsSame).
            if (!emittedEdges.Add(edge)) {
                continue;
            }

            const BRepAdaptor_Curve curve(edge);
            const GeomAbs_CurveType curveType = curve.GetType();
            if (curveType == GeomAbs_Line) {
                const gp_Pnt p1 = curve.Value(curve.FirstParameter());
                const gp_Pnt p2 = curve.Value(curve.LastParameter());
                walk.note(addLineSegment(result,
                                         plane.toSketch({p1.X(), p1.Y(), p1.Z()}),
                                         plane.toSketch({p2.X(), p2.Y(), p2.Z()}), options));
            } else if (curveType == GeomAbs_Circle) {
                emitCircularEdge(result, edge, curve, plane, options, walk);
            } else {
                emitPolyline(result, curve, plane, options, walk);
            }
        }

        // Oracle hasClosedBoundary law (:464-470), verbatim.
        if (walk.closedByCircle) {
            result.hasClosedBoundary = true;
        } else if (walk.curveCount >= 3 &&
                   ((wireIsClosed && walk.firstPoint >= 0) ||
                    (walk.firstPoint >= 0 && walk.firstPoint == walk.lastPoint))) {
            result.hasClosedBoundary = true;
        }
    }
}

}  // namespace

FaceBoundaryProjector::Result FaceBoundaryProjector::project(
    const TopoDS_Face& seedFace, const std::vector<TopoDS_Face>& coplanarFaces,
    const SketchPlane& plane, const Options& options) {
    Result result;
    if (seedFace.IsNull()) {
        result.errorMessage = "seed face is null";
        return result;
    }

    // A non-planar seed is REFUSED, never approximated (SCHEMA §7.6).
    gp_Pln seedPlane;
    gp_Dir seedNormal;
    if (!modeling::CoplanarFacePatch::planarFacePlaneAndNormal(seedFace, seedPlane, seedNormal)) {
        result.errorMessage = "seed face is not planar";
        return result;
    }

    Options opts = options;
    if (!(opts.pointMergeTolerance > 0.0)) {
        opts.pointMergeTolerance = 1e-5;
    }
    if (!(opts.radiusTolerance > 0.0)) {
        opts.radiusTolerance = 1e-5;
    }
    if (opts.fallbackSegmentsPerCurve < 2) {
        opts.fallbackSegmentsPerCurve = 2;
    }

    // Seed first, then the supplied companions in order; nulls / repeats dropped.
    std::vector<TopoDS_Face> faces;
    faces.push_back(seedFace);
    for (const TopoDS_Face& candidate : coplanarFaces) {
        if (candidate.IsNull()) {
            continue;
        }
        bool duplicate = false;
        for (const TopoDS_Face& kept : faces) {
            if (kept.IsSame(candidate)) {
                duplicate = true;
                break;
            }
        }
        if (!duplicate) {
            faces.push_back(candidate);
        }
    }

    try {
        TopTools_MapOfShape emittedEdges;
        for (const TopoDS_Face& face : faces) {
            projectFace(result, face, plane, opts, emittedEdges);
        }
    } catch (...) {
        result.errorMessage = "face boundary projection failed";
        return result;
    }

    // Oracle :490-495.
    result.ok = result.hasClosedBoundary || !result.entities.empty();
    if (!result.ok && result.errorMessage.empty()) {
        result.errorMessage = "no host face boundary geometry was projected";
    }
    return result;
}

}  // namespace onecad::core::sketch
