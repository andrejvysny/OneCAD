// test_edge_projector.cpp — WP-P: the `ProjectToSketchPlane` curve rules
// (`sketch/EdgeProjector`), SCHEMA §7.6.
//
// This file holds everything the canonical fixture CANNOT hold. `json_subset`
// compares numbers exactly and matches arrays as a SUBSET, so an irrational
// `minorR`, a threshold boundary, an EMPTY `entities[]`, and "this hash equals
// that hash" are all unassertable there. They are asserted here, against OCCT
// shapes each case builds from named dimensions — every expectation below is a
// consequence of the construction written three lines above it.
//
// The load-bearing properties, in the order they appear:
//   * the tilted projection is EXACT: `minorR = r*cos(theta)` at three angles,
//     and `rotation` is the projected `n_c x n_s` direction;
//   * both thresholds are RADIUS-RELATIVE — a circle just inside the parallel
//     test stays a Circle, one just outside becomes an Ellipse, and the same
//     bracket on the edge-on test is what stops a micron-thin sliver ellipse;
//   * edge-on CLOSED is the full `2r` chord, edge-on TRIMMED is the endpoint
//     chord — the second is not the first, and a 2r segment there would invent
//     geometry the model does not have;
//   * a refusal is ATOMIC: a refused source appends neither a point nor an
//     entity, so `points[]` numbering never records a source that produced
//     nothing (the fixture can only assert "is an array");
//   * `faceOutline` over a coplanar face equals `ProjectFaceBoundary`
//     `scope:"faceOnly"` entity for entity and ref for ref — the same equality
//     the fixture's rounds A/L pin at the wire, checked here at the projector so
//     a divergence names the projector rather than the verb;
//   * `projectedHash` covers the projected UV geometry ONLY: the same 2D curve
//     from different 3D sources hashes the same, sub-grid motion does not move
//     it, supra-grid motion does, and `ccw` is not an input.
//
// No framework: exit code == failure count.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <GeomAPI_PointsToBSpline.hxx>
#include <Geom_BSplineCurve.hxx>
#include <TColgp_Array1OfPnt.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax2.hxx>
#include <gp_Circ.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>

#include "kernel/topology/CoplanarFacePatch.h"
#include "nlohmann/json.hpp"
#include "sketch/EdgeProjector.h"
#include "sketch/FaceBoundaryProjector.h"

using nlohmann::json;
namespace sk = onecad::core::sketch;

using Projector = sk::EdgeProjector;
using Buffer = Projector::Buffer;
using Kind = Projector::EntityKind;
using Code = Projector::RefusalCode;

namespace {

int g_failures = 0;

void check(bool condition, const std::string& message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message.c_str());
        ++g_failures;
    }
}

void check_near(double got, double want, double tolerance, const std::string& message) {
    if (!(std::abs(got - want) <= tolerance)) {
        std::fprintf(stderr, "FAIL: %s (got %.17g want %.17g)\n", message.c_str(), got, want);
        ++g_failures;
    }
}

constexpr double kPi = 3.14159265358979323846;

/// A right-handed UV frame on the world plane z = `oz` (U -> +X, V -> +Y).
sk::SketchPlane xy_basis(double oz) {
    sk::SketchPlane plane;
    plane.origin = {0.0, 0.0, oz};
    plane.xAxis = {1.0, 0.0, 0.0};
    plane.yAxis = {0.0, 1.0, 0.0};
    plane.normal = {0.0, 0.0, 1.0};
    return plane;
}

TopoDS_Edge circle_edge(const gp_Pnt& center, const gp_Dir& axis, double radius) {
    return BRepBuilderAPI_MakeEdge(gp_Circ(gp_Ax2(center, axis), radius)).Edge();
}

TopoDS_Edge arc_edge(const gp_Pnt& center, const gp_Dir& axis, double radius, double from,
                     double to) {
    return BRepBuilderAPI_MakeEdge(gp_Circ(gp_Ax2(center, axis), radius), from, to).Edge();
}

/// An arc whose parametric frame is OURS, not OCCT's default choice of a Vx for
/// the given normal — the endpoint positions are the assertion, so the frame
/// cannot be left to a convention that may differ by OCCT version.
TopoDS_Edge arc_edge_in_frame(const gp_Pnt& center, const gp_Dir& axis, const gp_Dir& xDir,
                              double radius, double from, double to) {
    return BRepBuilderAPI_MakeEdge(gp_Circ(gp_Ax2(center, axis, xDir), radius), from, to).Edge();
}

