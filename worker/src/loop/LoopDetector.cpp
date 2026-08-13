// Ported from OneCAD-CPP src/core/loop/LoopDetector.cpp @ b4ddcccc (2026-07-16)
#include "LoopDetector.h"

#include "AdjacencyGraph.h"

#include "../sketch/Sketch.h"
#include "../sketch/SketchArc.h"
#include "../sketch/SketchCircle.h"
#include "../sketch/SketchEllipse.h"
#include "../sketch/SketchLine.h"
#include "../sketch/SketchPoint.h"

#include <algorithm>
#include <cmath>
#include <functional>
#include <limits>
#include <numbers>
#include <optional>
#include <set>
#include <unordered_map>
#include <unordered_set>

#include <Geom2dAPI_InterCurveCurve.hxx>
#include <Geom2dAPI_ProjectPointOnCurve.hxx>
#include <GCPnts_AbscissaPoint.hxx>
#include <Geom2d_Circle.hxx>
#include <Geom2dAdaptor_Curve.hxx>
#include <Geom2d_Ellipse.hxx>
#include <Geom2d_Line.hxx>
#include <Geom2d_TrimmedCurve.hxx>
#include <gp_Ax2d.hxx>
#include <gp_Circ2d.hxx>
#include <gp_Dir2d.hxx>
#include <gp_Elips2d.hxx>
#include <gp_Lin2d.hxx>

namespace onecad::core::loop {

constexpr double kMinArea = 1e-6;

sk::Vec2d toVec2(const gp_Pnt2d& p) {
    return {p.X(), p.Y()};
}

double distanceSquared(const sk::Vec2d& a, const sk::Vec2d& b) {
    double dx = a.x - b.x;
    double dy = a.y - b.y;
    return dx * dx + dy * dy;
}

double computeArcSweep(double startAngle, double endAngle, bool forward) {
    double sweep = endAngle - startAngle;
    if (forward) {
        if (sweep < 0.0) {
            sweep += 2.0 * std::numbers::pi_v<double>;
        }
    } else {
        if (sweep > 0.0) {
            sweep -= 2.0 * std::numbers::pi_v<double>;
        }
    }
    return sweep;
}

double computeTessellationTolerance(double radius, const LoopDetectorConfig& config) {
    double absRadius = std::abs(radius);
    double tol = config.tessellationTolerance;
    if (config.tessellationRelative > 0.0 && absRadius > 0.0) {
        double relativeTol = absRadius * config.tessellationRelative;
        tol = (tol > 0.0) ? std::min(tol, relativeTol) : relativeTol;
    }
    if (tol <= 0.0) {
        tol = 1e-6;
    }
    return tol;
}

int computeSegmentsForSweep(double radius, double absSweep,
                            int minSegments, int maxSegments,
                            const LoopDetectorConfig& config) {
    int minSeg = std::max(1, minSegments);
    int maxSeg = std::max(minSeg, maxSegments);
    if (absSweep <= 0.0) {
        return minSeg;
    }

    double absRadius = std::abs(radius);
    int segments = minSeg;
    if (absRadius > 0.0) {
        double tol = computeTessellationTolerance(absRadius, config);
        double ratio = 1.0 - tol / absRadius;
        ratio = std::clamp(ratio, -1.0, 1.0);
        double step = 2.0 * std::acos(ratio);
        if (std::isfinite(step) && step > 1e-6) {
            segments = static_cast<int>(std::ceil(absSweep / step));
        } else {
            segments = maxSeg;
        }
    }

    return std::clamp(segments, minSeg, maxSeg);
}

int computeArcSegments(double radius, double absSweep, const LoopDetectorConfig& config) {
    return computeSegmentsForSweep(radius, absSweep,
                                   config.minArcSegments, config.maxArcSegments,
                                   config);
}

int computeCircleSegments(double radius, const LoopDetectorConfig& config) {
    int minSeg = std::max(3, config.minCircleSegments);
    int maxSeg = std::max(minSeg, config.maxCircleSegments);
    return computeSegmentsForSweep(radius, 2.0 * std::numbers::pi_v<double>,
                                   minSeg, maxSeg,
                                   config);
}

void appendArcSamples(std::vector<sk::Vec2d>& points,
                      const sk::Vec2d& center,
                      double radius,
                      double startAngle,
                      double endAngle,
                      bool forward,
                      const LoopDetectorConfig& config) {
    double sweep = computeArcSweep(startAngle, endAngle, forward);
    double absSweep = std::abs(sweep);
    int segments = computeArcSegments(radius, absSweep, config);
    if (segments <= 0) {
        return;
    }
    double step = sweep / static_cast<double>(segments);
    for (int i = 1; i <= segments; ++i) {
        double angle = startAngle + step * static_cast<double>(i);
        points.push_back({center.x + radius * std::cos(angle),
                          center.y + radius * std::sin(angle)});
    }
}

struct BroadSegment {
    sk::Vec2d start;
    sk::Vec2d end;
    sk::EntityID baseId;
    std::size_t sourceIndex = 0;
};

struct AnalyticSource {
    CurveFragment fragment;
    Handle(Geom2d_Curve) basis;
    Handle(Geom2d_Curve) bounded;
    bool closed = false;
};

struct CurveSplit {
    double parameter = 0.0;
    sk::Vec2d point{};
};

struct ProfileTolerancePolicy {
    double coincidence = 1e-7;

