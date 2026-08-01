// test_face_projection.cpp — SKETCH-ON-FACE W1a: the face-boundary projector
// (`sketch/FaceBoundaryProjector`) + the `ProjectFaceBoundary` verb (SCHEMA §7.6).
//
// Everything asserted here is SELF-DERIVED: each case builds its own OCCT shape
// from named dimensions, so every numeric expectation is a consequence of the
// construction written three lines above it (no corpus recording is involved —
// the oracle has no equivalent multi-face/pure-buffer entry point to record).
//
// The load-bearing properties, in the order they appear:
//   * exact primitives — a Line stays a segment, a full circular hole stays a
//     Circle, a fillet stays an Arc; nothing is silently tessellated;
//   * the plane is INPUT — every `at` is in the caller's UV, and a tilted face
//     round-trips back onto itself through the frame the verb handed out;
//   * the coplanar scan is an ALL-FACES scan — a U-body's two disconnected prong
//     tops both project (a BFS finds one, and this file asserts that difference so
//     swapping the entry points back is a hard failure);
//   * a shared TopoDS_Edge is emitted ONCE, and coincident endpoints merge;
//   * a non-planar seed is REFUSED, never approximated;
//   * the response is byte-identical across fresh runs.
//
// No framework: exit code == failure count.
#include <cmath>
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRep_Tool.hxx>
#include <GeomAPI_PointsToBSpline.hxx>
#include <Geom_BSplineCurve.hxx>
#include <TColgp_Array1OfPnt.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Circ.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include "elementmap/ElementMapPartition.h"
#include "kernel/topology/CoplanarFacePatch.h"
#include "nlohmann/json.hpp"
#include "session/FaceProjection.h"
#include "session/HistoryHash.h"
#include "session/ScratchJob.h"
#include "session/Session.h"
#include "sketch/FaceBoundaryProjector.h"

using nlohmann::json;
using onecad::protocol::Envelope;
using onecad::session::Session;

namespace fb = onecad::core::sketch;
namespace kt = onecad::core::modeling;
namespace em = onecad::elementmap;
namespace km = onecad::kernel::elementmap;

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
        std::fprintf(stderr, "FAIL: %s (got %.12f want %.12f)\n", message.c_str(), got, want);
        ++g_failures;
    }
}

using Projector = fb::FaceBoundaryProjector;
using Kind = Projector::EntityKind;

constexpr double kPi = 3.14159265358979323846;
constexpr double kExact = 1e-9;  // OCCT reproduces box/cylinder dimensions to ~1e-13

// ── shape helpers ───────────────────────────────────────────────────────────

/// A right-handed UV frame on the world plane z = `oz` (U→+X, V→+Y).
fb::SketchPlane xy_basis(double oz) {
    fb::SketchPlane plane;
    plane.origin = {0.0, 0.0, oz};
    plane.xAxis = {1.0, 0.0, 0.0};
    plane.yAxis = {0.0, 1.0, 0.0};
    plane.normal = {0.0, 0.0, 1.0};
    return plane;
}

/// The sub-shape of `type` whose descriptor centre is nearest (cx,cy,cz) — the
/// same "pick what the user clicked" helper test_wp6_extrude / test_m6a_ops use.
TopoDS_Shape sub_shape_near(const TopoDS_Shape& shape, TopAbs_ShapeEnum type, double cx,
                            double cy, double cz) {
    TopTools_IndexedMapOfShape map;
    TopExp::MapShapes(shape, type, map);
    TopoDS_Shape best;
    double best_d2 = -1.0;
    for (int i = 1; i <= map.Extent(); ++i) {
        const km::ElementDescriptor d = em::ElementMapPartition::describe(map(i));
        const double dx = d.center.X() - cx, dy = d.center.Y() - cy, dz = d.center.Z() - cz;
        const double d2 = dx * dx + dy * dy + dz * dz;
        if (best_d2 < 0.0 || d2 < best_d2) {
            best_d2 = d2;
            best = map(i);
        }
    }
    return best;
}

TopoDS_Face face_near(const TopoDS_Shape& shape, double cx, double cy, double cz) {
    return TopoDS::Face(sub_shape_near(shape, TopAbs_FACE, cx, cy, cz));
}

/// The planar face whose orientation-corrected normal is closest to (nx,ny,nz).
TopoDS_Face face_by_normal(const TopoDS_Shape& shape, double nx, double ny, double nz) {
    TopoDS_Face best;
    double best_dot = -2.0;
    for (TopExp_Explorer exp(shape, TopAbs_FACE); exp.More(); exp.Next()) {
        const TopoDS_Face face = TopoDS::Face(exp.Current());
        gp_Pln plane;
        gp_Dir normal;
        if (!kt::CoplanarFacePatch::planarFacePlaneAndNormal(face, plane, normal)) continue;
        const double dot = normal.X() * nx + normal.Y() * ny + normal.Z() * nz;
        if (dot > best_dot) {
            best_dot = dot;
            best = face;
        }
    }
    return best;
}

/// The lateral (non-planar) face of a cylinder.
TopoDS_Face non_planar_face(const TopoDS_Shape& shape) {
    for (TopExp_Explorer exp(shape, TopAbs_FACE); exp.More(); exp.Next()) {
        const TopoDS_Face face = TopoDS::Face(exp.Current());
        BRepAdaptor_Surface surface(face, true);
        if (surface.GetType() != GeomAbs_Plane) return face;
    }
    return {};
}

int count_of(const TopoDS_Shape& shape, TopAbs_ShapeEnum type) {
    TopTools_IndexedMapOfShape map;
    TopExp::MapShapes(shape, type, map);
    return map.Extent();
}

// ── projector-result helpers ────────────────────────────────────────────────