/// The projector's single-source entry point, over a fresh buffer.
Projector::Refusal project_one(Buffer& buffer, const TopoDS_Edge& edge,
                               const sk::SketchPlane& plane,
                               Projector::Options options = {}) {
    return Projector::projectEdge(buffer, edge, plane, options, 0);
}

double u_of(const Buffer& b, int index) { return b.points[static_cast<std::size_t>(index)].u; }
double v_of(const Buffer& b, int index) { return b.points[static_cast<std::size_t>(index)].v; }

double line_length(const Buffer& b, const Projector::Entity& e) {
    const double du = u_of(b, e.p1) - u_of(b, e.p0);
    const double dv = v_of(b, e.p1) - v_of(b, e.p0);
    return std::sqrt(du * du + dv * dv);
}

/// Full-fidelity serialization — the in-process determinism probe.
std::string serialize(const Buffer& b) {
    json out;
    json points = json::array();
    for (const Projector::Point& p : b.points) points.push_back(json::array({p.u, p.v}));
    out["points"] = std::move(points);
    json entities = json::array();
    for (const Projector::Entity& e : b.entities) {
        entities.push_back(json{{"kind", static_cast<int>(e.kind)},
                                {"p0", e.p0},
                                {"p1", e.p1},
                                {"center", e.center},
                                {"radius", e.radius},
                                {"startAngle", e.startAngle},
                                {"endAngle", e.endAngle},
                                {"ccw", e.ccw},
                                {"majorR", e.majorR},
                                {"minorR", e.minorR},
                                {"rotation", e.rotation},
                                {"hash", Projector::projectedHash(b, e)}});
    }
    out["entities"] = std::move(entities);
    return out.dump();
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. Tilted, closed => Ellipse with minorR = r*cos(theta), exactly.
//
// The circle's axis is tilted by `theta` from the sketch normal, about the world
// +X axis. The direction n_c x n_s stays in the sketch plane and is NOT
// foreshortened (majorR == r); the perpendicular in-circle direction contracts by
// exactly |n_c . n_s| = cos(theta). This is the case a reuse of
// `FaceBoundaryProjector::emitCircularEdge` would answer with a full-radius
// Circle — geometry the model does not contain.
// ═══════════════════════════════════════════════════════════════════════════
void test_tilted_ellipse_three_angles() {
    const double radius = 10.0;
    for (const double theta : {30.0 * kPi / 180.0, 45.0 * kPi / 180.0, 60.0 * kPi / 180.0}) {
        Buffer buffer;
        const gp_Dir axis(0.0, std::sin(theta), std::cos(theta));
        const Projector::Refusal refusal =
            project_one(buffer, circle_edge(gp_Pnt(3.0, 0.0, 7.0), axis, radius), xy_basis(0.0));
        const std::string at = " (theta=" + std::to_string(theta * 180.0 / kPi) + ")";

        check(!refusal.refused(), "tilted closed circle projects" + at);
        check(buffer.entities.size() == 1, "tilted closed circle emits exactly one entity" + at);
        if (buffer.entities.size() != 1) continue;
        const Projector::Entity& e = buffer.entities.front();
        check(e.kind == Kind::Ellipse, "tilted closed circle is an Ellipse" + at);
        check_near(e.majorR, radius, 1e-9, "majorR is the radius verbatim" + at);
        check_near(e.minorR, radius * std::cos(theta), 1e-9, "minorR = r*cos(theta)" + at);
        // n_c x n_s = (sin(theta),0,0) -> +U, so the major axis lies on the U axis.
        check_near(e.rotation, 0.0, 1e-12, "rotation is the projected n_c x n_s angle" + at);
        // The centre is the projected 3D centre, in the plane's UV.
        check_near(u_of(buffer, e.center), 3.0, 1e-9, "ellipse centre u" + at);
        check_near(v_of(buffer, e.center), 0.0, 1e-9, "ellipse centre v" + at);
    }

    // A tilt about the world XY diagonal puts the major axis at -pi/4, which is
    // the value the fixture's round D can only carry as `$any`.
    Buffer diagonal;
    const double theta = 45.0 * kPi / 180.0;
    const double s = std::sin(theta) / std::sqrt(2.0);
    project_one(diagonal, circle_edge(gp_Pnt(0, 0, 0), gp_Dir(s, s, std::cos(theta)), radius),
                xy_basis(0.0));
    check(diagonal.entities.size() == 1 && diagonal.entities[0].kind == Kind::Ellipse,
          "diagonal tilt is an Ellipse");
    if (diagonal.entities.size() == 1) {
        check_near(diagonal.entities[0].rotation, -kPi / 4.0, 1e-12,
                   "diagonal tilt rotation is -pi/4");
        check_near(diagonal.entities[0].minorR, radius * std::cos(theta), 1e-9,
                   "diagonal tilt minorR = r*cos(45deg)");
    }

    // The major axis is a LINE, so `n_c` and `-n_c` are the same projected
    // picture. The rotation is folded onto (-pi/2, pi/2] and the two therefore
    // hash IDENTICALLY — without the fold, §7.6's "the same 2D curve hashes
    // identically" would be false for every Ellipse.
    Buffer flipped;
    project_one(flipped, circle_edge(gp_Pnt(0, 0, 0), gp_Dir(-s, -s, -std::cos(theta)), radius),
                xy_basis(0.0));
    check(flipped.entities.size() == 1, "flipped-axis tilt emits one entity");
    if (flipped.entities.size() == 1 && diagonal.entities.size() == 1) {
        check_near(flipped.entities[0].rotation, diagonal.entities[0].rotation, 1e-12,
                   "a reversed circle axis gives the same rotation");
        check(Projector::projectedHash(flipped, flipped.entities[0]) ==
                  Projector::projectedHash(diagonal, diagonal.entities[0]),
              "a reversed circle axis gives the same projectedHash");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. Edge-on: CLOSED is the 2r chord, TRIMMED is the endpoint chord.
// ═══════════════════════════════════════════════════════════════════════════
void test_edge_on_lines() {
    const double radius = 4.0;
    const gp_Pnt center(5.0, 0.0, 0.0);
    const gp_Dir axis(0.0, 1.0, 0.0);  // circle in the XZ plane; n_c . n_s = 0

    Buffer closed;
    check(!project_one(closed, circle_edge(center, axis, radius), xy_basis(0.0)).refused(),
          "edge-on closed circle projects");
    check(closed.entities.size() == 1, "edge-on closed circle emits exactly one entity");
    if (closed.entities.size() == 1) {
        const Projector::Entity& e = closed.entities.front();
        check(e.kind == Kind::Line, "edge-on closed circle is a Line");
        check_near(line_length(closed, e), 2.0 * radius, 1e-9,
                   "edge-on closed circle is the FULL 2r chord");
        // n_c x n_s = (1,0,0): the chord runs along +U through the centre.
        check_near(u_of(closed, e.p0), 1.0, 1e-9, "2r chord starts at centre - r");
        check_near(u_of(closed, e.p1), 9.0, 1e-9, "2r chord ends at centre + r");
        check_near(v_of(closed, e.p0), 0.0, 1e-9, "2r chord v0");
        check_near(v_of(closed, e.p1), 0.0, 1e-9, "2r chord v1");
    }

    // The same circle TRIMMED. A 2r segment here would invent geometry: the answer
    // is the chord between the two projected ENDPOINTS. In the frame below the
    // whole circle collapses onto the U axis and u(t) = u_c + r*cos(t), so an arc
    // over [0, 3pi/4] runs from u_c + r to u_c - r/sqrt(2) — a chord of
    // r*(1 + sqrt(2)/2), which is neither 2r nor r.
    Buffer trimmed;
    const TopoDS_Edge partial =
        arc_edge_in_frame(center, axis, gp_Dir(1, 0, 0), radius, 0.0, 3.0 * kPi / 4.0);
    check(!project_one(trimmed, partial, xy_basis(0.0)).refused(), "edge-on trimmed arc projects");
    check(trimmed.entities.size() == 1, "edge-on trimmed arc emits exactly one entity");
    if (trimmed.entities.size() == 1) {
        const Projector::Entity& e = trimmed.entities.front();
        check(e.kind == Kind::Line, "edge-on trimmed arc is a Line");
        check(std::abs(line_length(trimmed, e) - 2.0 * radius) > 1e-6,
              "edge-on trimmed arc is NOT the 2r chord");
        check_near(line_length(trimmed, e), radius * (1.0 + std::sqrt(2.0) / 2.0), 1e-9,
                   "edge-on trimmed arc is the endpoint chord");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. Both thresholds are RADIUS-RELATIVE, and they bracket exactly.
//
// Parallel test: r*(1 - |n_c . n_s|) <= radiusTolerance.
// Edge-on  test: r*|n_c . n_s|       <= pointMergeTolerance.
// An ABSOLUTE epsilon on the second would let a 10 mm circle at |n.n_s| = 1e-7
// through as an Ellipse with minorR = 1e-6 mm — a sliver neither the region
// detector nor `Sketch::addEllipse` normalization can handle. These four cases
// straddle both boundaries at +/-10%.
// ═══════════════════════════════════════════════════════════════════════════
void test_threshold_brackets() {
    const double radius = 10.0;
    Projector::Options options;  // 1e-5 / 1e-5, the wire defaults
    const gp_Pnt center(0.0, 0.0, 0.0);

    // --- parallel boundary: 1 - dot = 1e-6 exactly at r = 10 ---
    for (const bool inside : {true, false}) {
        const double gap = inside ? 0.9e-6 : 1.1e-6;
        const double cosine = 1.0 - gap;
        const gp_Dir axis(std::sqrt(1.0 - cosine * cosine), 0.0, cosine);
        Buffer buffer;
        project_one(buffer, circle_edge(center, axis, radius), xy_basis(0.0), options);
        check(buffer.entities.size() == 1, "parallel-boundary circle emits one entity");
        if (buffer.entities.size() != 1) continue;
        if (inside) {
            check(buffer.entities[0].kind == Kind::Circle,
                  "r*(1-dot) just INSIDE radiusTolerance stays a Circle");
            check_near(buffer.entities[0].radius, radius, 1e-12,
                       "the parallel branch keeps the radius verbatim");
        } else {
            check(buffer.entities[0].kind == Kind::Ellipse,
                  "r*(1-dot) just OUTSIDE radiusTolerance becomes an Ellipse");
            check_near(buffer.entities[0].minorR, radius * cosine, 1e-9,
                       "the Ellipse just outside is the continuous partner of the Circle");
        }
    }

    // --- edge-on boundary: r*dot = 1e-5 exactly at r = 10 ---
    for (const bool inside : {true, false}) {
        const double cosine = inside ? 0.9e-6 : 1.1e-6;
        const gp_Dir axis(std::sqrt(1.0 - cosine * cosine), 0.0, cosine);
        Buffer buffer;
        project_one(buffer, circle_edge(center, axis, radius), xy_basis(0.0), options);
        check(buffer.entities.size() == 1, "edge-on-boundary circle emits one entity");
        if (buffer.entities.size() != 1) continue;
        if (inside) {
            check(buffer.entities[0].kind == Kind::Line,
                  "r*dot just INSIDE pointMergeTolerance collapses to a Line");
            check_near(line_length(buffer, buffer.entities[0]), 2.0 * radius, 1e-9,
                       "the collapsed Line is the 2r chord — the continuous limit");
        } else {
            check(buffer.entities[0].kind == Kind::Ellipse,
                  "r*dot just OUTSIDE pointMergeTolerance stays an Ellipse");
            check_near(buffer.entities[0].minorR, radius * cosine, 1e-12,
                       "the thinnest surviving Ellipse is 1.1e-5 mm, not a 1e-6 sliver");
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. Refusals — including the two the canonical fixture cannot reach, and the
//    ATOMICITY the subset matcher cannot express.
// ═══════════════════════════════════════════════════════════════════════════
void test_refusals_are_atomic() {
    const sk::SketchPlane plane = xy_basis(0.0);

    // A trimmed TILTED arc: refused by name, never approximated.
    Buffer tilted;
    const Projector::Refusal trimmed_tilted = project_one(
        tilted, arc_edge(gp_Pnt(0, 0, 0), gp_Dir(0, 1, 1), 5.0, 0.0, kPi / 2.0), plane);
    check(trimmed_tilted.code == Code::TrimmedTiltedArc, "trimmed tilted arc refuses by name");
    check(tilted.points.empty() && tilted.entities.empty(),
          "a refused source appends NOTHING — not even a point");

    // An elliptical SOURCE is refused in V1, naming the GeomAbs type.
    Buffer elliptical;
    TColgp_Array1OfPnt poles(1, 4);
    poles.SetValue(1, gp_Pnt(0, 0, 0));
    poles.SetValue(2, gp_Pnt(1, 2, 0));
    poles.SetValue(3, gp_Pnt(3, -1, 0));
    poles.SetValue(4, gp_Pnt(5, 1, 0));
    const Handle(Geom_BSplineCurve) spline = GeomAPI_PointsToBSpline(poles).Curve();
    const Projector::Refusal spline_refusal =
        project_one(elliptical, BRepBuilderAPI_MakeEdge(spline).Edge(), plane);
    check(spline_refusal.code == Code::UnsupportedCurve, "a spline edge refuses unsupportedCurve");
    check(spline_refusal.message.find("GeomAbs_BSplineCurve") != std::string::npos,
          "the unsupportedCurve message names the GeomAbs type");
    check(elliptical.points.empty() && elliptical.entities.empty(),
          "an unsupported source appends nothing either");

    // An edge that projects onto a single point is `degenerate`, not an entity.
    Buffer collapsed;
    const Projector::Refusal degenerate = project_one(
        collapsed, BRepBuilderAPI_MakeEdge(gp_Pnt(2, 3, 0), gp_Pnt(2, 3, 9)).Edge(), plane);
    check(degenerate.code == Code::Degenerate, "an edge along the plane normal is degenerate");
    check(collapsed.points.empty() && collapsed.entities.empty(),
          "a degenerate source appends nothing");

    // `faceOutline` over a NON-PLANAR face is refused, never approximated.
    const TopoDS_Shape cylinder = BRepPrimAPI_MakeCylinder(4.0, 10.0).Shape();
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(cylinder, TopAbs_FACE, faces);
    TopoDS_Face lateral;
    for (int i = 1; i <= faces.Extent(); ++i) {
        const TopoDS_Face f = TopoDS::Face(faces(i));
        gp_Pln ignored;
        gp_Dir also_ignored;
        if (!onecad::core::modeling::CoplanarFacePatch::planarFacePlaneAndNormal(f, ignored,
                                                                                also_ignored)) {
            lateral = f;
            break;
        }
    }
    check(!lateral.IsNull(), "the test cylinder has a non-planar face to refuse");
    if (!lateral.IsNull()) {
        Buffer buffer;
        const Projector::Refusal refusal =
            Projector::projectFaceOutline(buffer, lateral, plane, {}, 0);
        check(refusal.code == Code::FaceNotPlanar, "faceOutline over a curved face refuses");
        check(buffer.points.empty() && buffer.entities.empty(),
              "a faceNotPlanar source appends nothing");
    }

    // Every code renders its camelCase wire token.
    check(std::string(Projector::refusalToken(Code::Absent)) == "absent", "token absent");
    check(std::string(Projector::refusalToken(Code::KindMismatch)) == "kindMismatch",
          "token kindMismatch");
    check(std::string(Projector::refusalToken(Code::UnsupportedCurve)) == "unsupportedCurve",
          "token unsupportedCurve");
    check(std::string(Projector::refusalToken(Code::TrimmedTiltedArc)) == "trimmedTiltedArc",
          "token trimmedTiltedArc");
    check(std::string(Projector::refusalToken(Code::Degenerate)) == "degenerate",
          "token degenerate");
    check(std::string(Projector::refusalToken(Code::FaceNotPlanar)) == "faceNotPlanar",
          "token faceNotPlanar");
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. A refused source does not disturb the sources around it, and the point
//    merge is GLOBAL across sources (that merge is what closes a projected
//    outline into a region).
// ═══════════════════════════════════════════════════════════════════════════
void test_batch_merge_and_survival() {
    const sk::SketchPlane plane = xy_basis(0.0);
    Buffer buffer;
    Projector::Options options;

    Projector::projectEdge(buffer, BRepBuilderAPI_MakeEdge(gp_Pnt(0, 0, 0), gp_Pnt(10, 0, 0)).Edge(),
                           plane, options, 0);
    const Projector::Refusal refusal = Projector::projectEdge(
        buffer, BRepBuilderAPI_MakeEdge(gp_Pnt(4, 4, 0), gp_Pnt(4, 4, 9)).Edge(), plane, options, 1);
    Projector::projectEdge(buffer, BRepBuilderAPI_MakeEdge(gp_Pnt(10, 0, 0), gp_Pnt(10, 6, 0)).Edge(),
                           plane, options, 2);

    check(refusal.code == Code::Degenerate, "the middle source refused");
    check(buffer.entities.size() == 2, "one dead pick does not void the other two sources");
    check(buffer.points.size() == 3, "the shared endpoint MERGED — 2 lines, 3 points");
    if (buffer.entities.size() == 2 && buffer.points.size() == 3) {
        check(buffer.entities[0].p1 == buffer.entities[1].p0,
              "a ref reused across entities IS the same point");
        check(buffer.entities[0].sourceIndex == 0 && buffer.entities[1].sourceIndex == 2,
              "each entity carries the index of the source that produced it");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 6. `faceOutline` over a COPLANAR face == `ProjectFaceBoundary` faceOnly.
//    SCHEMA §7.6 makes this normative; the fixture pins it at the wire, this
//    pins it at the projector so a divergence names the right file.
// ═══════════════════════════════════════════════════════════════════════════
void test_face_outline_matches_face_boundary_projector() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 20.0, 12.0, 8.0).Shape();
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(box, TopAbs_FACE, faces);
    TopoDS_Face top;
    for (int i = 1; i <= faces.Extent(); ++i) {
        gp_Pln pln;
        gp_Dir normal;
        const TopoDS_Face f = TopoDS::Face(faces(i));
        if (onecad::core::modeling::CoplanarFacePatch::planarFacePlaneAndNormal(f, pln, normal) &&
            std::abs(pln.Location().Z() - 8.0) < 1e-9 && std::abs(normal.Z() - 1.0) < 1e-9) {
            top = f;
            break;
        }
    }
    check(!top.IsNull(), "found the box's top face");
    if (top.IsNull()) return;

    const sk::SketchPlane plane = xy_basis(8.0);
    Buffer mine;
    check(!Projector::projectFaceOutline(mine, top, plane, {}, 0).refused(),
          "faceOutline projects the coplanar top face");
    const sk::FaceBoundaryProjector::Result theirs =
        sk::FaceBoundaryProjector::project(top, {}, plane, {});
    check(theirs.ok, "FaceBoundaryProjector projects the same face");

    check(mine.points.size() == theirs.points.size(), "same point COUNT as ProjectFaceBoundary");
    check(mine.entities.size() == theirs.entities.size(), "same entity COUNT");
    if (mine.points.size() != theirs.points.size() ||
        mine.entities.size() != theirs.entities.size()) {
        return;
    }
    for (std::size_t i = 0; i < mine.points.size(); ++i) {
        check(mine.points[i].u == theirs.points[i].u && mine.points[i].v == theirs.points[i].v,
              "point " + std::to_string(i) + " is identical, in the same slot");
    }
    for (std::size_t i = 0; i < mine.entities.size(); ++i) {
        const Projector::Entity& a = mine.entities[i];
        const sk::FaceBoundaryProjector::Entity& b = theirs.entities[i];
        check(static_cast<int>(a.kind) == static_cast<int>(b.kind),
              "entity " + std::to_string(i) + " has the same kind");
        check(a.p0 == b.p0 && a.p1 == b.p1 && a.center == b.center,
              "entity " + std::to_string(i) + " has the same refs");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 7. `projectedHash` — the projected UV geometry, and nothing else.
// ═══════════════════════════════════════════════════════════════════════════
void test_projected_hash() {
    // The definition, executable: FNV-1a 64 over the type token then the shape
    // scalars as llround(v/1e-6) i64 LE. These three are on exact grid points, so
    // the digests are platform-independent and are the reference a second track
    // can implement against.
    Buffer buffer;
    buffer.points.push_back({0.0, 0.0});
    buffer.points.push_back({10.0, 0.0});
    buffer.points.push_back({5.0, -2.0});

    Projector::Entity line;
    line.kind = Kind::Line;
    line.p0 = 0;
    line.p1 = 1;
    check(Projector::projectedHash(buffer, line) == "3923b4dea42782e9",
          "Line (0,0)-(10,0) hashes to its reference digest");

    Projector::Entity circle;
    circle.kind = Kind::Circle;
    circle.center = 2;
    circle.radius = 4.0;
    check(Projector::projectedHash(buffer, circle) == "302f73a6d21cb78f",
          "Circle (5,-2) r4 hashes to its reference digest");

    Projector::Entity ellipse;
    ellipse.kind = Kind::Ellipse;
    ellipse.center = 0;
    ellipse.majorR = 10.0;
    ellipse.minorR = 5.0;
    ellipse.rotation = 0.0;
    check(Projector::projectedHash(buffer, ellipse) == "b3d924c1617365ce",
          "Ellipse (0,0) 10x5 hashes to its reference digest");

    // `ccw` is NOT hashed: it is informational, and the CCW-ordered angle pair
    // already fixes the drawn curve.
    Projector::Entity arc_ccw;
    arc_ccw.kind = Kind::Arc;
    arc_ccw.center = 2;
    arc_ccw.radius = 4.0;
    arc_ccw.startAngle = 0.25;
    arc_ccw.endAngle = 1.25;
    arc_ccw.ccw = true;
    Projector::Entity arc_cw = arc_ccw;
    arc_cw.ccw = false;
    check(Projector::projectedHash(buffer, arc_ccw) == Projector::projectedHash(buffer, arc_cw),
          "ccw is NOT an input to projectedHash");

    // The 1e-6 grid: motion below it is invisible, motion above it is not. This
    // is what absorbs the libm difference between macOS and Linux.
    Buffer nudged = buffer;
    nudged.points[1].u = 10.0 + 4e-7;
    check(Projector::projectedHash(nudged, line) == Projector::projectedHash(buffer, line),
          "a sub-grid nudge does not move the hash");
    nudged.points[1].u = 10.0 + 4e-6;
    check(Projector::projectedHash(nudged, line) != Projector::projectedHash(buffer, line),
          "a supra-grid move DOES move the hash");

    // Two circular edges at different heights project onto the same 2D circle and
    // hash identically — deliberately. The question the hash answers is "did the
    // picture in this sketch change", not "did the model change".
    Buffer low;
    Buffer high;
    project_one(low, circle_edge(gp_Pnt(2, 3, 0), gp_Dir(0, 0, 1), 6.0), xy_basis(0.0));
    project_one(high, circle_edge(gp_Pnt(2, 3, 40), gp_Dir(0, 0, 1), 6.0), xy_basis(0.0));
    check(low.entities.size() == 1 && high.entities.size() == 1, "both height circles project");
    if (low.entities.size() == 1 && high.entities.size() == 1) {
        check(Projector::projectedHash(low, low.entities[0]) ==
                  Projector::projectedHash(high, high.entities[0]),
              "a source that slides along the sketch normal is NOT stale");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 8. Determinism, in-process: two fresh builds, byte-identical serialization.
//    (The ACROSS-PROCESS half is `canonical_project_to_sketch_plane_frames`,
//    which replays the canonical fixture's request stream twice.)
// ═══════════════════════════════════════════════════════════════════════════
void test_determinism() {
    std::string first;
    for (int run = 0; run < 2; ++run) {
        Buffer buffer;
        Projector::Options options;
        Projector::projectEdge(buffer, circle_edge(gp_Pnt(1, 2, 3), gp_Dir(0, 1, 2), 7.0),
                               xy_basis(0.0), options, 0);
        Projector::projectEdge(buffer, arc_edge(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1), 5.0, 0.0,
                                                kPi / 3.0),
                               xy_basis(0.0), options, 1);
        Projector::projectEdge(buffer, circle_edge(gp_Pnt(20, 0, 0), gp_Dir(1, 0, 0), 3.0),
                               xy_basis(0.0), options, 2);
        const std::string dump = serialize(buffer);
        if (run == 0) {
            first = dump;
        } else {
            check(dump == first, "determinism: two fresh projector runs are byte-identical");
        }
    }
    check(!first.empty(), "the determinism probe projected something");
}

}  // namespace

int main() {
    test_tilted_ellipse_three_angles();
    test_edge_on_lines();
    test_threshold_brackets();
    test_refusals_are_atomic();
    test_batch_merge_and_survival();
    test_face_outline_matches_face_boundary_projector();
    test_projected_hash();
    test_determinism();

    if (g_failures == 0) {
        std::fprintf(stderr, "test_edge_projector: OK\n");
    } else {
        std::fprintf(stderr, "test_edge_projector: %d failure(s)\n", g_failures);
    }
    return g_failures;
}
