// EdgeProjector.cpp — see EdgeProjector.h for the provenance, the shared-buffer
// discipline, and why the circular-edge branch is rewritten rather than reused.
#include "sketch/EdgeProjector.h"

#include "kernel/topology/CoplanarFacePatch.h"
#include "util/Hashing.h"

#include <BRepAdaptor_Curve.hxx>
#include <BRepTools.hxx>
#include <BRepTools_WireExplorer.hxx>
#include <BRep_Tool.hxx>
#include <GeomAbs_CurveType.hxx>
#include <TopAbs.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_MapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Wire.hxx>
#include <gp_Circ.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <numbers>
#include <utility>

namespace onecad::core::sketch {
namespace {

using Options = EdgeProjector::Options;
using Buffer = EdgeProjector::Buffer;
using Entity = EdgeProjector::Entity;
using EntityKind = EdgeProjector::EntityKind;
using Refusal = EdgeProjector::Refusal;
using RefusalCode = EdgeProjector::RefusalCode;

constexpr double kPointEpsilon = 1e-9;  // FaceBoundaryProjector.cpp:39 (oracle :31)
/// `quantizationVersion = 1` (SCHEMA §10) — the SAME grid `rankKey` uses. It is
/// what makes `projectedHash` survive the libm difference between macOS and Linux.
constexpr double kQuantizationStep = 1e-6;

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

/// An ellipse's major axis is a LINE, so `rotation` and `rotation ± pi` describe
/// the same curve. Folding onto (-pi/2, pi/2] is what makes SCHEMA §7.6's
/// "two entities describing the same 2D curve hash identically" true for an
/// Ellipse: the source circle's axis sense (`n_c` vs `-n_c`) is a fact about the
/// model, not about the projected picture, and it must not reach the hash.
double canonicalAxisAngle(double angle) {
    constexpr double pi = std::numbers::pi_v<double>;
    angle = std::fmod(angle, pi);
    if (angle <= -pi / 2.0) {
        angle += pi;
    } else if (angle > pi / 2.0) {
        angle -= pi;
    }
    return angle;
}

/// SCHEMA §7.3 ellipse parameter normalization: `majorR >= minorR`, swapping the
/// radii and adding pi/2 to the rotation. Applied before BOTH emission and
/// hashing so the wire value and the hashed value are the same number.
void normalizeEllipse(double& majorR, double& minorR, double& rotation) {
    if (minorR > majorR) {
        std::swap(majorR, minorR);
        rotation = normalizeAngle(rotation + std::numbers::pi_v<double> / 2.0);
    }
    rotation = canonicalAxisAngle(rotation);
}

double dot3(const Vec3d& a, const Vec3d& b) {
    return a.x * b.x + a.y * b.y + a.z * b.z;
}

Vec3d cross3(const Vec3d& a, const Vec3d& b) {
    return Vec3d{a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x};
}

/// Unit vector, or {0,0,0} when the input is below `kPointEpsilon`.
Vec3d normalize3(const Vec3d& v) {
    const double len = std::sqrt(dot3(v, v));
    if (len <= kPointEpsilon) return Vec3d{0.0, 0.0, 0.0};
    return Vec3d{v.x / len, v.y / len, v.z / len};
}

Vec3d toVec3(const gp_Pnt& p) {
    return Vec3d{p.X(), p.Y(), p.Z()};
}

Vec3d toVec3(const gp_Dir& d) {
    return Vec3d{d.X(), d.Y(), d.Z()};
}

// One projected curve in the plane's UV, before it is merged into the shared
// buffer. The two-phase split is what makes a refusal ATOMIC per source: a
// source that refuses has appended nothing, so `points[]` numbering never
// records a source that produced no entity.
struct Primitive {
    EntityKind kind = EntityKind::Line;
    Vec2d a{};
    Vec2d b{};
    Vec2d center{};
    double radius = 0.0;
    double startAngle = 0.0;
    double endAngle = 0.0;
    bool ccw = true;
    double majorR = 0.0;
    double minorR = 0.0;
    double rotation = 0.0;
};

Refusal refuse(RefusalCode code, std::string message) {
    return Refusal{code, std::move(message)};
}

// FaceBoundaryProjector.cpp:88-96 (oracle findOrCreatePoint :85-110): linear
// merge scan, then append. The merge is response-scoped and GLOBAL across
// sources — a ref reused across entities IS the same point, which is what lets a
// projected outline close into a region.
int findOrCreatePoint(Buffer& buffer, const Vec2d& position, double tolerance) {
    for (std::size_t i = 0; i < buffer.points.size(); ++i) {
        const Vec2d existing{buffer.points[i].u, buffer.points[i].v};
        if (samePoint(existing, position, tolerance)) {
            return static_cast<int>(i);
        }
    }
    buffer.points.push_back(EdgeProjector::Point{position.x, position.y});
    return static_cast<int>(buffer.points.size()) - 1;
}

Vec2d pointAt(const Buffer& buffer, int index) {
    return Vec2d{buffer.points[static_cast<std::size_t>(index)].u,
                 buffer.points[static_cast<std::size_t>(index)].v};
}

// The three duplicate-suppression scans (FaceBoundaryProjector.cpp:99-151),
// scoped to entities `[from, end)` — this source's own contribution. See the
// header for why the scope differs from FaceBoundaryProjector's.
bool lineExists(const Buffer& buffer, std::size_t from, int startIndex, int endIndex,
                double tolerance) {
    const Vec2d start = pointAt(buffer, startIndex);
    const Vec2d end = pointAt(buffer, endIndex);
    for (std::size_t i = from; i < buffer.entities.size(); ++i) {
        const Entity& entity = buffer.entities[i];
        if (entity.kind != EntityKind::Line) continue;
        const Vec2d a = pointAt(buffer, entity.p0);
        const Vec2d b = pointAt(buffer, entity.p1);
        const bool direct = samePoint(start, a, tolerance) && samePoint(end, b, tolerance);
        const bool reversed = samePoint(start, b, tolerance) && samePoint(end, a, tolerance);
        if (direct || reversed) return true;
    }
    return false;
}

bool circleExists(const Buffer& buffer, std::size_t from, const Vec2d& center, double radius,
                  double pointTolerance, double radiusTolerance) {
    for (std::size_t i = from; i < buffer.entities.size(); ++i) {
        const Entity& entity = buffer.entities[i];
        if (entity.kind != EntityKind::Circle) continue;
        if (samePoint(pointAt(buffer, entity.center), center, pointTolerance) &&
            nearlyEqual(entity.radius, radius, radiusTolerance)) {
            return true;
        }
    }
    return false;
}

bool arcExists(const Buffer& buffer, std::size_t from, const Vec2d& center, double radius,
               double startAngle, double endAngle, double pointTolerance,
               double radiusTolerance) {
    constexpr double kAngleTolerance = 1e-4;  // FaceBoundaryProjector.cpp:40 (oracle :32)
    for (std::size_t i = from; i < buffer.entities.size(); ++i) {
        const Entity& entity = buffer.entities[i];
        if (entity.kind != EntityKind::Arc) continue;
        if (!samePoint(pointAt(buffer, entity.center), center, pointTolerance)) continue;
        if (!nearlyEqual(entity.radius, radius, radiusTolerance)) continue;
        if (sameAngle(entity.startAngle, startAngle, kAngleTolerance) &&
            sameAngle(entity.endAngle, endAngle, kAngleTolerance)) {
            return true;
        }
    }
    return false;
}

// Phase two: merge one source's primitives into the shared buffer, in emission
// order, numbering points by first use.
void appendPrimitives(Buffer& buffer, const std::vector<Primitive>& primitives,
                      std::size_t sourceIndex, const Options& options) {
    const std::size_t base = buffer.entities.size();
    for (const Primitive& p : primitives) {
        Entity entity;
        entity.kind = p.kind;
        entity.sourceIndex = sourceIndex;
        switch (p.kind) {
            case EntityKind::Line: {
                const int startIndex = findOrCreatePoint(buffer, p.a, options.pointMergeTolerance);
                const int endIndex = findOrCreatePoint(buffer, p.b, options.pointMergeTolerance);
                if (lineExists(buffer, base, startIndex, endIndex, options.pointMergeTolerance)) {
                    continue;
                }
                entity.p0 = startIndex;
                entity.p1 = endIndex;
                break;
            }
            case EntityKind::Circle: {
                const int centerIndex =
                    findOrCreatePoint(buffer, p.center, options.pointMergeTolerance);
                if (circleExists(buffer, base, p.center, p.radius, options.pointMergeTolerance,
                                 options.radiusTolerance)) {
                    continue;
                }
                entity.center = centerIndex;
                entity.radius = p.radius;
                break;
            }
            case EntityKind::Arc: {
                const int centerIndex =
                    findOrCreatePoint(buffer, p.center, options.pointMergeTolerance);
                if (arcExists(buffer, base, p.center, p.radius, p.startAngle, p.endAngle,
                              options.pointMergeTolerance, options.radiusTolerance)) {
                    continue;
                }
                entity.center = centerIndex;
                entity.radius = p.radius;
                entity.startAngle = p.startAngle;
                entity.endAngle = p.endAngle;
                entity.ccw = p.ccw;
                break;
            }
            case EntityKind::Ellipse: {
                // No duplicate scan: §7.4 lets nothing constrain an ellipse, so
                // two coincident ones are inert, and suppressing the second
                // would drop a source's only answer with no refusal to explain it.
                entity.center = findOrCreatePoint(buffer, p.center, options.pointMergeTolerance);
                entity.majorR = p.majorR;
                entity.minorR = p.minorR;
                entity.rotation = p.rotation;
                break;
            }
        }
        buffer.entities.push_back(entity);
    }
}

const char* curveTypeName(GeomAbs_CurveType type) {
    switch (type) {
        case GeomAbs_Line: return "GeomAbs_Line";
        case GeomAbs_Circle: return "GeomAbs_Circle";
        case GeomAbs_Ellipse: return "GeomAbs_Ellipse";
        case GeomAbs_Hyperbola: return "GeomAbs_Hyperbola";
        case GeomAbs_Parabola: return "GeomAbs_Parabola";
        case GeomAbs_BezierCurve: return "GeomAbs_BezierCurve";
        case GeomAbs_BSplineCurve: return "GeomAbs_BSplineCurve";
        case GeomAbs_OffsetCurve: return "GeomAbs_OffsetCurve";
        case GeomAbs_OtherCurve: return "GeomAbs_OtherCurve";
    }
    return "GeomAbs_OtherCurve";
}

// The plane-parallel ARC branch: FaceBoundaryProjector.cpp:254-312 verbatim in
// its math (projected endpoints, CCW from the D1 tangent at mid-parameter), with
// its polyline fallback replaced by a NAMED refusal — §7.6 forbids tessellation
// on this verb.
Refusal emitParallelArc(std::vector<Primitive>& out, const BRepAdaptor_Curve& curve,
                        const SketchPlane& plane, const Vec2d& center, double radius) {
    const double first = curve.FirstParameter();
    const double last = curve.LastParameter();
    const gp_Pnt start3d = curve.Value(first);
    const gp_Pnt end3d = curve.Value(last);
    const Vec2d start = plane.toSketch(toVec3(start3d));
    const Vec2d end = plane.toSketch(toVec3(end3d));
    const double startAngle = std::atan2(start.y - center.y, start.x - center.x);
    const double endAngle = std::atan2(end.y - center.y, end.x - center.x);

    bool undetermined = false;
    bool ccw = true;
    gp_Pnt mid3d;
    gp_Vec tangent3d;
    try {
        curve.D1(first + 0.5 * (last - first), mid3d, tangent3d);
    } catch (...) {
        undetermined = true;
    }
    if (!undetermined) {
        const Vec2d mid = plane.toSketch(toVec3(mid3d));
        const Vec2d radial{mid.x - center.x, mid.y - center.y};
        const double tangentU = tangent3d.X() * plane.xAxis.x + tangent3d.Y() * plane.xAxis.y +
                                tangent3d.Z() * plane.xAxis.z;
        const double tangentV = tangent3d.X() * plane.yAxis.x + tangent3d.Y() * plane.yAxis.y +
                                tangent3d.Z() * plane.yAxis.z;
        const double cross = radial.x * tangentV - radial.y * tangentU;
        if (std::abs(cross) <= 1e-9) {
            undetermined = true;
        } else {
            ccw = cross > 0.0;
        }
    }
    if (undetermined) {
        return refuse(RefusalCode::Degenerate,
                      "circular edge has no determinable sweep direction");
    }

    double arcStart = startAngle;
    double arcEnd = endAngle;
    if (!ccw) std::swap(arcStart, arcEnd);
    const double sweep = normalizeAnglePositive(arcEnd - arcStart);
    if (sweep <= kPointEpsilon || nearlyEqual(sweep, twoPi(), 1e-3)) {
        return refuse(RefusalCode::Degenerate, "circular edge sweeps no measurable angle");
    }

    Primitive arc;
    arc.kind = EntityKind::Arc;
    arc.center = center;
    arc.radius = radius;
    arc.startAngle = arcStart;
    arc.endAngle = arcEnd;
    arc.ccw = ccw;
    out.push_back(arc);
    return Refusal{};
}

// SCHEMA §7.6 curve rules for a `GeomAbs_Circle` source. `n_c` is the circle's
// axis, `n_s` the supplied plane's normal; BOTH thresholds are RADIUS-RELATIVE,
// so the Line branch is the exact continuous limit of the Ellipse branch (an
// absolute epsilon would emit micron-thin sliver ellipses instead).
Refusal emitCircular(std::vector<Primitive>& out, const TopoDS_Edge& edge,
                     const BRepAdaptor_Curve& curve, const SketchPlane& plane,
                     const Options& options) {
    const gp_Circ circle = curve.Circle();
    const double radius = circle.Radius();
    if (radius <= kPointEpsilon) {
        return refuse(RefusalCode::Degenerate, "circular edge has no measurable radius");
    }

    const Vec3d nc = toVec3(circle.Axis().Direction());
    const Vec3d ns = normalize3(plane.normal);
    const double axisDot = std::abs(dot3(nc, ns));

    const Vec2d center = plane.toSketch(toVec3(circle.Location()));
    const double first = curve.FirstParameter();
    const double last = curve.LastParameter();
    const double span = std::abs(last - first);
    const bool closed = edge.Closed() || nearlyEqual(span, twoPi(), 1e-3);

    if (radius * (1.0 - axisDot) <= options.radiusTolerance) {
        // Plane-parallel: the projection keeps the radius, so a full circle stays
        // a Circle and a trimmed one an Arc.
        if (!closed) return emitParallelArc(out, curve, plane, center, radius);
        Primitive full;
        full.kind = EntityKind::Circle;
        full.center = center;
        full.radius = radius;
        out.push_back(full);
        return Refusal{};
    }

    if (radius * axisDot <= options.pointMergeTolerance) {
        // Edge-on: the ellipse has collapsed. A CLOSED edge is the full 2r chord
        // along n_c x n_s centred on the projected centre — a sketch on a side
        // face sees a hole as a line, and that is an ANSWER. A TRIMMED edge is
        // the chord between its own projected endpoints; 2r there would invent
        // geometry the model does not have.
        const Vec3d dir = normalize3(cross3(nc, ns));
        if (dir.x == 0.0 && dir.y == 0.0 && dir.z == 0.0) {
            return refuse(RefusalCode::Degenerate, "circular edge has no projectable direction");
        }
        Vec2d a;
        Vec2d b;
        if (closed) {
            const Vec3d c3 = toVec3(circle.Location());
            a = plane.toSketch(Vec3d{c3.x - radius * dir.x, c3.y - radius * dir.y,
                                     c3.z - radius * dir.z});
            b = plane.toSketch(Vec3d{c3.x + radius * dir.x, c3.y + radius * dir.y,
                                     c3.z + radius * dir.z});
        } else {
            a = plane.toSketch(toVec3(curve.Value(first)));
            b = plane.toSketch(toVec3(curve.Value(last)));
        }
        if (distanceSquared(a, b) <= kPointEpsilon * kPointEpsilon) {
            return refuse(RefusalCode::Degenerate, "circular edge projects onto a single point");
        }
        Primitive line;
        line.kind = EntityKind::Line;
        line.a = a;
        line.b = b;
        out.push_back(line);
        return Refusal{};
    }

    // Tilted. A TRIMMED tilted circle has no exact form anywhere in the stack:
    // the wire `Ellipse` carries no angular extent (§7.3), a full ellipse would
    // BOUND A REGION the model does not contain, and a polyline is lossy and
    // permanent. It is refused by name.
    if (!closed) {
        return refuse(RefusalCode::TrimmedTiltedArc,
                      "circular edge is trimmed and tilted; the wire Ellipse has no "
                      "angular extent");
    }
    const Vec3d majorDir = normalize3(cross3(nc, ns));
    if (majorDir.x == 0.0 && majorDir.y == 0.0 && majorDir.z == 0.0) {
        return refuse(RefusalCode::Degenerate, "circular edge has no projectable major axis");
    }
    Primitive ellipse;
    ellipse.kind = EntityKind::Ellipse;
    ellipse.center = center;
    ellipse.majorR = radius;
    ellipse.minorR = radius * axisDot;
    ellipse.rotation = std::atan2(dot3(majorDir, plane.yAxis), dot3(majorDir, plane.xAxis));
    normalizeEllipse(ellipse.majorR, ellipse.minorR, ellipse.rotation);
    out.push_back(ellipse);
    return Refusal{};
}

// One edge -> zero or more primitives. `Degenerate` with an EMPTY `out` is the
// "nothing to say about this edge" answer: `edges` mode reports it as the
// source's refusal, `faceOutline` skips it exactly as FaceBoundaryProjector
// skips a degenerate or zero-length boundary edge.
Refusal projectOneEdge(std::vector<Primitive>& out, const TopoDS_Edge& edge,
                       const SketchPlane& plane, const Options& options) {
    if (edge.IsNull()) {
        return refuse(RefusalCode::Degenerate, "edge is null");
    }
    if (BRep_Tool::Degenerated(edge)) {
        return refuse(RefusalCode::Degenerate, "edge is degenerate (no 3D curve)");
    }

    BRepAdaptor_Curve curve(edge);
    const GeomAbs_CurveType type = curve.GetType();
    if (type == GeomAbs_Line) {
        const Vec2d a = plane.toSketch(toVec3(curve.Value(curve.FirstParameter())));
        const Vec2d b = plane.toSketch(toVec3(curve.Value(curve.LastParameter())));
        if (distanceSquared(a, b) <= kPointEpsilon * kPointEpsilon) {
            return refuse(RefusalCode::Degenerate, "edge projects onto a single point");
        }
        Primitive line;
        line.kind = EntityKind::Line;
        line.a = a;
        line.b = b;
        out.push_back(line);
        return Refusal{};
    }
    if (type == GeomAbs_Circle) {
        return emitCircular(out, edge, curve, plane, options);
    }
    // Every other type, `GeomAbs_Ellipse` INCLUDED: an elliptical source needs a
    // 2x2 decomposition and inherits the trimmed problem, so V1 refuses it by
    // name. There is no polyline fallback on this verb.
    return refuse(RefusalCode::UnsupportedCurve,
                  std::string("unsupported curve type ") + curveTypeName(type));
}

// FaceBoundaryProjector.cpp:317-349 verbatim: OUTER wire first, then holes in
// TopExp_Explorer order.
std::vector<TopoDS_Wire> orderedWires(const TopoDS_Face& face) {
    std::vector<TopoDS_Wire> wires;
    for (TopExp_Explorer exp(face, TopAbs_WIRE); exp.More(); exp.Next()) {
        const TopoDS_Wire wire = TopoDS::Wire(exp.Current());
        if (!wire.IsNull()) wires.push_back(wire);
    }

    TopoDS_Wire outer;
    try {
        outer = BRepTools::OuterWire(face);
    } catch (...) {
        // Explorer order stands.
    }
    if (outer.IsNull()) return wires;
    for (std::size_t i = 0; i < wires.size(); ++i) {
        if (!wires[i].IsSame(outer)) continue;
        if (i != 0) {
            std::rotate(wires.begin(), wires.begin() + static_cast<std::ptrdiff_t>(i),
                        wires.begin() + static_cast<std::ptrdiff_t>(i) + 1);
        }
        break;
    }
    return wires;
}

}  // namespace

const char* EdgeProjector::refusalToken(RefusalCode code) {
    switch (code) {
        case RefusalCode::None: return "";
        case RefusalCode::Absent: return "absent";
        case RefusalCode::KindMismatch: return "kindMismatch";
        case RefusalCode::UnsupportedCurve: return "unsupportedCurve";
        case RefusalCode::TrimmedTiltedArc: return "trimmedTiltedArc";
        case RefusalCode::Degenerate: return "degenerate";
        case RefusalCode::FaceNotPlanar: return "faceNotPlanar";
    }
    return "";
}

EdgeProjector::Refusal EdgeProjector::projectEdge(Buffer& buffer, const TopoDS_Edge& edge,
                                                  const SketchPlane& plane,
                                                  const Options& options,
                                                  std::size_t sourceIndex) {
    std::vector<Primitive> primitives;
    Refusal refusal;
    try {
        refusal = projectOneEdge(primitives, edge, plane, options);
    } catch (...) {
        return refuse(RefusalCode::Degenerate, "edge projection failed");
    }
    if (refusal.refused()) return refusal;
    if (primitives.empty()) {
        return refuse(RefusalCode::Degenerate, "edge projected to no geometry");
    }
    appendPrimitives(buffer, primitives, sourceIndex, options);
    return Refusal{};
}

EdgeProjector::Refusal EdgeProjector::projectFaceOutline(Buffer& buffer, const TopoDS_Face& face,
                                                         const SketchPlane& plane,
                                                         const Options& options,
                                                         std::size_t sourceIndex) {
    if (face.IsNull()) {
        return refuse(RefusalCode::Absent, "face is null");
    }
    gp_Pln facePlane;
    gp_Dir faceNormal;
    if (!modeling::CoplanarFacePatch::planarFacePlaneAndNormal(face, facePlane, faceNormal)) {
        return refuse(RefusalCode::FaceNotPlanar, "faceOutline requires a planar face");
    }

    std::vector<Primitive> primitives;
    try {
        TopTools_MapOfShape emitted;
        for (const TopoDS_Wire& wire : orderedWires(face)) {
            for (BRepTools_WireExplorer edgeExp(wire, face); edgeExp.More(); edgeExp.Next()) {
                const TopoDS_Edge edge = edgeExp.Current();
                if (edge.IsNull() || BRep_Tool::Degenerated(edge)) continue;
                if (!emitted.Add(edge)) continue;  // shared TopoDS_Edge emitted ONCE
                const Refusal refusal = projectOneEdge(primitives, edge, plane, options);
                // A `Degenerate` boundary edge is SKIPPED, matching
                // FaceBoundaryProjector; any NAMED refusal fails the whole
                // outline, because half an outline does not close.
                if (refusal.refused() && refusal.code != RefusalCode::Degenerate) {
                    return refusal;
                }
            }
        }
    } catch (...) {
        return refuse(RefusalCode::Degenerate, "face outline projection failed");
    }

    if (primitives.empty()) {
        return refuse(RefusalCode::Degenerate, "face outline projected to no geometry");
    }
    appendPrimitives(buffer, primitives, sourceIndex, options);
    return Refusal{};
}

std::string EdgeProjector::projectedHash(const Buffer& buffer, const Entity& entity) {
    std::uint64_t hash = hashing::kFnvOffset;

    const auto token = [&hash](const char* text) {
        hash = hashing::fnv1a_update(hash, text, std::strlen(text));
    };
    const auto scalar = [&hash](double value) {
        const std::uint64_t quantized =
            static_cast<std::uint64_t>(std::llround(value / kQuantizationStep));
        std::uint8_t little_endian[8];
        for (int i = 0; i < 8; ++i) {
            little_endian[i] = static_cast<std::uint8_t>((quantized >> (8 * i)) & 0xFFU);
        }
        hash = hashing::fnv1a_update(hash, little_endian, sizeof(little_endian));
    };

    switch (entity.kind) {
        case EntityKind::Line: {
            const Vec2d a = pointAt(buffer, entity.p0);
            const Vec2d b = pointAt(buffer, entity.p1);
            token("Line");
            scalar(a.x);
            scalar(a.y);
            scalar(b.x);
            scalar(b.y);
            break;
        }
        case EntityKind::Circle: {
            const Vec2d c = pointAt(buffer, entity.center);
            token("Circle");
            scalar(c.x);
            scalar(c.y);
            scalar(entity.radius);
            break;
        }
        case EntityKind::Arc: {
            const Vec2d c = pointAt(buffer, entity.center);
            token("Arc");
            scalar(c.x);
            scalar(c.y);
            scalar(entity.radius);
            scalar(entity.startAngle);
            scalar(entity.endAngle);
            // `ccw` is NOT hashed: it is informational (§7.6), and the CCW-ordered
            // angle pair already fixes the drawn curve.
            break;
        }
        case EntityKind::Ellipse: {
            const Vec2d c = pointAt(buffer, entity.center);
            token("Ellipse");
            scalar(c.x);
            scalar(c.y);
            scalar(entity.majorR);
            scalar(entity.minorR);
            scalar(entity.rotation);
            break;
        }
    }
    return hashing::hex16(hash);
}

}  // namespace onecad::core::sketch