    static ProfileTolerancePolicy forSources(const std::vector<AnalyticSource>& sources,
                                             double configuredMaximum) {
        double minimumX = std::numeric_limits<double>::infinity();
        double minimumY = std::numeric_limits<double>::infinity();
        double maximumX = -std::numeric_limits<double>::infinity();
        double maximumY = -std::numeric_limits<double>::infinity();
        for (const AnalyticSource& source : sources) {
            const double span = source.fragment.lastParameter - source.fragment.firstParameter;
            for (const double fraction : {0.0, 0.25, 0.5, 0.75, 1.0}) {
                const gp_Pnt2d point = source.basis->Value(
                    source.fragment.firstParameter + span * fraction);
                minimumX = std::min(minimumX, point.X());
                minimumY = std::min(minimumY, point.Y());
                maximumX = std::max(maximumX, point.X());
                maximumY = std::max(maximumY, point.Y());
            }
        }
        const double extent = std::max({maximumX - minimumX, maximumY - minimumY, 1.0});
        const double maximum = std::max(configuredMaximum, 1e-9);
        return {std::clamp(extent * 1e-7, std::min(1e-7, maximum), maximum)};
    }
};

bool shareAuthoredEndpoint(const AnalyticSource& first, const AnalyticSource& second,
                           double tolerance) {
    if (first.closed || second.closed) return false;
    const double toleranceSquared = tolerance * tolerance;
    return distanceSquared(first.fragment.startPoint, second.fragment.startPoint) <= toleranceSquared ||
           distanceSquared(first.fragment.startPoint, second.fragment.endPoint) <= toleranceSquared ||
           distanceSquared(first.fragment.endPoint, second.fragment.startPoint) <= toleranceSquared ||
           distanceSquared(first.fragment.endPoint, second.fragment.endPoint) <= toleranceSquared;
}

bool pointsShareSupport(const AnalyticSource& source, const AnalyticSource& other,
                        double tolerance) {
    const double span = source.fragment.lastParameter - source.fragment.firstParameter;
    for (const double fraction : {0.0, 0.37, 0.73}) {
        const gp_Pnt2d point = source.basis->Value(source.fragment.firstParameter + span * fraction);
        Geom2dAPI_ProjectPointOnCurve projection(point, other.basis);
        if (projection.NbPoints() == 0 || projection.LowerDistance() > tolerance) return false;
    }
    return true;
}

bool shareAnalyticSupport(const AnalyticSource& first, const AnalyticSource& second,
                          double tolerance) {
    return pointsShareSupport(first, second, tolerance) &&
           pointsShareSupport(second, first, tolerance);
}

struct SplitPoint {
    double t = 0.0;
    sk::Vec2d point{0.0, 0.0};
};

sk::Vec2d diff(const sk::Vec2d& a, const sk::Vec2d& b) {
    return {a.x - b.x, a.y - b.y};
}

double cross2d(const sk::Vec2d& a, const sk::Vec2d& b) {
    return a.x * b.y - a.y * b.x;
}

double segmentParam(const sk::Vec2d& a, const sk::Vec2d& b, const sk::Vec2d& p) {
    sk::Vec2d ab = diff(b, a);
    double denom = ab.x * ab.x + ab.y * ab.y;
    if (denom <= 0.0) {
        return 0.0;
    }
    return ((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / denom;
}

bool pointOnSegment(const sk::Vec2d& a, const sk::Vec2d& b, const sk::Vec2d& p, double tolerance) {
    sk::Vec2d ab = diff(b, a);
    sk::Vec2d ap = diff(p, a);
    if (std::abs(cross2d(ab, ap)) > tolerance) {
        return false;
    }
    double dot = (p.x - a.x) * (p.x - b.x) + (p.y - a.y) * (p.y - b.y);
    return dot <= tolerance * tolerance;
}

void addSplitPoint(std::vector<SplitPoint>& splits, double t, const sk::Vec2d& point,
                   double tolerance) {
    t = std::clamp(t, 0.0, 1.0);
    double tol2 = tolerance * tolerance;
    for (const auto& existing : splits) {
        if (distanceSquared(existing.point, point) <= tol2) {
            return;
        }
    }
    splits.push_back({t, point});
}

bool segmentIntersection(const sk::Vec2d& p1, const sk::Vec2d& p2,
                         const sk::Vec2d& q1, const sk::Vec2d& q2,
                         double tolerance,
                         double& tOut, double& uOut, sk::Vec2d& outPoint) {
    sk::Vec2d r = diff(p2, p1);
    sk::Vec2d s = diff(q2, q1);
    double denom = cross2d(r, s);
    if (std::abs(denom) <= tolerance) {
        return false;
    }

    sk::Vec2d qp = diff(q1, p1);
    double t = cross2d(qp, s) / denom;
    double u = cross2d(qp, r) / denom;
    if (t < -tolerance || t > 1.0 + tolerance ||
        u < -tolerance || u > 1.0 + tolerance) {
        return false;
    }

    t = std::clamp(t, 0.0, 1.0);
    u = std::clamp(u, 0.0, 1.0);
    outPoint = {p1.x + t * r.x, p1.y + t * r.y};
    tOut = t;
    uOut = u;
    return true;
}

std::vector<sk::Vec2d> tessellateArcPoints(const sk::Vec2d& center,
                                           double radius,
                                           double startAngle,
                                           double endAngle,
                                           const LoopDetectorConfig& config) {
    std::vector<sk::Vec2d> points;
    points.reserve(16);
    points.push_back({center.x + radius * std::cos(startAngle),
                      center.y + radius * std::sin(startAngle)});
    appendArcSamples(points, center, radius, startAngle, endAngle, true, config);
    return points;
}

std::vector<sk::Vec2d> tessellateCirclePoints(const sk::Vec2d& center,
                                               double radius,
                                               const LoopDetectorConfig& config) {
    std::vector<sk::Vec2d> points;
    int segments = computeCircleSegments(radius, config);
    points.reserve(static_cast<size_t>(segments) + 1);
    for (int i = 0; i <= segments; ++i) {
        double angle = (2.0 * std::numbers::pi_v<double> * i) / static_cast<double>(segments);
        points.push_back({center.x + radius * std::cos(angle),
                          center.y + radius * std::sin(angle)});
    }
    return points;
}

std::vector<sk::Vec2d> tessellateEllipsePoints(const sk::Vec2d& center,
                                               double majorRadius,
                                               double minorRadius,
                                               double rotation,
                                               const LoopDetectorConfig& config) {
    std::vector<sk::Vec2d> points;
    const double radiusForTolerance = std::max(std::abs(majorRadius), std::abs(minorRadius));
    int segments = computeCircleSegments(radiusForTolerance, config);
    points.reserve(static_cast<size_t>(segments) + 1);

    const double cosR = std::cos(rotation);
    const double sinR = std::sin(rotation);
    for (int i = 0; i <= segments; ++i) {
        double angle = (2.0 * std::numbers::pi_v<double> * i) / static_cast<double>(segments);
        double localX = majorRadius * std::cos(angle);
        double localY = minorRadius * std::sin(angle);
        points.push_back({center.x + localX * cosR - localY * sinR,
                          center.y + localX * sinR + localY * cosR});
    }
    return points;
}

std::optional<AnalyticSource> makeAnalyticSource(const sk::SketchEntity& entity,
                                                  const sk::Sketch& sketch) {
    AnalyticSource source;
    source.fragment.baseEntityId = entity.id();

    if (entity.type() == sk::EntityType::Line) {
        const auto* line = dynamic_cast<const sk::SketchLine*>(&entity);
        if (!line) return std::nullopt;
        const auto* start = sketch.getEntityAs<sk::SketchPoint>(line->startPointId());
        const auto* end = sketch.getEntityAs<sk::SketchPoint>(line->endPointId());
        if (!start || !end) return std::nullopt;
        const sk::Vec2d a = toVec2(start->position());
        const sk::Vec2d b = toVec2(end->position());
        const double length = std::sqrt(distanceSquared(a, b));
        if (length <= 0.0) return std::nullopt;
        source.fragment.kind = CurveFragmentKind::Line;
        source.fragment.firstParameter = 0.0;
        source.fragment.lastParameter = length;
        source.fragment.startPoint = a;
        source.fragment.endPoint = b;
        source.basis = new Geom2d_Line(gp_Lin2d(gp_Pnt2d(a.x, a.y),
                                                gp_Dir2d((b.x - a.x) / length,
                                                         (b.y - a.y) / length)));
        source.bounded = new Geom2d_TrimmedCurve(source.basis, 0.0, length);
        source.fragment.sourceFirstParameter = source.fragment.firstParameter;
        source.fragment.sourceLastParameter = source.fragment.lastParameter;
        return source;
    }

    const sk::SketchPoint* centerPoint = nullptr;
    if (entity.type() == sk::EntityType::Arc) {
        const auto* arc = dynamic_cast<const sk::SketchArc*>(&entity);
        if (!arc || arc->radius() <= 0.0) return std::nullopt;
        centerPoint = sketch.getEntityAs<sk::SketchPoint>(arc->centerPointId());
        if (!centerPoint) return std::nullopt;
        const sk::Vec2d center = toVec2(centerPoint->position());
        source.fragment.kind = CurveFragmentKind::Arc;
        source.fragment.firstParameter = arc->startAngle();
        source.fragment.lastParameter = arc->startAngle() + arc->sweepAngle();
        source.fragment.startPoint = {center.x + arc->radius() * std::cos(source.fragment.firstParameter),
                                      center.y + arc->radius() * std::sin(source.fragment.firstParameter)};
        source.fragment.endPoint = {center.x + arc->radius() * std::cos(source.fragment.lastParameter),
                                    center.y + arc->radius() * std::sin(source.fragment.lastParameter)};
        source.basis = new Geom2d_Circle(gp_Circ2d(gp_Ax2d(gp_Pnt2d(center.x, center.y),
                                                           gp_Dir2d(1.0, 0.0)),
                                                  arc->radius()));
        source.bounded = new Geom2d_TrimmedCurve(source.basis, source.fragment.firstParameter,
                                                   source.fragment.lastParameter);
        source.fragment.sourceFirstParameter = source.fragment.firstParameter;
        source.fragment.sourceLastParameter = source.fragment.lastParameter;
        return source;
    }

    if (entity.type() == sk::EntityType::Circle) {
        const auto* circle = dynamic_cast<const sk::SketchCircle*>(&entity);
        if (!circle || circle->radius() <= 0.0) return std::nullopt;
        centerPoint = sketch.getEntityAs<sk::SketchPoint>(circle->centerPointId());
        if (!centerPoint) return std::nullopt;
        const sk::Vec2d center = toVec2(centerPoint->position());
        source.fragment.kind = CurveFragmentKind::Circle;
        source.fragment.firstParameter = 0.0;
        source.fragment.lastParameter = 2.0 * std::numbers::pi_v<double>;
        source.fragment.startPoint = {center.x + circle->radius(), center.y};
        source.fragment.endPoint = source.fragment.startPoint;
        source.basis = new Geom2d_Circle(gp_Circ2d(gp_Ax2d(gp_Pnt2d(center.x, center.y),
                                                           gp_Dir2d(1.0, 0.0)),
                                                  circle->radius()));
        source.bounded = new Geom2d_TrimmedCurve(source.basis, 0.0,
                                                   source.fragment.lastParameter);
        source.closed = true;
        source.fragment.sourceFirstParameter = source.fragment.firstParameter;
        source.fragment.sourceLastParameter = source.fragment.lastParameter;
        return source;
    }

    if (entity.type() != sk::EntityType::Ellipse) return std::nullopt;
    const auto* ellipse = dynamic_cast<const sk::SketchEllipse*>(&entity);
    if (!ellipse || ellipse->majorRadius() <= 0.0 || ellipse->minorRadius() <= 0.0) {
        return std::nullopt;
    }
    centerPoint = sketch.getEntityAs<sk::SketchPoint>(ellipse->centerPointId());
    if (!centerPoint) return std::nullopt;
    const sk::Vec2d center = toVec2(centerPoint->position());
    const double rotation = ellipse->rotation();
    source.fragment.kind = CurveFragmentKind::Ellipse;
    source.fragment.firstParameter = 0.0;
    source.fragment.lastParameter = 2.0 * std::numbers::pi_v<double>;
    source.fragment.startPoint = {center.x + ellipse->majorRadius() * std::cos(rotation),
                                  center.y + ellipse->majorRadius() * std::sin(rotation)};
    source.fragment.endPoint = source.fragment.startPoint;
    source.basis = new Geom2d_Ellipse(
        gp_Elips2d(gp_Ax2d(gp_Pnt2d(center.x, center.y),
                           gp_Dir2d(std::cos(rotation), std::sin(rotation))),
                   ellipse->majorRadius(), ellipse->minorRadius()));
    source.bounded = new Geom2d_TrimmedCurve(source.basis, 0.0,
                                               source.fragment.lastParameter);
    source.closed = true;
    source.fragment.sourceFirstParameter = source.fragment.firstParameter;
    source.fragment.sourceLastParameter = source.fragment.lastParameter;
    return source;
}

std::optional<double> physicalCurveIntervalLength(const AnalyticSource& source,
                                                  double firstParameter,
                                                  double lastParameter,
                                                  double tolerance) {
    try {
        Geom2dAdaptor_Curve adaptor(source.basis);
        const double distance = GCPnts_AbscissaPoint::Length(
            adaptor, firstParameter, lastParameter, tolerance);
        if (!std::isfinite(distance)) return std::nullopt;
        return distance;
    } catch (const Standard_Failure&) {
        return std::nullopt;
    }
}

std::optional<double> physicalCurveProximity(const AnalyticSource& source,
                                             double firstParameter,
                                             double lastParameter,
                                             double tolerance) {
    const double low = std::min(firstParameter, lastParameter);
    const double high = std::max(firstParameter, lastParameter);
    const std::optional<double> direct =
        physicalCurveIntervalLength(source, low, high, tolerance);
    if (!direct.has_value() || !source.closed) return direct;
    const std::optional<double> before = physicalCurveIntervalLength(
        source, source.fragment.sourceFirstParameter, low, tolerance);
    const std::optional<double> after = physicalCurveIntervalLength(
        source, high, source.fragment.sourceLastParameter, tolerance);
    if (!before.has_value() || !after.has_value()) return std::nullopt;
    return std::min(*direct, *before + *after);
}

bool addCurveSplit(std::vector<CurveSplit>& splits, const AnalyticSource& source,
                   double parameter, const sk::Vec2d& point, double tolerance,
                   CurveRefinementPolicy policy) {
    for (const CurveSplit& existing : splits) {
        if (policy == CurveRefinementPolicy::V2ParameterProximity &&
            std::abs(existing.parameter - parameter) <= tolerance) return true;
        if (policy == CurveRefinementPolicy::V2ParameterProximity) continue;
        const std::optional<double> distance = physicalCurveProximity(
            source, existing.parameter, parameter, tolerance);
        if (!distance.has_value()) return false;
        const bool same = *distance <= tolerance;
        if (same) return true;
    }
    splits.push_back({parameter, point});
    return true;
}

bool exactFragmentIsContinuous(const AnalyticSource& source,
                               const CurveSplit& start,
                               const CurveSplit& end,
                               double tolerance) {
    try {
        const gp_Pnt2d exactStart = source.basis->Value(start.parameter);
        const gp_Pnt2d exactEnd = source.basis->Value(end.parameter);
        const gp_Vec2d startTangent = source.basis->DN(start.parameter, 1);
        const gp_Vec2d endTangent = source.basis->DN(end.parameter, 1);
        const auto finiteTangent = [](const gp_Vec2d& tangent) {
            return std::isfinite(tangent.X()) && std::isfinite(tangent.Y()) &&
                   tangent.SquareMagnitude() > 0.0;
        };
        return exactStart.Distance(gp_Pnt2d(start.point.x, start.point.y)) <= tolerance &&
               exactEnd.Distance(gp_Pnt2d(end.point.x, end.point.y)) <= tolerance &&
               finiteTangent(startTangent) && finiteTangent(endTangent);
    } catch (const Standard_Failure&) {
        return false;
    }
}

std::optional<double> projectCurveParameter(const AnalyticSource& source,
                                            const gp_Pnt2d& point,
                                            double tolerance) {
    Geom2dAPI_ProjectPointOnCurve projector(point, source.basis,
                                             source.fragment.firstParameter,
                                             source.fragment.lastParameter);
    if (projector.NbPoints() == 0 || projector.LowerDistance() > tolerance) {
        return std::nullopt;
    }
    return projector.LowerDistanceParameter();
}

std::string makeCycleKey(const std::vector<sk::EntityID>& edges) {
    std::vector<sk::EntityID> sorted = edges;
    std::sort(sorted.begin(), sorted.end());
    std::string key;
    key.reserve(sorted.size() * 40);
    for (const auto& id : sorted) {
        key.append(id);
        key.push_back('|');
    }
    return key;
}

bool segmentsIntersect(const sk::Vec2d& a1, const sk::Vec2d& a2,
                       const sk::Vec2d& b1, const sk::Vec2d& b2) {
    auto cross = [](const sk::Vec2d& a, const sk::Vec2d& b, const sk::Vec2d& c) {
        return (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    };

    auto onSegment = [](const sk::Vec2d& a, const sk::Vec2d& b, const sk::Vec2d& c) {
        return std::min(a.x, b.x) <= c.x && c.x <= std::max(a.x, b.x) &&
               std::min(a.y, b.y) <= c.y && c.y <= std::max(a.y, b.y);
    };

    double d1 = cross(a1, a2, b1);
    double d2 = cross(a1, a2, b2);
    double d3 = cross(b1, b2, a1);
    double d4 = cross(b1, b2, a2);

    if (((d1 > 0 && d2 < 0) || (d1 < 0 && d2 > 0)) &&
        ((d3 > 0 && d4 < 0) || (d3 < 0 && d4 > 0))) {
        return true;
    }

    if (d1 == 0.0 && onSegment(a1, a2, b1)) return true;
    if (d2 == 0.0 && onSegment(a1, a2, b2)) return true;
    if (d3 == 0.0 && onSegment(b1, b2, a1)) return true;
    if (d4 == 0.0 && onSegment(b1, b2, a2)) return true;

    return false;
}

double distanceToSegmentSquared(const sk::Vec2d& a, const sk::Vec2d& b, const sk::Vec2d& p) {
    sk::Vec2d ab{b.x - a.x, b.y - a.y};
    double denom = ab.x * ab.x + ab.y * ab.y;
    if (denom == 0.0) {
        return distanceSquared(a, p);
    }
    double t = ((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / denom;
    t = std::clamp(t, 0.0, 1.0);
    sk::Vec2d proj{a.x + t * ab.x, a.y + t * ab.y};
    return distanceSquared(p, proj);
}

bool isPointInPolygonOrOnEdge(const sk::Vec2d& point,
                              const std::vector<sk::Vec2d>& polygon,
                              double tolerance) {
    if (isPointInPolygon(point, polygon)) {
        return true;
    }
    if (polygon.size() < 2) {
        return false;
    }
    double tol2 = tolerance * tolerance;
    for (size_t i = 0; i < polygon.size(); ++i) {
        size_t next = (i + 1) % polygon.size();
        if (distanceToSegmentSquared(polygon[i], polygon[next], point) <= tol2) {
            return true;
        }
    }
    return false;
}

bool loopContainsLoop(const Loop& outer, const Loop& inner, double tolerance) {
    if (outer.polygon.size() < 3 || inner.polygon.size() < 3) {
        return false;
    }
    if (inner.boundsMin.x < outer.boundsMin.x - tolerance ||
        inner.boundsMin.y < outer.boundsMin.y - tolerance ||
        inner.boundsMax.x > outer.boundsMax.x + tolerance ||
        inner.boundsMax.y > outer.boundsMax.y + tolerance) {
        return false;
    }
    for (const auto& p : inner.polygon) {
        if (!isPointInPolygonOrOnEdge(p, outer.polygon, tolerance)) {
            return false;
        }
    }
    return true;
}

void reverseLoop(Loop& loop) {
    std::reverse(loop.wire.edges.begin(), loop.wire.edges.end());
    std::reverse(loop.wire.forward.begin(), loop.wire.forward.end());
    for (size_t i = 0; i < loop.wire.forward.size(); ++i) {
        loop.wire.forward[i] = !loop.wire.forward[i];
    }
    std::reverse(loop.fragments.begin(), loop.fragments.end());
    for (CurveFragment& fragment : loop.fragments) {
        fragment.forward = !fragment.forward;
    }
    std::reverse(loop.polygon.begin(), loop.polygon.end());
    loop.signedArea = -loop.signedArea;
}

LoopDetector::LoopDetector()
    : config_() {
}

LoopDetector::LoopDetector(const LoopDetectorConfig& config)
    : config_(config) {
}

LoopDetectionResult LoopDetector::detect(const sk::Sketch& sketch) const {
    return detect(sketch, {});
}

LoopDetectionResult LoopDetector::detect(const sk::Sketch& sketch,
                                         const std::vector<sk::EntityID>& selectedEntities) const {
    LoopDetectionResult result;

    std::unordered_set<sk::EntityID> selection;
    if (!selectedEntities.empty()) {
        selection.insert(selectedEntities.begin(), selectedEntities.end());
    }

    auto graph = buildGraph(sketch, selection.empty() ? nullptr : &selection,
                            config_.planarizeIntersections);
    if (!graph) {
        result.success = false;
        result.errorMessage = "Failed to build adjacency graph";
        return result;
    }
    if (!graph->errorMessage.empty()) {
        result.success = false;
        result.errorMessage = graph->errorMessage;
        return result;
    }

    std::vector<Loop> loops;
    std::unordered_set<sk::EntityID> edgesInLoops;

    if (config_.planarizeIntersections) {
        loops = findFaces(*graph);
        for (const CurveFragment& fragment : graph->closedCurves) {
            Loop closedCurveLoop;
            closedCurveLoop.wire.edges.push_back(fragment.baseEntityId);
            closedCurveLoop.wire.forward.push_back(true);
            closedCurveLoop.wire.startPoint = fragment.baseEntityId;
            closedCurveLoop.wire.endPoint = fragment.baseEntityId;
            closedCurveLoop.fragments.push_back(fragment);
            computeLoopProperties(closedCurveLoop, sketch);
            loops.push_back(std::move(closedCurveLoop));
        }
        if (config_.maxLoops > 0 && loops.size() > config_.maxLoops) {
            loops.resize(config_.maxLoops);
        }
        for (auto& loop : loops) {
            if (config_.validate && !validateLoop(loop, sketch)) {
                if (!config_.findAllLoops) {
                    loop.polygon.clear();
                    continue;
                }
            }
            if (!config_.findAllLoops && std::abs(loop.signedArea) < kMinArea) {
                loop.polygon.clear();
                continue;
            }
            for (const auto& id : loop.wire.edges) {
                edgesInLoops.insert(id);
            }
        }
        loops.erase(std::remove_if(loops.begin(), loops.end(),
                                   [](const Loop& loop) {
                                       return loop.polygon.empty();
                                   }),
                    loops.end());
    } else {
        std::vector<Wire> cycles = findCycles(*graph);
        for (auto& wire : cycles) {
            if (config_.maxLoops > 0 && loops.size() >= config_.maxLoops) {
                break;
            }

            Loop loop;
            loop.wire = wire;
            computeLoopProperties(loop, sketch);

            if (config_.validate && !validateLoop(loop, sketch)) {
                if (!config_.findAllLoops) {
                    continue;
                }
            }

            if (!config_.findAllLoops && std::abs(loop.signedArea) < kMinArea) {
                continue;
            }

            loops.push_back(loop);
            for (const auto& id : loop.wire.edges) {
                edgesInLoops.insert(id);
            }
        }

        for (const auto& entity : sketch.getAllEntities()) {
            // REFERENCE-LOCK PIN: `isConstruction` is the ONLY entity flag that excludes
            // geometry here. Reference-locked (host-face projected) geometry MUST keep
            // bounding regions — a profile extruded off the face boundary depends on it.
            if (!entity || entity->isConstruction()) {
                continue;
            }
            if (!selection.empty() && selection.find(entity->id()) == selection.end()) {
                continue;
            }
            if (entity->type() != sk::EntityType::Circle && entity->type() != sk::EntityType::Ellipse) {
                continue;
            }
            if (edgesInLoops.find(entity->id()) != edgesInLoops.end()) {
                continue;
            }

            Loop closedCurveLoop;
            closedCurveLoop.wire.edges.push_back(entity->id());
            closedCurveLoop.wire.forward.push_back(true);
            closedCurveLoop.wire.startPoint = entity->id();
            closedCurveLoop.wire.endPoint = entity->id();
            computeLoopProperties(closedCurveLoop, sketch);

            if (config_.validate && !validateLoop(closedCurveLoop, sketch)) {
                if (!config_.findAllLoops) {
                    continue;
                }
            }

            loops.push_back(closedCurveLoop);
            edgesInLoops.insert(entity->id());
        }
    }

    result.totalLoopsFound = static_cast<int>(loops.size());

    if (config_.resolveHoles) {
        result.faces = buildFaceHierarchy(std::move(loops));
        for (const auto& face : result.faces) {
            if (!face.innerLoops.empty()) {
                result.facesWithHoles++;
            }
        }
    } else {
        for (auto& loop : loops) {
            Face face;
            face.outerLoop = std::move(loop);
            result.faces.push_back(std::move(face));
        }
    }

    std::unordered_set<int> usedEdges;
    for (const auto& face : result.faces) {
        for (const auto& edge : face.outerLoop.wire.edges) {
            auto it = graph->edgeByEntity.find(edge);
            if (it != graph->edgeByEntity.end()) {
                usedEdges.insert(it->second);
            }
        }
        for (const auto& hole : face.innerLoops) {
            for (const auto& edge : hole.wire.edges) {
                auto it = graph->edgeByEntity.find(edge);
                if (it != graph->edgeByEntity.end()) {
                    usedEdges.insert(it->second);
                }
            }
        }
    }

    std::unordered_set<int> openUsed;
    for (size_t i = 0; i < graph->nodes.size(); ++i) {
        if (graph->nodes[i].edges.size() != 1) {
            continue;
        }
        int current = static_cast<int>(i);
        int prevEdge = -1;
        Wire wire;
        while (true) {
            int nextEdge = -1;
            for (int edgeIndex : graph->nodes[current].edges) {
                if (edgeIndex == prevEdge || usedEdges.count(edgeIndex) || openUsed.count(edgeIndex)) {
                    continue;
                }
                nextEdge = edgeIndex;
                break;
            }
            if (nextEdge == -1) {
                break;
            }

            const auto& edge = graph->edges[nextEdge];
            int nextNode = (edge.startNode == current) ? edge.endNode : edge.startNode;
            wire.edges.push_back(edge.entityId);
            wire.forward.push_back(edge.startNode == current && edge.endNode == nextNode);
            openUsed.insert(nextEdge);
            prevEdge = nextEdge;
            current = nextNode;
        }

        if (!wire.edges.empty()) {
            wire.startPoint = graph->nodes[i].id;
            wire.endPoint = graph->nodes[current].id;
            result.openWires.push_back(std::move(wire));
        }
    }

    for (size_t edgeIndex = 0; edgeIndex < graph->edges.size(); ++edgeIndex) {
        if (usedEdges.count(static_cast<int>(edgeIndex)) || openUsed.count(static_cast<int>(edgeIndex))) {
            continue;
        }
        result.unusedEdges.push_back(graph->edges[edgeIndex].entityId);
    }

    for (const auto& entity : sketch.getAllEntities()) {
        if (!entity || entity->type() != sk::EntityType::Point) {
            continue;
        }
        auto* point = dynamic_cast<const sk::SketchPoint*>(entity.get());
        if (!point) {
            continue;
        }

        bool referenced = false;
        for (const auto& e : sketch.getAllEntities()) {
            // REFERENCE-LOCK PIN: `isConstruction` is the ONLY entity flag that excludes
            // geometry here. Reference-locked (host-face projected) geometry MUST keep
            // bounding regions — a profile extruded off the face boundary depends on it.
            if (!e || e->isConstruction()) {
                continue;
            }
            if (e->type() == sk::EntityType::Line) {
                auto* line = dynamic_cast<const sk::SketchLine*>(e.get());
                if (line && (line->startPointId() == point->id() || line->endPointId() == point->id())) {
                    referenced = true;
                    break;
                }
            } else if (e->type() == sk::EntityType::Arc) {
                auto* arc = dynamic_cast<const sk::SketchArc*>(e.get());
                if (arc && arc->centerPointId() == point->id()) {
                    referenced = true;
                    break;
                }
            } else if (e->type() == sk::EntityType::Circle) {
                auto* circle = dynamic_cast<const sk::SketchCircle*>(e.get());
                if (circle && circle->centerPointId() == point->id()) {
                    referenced = true;
                    break;
                }
            }
        }

        if (!referenced) {
            result.isolatedPoints.push_back(point->id());
        }
    }

    return result;
}

std::optional<Face> LoopDetector::findLoopAtPoint(const sk::Sketch& sketch,
                                                  const sk::Vec2d& point) const {
    LoopDetectionResult result = detect(sketch);
    double bestArea = std::numeric_limits<double>::infinity();
    std::optional<Face> bestFace;

    for (const auto& face : result.faces) {
        if (!face.outerLoop.contains(point)) {
            continue;
        }
        bool inHole = false;
        for (const auto& hole : face.innerLoops) {
            if (hole.contains(point)) {
                inHole = true;
                break;
            }
        }
        if (inHole) {
            continue;
        }
        double area = face.outerLoop.area();
        if (area < bestArea) {
            bestArea = area;
            bestFace = face;
        }
    }

    return bestFace;
}

bool LoopDetector::isClosedLoop(const sk::Sketch& sketch,
                                const std::vector<sk::EntityID>& entities) const {
    auto wire = buildWire(sketch, entities);
    return wire && wire->isClosed();
}

std::optional<Wire> LoopDetector::buildWire(const sk::Sketch& sketch,
                                            const std::vector<sk::EntityID>& entities) const {
    if (entities.empty()) {
        return std::nullopt;
    }

    std::unordered_set<sk::EntityID> selection(entities.begin(), entities.end());
    auto graph = buildGraph(sketch, &selection, false);
    if (!graph) {
        return std::nullopt;
    }

    std::vector<int> edgeIndices;
    edgeIndices.reserve(entities.size());
    for (const auto& id : entities) {
        auto it = graph->edgeByEntity.find(id);
        if (it != graph->edgeByEntity.end()) {
            edgeIndices.push_back(it->second);
        }
    }

    if (edgeIndices.empty()) {
        return std::nullopt;
    }

    int startNode = -1;
    for (int edgeIndex : edgeIndices) {
        const auto& edge = graph->edges[edgeIndex];
        if (graph->nodes[edge.startNode].edges.size() == 1) {
            startNode = edge.startNode;
            break;
        }
        if (graph->nodes[edge.endNode].edges.size() == 1) {
            startNode = edge.endNode;
            break;
        }
    }
    if (startNode < 0) {
        startNode = graph->edges[edgeIndices.front()].startNode;
    }

    std::unordered_set<int> allowed(edgeIndices.begin(), edgeIndices.end());
    std::unordered_set<int> visited;

    Wire wire;
    int current = startNode;
    int prevEdge = -1;
    while (true) {
        int nextEdge = -1;
        for (int edgeIndex : graph->nodes[current].edges) {
            if (edgeIndex == prevEdge || !allowed.count(edgeIndex) || visited.count(edgeIndex)) {
                continue;
            }
            nextEdge = edgeIndex;
            break;
        }
        if (nextEdge == -1) {
            break;
        }

        const auto& edge = graph->edges[nextEdge];
        int nextNode = (edge.startNode == current) ? edge.endNode : edge.startNode;
        wire.edges.push_back(edge.entityId);
        wire.forward.push_back(edge.startNode == current && edge.endNode == nextNode);
        visited.insert(nextEdge);
        prevEdge = nextEdge;
        current = nextNode;
    }

    if (visited.size() != allowed.size()) {
        return std::nullopt;
    }

    wire.startPoint = graph->nodes[startNode].id;
    wire.endPoint = graph->nodes[current].id;
    return wire;
}

std::unique_ptr<AdjacencyGraph> LoopDetector::buildGraph(
    const sk::Sketch& sketch,
    const std::unordered_set<sk::EntityID>* selection,
    bool planarize) const {
    auto graph = std::make_unique<AdjacencyGraph>();
    double tolerance = config_.coincidenceTolerance;

    if (!planarize) {
        for (const auto& entity : sketch.getAllEntities()) {
            // REFERENCE-LOCK PIN: `isConstruction` is the ONLY entity flag that excludes
            // geometry here. Reference-locked (host-face projected) geometry MUST keep
            // bounding regions — a profile extruded off the face boundary depends on it.
            if (!entity || entity->isConstruction()) {
                continue;
            }
            if (selection && !selection->empty() && selection->find(entity->id()) == selection->end()) {
                continue;
            }

            if (entity->type() == sk::EntityType::Line) {
                auto* line = dynamic_cast<const sk::SketchLine*>(entity.get());
                if (!line) {
                    continue;
                }
                auto* start = sketch.getEntityAs<sk::SketchPoint>(line->startPointId());
                auto* end = sketch.getEntityAs<sk::SketchPoint>(line->endPointId());
                if (!start || !end) {
                    continue;
                }

                sk::Vec2d startPos = toVec2(start->position());
                sk::Vec2d endPos = toVec2(end->position());

                int startNode = graph->findOrCreateNode(startPos, line->startPointId(), tolerance);
                int endNode = graph->findOrCreateNode(endPos, line->endPointId(), tolerance);

                GraphEdge edge;
                edge.entityId = line->id();
                edge.startNode = startNode;
                edge.endNode = endNode;
                edge.startPos = startPos;
                edge.endPos = endPos;
                graph->edgeByEntity[edge.entityId] = static_cast<int>(graph->edges.size());
                graph->edges.push_back(std::move(edge));

                graph->nodes[startNode].edges.push_back(static_cast<int>(graph->edges.size() - 1));
                graph->nodes[endNode].edges.push_back(static_cast<int>(graph->edges.size() - 1));
            } else if (entity->type() == sk::EntityType::Arc) {
                auto* arc = dynamic_cast<const sk::SketchArc*>(entity.get());
                if (!arc) {
                    continue;
                }
                auto* centerPoint = sketch.getEntityAs<sk::SketchPoint>(arc->centerPointId());
                if (!centerPoint) {
                    continue;
                }

                gp_Pnt2d centerPos = centerPoint->position();
                gp_Pnt2d arcStart = arc->startPoint(centerPos);
                gp_Pnt2d arcEnd = arc->endPoint(centerPos);

                sk::Vec2d startPos = toVec2(arcStart);
                sk::Vec2d endPos = toVec2(arcEnd);

                int startNode = graph->findOrCreateNode(startPos, std::nullopt, tolerance);
                int endNode = graph->findOrCreateNode(endPos, std::nullopt, tolerance);

                GraphEdge edge;
                edge.entityId = arc->id();
                edge.isArc = true;
                edge.startNode = startNode;
                edge.endNode = endNode;
                edge.startPos = startPos;
                edge.endPos = endPos;
                edge.centerPos = toVec2(centerPos);
                edge.radius = arc->radius();
                edge.startAngle = arc->startAngle();
                edge.endAngle = arc->endAngle();
                graph->edgeByEntity[edge.entityId] = static_cast<int>(graph->edges.size());
                graph->edges.push_back(std::move(edge));

                graph->nodes[startNode].edges.push_back(static_cast<int>(graph->edges.size() - 1));
                graph->nodes[endNode].edges.push_back(static_cast<int>(graph->edges.size() - 1));
            }
        }

        return graph;
    }

    if (config_.exactAnalyticFragments) {
    std::vector<AnalyticSource> sources;
    sources.reserve(sketch.getAllEntities().size());

    for (const auto& entity : sketch.getAllEntities()) {
        if (config_.isCancelled && config_.isCancelled()) {
            graph->errorMessage = "profile refinement cancelled";
            return graph;
        }
        if (!entity || entity->isConstruction()) continue;
        if (selection && !selection->empty() && selection->find(entity->id()) == selection->end()) {
            continue;
        }
        auto source = makeAnalyticSource(*entity, sketch);
        if (!source.has_value()) {
            const bool supported = entity->type() == sk::EntityType::Line ||
                                   entity->type() == sk::EntityType::Arc ||
                                   entity->type() == sk::EntityType::Circle ||
                                   entity->type() == sk::EntityType::Ellipse;
            if (supported &&
                config_.curveRefinementPolicy == CurveRefinementPolicy::V3PhysicalProximity) {
                graph->errorMessage = "profile refinement could not preserve analytic provenance";
                return graph;
            }
            continue;
        }
        if (sources.size() >= config_.maxPlanarizedSources) {
            graph->errorMessage = "profile refinement exceeds analytic-source limit";
            return graph;
        }
        sources.push_back(std::move(*source));
    }

    if (sources.empty()) return graph;
    const ProfileTolerancePolicy tolerancePolicy =
        ProfileTolerancePolicy::forSources(sources, config_.coincidenceTolerance);
    tolerance = tolerancePolicy.coincidence;

    std::set<std::pair<std::size_t, std::size_t>> candidatePairs;
    for (std::size_t first = 0; first < sources.size(); ++first) {
        for (std::size_t second = first + 1; second < sources.size(); ++second) {
            if (config_.isCancelled && config_.isCancelled()) {
                graph->errorMessage = "profile refinement cancelled";
                return graph;
            }
            candidatePairs.emplace(first, second);
            if (candidatePairs.size() > config_.maxPlanarizedCurvePairs) {
                graph->errorMessage = "profile refinement exceeds curve-pair limit";
                return graph;
            }
        }
    }

    std::vector<std::vector<CurveSplit>> splits(sources.size());
    for (std::size_t i = 0; i < sources.size(); ++i) {
        if (!sources[i].closed) {
            if (!addCurveSplit(splits[i], sources[i],
                               sources[i].fragment.firstParameter,
                               sources[i].fragment.startPoint, tolerance,
                               config_.curveRefinementPolicy) ||
                !addCurveSplit(splits[i], sources[i],
                               sources[i].fragment.lastParameter,
                               sources[i].fragment.endPoint, tolerance,
                               config_.curveRefinementPolicy)) {
                graph->errorMessage = "profile refinement could not measure exact curve distance";
                return graph;
            }
        }
    }

    for (const auto& [first, second] : candidatePairs) {
        if (config_.isCancelled && config_.isCancelled()) {
            graph->errorMessage = "profile refinement cancelled";
            return graph;
        }
        try {
        Geom2dAPI_InterCurveCurve intersector(sources[first].bounded, sources[second].bounded,
                                               tolerance);
        for (int index = 1; index <= intersector.NbPoints(); ++index) {
            const gp_Pnt2d point = intersector.Point(index);
            const auto firstParameter = projectCurveParameter(sources[first], point, tolerance);
            const auto secondParameter = projectCurveParameter(sources[second], point, tolerance);
            if (!firstParameter || !secondParameter) {
                graph->errorMessage = "profile refinement could not parameterize analytic intersection";
                return graph;
            }
            const sk::Vec2d shared{point.X(), point.Y()};
            if (!addCurveSplit(splits[first], sources[first], *firstParameter, shared,
                               tolerance, config_.curveRefinementPolicy) ||
                !addCurveSplit(splits[second], sources[second], *secondParameter, shared,
                               tolerance, config_.curveRefinementPolicy)) {
                graph->errorMessage = "profile refinement could not measure exact curve distance";
                return graph;
            }
        }
        // OCCT reports a point tangency both as a point and a zero-length
        // segment on some curve pairs. The point is already split above; do
        // not ask it to materialize that degenerate segment.
        if (intersector.NbPoints() > 0) continue;
        for (int index = 1; index <= intersector.NbSegments(); ++index) {
            Handle(Geom2d_Curve) firstSegment;
            Handle(Geom2d_Curve) secondSegment;
            intersector.Segment(index, firstSegment, secondSegment);
            if (shareAnalyticSupport(sources[first], sources[second], tolerance)) {
                graph->errorMessage = "profile has overlapping or coincident analytic curves";
                return graph;
            }
            // Different supports can only meet at a point. OCCT's tolerance may
            // represent that tangency as a short segment; collapse it to one
            // shared split instead of falsely refusing the profile.
            const gp_Pnt2d point = firstSegment->Value(
                (firstSegment->FirstParameter() + firstSegment->LastParameter()) * 0.5);
            const auto firstParameter = projectCurveParameter(sources[first], point, tolerance);
            const auto secondParameter = projectCurveParameter(sources[second], point, tolerance);
            if (!firstParameter || !secondParameter) {
                graph->errorMessage = "profile refinement could not parameterize tangent contact";
                return graph;
            }
            const sk::Vec2d shared{point.X(), point.Y()};
            if (!addCurveSplit(splits[first], sources[first], *firstParameter, shared,
                               tolerance, config_.curveRefinementPolicy) ||
                !addCurveSplit(splits[second], sources[second], *secondParameter, shared,
                               tolerance, config_.curveRefinementPolicy)) {
                graph->errorMessage = "profile refinement could not measure exact curve distance";
                return graph;
            }
        }
        } catch (const Standard_Failure&) {
            // OCCT rejects two adjacent trims of the same support with U1 == U2.
            // Their shared authored endpoint is already in `splits`; they need no
            // intersection refinement. Other failures remain deterministic refusal.
            if (shareAuthoredEndpoint(sources[first], sources[second], tolerance)) continue;
            graph->errorMessage = "profile refinement failed for analytic curve pair";
            return graph;
        }
    }

    std::size_t fragmentCount = 0;
    for (std::size_t sourceIndex = 0; sourceIndex < sources.size(); ++sourceIndex) {
        if (config_.isCancelled && config_.isCancelled()) {
            graph->errorMessage = "profile refinement cancelled";
            return graph;
        }
        auto& sourceSplits = splits[sourceIndex];
        std::sort(sourceSplits.begin(), sourceSplits.end(), [](const CurveSplit& a, const CurveSplit& b) {
            return a.parameter < b.parameter;
        });
        sourceSplits.erase(std::unique(sourceSplits.begin(), sourceSplits.end(),
            [tolerance, policy = config_.curveRefinementPolicy,
             &source = sources[sourceIndex]](const CurveSplit& a, const CurveSplit& b) {
                if (policy == CurveRefinementPolicy::V2ParameterProximity) {
                    return std::abs(a.parameter - b.parameter) <= tolerance;
                }
                const std::optional<double> distance = physicalCurveProximity(
                    source, a.parameter, b.parameter, tolerance);
                return distance.has_value() && *distance <= tolerance;
            }), sourceSplits.end());

        if (sources[sourceIndex].closed && sourceSplits.size() < 2) {
            graph->closedCurves.push_back(sources[sourceIndex].fragment);
            continue;
        }
        std::vector<CurveSplit> emittedSplits = sourceSplits;
        if (sources[sourceIndex].closed) {
            CurveSplit seam = sourceSplits.front();
            seam.parameter += 2.0 * std::numbers::pi_v<double>;
            emittedSplits.push_back(seam);
        }
        for (std::size_t splitIndex = 0; splitIndex + 1 < emittedSplits.size(); ++splitIndex) {
            if (config_.isCancelled && config_.isCancelled()) {
                graph->errorMessage = "profile refinement cancelled";
                return graph;
            }
            const CurveSplit& start = emittedSplits[splitIndex];
            const CurveSplit& end = emittedSplits[splitIndex + 1];
            if (config_.curveRefinementPolicy == CurveRefinementPolicy::V2ParameterProximity) {
                if (end.parameter - start.parameter <= tolerance) continue;
            } else {
                const std::optional<double> distance = physicalCurveIntervalLength(
                    sources[sourceIndex], start.parameter, end.parameter, tolerance);
                if (!distance.has_value()) {
                    graph->errorMessage =
                        "profile refinement could not measure exact curve distance";
                    return graph;
                }
                if (*distance <= tolerance) continue;
            }
            if (config_.curveRefinementPolicy == CurveRefinementPolicy::V3PhysicalProximity &&
                !exactFragmentIsContinuous(sources[sourceIndex], start, end, tolerance)) {
                graph->errorMessage = "profile refinement found an exact-curve discontinuity";
                return graph;
            }
            if (++fragmentCount > config_.maxPlanarizedFragments) {
                graph->errorMessage = "profile refinement exceeds fragment limit";
                return graph;
            }
            int startNode = graph->findOrCreateNode(start.point, std::nullopt, tolerance);
            int endNode = graph->findOrCreateNode(end.point, std::nullopt, tolerance);
            if (startNode == endNode) continue;
            CurveFragment fragment = sources[sourceIndex].fragment;
            fragment.firstParameter = start.parameter;
            fragment.lastParameter = end.parameter;
            fragment.startPoint = start.point;
            fragment.endPoint = end.point;
            const int sampleCount = fragment.kind == CurveFragmentKind::Line ? 1 : 16;
            fragment.samples.reserve(static_cast<std::size_t>(sampleCount) + 1);
            for (int sample = 0; sample <= sampleCount; ++sample) {
                const double ratio = static_cast<double>(sample) / static_cast<double>(sampleCount);
                const double parameter = fragment.firstParameter +
                    (fragment.lastParameter - fragment.firstParameter) * ratio;
            const gp_Pnt2d point = sources[sourceIndex].basis->Value(parameter);
                fragment.samples.push_back({point.X(), point.Y()});
            }
            GraphEdge edge;
            edge.entityId = fragment.baseEntityId + "#seg0_p" + std::to_string(splitIndex);
            edge.startNode = startNode;
            edge.endNode = endNode;
            edge.startPos = start.point;
            edge.endPos = end.point;
            const gp_Vec2d startTangent =
                sources[sourceIndex].basis->DN(fragment.firstParameter, 1);
            const gp_Vec2d endTangent =
                sources[sourceIndex].basis->DN(fragment.lastParameter, 1);
            edge.startTangent = {startTangent.X(), startTangent.Y()};
            edge.endTangent = {endTangent.X(), endTangent.Y()};
            edge.fragment = std::move(fragment);
            graph->edgeByEntity[edge.entityId] = static_cast<int>(graph->edges.size());
            graph->edges.push_back(std::move(edge));
            const int edgeIndex = static_cast<int>(graph->edges.size() - 1);
            graph->nodes[startNode].edges.push_back(edgeIndex);
            graph->nodes[endNode].edges.push_back(edgeIndex);
        }
    }
    return graph;
    }

    std::vector<BroadSegment> segments;
    segments.reserve(sketch.getAllEntities().size() * 4);

    for (const auto& entity : sketch.getAllEntities()) {
        // REFERENCE-LOCK PIN: `isConstruction` is the ONLY entity flag that excludes
        // geometry here. Reference-locked (host-face projected) geometry MUST keep
        // bounding regions — a profile extruded off the face boundary depends on it.
        if (!entity || entity->isConstruction()) {
            continue;
        }
        if (selection && !selection->empty() && selection->find(entity->id()) == selection->end()) {
            continue;
        }

        if (entity->type() == sk::EntityType::Line) {
            auto* line = dynamic_cast<const sk::SketchLine*>(entity.get());
            if (!line) {
                continue;
            }
            auto* start = sketch.getEntityAs<sk::SketchPoint>(line->startPointId());
            auto* end = sketch.getEntityAs<sk::SketchPoint>(line->endPointId());
            if (!start || !end) {
                continue;
            }
            sk::Vec2d startPos = toVec2(start->position());
            sk::Vec2d endPos = toVec2(end->position());
            if (distanceSquared(startPos, endPos) <= tolerance * tolerance) {
                continue;
            }
            segments.push_back({startPos, endPos, line->id() + "#seg0"});
        } else if (entity->type() == sk::EntityType::Arc) {
            auto* arc = dynamic_cast<const sk::SketchArc*>(entity.get());
            if (!arc) {
                continue;
            }
            auto* centerPoint = sketch.getEntityAs<sk::SketchPoint>(arc->centerPointId());
            if (!centerPoint) {
                continue;
            }
            sk::Vec2d centerPos = toVec2(centerPoint->position());
            auto points = tessellateArcPoints(centerPos, arc->radius(),
                                              arc->startAngle(), arc->endAngle(),
                                              config_);
            for (size_t i = 0; i + 1 < points.size(); ++i) {
                if (distanceSquared(points[i], points[i + 1]) <= tolerance * tolerance) {
                    continue;
                }
                segments.push_back({points[i], points[i + 1],
                                    arc->id() + "#seg" + std::to_string(i)});
            }
        } else if (entity->type() == sk::EntityType::Circle) {
            auto* circle = dynamic_cast<const sk::SketchCircle*>(entity.get());
            if (!circle) {
                continue;
            }
            auto* centerPoint = sketch.getEntityAs<sk::SketchPoint>(circle->centerPointId());
            if (!centerPoint) {
                continue;
            }
            sk::Vec2d centerPos = toVec2(centerPoint->position());
            auto points = tessellateCirclePoints(centerPos, circle->radius(), config_);
            for (size_t i = 0; i + 1 < points.size(); ++i) {
                if (distanceSquared(points[i], points[i + 1]) <= tolerance * tolerance) {
                    continue;
                }
                segments.push_back({points[i], points[i + 1],
                                    circle->id() + "#seg" + std::to_string(i)});
            }
        } else if (entity->type() == sk::EntityType::Ellipse) {
            auto* ellipse = dynamic_cast<const sk::SketchEllipse*>(entity.get());
            if (!ellipse) {
                continue;
            }
            auto* centerPoint = sketch.getEntityAs<sk::SketchPoint>(ellipse->centerPointId());
            if (!centerPoint) {
                continue;
            }
            sk::Vec2d centerPos = toVec2(centerPoint->position());
            auto points = tessellateEllipsePoints(centerPos, ellipse->majorRadius(), ellipse->minorRadius(),
                                                  ellipse->rotation(), config_);
            for (size_t i = 0; i + 1 < points.size(); ++i) {
                if (distanceSquared(points[i], points[i + 1]) <= tolerance * tolerance) {
                    continue;
                }
                segments.push_back({points[i], points[i + 1],
                                    ellipse->id() + "#seg" + std::to_string(i)});
            }
        }
    }

    if (segments.empty()) {
        return graph;
    }

    std::vector<std::vector<SplitPoint>> splitPoints(segments.size());
    for (size_t i = 0; i < segments.size(); ++i) {
        addSplitPoint(splitPoints[i], 0.0, segments[i].start, tolerance);
        addSplitPoint(splitPoints[i], 1.0, segments[i].end, tolerance);
    }

    std::unordered_set<std::string> edgeKeys;
    auto pointKey = [tolerance](const sk::Vec2d& p) {
        double snap = tolerance > 0.0 ? tolerance : 1e-9;
        long long x = static_cast<long long>(std::llround(p.x / snap));
        long long y = static_cast<long long>(std::llround(p.y / snap));
        return std::to_string(x) + "," + std::to_string(y);
    };
    auto edgeKey = [&](const sk::Vec2d& a, const sk::Vec2d& b) {
        std::string ka = pointKey(a);
        std::string kb = pointKey(b);
        if (ka <= kb) {
            return ka + "|" + kb;
        }
        return kb + "|" + ka;
    };

    for (size_t i = 0; i < segments.size(); ++i) {
        for (size_t j = i + 1; j < segments.size(); ++j) {
            const auto& a = segments[i];
            const auto& b = segments[j];

            sk::Vec2d r = diff(a.end, a.start);
            sk::Vec2d s = diff(b.end, b.start);
            double denom = cross2d(r, s);
            if (std::abs(denom) <= tolerance) {
                if (pointOnSegment(a.start, a.end, b.start, tolerance)) {
                    addSplitPoint(splitPoints[i], segmentParam(a.start, a.end, b.start), b.start, tolerance);
                }
                if (pointOnSegment(a.start, a.end, b.end, tolerance)) {
                    addSplitPoint(splitPoints[i], segmentParam(a.start, a.end, b.end), b.end, tolerance);
                }
                if (pointOnSegment(b.start, b.end, a.start, tolerance)) {
                    addSplitPoint(splitPoints[j], segmentParam(b.start, b.end, a.start), a.start, tolerance);
                }
                if (pointOnSegment(b.start, b.end, a.end, tolerance)) {
                    addSplitPoint(splitPoints[j], segmentParam(b.start, b.end, a.end), a.end, tolerance);
                }
                continue;
            }

            double t = 0.0;
            double u = 0.0;
            sk::Vec2d intersection;
            if (segmentIntersection(a.start, a.end, b.start, b.end, tolerance, t, u, intersection)) {
                addSplitPoint(splitPoints[i], t, intersection, tolerance);
                addSplitPoint(splitPoints[j], u, intersection, tolerance);
            }
        }
    }

    for (size_t i = 0; i < segments.size(); ++i) {
        auto& splits = splitPoints[i];
        if (splits.size() < 2) {
            continue;
        }
        std::sort(splits.begin(), splits.end(), [](const SplitPoint& a, const SplitPoint& b) {
            return a.t < b.t;
        });

        std::vector<SplitPoint> uniqueSplits;
        uniqueSplits.reserve(splits.size());
        for (const auto& split : splits) {
            if (uniqueSplits.empty() ||
                distanceSquared(uniqueSplits.back().point, split.point) > tolerance * tolerance) {
                uniqueSplits.push_back(split);
            }
        }

        int subIndex = 0;
        for (size_t s = 0; s + 1 < uniqueSplits.size(); ++s) {
            const auto& p1 = uniqueSplits[s].point;
            const auto& p2 = uniqueSplits[s + 1].point;
            if (distanceSquared(p1, p2) <= tolerance * tolerance) {
                continue;
            }

            std::string key = edgeKey(p1, p2);
            if (!edgeKeys.insert(key).second) {
                continue;
            }

            int startNode = graph->findOrCreateNode(p1, std::nullopt, tolerance);
            int endNode = graph->findOrCreateNode(p2, std::nullopt, tolerance);
            if (startNode == endNode) {
                continue;
            }

            GraphEdge edge;
            edge.entityId = segments[i].baseId + "_p" + std::to_string(subIndex++);
            edge.startNode = startNode;
            edge.endNode = endNode;
            edge.startPos = p1;
            edge.endPos = p2;

            graph->edgeByEntity[edge.entityId] = static_cast<int>(graph->edges.size());
            graph->edges.push_back(std::move(edge));

            graph->nodes[startNode].edges.push_back(static_cast<int>(graph->edges.size() - 1));
            graph->nodes[endNode].edges.push_back(static_cast<int>(graph->edges.size() - 1));
        }
    }

    return graph;
}

std::vector<Wire> LoopDetector::findCycles(const AdjacencyGraph& graphBase) const {
    const auto& graph = graphBase;
    std::vector<Wire> cycles;
    std::unordered_set<std::string> seen;

    std::vector<int> pathNodes;
    std::vector<int> pathEdges;
    std::unordered_set<int> visitedNodes;

    std::function<void(int, int, int)> dfs = [&](int start, int current, int parentEdge) {
        for (int edgeIndex : graph.nodes[current].edges) {
            if (edgeIndex == parentEdge) {
                continue;
            }
            const auto& edge = graph.edges[edgeIndex];
            int next = (edge.startNode == current) ? edge.endNode : edge.startNode;
            if (next == start && pathEdges.size() >= 1) {
                std::vector<int> cycleEdges = pathEdges;
                cycleEdges.push_back(edgeIndex);
                std::vector<sk::EntityID> cycleEdgeIds;
                cycleEdgeIds.reserve(cycleEdges.size());
                for (int idx : cycleEdges) {
                    cycleEdgeIds.push_back(graph.edges[idx].entityId);
                }
                std::string key = makeCycleKey(cycleEdgeIds);
                if (seen.insert(key).second) {
                    Wire wire;
                    std::vector<int> cycleNodes = pathNodes;
                    cycleNodes.push_back(start);
                    for (size_t i = 0; i < cycleEdges.size(); ++i) {
                        int edgeIdx = cycleEdges[i];
                        int from = cycleNodes[i];
                        int to = cycleNodes[i + 1];
                        const auto& edgeRef = graph.edges[edgeIdx];
                        wire.edges.push_back(edgeRef.entityId);
                        wire.forward.push_back(edgeRef.startNode == from && edgeRef.endNode == to);
                    }
                    wire.startPoint = graph.nodes[start].id;
                    wire.endPoint = graph.nodes[start].id;
                    cycles.push_back(std::move(wire));
                }
                continue;
            }

            if (visitedNodes.count(next)) {
                continue;
            }

            visitedNodes.insert(next);
            pathNodes.push_back(next);
            pathEdges.push_back(edgeIndex);
            dfs(start, next, edgeIndex);
            pathEdges.pop_back();
            pathNodes.pop_back();
            visitedNodes.erase(next);
        }
    };

    for (size_t i = 0; i < graph.nodes.size(); ++i) {
        pathNodes.clear();
        pathEdges.clear();
        visitedNodes.clear();
        visitedNodes.insert(static_cast<int>(i));
        pathNodes.push_back(static_cast<int>(i));
        dfs(static_cast<int>(i), static_cast<int>(i), -1);
    }

    return cycles;
}

std::vector<Loop> LoopDetector::findFaces(const AdjacencyGraph& graph) const {
    struct HalfEdge {
        int from = -1;
        int to = -1;
        int edgeIndex = -1;
        int twin = -1;
        int next = -1;
        double angle = 0.0;
        bool visited = false;
    };

    std::vector<HalfEdge> halfEdges;
    halfEdges.reserve(graph.edges.size() * 2);
    std::vector<std::vector<int>> outgoing(graph.nodes.size());

    for (size_t edgeIndex = 0; edgeIndex < graph.edges.size(); ++edgeIndex) {
        const auto& edge = graph.edges[edgeIndex];
        if (edge.startNode < 0 || edge.endNode < 0) {
            continue;
        }

        HalfEdge h1;
        h1.from = edge.startNode;
        h1.to = edge.endNode;
        h1.edgeIndex = static_cast<int>(edgeIndex);
        const sk::Vec2d startDirection =
            (edge.startTangent.x != 0.0 || edge.startTangent.y != 0.0) ?
                edge.startTangent : diff(edge.endPos, edge.startPos);
        h1.angle = std::atan2(startDirection.y, startDirection.x);

        HalfEdge h2;
        h2.from = edge.endNode;
        h2.to = edge.startNode;
        h2.edgeIndex = static_cast<int>(edgeIndex);
        const sk::Vec2d endDirection =
            (edge.endTangent.x != 0.0 || edge.endTangent.y != 0.0) ?
                sk::Vec2d{-edge.endTangent.x, -edge.endTangent.y} :
                diff(edge.startPos, edge.endPos);
        h2.angle = std::atan2(endDirection.y, endDirection.x);

        int idx1 = static_cast<int>(halfEdges.size());
        halfEdges.push_back(h1);
        int idx2 = static_cast<int>(halfEdges.size());
        halfEdges.push_back(h2);

        halfEdges[idx1].twin = idx2;
        halfEdges[idx2].twin = idx1;

        outgoing[edge.startNode].push_back(idx1);
        outgoing[edge.endNode].push_back(idx2);
    }

    std::vector<std::unordered_map<int, int>> edgeOrder(graph.nodes.size());
    for (size_t nodeIndex = 0; nodeIndex < outgoing.size(); ++nodeIndex) {
        auto& edges = outgoing[nodeIndex];
        if (edges.empty()) {
            continue;
        }
        std::sort(edges.begin(), edges.end(), [&](int a, int b) {
            return halfEdges[a].angle < halfEdges[b].angle;
        });
        auto& order = edgeOrder[nodeIndex];
        order.reserve(edges.size());
        for (size_t i = 0; i < edges.size(); ++i) {
            order[edges[i]] = static_cast<int>(i);
        }
    }

    for (size_t nodeIndex = 0; nodeIndex < outgoing.size(); ++nodeIndex) {
        const auto& edges = outgoing[nodeIndex];
        if (edges.empty()) {
            continue;
        }
        for (int edgeIdx : edges) {
            int twin = halfEdges[edgeIdx].twin;
            int toNode = halfEdges[edgeIdx].to;
            const auto& toEdges = outgoing[static_cast<size_t>(toNode)];
            if (toEdges.empty()) {
                continue;
            }
            const auto& order = edgeOrder[static_cast<size_t>(toNode)];
            auto it = order.find(twin);
            if (it == order.end()) {
                continue;
            }
            int pos = it->second;
            int nextPos = (pos - 1 + static_cast<int>(toEdges.size())) %
                          static_cast<int>(toEdges.size());
            halfEdges[edgeIdx].next = toEdges[static_cast<size_t>(nextPos)];
        }
    }

    std::vector<Loop> loops;
    for (size_t i = 0; i < halfEdges.size(); ++i) {
        if (halfEdges[i].visited) {
            continue;
        }
        int start = static_cast<int>(i);
        int current = start;
        Loop loop;
        size_t guard = 0;
        bool closed = false;
        while (!halfEdges[current].visited) {
            halfEdges[current].visited = true;
            const auto& edge = graph.edges[static_cast<size_t>(halfEdges[current].edgeIndex)];
            const bool forward = edge.startNode == halfEdges[current].from &&
                                 edge.endNode == halfEdges[current].to;
            if (edge.fragment.has_value() && !edge.fragment->samples.empty()) {
                const auto& samples = edge.fragment->samples;
                if (forward) {
                    loop.polygon.insert(loop.polygon.end(), samples.begin(), samples.end() - 1);
                } else {
                    loop.polygon.insert(loop.polygon.end(), samples.rbegin(), samples.rend() - 1);
                }
            } else {
                loop.polygon.push_back(graph.nodes[halfEdges[current].from].position);
            }
            loop.wire.edges.push_back(edge.entityId);
            loop.wire.forward.push_back(forward);
            if (edge.fragment.has_value()) {
                CurveFragment fragment = *edge.fragment;
                fragment.forward = forward;
                loop.fragments.push_back(std::move(fragment));
            }

            current = halfEdges[current].next;
            if (current < 0 || current >= static_cast<int>(halfEdges.size())) {
                break;
            }
            if (current == start) {
                closed = true;
                break;
            }
            if (++guard > halfEdges.size()) {
                break;
            }
        }

        if (!closed || loop.polygon.size() < 3) {
            continue;
        }
        loop.wire.startPoint = graph.nodes[halfEdges[start].from].id;
        loop.wire.endPoint = loop.wire.startPoint;

        computeLoopPropertiesFromPolygon(loop);
        if (loop.signedArea <= kMinArea) {
            continue;
        }
        loops.push_back(std::move(loop));
    }

    return loops;
}

void LoopDetector::computeLoopProperties(Loop& loop, const sk::Sketch& sketch) const {
    loop.polygon.clear();

    if (loop.wire.edges.empty()) {
        loop.signedArea = 0.0;
        loop.boundsMin = {0.0, 0.0};
        loop.boundsMax = {0.0, 0.0};
        loop.centroid = {0.0, 0.0};
        return;
    }

    bool hasCurrent = false;

    for (size_t i = 0; i < loop.wire.edges.size(); ++i) {
        const auto& edgeId = loop.wire.edges[i];
        bool forward = (i < loop.wire.forward.size()) ? loop.wire.forward[i] : true;

        const auto* entity = sketch.getEntity(edgeId);
        if (!entity) {
            continue;
        }

        if (entity->type() == sk::EntityType::Line) {
            auto* line = sketch.getEntityAs<sk::SketchLine>(edgeId);
            if (!line) {
                continue;
            }
            auto* start = sketch.getEntityAs<sk::SketchPoint>(line->startPointId());
            auto* end = sketch.getEntityAs<sk::SketchPoint>(line->endPointId());
            if (!start || !end) {
                continue;
            }

            sk::Vec2d startPos = toVec2(start->position());
            sk::Vec2d endPos = toVec2(end->position());
            sk::Vec2d from = forward ? startPos : endPos;
            sk::Vec2d to = forward ? endPos : startPos;
            if (!hasCurrent) {
                loop.polygon.push_back(from);
                hasCurrent = true;
            }
            loop.polygon.push_back(to);
        } else if (entity->type() == sk::EntityType::Arc) {
            auto* arc = sketch.getEntityAs<sk::SketchArc>(edgeId);
            if (!arc) {
                continue;
            }
            auto* centerPoint = sketch.getEntityAs<sk::SketchPoint>(arc->centerPointId());
            if (!centerPoint) {
                continue;
            }
            sk::Vec2d centerPos = toVec2(centerPoint->position());
            gp_Pnt2d arcStart = arc->startPoint(centerPoint->position());
            gp_Pnt2d arcEnd = arc->endPoint(centerPoint->position());
            sk::Vec2d from = forward ? toVec2(arcStart) : toVec2(arcEnd);
            // W-WP3a: removed unused `to` (dead in W-WP2 copy). Behavior unchanged.
            if (!hasCurrent) {
                loop.polygon.push_back(from);
                hasCurrent = true;
            }
            appendArcSamples(loop.polygon, centerPos, arc->radius(),
                             forward ? arc->startAngle() : arc->endAngle(),
                             forward ? arc->endAngle() : arc->startAngle(),
                             forward,
                             config_);
        } else if (entity->type() == sk::EntityType::Circle) {
            auto* circle = sketch.getEntityAs<sk::SketchCircle>(edgeId);
            if (!circle) {
                continue;
            }
            auto* centerPoint = sketch.getEntityAs<sk::SketchPoint>(circle->centerPointId());
            if (!centerPoint) {
                continue;
            }
            sk::Vec2d centerPos = toVec2(centerPoint->position());
            int segments = computeCircleSegments(circle->radius(), config_);
            for (int s = 0; s <= segments; ++s) {
                double angle = (2.0 * std::numbers::pi_v<double> * s) / static_cast<double>(segments);
                loop.polygon.push_back({centerPos.x + circle->radius() * std::cos(angle),
                                        centerPos.y + circle->radius() * std::sin(angle)});
            }
            hasCurrent = true;
        } else if (entity->type() == sk::EntityType::Ellipse) {
            auto* ellipse = sketch.getEntityAs<sk::SketchEllipse>(edgeId);
            if (!ellipse) {
                continue;
            }
            auto* centerPoint = sketch.getEntityAs<sk::SketchPoint>(ellipse->centerPointId());
            if (!centerPoint) {
                continue;
            }
            sk::Vec2d centerPos = toVec2(centerPoint->position());
            auto points = tessellateEllipsePoints(centerPos, ellipse->majorRadius(), ellipse->minorRadius(),
                                                  ellipse->rotation(), config_);
            loop.polygon.insert(loop.polygon.end(), points.begin(), points.end());
            hasCurrent = true;
        }
    }

    computeLoopPropertiesFromPolygon(loop);
}

void LoopDetector::computeLoopPropertiesFromPolygon(Loop& loop) const {
    if (!loop.polygon.empty()) {
        std::vector<sk::Vec2d> filtered;
        filtered.reserve(loop.polygon.size());
        filtered.push_back(loop.polygon.front());
        double tol2 = config_.coincidenceTolerance * config_.coincidenceTolerance;
        for (size_t i = 1; i < loop.polygon.size(); ++i) {
            if (distanceSquared(filtered.back(), loop.polygon[i]) > tol2) {
                filtered.push_back(loop.polygon[i]);
            }
        }
        loop.polygon.swap(filtered);
    }

    if (loop.polygon.size() >= 3) {
        loop.signedArea = computeSignedArea(loop.polygon);
        loop.centroid = computeCentroid(loop.polygon);
        loop.boundsMin = {loop.polygon.front().x, loop.polygon.front().y};
        loop.boundsMax = loop.boundsMin;
        for (const auto& p : loop.polygon) {
            loop.boundsMin.x = std::min(loop.boundsMin.x, p.x);
            loop.boundsMin.y = std::min(loop.boundsMin.y, p.y);
            loop.boundsMax.x = std::max(loop.boundsMax.x, p.x);
            loop.boundsMax.y = std::max(loop.boundsMax.y, p.y);
        }
    } else {
        loop.signedArea = 0.0;
        loop.centroid = {0.0, 0.0};
        loop.boundsMin = {0.0, 0.0};
        loop.boundsMax = {0.0, 0.0};
    }
}

std::vector<Face> LoopDetector::buildFaceHierarchy(std::vector<Loop> loops) const {
    std::vector<Face> faces;
    if (loops.empty()) {
        return faces;
    }

    std::vector<size_t> order(loops.size());
    for (size_t i = 0; i < loops.size(); ++i) {
        order[i] = i;
    }
    std::sort(order.begin(), order.end(), [&](size_t a, size_t b) {
        return loops[a].area() > loops[b].area();
    });

    std::vector<int> parent(loops.size(), -1);
    std::vector<int> depth(loops.size(), 0);

    for (size_t i = 0; i < loops.size(); ++i) {
        size_t loopIdx = order[i];
        double bestArea = std::numeric_limits<double>::infinity();
        int bestParent = -1;

        for (size_t j = 0; j < loops.size(); ++j) {
            size_t candidateIdx = order[j];
            if (candidateIdx == loopIdx) {
                continue;
            }
            if (loops[candidateIdx].area() <= loops[loopIdx].area()) {
                continue;
            }
            if (!loopContainsLoop(loops[candidateIdx], loops[loopIdx], config_.coincidenceTolerance)) {
                continue;
            }
            if (polygonsIntersect(loops[candidateIdx].polygon, loops[loopIdx].polygon)) {
                continue;
            }
            double area = loops[candidateIdx].area();
            if (area < bestArea) {
                bestArea = area;
                bestParent = static_cast<int>(candidateIdx);
            }
        }

        parent[loopIdx] = bestParent;
        if (bestParent >= 0) {
            depth[loopIdx] = depth[bestParent] + 1;
        }
    }

    for (size_t i = 0; i < loops.size(); ++i) {
        if (loops[i].polygon.size() < 3) {
            continue;
        }
        bool shouldBeCCW = (depth[i] % 2 == 0);
        if (loops[i].isCCW() != shouldBeCCW) {
            reverseLoop(loops[i]);
        }
    }

    std::unordered_map<int, size_t> faceByLoop;
    for (size_t i = 0; i < loops.size(); ++i) {
        if (depth[i] % 2 != 0) {
            continue;
        }
        Face face;
        face.outerLoop = std::move(loops[i]);
        faceByLoop[static_cast<int>(i)] = faces.size();
        faces.push_back(std::move(face));
    }

    for (size_t i = 0; i < loops.size(); ++i) {
        if (depth[i] % 2 == 0) {
            continue;
        }
        int ancestor = parent[i];
        while (ancestor >= 0 && depth[ancestor] % 2 != 0) {
            ancestor = parent[ancestor];
        }
        if (ancestor < 0) {
            continue;
        }
        auto faceIt = faceByLoop.find(ancestor);
        if (faceIt == faceByLoop.end()) {
            continue;
        }
        faces[faceIt->second].innerLoops.push_back(std::move(loops[i]));
    }

    return faces;
}

bool LoopDetector::validateLoop(const Loop& loop, const sk::Sketch& /*sketch*/) const {
    if (!loop.wire.isClosed()) {
        return false;
    }
    if (loop.polygon.size() < 3) {
        return false;
    }
    if (std::abs(loop.signedArea) < kMinArea) {
        return false;
    }

    size_t n = loop.polygon.size();
    if (n < 4) {
        return true;
    }

    for (size_t i = 0; i < n; ++i) {
        size_t iNext = (i + 1) % n;
        for (size_t j = i + 1; j < n; ++j) {
            size_t jNext = (j + 1) % n;
            if (i == j || iNext == j || jNext == i) {
                continue;
            }
            if (i == 0 && jNext == n - 1) {
                continue;
            }
            if (segmentsIntersect(loop.polygon[i], loop.polygon[iNext],
                                  loop.polygon[j], loop.polygon[jNext])) {
                return false;
            }
        }
    }

    return true;
}

double computeSignedArea(const std::vector<sk::Vec2d>& polygon) {
    if (polygon.size() < 3) {
        return 0.0;
    }
    double area = 0.0;
    for (size_t i = 0; i < polygon.size(); ++i) {
        const auto& p1 = polygon[i];
        const auto& p2 = polygon[(i + 1) % polygon.size()];
        area += p1.x * p2.y - p2.x * p1.y;
    }
    return 0.5 * area;
}

bool isPointInPolygon(const sk::Vec2d& point, const std::vector<sk::Vec2d>& polygon) {
    if (polygon.size() < 3) {
        return false;
    }
    bool inside = false;
    for (size_t i = 0, j = polygon.size() - 1; i < polygon.size(); j = i++) {
        const auto& pi = polygon[i];
        const auto& pj = polygon[j];
        bool intersect = ((pi.y > point.y) != (pj.y > point.y)) &&
                         (point.x < (pj.x - pi.x) * (point.y - pi.y) / (pj.y - pi.y) + pi.x);
        if (intersect) {
            inside = !inside;
        }
    }
    return inside;
}

sk::Vec2d computeCentroid(const std::vector<sk::Vec2d>& polygon) {
    sk::Vec2d centroid{0.0, 0.0};
    if (polygon.empty()) {
        return centroid;
    }

    double area = computeSignedArea(polygon);
    if (std::abs(area) < kMinArea) {
        for (const auto& p : polygon) {
            centroid.x += p.x;
            centroid.y += p.y;
        }
        centroid.x /= static_cast<double>(polygon.size());
        centroid.y /= static_cast<double>(polygon.size());
        return centroid;
    }

    double factor = 0.0;
    for (size_t i = 0; i < polygon.size(); ++i) {
        const auto& p1 = polygon[i];
        const auto& p2 = polygon[(i + 1) % polygon.size()];
        double cross = p1.x * p2.y - p2.x * p1.y;
        centroid.x += (p1.x + p2.x) * cross;
        centroid.y += (p1.y + p2.y) * cross;
        factor += cross;
    }

    factor = 1.0 / (3.0 * factor);
    centroid.x *= factor;
    centroid.y *= factor;
    return centroid;
}

bool polygonsIntersect(const std::vector<sk::Vec2d>& poly1,
                       const std::vector<sk::Vec2d>& poly2) {
    if (poly1.empty() || poly2.empty()) {
        return false;
    }
    size_t n1 = poly1.size();
    size_t n2 = poly2.size();

    for (size_t i = 0; i < n1; ++i) {
        size_t iNext = (i + 1) % n1;
        for (size_t j = 0; j < n2; ++j) {
            size_t jNext = (j + 1) % n2;
            if (segmentsIntersect(poly1[i], poly1[iNext], poly2[j], poly2[jNext])) {
                return true;
            }
        }
    }
    return false;
}

bool Loop::contains(const sk::Vec2d& point) const {
    return isPointInPolygon(point, polygon);
}

bool Loop::contains(const Loop& other) const {
    if (polygon.empty()) {
        return false;
    }
    return isPointInPolygon(other.centroid, polygon);
}

bool Face::isValid() const {
    if (!outerLoop.isCCW()) {
        return false;
    }
    for (const auto& hole : innerLoops) {
        if (hole.isCCW()) {
            return false;
        }
    }
    return true;
}

} // namespace onecad::core::loop