int point_at(const Projector::Result& r, double u, double v) {
    for (std::size_t i = 0; i < r.points.size(); ++i) {
        if (std::abs(r.points[i].u - u) <= kExact && std::abs(r.points[i].v - v) <= kExact) {
            return static_cast<int>(i);
        }
    }
    return -1;
}

int count_points_at(const Projector::Result& r, double u, double v) {
    int n = 0;
    for (const Projector::Point& p : r.points) {
        if (std::abs(p.u - u) <= kExact && std::abs(p.v - v) <= kExact) ++n;
    }
    return n;
}

int count_of_kind(const Projector::Result& r, Kind kind) {
    int n = 0;
    for (const Projector::Entity& e : r.entities) {
        if (e.kind == kind) ++n;
    }
    return n;
}

/// A Line joining (ax,ay)-(bx,by) in either direction.
bool has_line(const Projector::Result& r, double ax, double ay, double bx, double by) {
    const int a = point_at(r, ax, ay);
    const int b = point_at(r, bx, by);
    if (a < 0 || b < 0) return false;
    for (const Projector::Entity& e : r.entities) {
        if (e.kind != Kind::Line) continue;
        if ((e.p0 == a && e.p1 == b) || (e.p0 == b && e.p1 == a)) return true;
    }
    return false;
}

/// Every Line of `subset` also appears in `superset` (as an unordered UV pair).
bool lines_are_subset(const Projector::Result& subset, const Projector::Result& superset) {
    for (const Projector::Entity& e : subset.entities) {
        if (e.kind != Kind::Line) continue;
        const Projector::Point a = subset.points[static_cast<std::size_t>(e.p0)];
        const Projector::Point b = subset.points[static_cast<std::size_t>(e.p1)];
        if (!has_line(superset, a.u, a.v, b.u, b.v)) return false;
    }
    return true;
}

/// Full-fidelity serialization — the determinism probe.
std::string serialize(const Projector::Result& r) {
    json out;
    out["ok"] = r.ok;
    out["hasClosedBoundary"] = r.hasClosedBoundary;
    out["errorMessage"] = r.errorMessage;
    json points = json::array();
    for (const Projector::Point& p : r.points) points.push_back(json::array({p.u, p.v}));
    out["points"] = std::move(points);
    json entities = json::array();
    for (const Projector::Entity& e : r.entities) {
        entities.push_back(json{{"kind", static_cast<int>(e.kind)},
                                {"p0", e.p0},
                                {"p1", e.p1},
                                {"center", e.center},
                                {"radius", e.radius},
                                {"startAngle", e.startAngle},
                                {"endAngle", e.endAngle},
                                {"ccw", e.ccw}});
    }
    out["entities"] = std::move(entities);
    return out.dump();
}

// ── shape constructors (each case's provenance) ─────────────────────────────

/// 80 × 60 × 30 box with its min corner at the world origin ⇒ top face at z=30
/// with corners (0,0) (80,0) (80,60) (0,60) in the xy_basis(30) UV.
TopoDS_Shape make_box() {
    return BRepPrimAPI_MakeBox(80.0, 60.0, 30.0).Shape();
}

/// The same box minus a through-cylinder of radius 10 centred at (40,30) ⇒ the
/// top face gains one circular hole wire.
TopoDS_Shape make_box_with_hole() {
    const TopoDS_Shape cylinder =
        BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(40.0, 30.0, -5.0), gp_Dir(0, 0, 1)), 10.0, 50.0)
            .Shape();
    return BRepAlgoAPI_Cut(make_box(), cylinder).Shape();
}

/// The same box with a radius-10 fillet on the vertical edge at (80,60) ⇒ the top
/// face boundary becomes 4 lines + a quarter arc centred at (70,50).
TopoDS_Shape make_box_with_fillet() {
    const TopoDS_Shape box = make_box();
    const TopoDS_Edge edge = TopoDS::Edge(sub_shape_near(box, TopAbs_EDGE, 80.0, 60.0, 15.0));
    BRepFilletAPI_MakeFillet fillet(box);
    fillet.Add(10.0, edge);
    fillet.Build();
    return fillet.Shape();
}

/// L in plan: 80×60 block fused with a 30×40 block at y∈[60,100] — the two top
/// faces are coplanar at z=30, CONNECTED, and share the edge (0,60)-(30,60).
TopoDS_Shape make_l_body() {
    const TopoDS_Shape a = BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 80.0, 60.0, 30.0).Shape();
    const TopoDS_Shape b = BRepPrimAPI_MakeBox(gp_Pnt(0, 60, 0), 30.0, 40.0, 30.0).Shape();
    return BRepAlgoAPI_Fuse(a, b).Shape();
}

/// U in section: a 100×60×10 base with two 20×60×20 prongs at x∈[0,20] and
/// x∈[80,100] — the two prong tops are coplanar at z=30 but DISCONNECTED.
TopoDS_Shape make_u_body() {
    const TopoDS_Shape base = BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 100.0, 60.0, 10.0).Shape();
    const TopoDS_Shape left = BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 10), 20.0, 60.0, 20.0).Shape();
    const TopoDS_Shape right = BRepPrimAPI_MakeBox(gp_Pnt(80, 0, 10), 20.0, 60.0, 20.0).Shape();
    return BRepAlgoAPI_Fuse(BRepAlgoAPI_Fuse(base, left).Shape(), right).Shape();
}

/// The box rotated 30° about the world X axis ⇒ its top face plane is tilted
/// (normal (0, −sin30, cos30)) and no longer axis-aligned.
TopoDS_Shape make_tilted_box() {
    gp_Trsf rotate;
    rotate.SetRotation(gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(1, 0, 0)), 30.0 * kPi / 180.0);
    return BRepBuilderAPI_Transform(make_box(), rotate, true).Shape();
}

/// A planar face on z=0 bounded by three segments and one B-spline — the only
/// curve type in this file that has no exact 2D primitive.
TopoDS_Face make_spline_face() {
    TColgp_Array1OfPnt pts(1, 5);
    pts.SetValue(1, gp_Pnt(100, 50, 0));
    pts.SetValue(2, gp_Pnt(75, 70, 0));
    pts.SetValue(3, gp_Pnt(50, 40, 0));
    pts.SetValue(4, gp_Pnt(25, 70, 0));
    pts.SetValue(5, gp_Pnt(0, 50, 0));
    const Handle(Geom_BSplineCurve) spline = GeomAPI_PointsToBSpline(pts).Curve();

    BRepBuilderAPI_MakeWire wire;
    wire.Add(BRepBuilderAPI_MakeEdge(gp_Pnt(0, 0, 0), gp_Pnt(100, 0, 0)).Edge());
    wire.Add(BRepBuilderAPI_MakeEdge(gp_Pnt(100, 0, 0), gp_Pnt(100, 50, 0)).Edge());
    wire.Add(BRepBuilderAPI_MakeEdge(spline).Edge());
    wire.Add(BRepBuilderAPI_MakeEdge(gp_Pnt(0, 50, 0), gp_Pnt(0, 0, 0)).Edge());
    return BRepBuilderAPI_MakeFace(gp_Pln(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), wire.Wire()).Face();
}

/// The faces of `body` coplanar with `plane`, minus `seed` — what the verb hands
/// the projector in `coplanarBody` scope.
std::vector<TopoDS_Face> companions_of(const TopoDS_Shape& body, const TopoDS_Face& seed,
                                       const gp_Pln& plane) {
    std::vector<TopoDS_Face> out;
    for (const TopoDS_Face& face : kt::CoplanarFacePatch::collectCoplanarFaces(body, plane, {})) {
        if (!face.IsNull() && !face.IsSame(seed)) out.push_back(face);
    }
    return out;
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. Box top face: 4 points, 4 exact Lines, closed, exact corner UV.
// ═══════════════════════════════════════════════════════════════════════════
void test_box_top_exact_corners() {
    const TopoDS_Shape box = make_box();
    const TopoDS_Face top = face_near(box, 40, 30, 30);
    const Projector::Result r = Projector::project(top, {}, xy_basis(30.0), {});

    check(r.ok, "box top: ok");
    check(r.hasClosedBoundary, "box top: closed boundary");
    check(r.points.size() == 4, "box top: 4 points (one per corner)");
    check(r.entities.size() == 4, "box top: 4 entities");
    check(count_of_kind(r, Kind::Line) == 4, "box top: every entity is an exact Line");

    // The 80×60 rectangle, exactly, in the supplied basis.
    check(point_at(r, 0.0, 0.0) >= 0, "box top: corner (0,0)");
    check(point_at(r, 80.0, 0.0) >= 0, "box top: corner (80,0)");
    check(point_at(r, 80.0, 60.0) >= 0, "box top: corner (80,60)");
    check(point_at(r, 0.0, 60.0) >= 0, "box top: corner (0,60)");
    check(has_line(r, 0, 0, 80, 0), "box top: edge (0,0)-(80,0)");
    check(has_line(r, 80, 0, 80, 60), "box top: edge (80,0)-(80,60)");
    check(has_line(r, 80, 60, 0, 60), "box top: edge (80,60)-(0,60)");
    check(has_line(r, 0, 60, 0, 0), "box top: edge (0,60)-(0,0)");

    // The ORDER is normative (SCHEMA §7.6): wire-walk order, points numbered by
    // first use. Pinned so a reordering of the walk is a hard failure.
    check(point_at(r, 0.0, 0.0) == 0 && point_at(r, 0.0, 60.0) == 1 &&
              point_at(r, 80.0, 0.0) == 2 && point_at(r, 80.0, 60.0) == 3,
          "box top: points numbered by first use along the wire walk");
    check(r.entities[0].p0 == 0 && r.entities[0].p1 == 1 && r.entities[1].p0 == 0 &&
              r.entities[1].p1 == 2 && r.entities[2].p0 == 2 && r.entities[2].p1 == 3 &&
              r.entities[3].p0 == 1 && r.entities[3].p1 == 3,
          "box top: entities in BRepTools_WireExplorer order");
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. Face with a circular hole: outer wire (4 Lines) FIRST, then the hole
//    (1 exact Circle). The hole centre is one merged point.
// ═══════════════════════════════════════════════════════════════════════════
void test_hole_face_exact_circle() {
    const TopoDS_Shape body = make_box_with_hole();
    const TopoDS_Face top = face_near(body, 40, 30, 30);
    const Projector::Result r = Projector::project(top, {}, xy_basis(30.0), {});

    check(r.ok && r.hasClosedBoundary, "hole face: ok + closed");
    check(r.points.size() == 5, "hole face: 4 corners + 1 hole centre");
    check(r.entities.size() == 5, "hole face: 4 Lines + 1 Circle");
    check(count_of_kind(r, Kind::Line) == 4, "hole face: 4 Lines");
    check(count_of_kind(r, Kind::Circle) == 1, "hole face: the hole stayed an exact Circle");

    // Outer wire first, hole after (normative ordering).
    check(r.entities[0].kind == Kind::Line && r.entities[1].kind == Kind::Line &&
              r.entities[2].kind == Kind::Line && r.entities[3].kind == Kind::Line &&
              r.entities[4].kind == Kind::Circle,
          "hole face: outer wire emitted before the hole wire");

    for (const Projector::Entity& e : r.entities) {
        if (e.kind != Kind::Circle) continue;
        check_near(e.radius, 10.0, kExact, "hole face: exact cut radius 10");
        check_near(r.points[static_cast<std::size_t>(e.center)].u, 40.0, kExact,
                   "hole face: centre u");
        check_near(r.points[static_cast<std::size_t>(e.center)].v, 30.0, kExact,
                   "hole face: centre v");
    }
    check(count_points_at(r, 40.0, 30.0) == 1, "hole face: the hole centre is ONE merged point");
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. Filleted corner: an exact Arc, radius 10, centred at (70,50), CCW-ordered
//    angles, and `ccw` agreeing with an INDEPENDENT sampling of the edge.
// ═══════════════════════════════════════════════════════════════════════════
void test_fillet_arc() {
    const TopoDS_Shape body = make_box_with_fillet();
    const TopoDS_Face top = face_near(body, 40, 30, 30);
    const fb::SketchPlane plane = xy_basis(30.0);
    const Projector::Result r = Projector::project(top, {}, plane, {});

    check(r.ok && r.hasClosedBoundary, "fillet: ok + closed");
    check(count_of_kind(r, Kind::Arc) == 1, "fillet: the corner stayed an exact Arc");
    check(count_of_kind(r, Kind::Line) == 4, "fillet: the four straight edges stayed Lines");

    // Independent CCW derivation: sample the real circular edge of this face at
    // first/mid/last, project into the SAME basis, and take the signed area. This
    // shares nothing with the projector's D1-tangent rule.
    bool sampled_ccw = false;
    bool found_arc_edge = false;
    for (TopExp_Explorer exp(top, TopAbs_EDGE); exp.More(); exp.Next()) {
        const TopoDS_Edge edge = TopoDS::Edge(exp.Current());
        const BRepAdaptor_Curve curve(edge);
        if (curve.GetType() != GeomAbs_Circle) continue;
        found_arc_edge = true;
        const double t0 = curve.FirstParameter();
        const double t1 = curve.LastParameter();
        fb::Vec2d s[3];
        for (int i = 0; i < 3; ++i) {
            const gp_Pnt p = curve.Value(t0 + (t1 - t0) * (0.5 * i));
            s[i] = plane.toSketch({p.X(), p.Y(), p.Z()});
        }
        sampled_ccw = ((s[1].x - s[0].x) * (s[2].y - s[0].y) -
                       (s[1].y - s[0].y) * (s[2].x - s[0].x)) > 0.0;
        break;
    }
    check(found_arc_edge, "fillet: the top face really carries a circular edge");

    for (const Projector::Entity& e : r.entities) {
        if (e.kind != Kind::Arc) continue;
        check_near(e.radius, 10.0, kExact, "fillet: exact arc radius 10");
        check_near(r.points[static_cast<std::size_t>(e.center)].u, 70.0, kExact,
                   "fillet: arc centre u = 80−10");
        check_near(r.points[static_cast<std::size_t>(e.center)].v, 50.0, kExact,
                   "fillet: arc centre v = 60−10");
        // (80,50) is at angle 0 and (70,60) at +π/2 about (70,50); the emitted pair
        // is always the CCW-ordered one, so the sweep is exactly a quarter turn.
        check_near(e.startAngle, 0.0, 1e-9, "fillet: startAngle 0 (the (80,50) end)");
        check_near(e.endAngle, kPi / 2.0, 1e-9, "fillet: endAngle π/2 (the (70,60) end)");
        check(e.ccw == sampled_ccw, "fillet: ccw matches an independent 3-sample orientation");
        // The arc's ends coincide with the two straight edges it replaced.
        check(point_at(r, 80.0, 50.0) >= 0, "fillet: arc start point (80,50) exists");
        check(point_at(r, 70.0, 60.0) >= 0, "fillet: arc end point (70,60) exists");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. Non-planar seed is REFUSED at the projector, never approximated.
// ═══════════════════════════════════════════════════════════════════════════
void test_non_planar_seed_refused() {
    const TopoDS_Shape cylinder = BRepPrimAPI_MakeCylinder(10.0, 20.0).Shape();
    const TopoDS_Face lateral = non_planar_face(cylinder);
    check(!lateral.IsNull(), "non-planar: the cylinder really has a lateral face");

    const Projector::Result r = Projector::project(lateral, {}, xy_basis(0.0), {});
    check(!r.ok, "non-planar: projector refuses");
    check(r.errorMessage.find("not planar") != std::string::npos,
          "non-planar: names the reason (got '" + r.errorMessage + "')");
    check(r.entities.empty() && r.points.empty(), "non-planar: nothing approximated");
}

// ═══════════════════════════════════════════════════════════════════════════
// 6. L body, coplanarBody: two CONNECTED coplanar faces, shared edge ONCE.
// ═══════════════════════════════════════════════════════════════════════════
void test_lshape_shared_edge_emitted_once() {
    const TopoDS_Shape body = make_l_body();
    const TopoDS_Face seed = face_near(body, 40, 30, 30);
    const gp_Pln plane(gp_Pnt(0, 0, 30), gp_Dir(0, 0, 1));
    const std::vector<TopoDS_Face> companions = companions_of(body, seed, plane);
    check(companions.size() == 1, "L: exactly one coplanar companion face");

    const Projector::Result r = Projector::project(seed, companions, xy_basis(30.0), {});
    check(r.ok && r.hasClosedBoundary, "L: ok + closed");
    // Seed face has 5 edges (its y=60 side is split at x=30 by the other block),
    // the companion has 4, and (0,60)-(30,60) is SHARED ⇒ 5 + 4 − 1 = 8.
    // A double-emitted shared edge would make this 9.
    check(r.entities.size() == 8, "L: 8 entities — the shared interior edge emitted ONCE");
    check(count_of_kind(r, Kind::Line) == 8, "L: all Lines");
    check(r.points.size() == 7, "L: 7 merged points");
    check(has_line(r, 0, 60, 30, 60), "L: the shared interior edge is present (it splits regions)");
    check(has_line(r, 30, 60, 80, 60), "L: the free half of the seed's split edge");
    check(has_line(r, 0, 100, 30, 100), "L: the companion's far edge");
}

// ═══════════════════════════════════════════════════════════════════════════
// 7. U body: the two prong tops are coplanar but DISCONNECTED. This is the case
//    that discriminates the all-faces scan from the shipped BFS — if anyone
//    swaps `collectCoplanarFaces` back to `collectConnectedFaces`, this fails.
// ═══════════════════════════════════════════════════════════════════════════
void test_ushape_disconnected_coplanar_faces() {
    const TopoDS_Shape body = make_u_body();
    const TopoDS_Face seed = face_near(body, 10, 30, 30);
    const gp_Pln plane(gp_Pnt(0, 0, 30), gp_Dir(0, 0, 1));

    const std::vector<TopoDS_Face> scanned =
        kt::CoplanarFacePatch::collectCoplanarFaces(body, plane, {});
    const std::vector<TopoDS_Face> bfs = kt::CoplanarFacePatch::collectConnectedFaces(body, seed);
    check(scanned.size() == 2, "U: the all-faces scan finds BOTH prong tops");
    check(bfs.size() == 1, "U: the adjacency BFS finds only the seed's prong (the discriminator)");

    const Projector::Result r =
        Projector::project(seed, companions_of(body, seed, plane), xy_basis(30.0), {});
    check(r.ok && r.hasClosedBoundary, "U: ok + closed");
    check(r.entities.size() == 8, "U: 4 + 4 edges, nothing shared");
    check(r.points.size() == 8, "U: 8 corners");
    check(has_line(r, 0, 0, 20, 0) && has_line(r, 20, 0, 20, 60),
          "U: the seed prong's boundary");
    check(has_line(r, 80, 0, 100, 0) && has_line(r, 100, 0, 100, 60),
          "U: the DISCONNECTED prong's boundary projected too");
}

// ═══════════════════════════════════════════════════════════════════════════
// 8. faceOnly is a strict subset of coplanarBody on the same body.
// ═══════════════════════════════════════════════════════════════════════════
void test_face_only_is_strict_subset() {
    const TopoDS_Shape body = make_l_body();
    const TopoDS_Face seed = face_near(body, 40, 30, 30);
    const gp_Pln plane(gp_Pnt(0, 0, 30), gp_Dir(0, 0, 1));

    const Projector::Result whole =
        Projector::project(seed, companions_of(body, seed, plane), xy_basis(30.0), {});
    const Projector::Result only = Projector::project(seed, {}, xy_basis(30.0), {});

    check(only.ok && only.hasClosedBoundary, "faceOnly: ok + closed");
    check(only.entities.size() == 5, "faceOnly: just the seed face's 5 edges");
    check(only.entities.size() < whole.entities.size(), "faceOnly: STRICTLY fewer than coplanarBody");
    check(lines_are_subset(only, whole), "faceOnly: every emitted line is also in coplanarBody");
}

// ═══════════════════════════════════════════════════════════════════════════
// 9. Unsupported curve type ⇒ documented polyline fallback, honouring
//    fallbackSegmentsPerCurve exactly, and still closing the boundary.
// ═══════════════════════════════════════════════════════════════════════════
void test_spline_polyline_fallback() {
    const TopoDS_Face face = make_spline_face();

    const Projector::Result r = Projector::project(face, {}, xy_basis(0.0), {});
    check(r.ok && r.hasClosedBoundary, "spline: ok + still closed");
    // 3 exact segments + 24 fallback segments for the one B-spline.
    check(r.entities.size() == 27, "spline: 3 Lines + 24 fallback segments (default)");
    check(count_of_kind(r, Kind::Line) == 27, "spline: the fallback emits Lines only");
    // 4 wire corners + 23 interior sample points (the sample endpoints merge).
    check(r.points.size() == 27, "spline: sample endpoints merged onto the corners");

    Projector::Options coarse;
    coarse.fallbackSegmentsPerCurve = 8;
    const Projector::Result c = Projector::project(face, {}, xy_basis(0.0), coarse);
    check(c.entities.size() == 11, "spline: fallbackSegmentsPerCurve=8 ⇒ 3 + 8 entities");
    check(c.hasClosedBoundary, "spline: coarse fallback still closes");
}

// ═══════════════════════════════════════════════════════════════════════════
// 10. Point merge: adjacent edges SHARE endpoints — one point per face vertex,
//     not one per edge end.
// ═══════════════════════════════════════════════════════════════════════════
void test_point_merge() {
    const TopoDS_Shape box = make_box();
    const TopoDS_Face top = face_near(box, 40, 30, 30);
    const Projector::Result r = Projector::project(top, {}, xy_basis(30.0), {});

    const int vertices = count_of(top, TopAbs_VERTEX);
    check(vertices == 4, "merge: the top face has 4 vertices");
    check(static_cast<int>(r.points.size()) == vertices,
          "merge: one point per VERTEX (4), not per edge end (8)");
    for (const Projector::Entity& e : r.entities) {
        check(count_points_at(r, r.points[static_cast<std::size_t>(e.p0)].u,
                              r.points[static_cast<std::size_t>(e.p0)].v) == 1,
              "merge: no duplicate slot for a shared endpoint");
    }

    // The L body exercises the same rule across two faces: 5 + 4 edge ends worth
    // of corners collapse to 7 distinct points.
    const TopoDS_Shape l = make_l_body();
    const TopoDS_Face seed = face_near(l, 40, 30, 30);
    const gp_Pln plane(gp_Pnt(0, 0, 30), gp_Dir(0, 0, 1));
    const Projector::Result lr =
        Projector::project(seed, companions_of(l, seed, plane), xy_basis(30.0), {});
    check(lr.points.size() == 7, "merge: cross-face endpoints merge too (L body → 7 points)");
}

// ═══════════════════════════════════════════════════════════════════════════
// 11a. Determinism at the projector: two fresh builds, byte-identical result.
// ═══════════════════════════════════════════════════════════════════════════
void test_projector_determinism() {
    std::string first;
    for (int run = 0; run < 2; ++run) {
        const TopoDS_Shape body = make_l_body();  // rebuilt from scratch each run
        const TopoDS_Face seed = face_near(body, 40, 30, 30);
        const gp_Pln plane(gp_Pnt(0, 0, 30), gp_Dir(0, 0, 1));
        const std::string dump = serialize(
            Projector::project(seed, companions_of(body, seed, plane), xy_basis(30.0), {}));
        if (run == 0) {
            first = dump;
        } else {
            check(dump == first, "determinism: two fresh projector runs are byte-identical");
        }
    }

    // …and for a shape whose emission includes an Arc + a Circle.
    std::string firstMixed;
    for (int run = 0; run < 2; ++run) {
        const TopoDS_Shape body = make_box_with_hole();
        const std::string dump =
            serialize(Projector::project(face_near(body, 40, 30, 30), {}, xy_basis(30.0), {}));
        if (run == 0) {
            firstMixed = dump;
        } else {
            check(dump == firstMixed, "determinism: Circle-bearing result is byte-identical too");
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Verb-level (SCHEMA §7.6): in-process through `handle_project_face_boundary`.
// ═══════════════════════════════════════════════════════════════════════════

/// Seed one body (and optionally one minted element) into the session HEAD via
/// the REAL transaction — the same route test_preview_op.cpp takes, because
/// `Session` has no test-only setter and inventing one would exercise a path
/// production never takes.
void seed_head(Session& session, const std::string& body_id, const TopoDS_Shape& shape,
               const std::string& element_id = {}, const std::string& topo_key = {}) {
    session.open("doc", /*documentRevision=*/1, /*workerEpoch=*/1, "determinism");
    onecad::session::FenceOutcome fence = session.fence_and_clone(
        /*job_id=*/1, /*document_revision=*/1, /*worker_epoch=*/1,
        onecad::session::kEmptyPrefixHash);
    onecad::session::ScratchJob job;
    job.job_id = 1;
    job.plan_document_revision = 1;
    job.bodies = std::move(fence.cloned_bodies);
    job.partition = std::move(fence.cloned_partition);
    job.prepared_snapshot_id = fence.prepared_snapshot_id;
    job.bodies.create(body_id, "op_seed", shape);
    if (!element_id.empty()) {
        const TopoDS_Shape sub = em::ElementMapPartition::shape_for_topokey(shape, topo_key);
        job.partition.mint(body_id, element_id, km::ElementKind::Face, sub, shape);
    }
    session.store_prepared(std::move(job));
    session.accept_prepared(/*job_id=*/1, /*document_revision=*/1, /*worker_epoch=*/1);
}

Envelope project_req(json args) {
    Envelope req;
    req.id = 7;
    req.verb = "ProjectFaceBoundary";
    req.args = std::move(args);
    return req;
}

json plane_args(const fb::SketchPlane& p) {
    return json{{"origin", {p.origin.x, p.origin.y, p.origin.z}},
                {"xAxis", {p.xAxis.x, p.xAxis.y, p.xAxis.z}},
                {"yAxis", {p.yAxis.x, p.yAxis.y, p.yAxis.z}},
                {"normal", {p.normal.x, p.normal.y, p.normal.z}}};
}

std::string topo_key_of(const TopoDS_Shape& body, const TopoDS_Shape& face) {
    return em::ElementMapPartition::topokey_for_shape(body, face, km::ElementKind::Face);
}

// V1/V2: both addressing forms reach the same face and produce the same payload.
// The elementId form is the extra rung QueryElement does NOT have: partition entry
// → (bodyId, topoKey) → shape_for_topokey → a real TopoDS_Face.
void test_verb_addressing_forms_agree() {
    const TopoDS_Shape box = make_box();
    const TopoDS_Face top = face_near(box, 40, 30, 30);
    const std::string key = topo_key_of(box, top);
    check(!key.empty() && key[0] == 'f', "verb: the seed resolves to a face topoKey");

    Session session;
    seed_head(session, "body_1", box, "el_top", key);

    const json plane = plane_args(xy_basis(30.0));
    const Envelope by_key = onecad::session::handle_project_face_boundary(
        session, project_req(json{{"snapshotId", 1},
                                  {"bodyId", "body_1"},
                                  {"topoKey", key},
                                  {"plane", plane},
                                  {"scope", "faceOnly"}}));
    check(by_key.ok.value_or(false), "verb(topoKey): ok");
    check(by_key.result.value("present", false), "verb(topoKey): present");
    check(by_key.result.value("hasClosedBoundary", false), "verb(topoKey): closed");
    check(by_key.result.value("faceCount", 0) == 1, "verb(topoKey): faceCount 1");
    check(by_key.result["points"].size() == 4, "verb(topoKey): 4 points");
    check(by_key.result["entities"].size() == 4, "verb(topoKey): 4 entities");
    check(by_key.result["points"][0]["ref"] == "p0", "verb: refs are response-local p<N>");
    check(by_key.result["entities"][0]["type"] == "Line", "verb: Line entities");
    check(by_key.result["entities"][0]["p0Ref"] == "p0" &&
              by_key.result["entities"][0]["p1Ref"] == "p1",
          "verb: p0Ref/p1Ref resolve within points[]");
    // exact: the kernel gp_Pln of the seed face, orientation-corrected.
    check_near(by_key.result["exact"]["normal"][2].get<double>(), 1.0, kExact,
               "verb: exact.normal is +Z for the top face");
    check_near(by_key.result["exact"]["origin"][2].get<double>(), 30.0, kExact,
               "verb: exact.origin lies on z=30");

    const Envelope by_id = onecad::session::handle_project_face_boundary(
        session, project_req(json{{"snapshotId", 1},
                                  {"elementId", "el_top"},
                                  {"plane", plane},
                                  {"scope", "faceOnly"}}));
    check(by_id.ok.value_or(false) && by_id.result.value("present", false),
          "verb(elementId): present — the partition→topoKey→shape rung works");
    check(by_id.result == by_key.result,
          "verb: elementId and topoKey addressing produce the SAME payload");
}

// V3/V4: a stale or absent reference is an ANSWER (`present:false`), not an error.
void test_verb_absent_is_not_an_error() {
    Session session;
    seed_head(session, "body_1", make_box());
    const json plane = plane_args(xy_basis(30.0));

    const Envelope unknown_id = onecad::session::handle_project_face_boundary(
        session, project_req(json{{"elementId", "el_nope"}, {"plane", plane}}));
    check(unknown_id.ok.value_or(false), "verb: unknown elementId is ok:true");
    check(unknown_id.result.value("present", true) == false, "verb: unknown elementId ⇒ present:false");

    const Envelope stale_key = onecad::session::handle_project_face_boundary(
        session, project_req(json{{"bodyId", "body_1"}, {"topoKey", "f:999"}, {"plane", plane}}));
    check(stale_key.ok.value_or(false) && stale_key.result.value("present", true) == false,
          "verb: stale topoKey ⇒ present:false");

    const Envelope no_body = onecad::session::handle_project_face_boundary(
        session, project_req(json{{"bodyId", "body_gone"}, {"topoKey", "f:1"}, {"plane", plane}}));
    check(no_body.ok.value_or(false) && no_body.result.value("present", true) == false,
          "verb: missing body ⇒ present:false");
}

// V5/V7: loud, recoverable failures — a non-planar seed and a missing plane.
void test_verb_refusals() {
    const TopoDS_Shape cylinder = BRepPrimAPI_MakeCylinder(10.0, 20.0).Shape();
    const TopoDS_Face lateral = non_planar_face(cylinder);
    const std::string key = em::ElementMapPartition::topokey_for_shape(cylinder, lateral,
                                                                      km::ElementKind::Face);
    Session session;
    seed_head(session, "body_cyl", cylinder);

    const Envelope non_planar = onecad::session::handle_project_face_boundary(
        session, project_req(json{{"bodyId", "body_cyl"},
                                  {"topoKey", key},
                                  {"plane", plane_args(xy_basis(0.0))}}));
    check(!non_planar.ok.value_or(true), "verb: non-planar seed fails");
    check(non_planar.error.has_value() && non_planar.error->code == "OP_FAILED",
          "verb: non-planar seed ⇒ OP_FAILED (recoverable, never approximated)");

    // frameOnly must ALSO refuse a non-planar seed — there is no frame to hand out.
    const Envelope frame_non_planar = onecad::session::handle_project_face_boundary(
        session, project_req(json{{"bodyId", "body_cyl"}, {"topoKey", key}, {"frameOnly", true}}));
    check(!frame_non_planar.ok.value_or(true) && frame_non_planar.error->code == "OP_FAILED",
          "verb: frameOnly on a non-planar seed ⇒ OP_FAILED");

    Session box_session;
    const TopoDS_Shape box = make_box();
    seed_head(box_session, "body_1", box);
    const std::string top_key = topo_key_of(box, face_near(box, 40, 30, 30));
    const Envelope no_plane = onecad::session::handle_project_face_boundary(
        box_session, project_req(json{{"bodyId", "body_1"}, {"topoKey", top_key}}));
    check(!no_plane.ok.value_or(true) && no_plane.error->code == "PROTOCOL_ERROR",
          "verb: plane is REQUIRED unless frameOnly");
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. Tilted face: the frameOnly plane is EXACT (on the face, unit normal), and a
//    basis built from it round-trips every projected point back onto the face.
// ═══════════════════════════════════════════════════════════════════════════
void test_tilted_face_exact_frame_and_roundtrip() {
    const TopoDS_Shape body = make_tilted_box();
    // The box's top face rotated 30° about X ⇒ normal (0, −sin30, cos30).
    const TopoDS_Face top = face_by_normal(body, 0.0, -0.5, std::sqrt(3.0) / 2.0);
    const std::string key = topo_key_of(body, top);

    Session session;
    seed_head(session, "body_tilt", body);

    const Envelope frame = onecad::session::handle_project_face_boundary(
        session,
        project_req(json{{"bodyId", "body_tilt"}, {"topoKey", key}, {"frameOnly", true}}));
    check(frame.ok.value_or(false) && frame.result.value("present", false), "tilted: frameOnly ok");
    check(!frame.result.contains("points") && !frame.result.contains("entities"),
          "tilted: frameOnly returns ONLY the exact frame");

    const gp_Pnt origin(frame.result["exact"]["origin"][0].get<double>(),
                        frame.result["exact"]["origin"][1].get<double>(),
                        frame.result["exact"]["origin"][2].get<double>());
    const double nx = frame.result["exact"]["normal"][0].get<double>();
    const double ny = frame.result["exact"]["normal"][1].get<double>();
    const double nz = frame.result["exact"]["normal"][2].get<double>();
    check_near(std::sqrt(nx * nx + ny * ny + nz * nz), 1.0, 1e-12, "tilted: normal is unit");
    check_near(ny, -0.5, 1e-9, "tilted: normal y = −sin30 (orientation-corrected outward)");
    check_near(nz, std::sqrt(3.0) / 2.0, 1e-9, "tilted: normal z = cos30");

    // ▸RB1: the exact origin lies ON the face plane — a descriptor bbox centre
    // would not (it is an axis-aligned box centre, off-plane for a tilted face).
    for (TopExp_Explorer exp(top, TopAbs_VERTEX); exp.More(); exp.Next()) {
        const gp_Pnt v = BRep_Tool::Pnt(TopoDS::Vertex(exp.Current()));
        const double signed_distance =
            nx * (origin.X() - v.X()) + ny * (origin.Y() - v.Y()) + nz * (origin.Z() - v.Z());
        check_near(signed_distance, 0.0, 1e-9, "tilted: |n·(origin − faceVertex)| ≤ 1e-9");
    }

    // Build a right-handed basis from the EXACT frame (what Rust does between the
    // two round trips) and re-project through the verb.
    const gp_Dir normal(nx, ny, nz);
    const gp_Dir seed_dir = (std::abs(nz) < 0.9) ? gp_Dir(0, 0, 1) : gp_Dir(1, 0, 0);
    const gp_Dir x_axis(gp_Vec(seed_dir).Crossed(gp_Vec(normal)).Normalized());
    const gp_Dir y_axis(gp_Vec(normal).Crossed(gp_Vec(x_axis)));
    fb::SketchPlane plane;
    plane.origin = {origin.X(), origin.Y(), origin.Z()};
    plane.xAxis = {x_axis.X(), x_axis.Y(), x_axis.Z()};
    plane.yAxis = {y_axis.X(), y_axis.Y(), y_axis.Z()};
    plane.normal = {nx, ny, nz};

    const Envelope full = onecad::session::handle_project_face_boundary(
        session, project_req(json{{"bodyId", "body_tilt"},
                                  {"topoKey", key},
                                  {"plane", plane_args(plane)},
                                  {"scope", "coplanarBody"}}));
    check(full.ok.value_or(false) && full.result.value("present", false), "tilted: full projection ok");
    check(full.result.value("hasClosedBoundary", false), "tilted: closed boundary");
    check(full.result.value("faceCount", 0) == 1, "tilted: only the tilted top is on this plane");
    check(full.result["points"].size() == 4, "tilted: 4 corner points");
    check(full.result["entities"].size() == 4, "tilted: 4 Lines");

    // Round trip: UV → world must land on an actual vertex of the face.
    for (const json& p : full.result["points"]) {
        const double u = p["at"][0].get<double>();
        const double v = p["at"][1].get<double>();
        const gp_Pnt world(plane.origin.x + u * plane.xAxis.x + v * plane.yAxis.x,
                           plane.origin.y + u * plane.xAxis.y + v * plane.yAxis.y,
                           plane.origin.z + u * plane.xAxis.z + v * plane.yAxis.z);
        double best = 1e18;
        for (TopExp_Explorer exp(top, TopAbs_VERTEX); exp.More(); exp.Next()) {
            best = std::min(best, world.Distance(BRep_Tool::Pnt(TopoDS::Vertex(exp.Current()))));
        }
        check_near(best, 0.0, 1e-9, "tilted: UV round-trips back onto a face vertex within 1e-9");
    }
}

// V10: `scope` really switches the face set at the verb.
void test_verb_scope_face_count() {
    const TopoDS_Shape body = make_l_body();
    const TopoDS_Face seed = face_near(body, 40, 30, 30);
    const std::string key = topo_key_of(body, seed);
    Session session;
    seed_head(session, "body_l", body);
    const json plane = plane_args(xy_basis(30.0));

    const Envelope whole = onecad::session::handle_project_face_boundary(
        session, project_req(json{{"bodyId", "body_l"}, {"topoKey", key}, {"plane", plane}}));
    check(whole.result.value("faceCount", 0) == 2,
          "verb: scope defaults to coplanarBody ⇒ faceCount 2");
    check(whole.result["entities"].size() == 8, "verb: coplanarBody ⇒ 8 entities");

    const Envelope only = onecad::session::handle_project_face_boundary(
        session, project_req(json{{"bodyId", "body_l"},
                                  {"topoKey", key},
                                  {"plane", plane},
                                  {"scope", "faceOnly"}}));
    check(only.result.value("faceCount", 0) == 1, "verb: faceOnly ⇒ faceCount 1");
    check(only.result["entities"].size() == 5, "verb: faceOnly ⇒ 5 entities");
}

// 11b: determinism at the verb — two FRESH sessions, byte-identical envelopes.
void test_verb_determinism() {
    std::string first;
    for (int run = 0; run < 2; ++run) {
        const TopoDS_Shape body = make_box_with_hole();  // rebuilt from scratch
        const TopoDS_Face top = face_near(body, 40, 30, 30);
        Session session;  // fresh session: fresh head, fresh partition, fresh ids
        seed_head(session, "body_1", body);
        const Envelope resp = onecad::session::handle_project_face_boundary(
            session, project_req(json{{"bodyId", "body_1"},
                                      {"topoKey", topo_key_of(body, top)},
                                      {"plane", plane_args(xy_basis(30.0))}}));
        const std::string dump = onecad::protocol::serialize(resp);
        if (run == 0) {
            first = dump;
        } else {
            check(dump == first, "determinism: two fresh sessions serialize byte-identically");
        }
    }
}

}  // namespace

int main() {
    test_box_top_exact_corners();
    test_hole_face_exact_circle();
    test_fillet_arc();
    test_non_planar_seed_refused();
    test_lshape_shared_edge_emitted_once();
    test_ushape_disconnected_coplanar_faces();
    test_face_only_is_strict_subset();
    test_spline_polyline_fallback();
    test_point_merge();
    test_projector_determinism();

    test_verb_addressing_forms_agree();
    test_verb_absent_is_not_an_error();
    test_verb_refusals();
    test_tilted_face_exact_frame_and_roundtrip();
    test_verb_scope_face_count();
    test_verb_determinism();

    if (g_failures == 0) std::fprintf(stderr, "face_projection: OK\n");
    return g_failures;
}
