// test_wp6_extrude.cpp — W-WP6 extrude completion (scope C): ToFace / ToNext end
// conditions (typed targetFace ref via the ladder) + draft angle. In-process via
// execute_extrude with real OCCT. No framework: exit code == failure count.
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRep_Tool.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <ShapeFix_ShapeTolerance.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <TopoDS_Vertex.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include "elementmap/ElementMapPartition.h"
#include "loop/RegionTable.h"
#include "loop/RegionUtils.h"
#include "nlohmann/json.hpp"
#include "ops/ExtrudeOp.h"
#include "ops/OpTypes.h"
#include "session/BodyStore.h"
#include "session/ShapeMetrics.h"
#include "sketch/SketchTypes.h"
#include "sketch/WireSketch.h"
#include "util/Cancel.h"

using nlohmann::json;
namespace ops = onecad::ops;
namespace em = onecad::elementmap;
namespace km = onecad::kernel::elementmap;
namespace loop = onecad::core::loop;
namespace sk = onecad::core::sketch;
using onecad::session::BodyStore;

namespace {
int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) { std::fprintf(stderr, "FAIL: %s\n", msg.c_str()); ++g_failures; }
}
void check_near(double got, double want, double tol, const std::string& msg) {
    if (std::abs(got - want) > tol) {
        // %.10g, not %.4f: the scale fixtures assert exact values at 1e-2 mm and
        // 1e4 mm, where four fixed decimals report every failure as "0.0200 want
        // 0.0200" and diagnose nothing.
        std::fprintf(stderr, "FAIL: %s (got %.10g want %.10g)\n", msg.c_str(), got, want);
        ++g_failures;
    }
}

TopoDS_Shape face_by_center(const TopoDS_Shape& shape, double cx, double cy, double cz) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    TopoDS_Shape best;
    double best_d2 = -1.0;
    for (int i = 1; i <= faces.Extent(); ++i) {
        const km::ElementDescriptor d = em::ElementMapPartition::describe(faces(i));
        const double dx = d.center.X() - cx, dy = d.center.Y() - cy, dz = d.center.Z() - cz;
        const double d2 = dx * dx + dy * dy + dz * dz;
        if (best_d2 < 0.0 || d2 < best_d2) { best_d2 = d2; best = faces(i); }
    }
    return best;
}

json semantic_face_ref(const std::string& body_id, const std::string& element_id,
                       const TopoDS_Shape& face) {
    const km::ElementDescriptor descriptor =
        em::ElementMapPartition::describe(face);
    return {{"primary", {{"bodyId", body_id}, {"elementId", element_id}, {"kind", "face"}}},
            {"intent", {{"kind", "face"},
                         {"descriptor", em::ElementMapPartition::descriptor_to_json(
                                            descriptor)}}},
            {"anchor", {{"worldPoint", {descriptor.center.X(), descriptor.center.Y(),
                                           descriptor.center.Z()}}}}};
}

// Four Line entities `<tag>1..4` forming the closed w×h rectangle whose lower-left
// sketch corner is (u0,v0). World map (u,v)->(-v,u,0), so the loop covers world
// x∈[-(v0+h),-v0], y∈[u0,u0+w], z=0 (area w·h).
json rect_loop(const std::string& tag, double u0, double v0, double w, double h) {
    const double u1 = u0 + w, v1 = v0 + h;
    return json::array({
        json{{"id", tag + "1"}, {"type", "Line"}, {"p0", {u0, v0}}, {"p1", {u1, v0}}},
        json{{"id", tag + "2"}, {"type", "Line"}, {"p0", {u1, v0}}, {"p1", {u1, v1}}},
        json{{"id", tag + "3"}, {"type", "Line"}, {"p0", {u1, v1}}, {"p1", {u0, v1}}},
        json{{"id", tag + "4"}, {"type", "Line"}, {"p0", {u0, v1}}, {"p1", {u0, v0}}},
    });
}

json sketch_of(const std::string& sid, json entities) {
    json sk;
    sk["sketchId"] = sid;
    sk["plane"] = json{{"kind", "XY"}};
    sk["entities"] = std::move(entities);
    sk["constraints"] = json::array();
    return sk;
}

// Sketch params: a w×h rectangle on the XY plane. World map (u,v)->(-v,u,0), so a
// w(u)×h(v) rect → world x∈[-h,0], y∈[0,w], z=0 (area w·h).
json rect_sketch(const std::string& sid, double w, double h) {
    return sketch_of(sid, rect_loop("e", 0.0, 0.0, w, h));
}

// The same rectangle offset to sketch corner (u0,v0) — i.e. world y+u0, world x-v0.
json rect_sketch_at(const std::string& sid, double u0, double v0, double w, double h) {
    return sketch_of(sid, rect_loop("e", u0, v0, w, h));
}

// An ANNULAR profile: outer w×h rect at (u0,v0) with a concentric hw×hh hole at
// (hu0,hv0). Two nested loops ⇒ TWO selectable regions (the ring and the inner
// rect), so a caller must name the one it means.
json annular_sketch(const std::string& sid, double u0, double v0, double w, double h,
                    double hu0, double hv0, double hw, double hh) {
    json entities = rect_loop("o", u0, v0, w, h);
    for (const json& e : rect_loop("i", hu0, hv0, hw, hh)) entities.push_back(e);
    return sketch_of(sid, entities);
}

struct Ctx {
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;
    ops::OpContext make(BodyStore& b, em::ElementMapPartition& p) {
        return ops::OpContext{b, &sketches, p, &last_sketch, false, json::object(), &cancel};
    }
};

// A 10×10 profile at z=0 extruded ToFace up to the bottom face (z=20) of a target
// box at z∈[20,30] ⇒ height 20, volume 2000.
void test_to_face() {
    // Target box: world x∈[-10,0], y∈[0,10], z∈[20,30]; bottom face z=20 centre (-5,5,20).
    const TopoDS_Shape target = BRepPrimAPI_MakeBox(gp_Pnt(-10, 0, 20), 10.0, 10.0, 10.0).Shape();
    BodyStore bodies;
    bodies.create("body_target", "op_t", target);
    em::ElementMapPartition part;

    const TopoDS_Shape bottom = face_by_center(target, -5, 5, 20);
    json target_face = {
        {"primary", {{"bodyId", "body_target"}, {"elementId", "el_tf"}, {"kind", "face"}}},
        {"intent", {{"kind", "face"}, {"descriptor", em::ElementMapPartition::descriptor_to_json(
                                                          em::ElementMapPartition::describe(bottom))}}},
        {"anchor", {{"worldPoint", {-5.0, 5.0, 20.0}}}}};

    Ctx c;
    c.sketches.push_back({"sk1", rect_sketch("sk1", 10, 10)});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json op = {{"opType", "Extrude"}, {"opId", "ope"},
               {"params", {{"sketchId", "sk1"}, {"extrudeMode", "ToFace"}, {"booleanMode", "NewBody"},
                           {"targetFace", target_face}}}};
    ops::OpOutcome oc = ops::execute_extrude(ctx, op, "ope");
    check(oc.status == ops::OpOutcome::Status::Ok, "toFace: Ok");
    check(oc.needs_repair.empty(), "toFace: resolved (no NeedsRepair)");
    check(bodies.contains("body_ope"), "toFace: NewBody created");
    if (bodies.contains("body_ope"))
        check_near(onecad::session::shape_volume(bodies.get("body_ope")->geom), 2000.0, 1.0,
                   "toFace: reaches z=20 → volume 2000");
}

void test_to_face_refuses_a_laterally_disjoint_bounded_face() {
    const TopoDS_Shape target =
        BRepPrimAPI_MakeBox(gp_Pnt(50, 50, 20), 10.0, 10.0, 10.0).Shape();
    const TopoDS_Shape bottom = face_by_center(target, 55, 55, 20);
    BodyStore bodies;
    bodies.create("body_target", "op_t", target);
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", rect_sketch("sk1", 10, 10)});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json op = {{"opType", "Extrude"},
               {"opId", "ope"},
               {"params",
                {{"sketchId", "sk1"},
                 {"extrudeMode", "ToFace"},
                 {"booleanMode", "NewBody"},
                 {"targetFace", semantic_face_ref("body_target", "el_tf", bottom)}}}};
    const ops::OpOutcome outcome = ops::execute_extrude(ctx, op, "ope");
    check(outcome.status == ops::OpOutcome::Status::Failed,
          "toFace disjoint bounded face: refused");
    check(outcome.error_message.find("does not cover") != std::string::npos,
          "toFace disjoint bounded face: refusal names bounded coverage");
    check(!bodies.contains("body_ope"), "toFace disjoint bounded face: no body published");
}

void test_to_face_refuses_tilted_surface_until_exact_termination_exists() {
    const TopoDS_Shape base =
        BRepPrimAPI_MakeBox(gp_Pnt(-10, 0, 0), 10.0, 10.0, 10.0).Shape();
    gp_Trsf rotation;
    rotation.SetRotation(gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(0, 1, 0)), 0.2);
    gp_Trsf translation;
    translation.SetTranslation(gp_Vec(0, 0, 20));
    BRepBuilderAPI_Transform xf(base, translation * rotation, Standard_True);
    const TopoDS_Shape target = xf.Shape();
    TopoDS_Shape tilted;
    for (TopExp_Explorer it(target, TopAbs_FACE); it.More(); it.Next()) {
        const TopoDS_Face face = TopoDS::Face(it.Current());
        BRepAdaptor_Surface surface(face, true);
        if (surface.GetType() != GeomAbs_Plane) continue;
        const double z = std::abs(surface.Plane().Axis().Direction().Z());
        if (z > 0.9 && z < 0.999) {
            tilted = face;
            break;
        }
    }
    check(!tilted.IsNull(), "toFace tilted: target face found");
    if (tilted.IsNull()) return;

    BodyStore bodies;
    bodies.create("body_target", "op_t", target);
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", rect_sketch("sk1", 10, 10)});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json op = {{"opType", "Extrude"},
               {"opId", "ope"},
               {"params",
                {{"sketchId", "sk1"},
                 {"extrudeMode", "ToFace"},
                 {"booleanMode", "NewBody"},
                 {"targetFace", semantic_face_ref("body_target", "el_tf", tilted)}}}};
    const ops::OpOutcome outcome = ops::execute_extrude(ctx, op, "ope");
    check(outcome.status == ops::OpOutcome::Status::Failed,
          "toFace tilted: refused instead of flat-cap success");
    check(outcome.error_message.find("tilted target faces are refused") != std::string::npos,
          "toFace tilted: refusal names unsupported exact termination");
    check(!bodies.contains("body_ope"), "toFace tilted: no body published");
}

// A ToFace target whose body is gone ⇒ NeedsRepair STATE (never Err, never a bind).
void test_to_face_unresolved_needs_repair() {
    BodyStore bodies;
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", rect_sketch("sk1", 10, 10)});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json target_face = {
        {"primary", {{"bodyId", "body_missing"}, {"elementId", "el_tf"}, {"kind", "face"}}},
        {"anchor", {{"worldPoint", {0.0, 0.0, 20.0}}}}};
    json op = {{"opType", "Extrude"}, {"opId", "ope"},
               {"params", {{"sketchId", "sk1"}, {"extrudeMode", "ToFace"}, {"booleanMode", "NewBody"},
                           {"targetFace", target_face}}}};
    ops::OpOutcome oc = ops::execute_extrude(ctx, op, "ope");
    check(oc.status == ops::OpOutcome::Status::Ok, "toFace unresolved: state not Err");
    check(!oc.needs_repair.empty(), "toFace unresolved: NeedsRepair emitted");
    check(!bodies.contains("body_ope"), "toFace unresolved: no body created (never a wrong bind)");
}

// ToNext: extrude toward a target body, stopping at its NEAREST planar face (z=20),
// not its far face (z=30) ⇒ height 20, volume 2000.
void test_to_next() {
    const TopoDS_Shape target = BRepPrimAPI_MakeBox(gp_Pnt(-10, 0, 20), 10.0, 10.0, 10.0).Shape();
    BodyStore bodies;
    bodies.create("body_target", "op_t", target);
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", rect_sketch("sk1", 10, 10)});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json op = {{"opType", "Extrude"}, {"opId", "ope"},
               {"params", {{"sketchId", "sk1"}, {"extrudeMode", "ToNext"}, {"booleanMode", "NewBody"},
                           {"targetBodyId", "body_target"}}}};
    ops::OpOutcome oc = ops::execute_extrude(ctx, op, "ope");
    check(oc.status == ops::OpOutcome::Status::Ok, "toNext: Ok");
    check(bodies.contains("body_ope"), "toNext: NewBody created");
    if (bodies.contains("body_ope"))
        check_near(onecad::session::shape_volume(bodies.get("body_ope")->geom), 2000.0, 1.0,
                   "toNext: stops at nearest face z=20 → volume 2000");
}

// ToNext against a body the profile NEVER reaches (laterally offset pillar) must
// fail loudly — the legacy nearest-ray-PLANE rule bound its z=20 plane and
// silently extruded 2000 (review defect b).
void test_to_next_miss_fails() {
    const TopoDS_Shape target = BRepPrimAPI_MakeBox(gp_Pnt(50, 50, 20), 10.0, 10.0, 10.0).Shape();
    BodyStore bodies;
    bodies.create("body_target", "op_t", target);
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", rect_sketch("sk1", 10, 10)});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json op = {{"opType", "Extrude"}, {"opId", "ope"},
               {"params", {{"sketchId", "sk1"}, {"extrudeMode", "ToNext"}, {"booleanMode", "NewBody"},
                           {"targetBodyId", "body_target"}}}};
    ops::OpOutcome oc = ops::execute_extrude(ctx, op, "ope");
    check(oc.status != ops::OpOutcome::Status::Ok, "toNext miss: fails, never a plane bind");
    check(!bodies.contains("body_ope"), "toNext miss: no body created");
    check(oc.error_message.find("no face found ahead") != std::string::npos,
          "toNext miss: names the reason (got '" + oc.error_message + "')");
}

// ToNext with a NEARER face the profile misses laterally: the bound distance must
// come from the face the profile actually reaches (z=8), not the nearer plane
// (z=5) of a face it never crosses.
void test_to_next_skips_missed_nearer_face() {
    const TopoDS_Shape near_missed =
        BRepPrimAPI_MakeBox(gp_Pnt(30, 0, 5), 10.0, 10.0, 10.0).Shape();
    const TopoDS_Shape covering =
        BRepPrimAPI_MakeBox(gp_Pnt(-10, 0, 8), 10.0, 10.0, 10.0).Shape();
    const TopoDS_Shape target = BRepAlgoAPI_Fuse(near_missed, covering).Shape();
    BodyStore bodies;
    bodies.create("body_target", "op_t", target);
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", rect_sketch("sk1", 10, 10)});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json op = {{"opType", "Extrude"}, {"opId", "ope"},
               {"params", {{"sketchId", "sk1"}, {"extrudeMode", "ToNext"}, {"booleanMode", "NewBody"},
                           {"targetBodyId", "body_target"}}}};
    ops::OpOutcome oc = ops::execute_extrude(ctx, op, "ope");
    check(oc.status == ops::OpOutcome::Status::Ok, "toNext skip: Ok");
    check(bodies.contains("body_ope"), "toNext skip: NewBody created");
    if (bodies.contains("body_ope"))
        check_near(onecad::session::shape_volume(bodies.get("body_ope")->geom), 800.0, 1.0,
                   "toNext skip: binds the face the profile reaches (z=8), not the nearer plane");
}

void test_to_next_finds_a_tiny_interior_first_contact() {
    const TopoDS_Shape ceiling =
        BRepPrimAPI_MakeBox(gp_Pnt(-10, 0, 8), 10.0, 10.0, 2.0).Shape();
    // Attached underside boss: first contact z=5 over x∈[-8,-7], y∈[2,3].
    // Profile corners and centroid all miss it; finite-ray sampling returned z=8.
    const TopoDS_Shape boss =
        BRepPrimAPI_MakeBox(gp_Pnt(-8, 2, 5), 1.0, 1.0, 3.0).Shape();
    BRepAlgoAPI_Fuse fuse(ceiling, boss);
    fuse.Build();
    check(fuse.IsDone() && !fuse.Shape().IsNull(),
          "toNext tiny contact: connected target built");

    BodyStore bodies;
    bodies.create("body_target", "op_t", fuse.Shape());
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", rect_sketch("sk1", 10, 10)});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json op = {{"opType", "Extrude"},
               {"opId", "ope"},
               {"params",
                {{"sketchId", "sk1"},
                 {"extrudeMode", "ToNext"},
                 {"booleanMode", "NewBody"},
                 {"targetBodyId", "body_target"}}}};
    const ops::OpOutcome outcome = ops::execute_extrude(ctx, op, "ope");
    check(outcome.status == ops::OpOutcome::Status::Ok,
          "toNext tiny contact: whole-profile probe succeeds");
    check(bodies.contains("body_ope"), "toNext tiny contact: body created");
    if (bodies.contains("body_ope")) {
        check_near(onecad::session::shape_volume(bodies.get("body_ope")->geom),
                   500.0, 1.0,
                   "toNext tiny contact: first swept-region contact is z=5");
    }
}

// A SEATED profile: the sketch sits on the target body's own face, which is how a
// through-pocket is normally authored. "Next" is then where the sweep LEAVES that
// material (z=6), not the sketch plane it started on — the intersection begins at the
// profile, so its bare minimum is the seat itself and would extrude nothing.
void test_to_next_seated_profile_stops_at_the_material_exit() {
    const TopoDS_Shape target =
        BRepPrimAPI_MakeBox(gp_Pnt(-10, 0, 0), 10.0, 10.0, 6.0).Shape();
    BodyStore bodies;
    bodies.create("body_target", "op_t", target);
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", rect_sketch("sk1", 10, 10)});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json op = {{"opType", "Extrude"}, {"opId", "ope"},
               {"params", {{"sketchId", "sk1"}, {"extrudeMode", "ToNext"},
                           {"booleanMode", "NewBody"}, {"targetBodyId", "body_target"}}}};
    const ops::OpOutcome oc = ops::execute_extrude(ctx, op, "ope");
    check(oc.status == ops::OpOutcome::Status::Ok,
          "toNext seated: Ok (got '" + oc.error_message + "')");
    if (bodies.contains("body_ope"))
        check_near(onecad::session::shape_volume(bodies.get("body_ope")->geom), 600.0, 1.0,
                   "toNext seated: terminates at the seated material's exit (z=6)");
}

// Shared driver for the curved-termination fixtures: 10×10 profile at z=0
// (world x∈[-10,0], y∈[0,10]) extruded ToNext against `target`.
double to_next_height(const TopoDS_Shape& target, const std::string& label) {
    BodyStore bodies;
    bodies.create("body_target", "op_t", target);
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", rect_sketch("sk1", 10, 10)});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json op = {{"opType", "Extrude"},
               {"opId", "ope"},
               {"params",
                {{"sketchId", "sk1"},
                 {"extrudeMode", "ToNext"},
                 {"booleanMode", "NewBody"},
                 {"targetBodyId", "body_target"}}}};
    const ops::OpOutcome outcome = ops::execute_extrude(ctx, op, "ope");
    check(outcome.status == ops::OpOutcome::Status::Ok,
          label + ": Ok (got '" + outcome.error_message + "')");
    if (!bodies.contains("body_ope")) {
        check(false, label + ": body created");
        return -1.0;
    }
    // The profile area is 100, so the swept volume IS the termination distance ×100.
    return onecad::session::shape_volume(bodies.get("body_ope")->geom) / 100.0;
}

// FACE-INTERIOR first contact. A cylinder lying along +X above the profile touches
// the sweep first along its bottom generatrix (z = 20-6 = 14), which is interior to
// the cylindrical face — no vertex sits there. The seam is pinned to +Z (the TOP of
// the cylinder) so it cannot coincidentally land on the extremum.
//
// A TopAbs_VERTEX scan of the sweep∩body common answers 20-sqrt(36-25) ≈ 16.68 here
// (where the prism's y=0 / y=10 walls cut the cylinder), i.e. a LATER contact than
// the geometry has. This fixture fails on the vertex-only implementation.
void test_to_next_curved_face_interior_contact() {
    const TopoDS_Shape cylinder =
        BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(-20.0, 5.0, 20.0), gp_Dir(1, 0, 0),
                                        gp_Dir(0, 0, 1)),
                                 6.0, 40.0)
            .Shape();
    const double height = to_next_height(cylinder, "toNext curved face");
    if (height > 0.0)
        check_near(height, 14.0, 1.0e-3,
                   "toNext curved face: terminates on the bottom generatrix (z=14), "
                   "not the vertex ring at z≈16.68");
}

// EDGE-INTERIOR first contact. A cylinder tilted 45° in the YZ plane presents its
// bottom rim circle to the sweep; the lowest point of that rim is at
// z = 20 - 4/√2 ≈ 17.1716 and lies mid-arc. The rim's only vertex is its seam, which
// is pinned to the HIGHEST point of the circle, so a vertex scan answers ≈22.83.
void test_to_next_curved_edge_interior_contact() {
    const TopoDS_Shape cylinder =
        BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(-5.0, 5.0, 20.0), gp_Dir(0, 1, 1),
                                        gp_Dir(0, -1, 1)),
                                 4.0, 20.0)
            .Shape();
    const double height = to_next_height(cylinder, "toNext curved edge");
    if (height > 0.0)
        check_near(height, 20.0 - 4.0 / std::sqrt(2.0), 1.0e-3,
                   "toNext curved edge: terminates at the rim's low point, not its seam");
}

// Exact Z span of a flat-capped prism: max vertex Z − min vertex Z. Every ToNext
// result is such a prism rising from the z=0 sketch plane, so this IS the resolved
// termination distance — and unlike volume/area it stays exact when the profile is
// annular or the fixture is rescaled.
double z_span(const TopoDS_Shape& shape) {
    bool first = true;
    double lo = 0.0, hi = 0.0;
    for (TopExp_Explorer exp(shape, TopAbs_VERTEX); exp.More(); exp.Next()) {
        const double z = BRep_Tool::Pnt(TopoDS::Vertex(exp.Current())).Z();
        if (first) { lo = hi = z; first = false; continue; }
        lo = std::min(lo, z);
        hi = std::max(hi, z);
    }
    return first ? -1.0 : hi - lo;
}

// ToNext outcome for an arbitrary profile sketch against `target`.
struct ToNextRun {
    ops::OpOutcome outcome;
    double span = -1.0;    // termination distance, or -1 when nothing was published
    double volume = -1.0;  // published solid's volume (profile area × span)
};

ToNextRun run_to_next(const json& sketch, const std::string& region_id,
                      const TopoDS_Shape& target) {
    ToNextRun run;
    BodyStore bodies;
    bodies.create("body_target", "op_t", target);
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", sketch});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json params = {{"sketchId", "sk1"},
                   {"extrudeMode", "ToNext"},
                   {"booleanMode", "NewBody"},
                   {"targetBodyId", "body_target"}};
    if (!region_id.empty()) {
        params["regionId"] = region_id;
        // A named region rides with its identity version, exactly as the planner
        // sends it (`worker/wire.rs` profile lift). V2 is the exact-fragment table
        // `holed_region_id` derives its id from.
        params["regionIdentityVersion"] = 2;
    }
    json op = {{"opType", "Extrude"}, {"opId", "ope"}, {"params", params}};
    run.outcome = ops::execute_extrude(ctx, op, "ope");
    if (bodies.contains("body_ope")) {
        run.span = z_span(bodies.get("body_ope")->geom);
        run.volume = onecad::session::shape_volume(bodies.get("body_ope")->geom);
    }
    return run;
}

// Ok + published + the exact termination distance (or -1, which every caller's
// check_near then reports).
double to_next_span(const json& sketch, const TopoDS_Shape& target,
                    const std::string& label) {
    const ToNextRun run = run_to_next(sketch, "", target);
    check(run.outcome.status == ops::OpOutcome::Status::Ok,
          label + ": Ok (got '" + run.outcome.error_message + "')");
    check(run.span >= 0.0, label + ": body created");
    return run.span;
}

// The id of the sketch's annular (holed) region, derived through the SAME region
// table `build_profile_face` selects from. The detector's region ORDER is not part
// of any contract, so an annular fixture must NAME its profile rather than trust
// `regions.front()`.
std::string holed_region_id(const json& sketch_params) {
    onecad::wire::TranslateResult tr = onecad::wire::translate(sketch_params);
    if (!tr.ok || !tr.sketch->solve().success) return {};
    loop::LoopDetector detector;
    detector.setConfig(loop::makeRegionDetectionConfig());
    const loop::LoopDetectionResult detection = detector.detect(*tr.sketch);
    const loop::RegionTable table = loop::buildRegionTable(
        detection,
        [&](const sk::EntityID& internal) -> std::string {
            const auto it = tr.index.internal_edge_to_wire.find(internal);
            return it != tr.index.internal_edge_to_wire.end() ? it->second : internal;
        },
        sk::constants::COINCIDENCE_TOLERANCE);
    if (!table.success) return {};
    for (const loop::RegionDefinition& region : table.regions) {
        if (!region.holes.empty()) return region.id;
    }
    return {};
}

// SEATED over a CLOSED INTERNAL VOID that does not span the profile. The void
// x∈[-8,-2], y∈[2,8] is strictly inside the profile column x∈[-10,0], y∈[0,10], so
// material wraps it laterally and sweep∩body stays ONE connected solid: the run does
// NOT stop at the void's near wall (z=6), it exits the block's far face.
void test_to_next_seated_over_a_closed_internal_void() {
    const TopoDS_Shape block =
        BRepPrimAPI_MakeBox(gp_Pnt(-12, -1, 0), 14.0, 12.0, 20.0).Shape();
    const TopoDS_Shape cavity =
        BRepPrimAPI_MakeBox(gp_Pnt(-8, 2, 6), 6.0, 6.0, 6.0).Shape();
    BRepAlgoAPI_Cut cut(block, cavity);
    cut.Build();
    check(cut.IsDone() && !cut.Shape().IsNull(), "toNext closed void: target built");
    // 14·12·20 − 6·6·6 = 3360 − 216: the void is wholly enclosed, so it removes its
    // full volume and touches no outer face.
    check_near(onecad::session::shape_volume(cut.Shape()), 3144.0, 1.0e-6,
               "toNext closed void: fixture is a block with a fully enclosed void");

    const double span = to_next_span(rect_sketch("sk1", 10, 10), cut.Shape(),
                                     "toNext closed void");
    check_near(span, 20.0, 1.0e-6,
               "toNext closed void: one connected seated run ⇒ exit at the block's "
               "far face z=20, not the void's near wall z=6");
}

// A cavity under only PART of the profile. The pocket roofs at z=16 over x∈[-10,-5];
// the other half of the profile meets solid at z=10. Above z=16 the two columns share
// the plane x=-5, so the intersection is ONE run whose entry is its GLOBAL minimum.
void test_to_next_partial_cavity_under_the_profile() {
    const TopoDS_Shape block =
        BRepPrimAPI_MakeBox(gp_Pnt(-12, -1, 10), 14.0, 12.0, 20.0).Shape();
    // Opens through the block's bottom face (z from 5 up to 16) and overhangs the
    // profile in x and y so no pocket wall is coincident with a sweep wall.
    const TopoDS_Shape pocket =
        BRepPrimAPI_MakeBox(gp_Pnt(-10.5, -0.5, 5), 5.5, 11.0, 11.0).Shape();
    BRepAlgoAPI_Cut cut(block, pocket);
    cut.Build();
    check(cut.IsDone() && !cut.Shape().IsNull(), "toNext partial cavity: target built");
    // 14·12·20 − 5.5·11·(16−10) = 3360 − 363.
    check_near(onecad::session::shape_volume(cut.Shape()), 2997.0, 1.0e-6,
               "toNext partial cavity: fixture is a block pocketed from below");

    const double span = to_next_span(rect_sketch("sk1", 10, 10), cut.Shape(),
                                     "toNext partial cavity");
    check_near(span, 10.0, 1.0e-6,
               "toNext partial cavity: the run's entry is the nearest contact under "
               "ANY part of the profile (z=10), not the pocket roof z=16");
}

// Material connected laterally AROUND a slot. A U opening downward: legs x∈[-12,-9]
// and x∈[-1,2] over z∈[0,10], bridged by z∈[10,20]. The profile spans the U's mouth
// and is seated on the leg bottoms, so the seated run runs up the legs, across the
// bridge and out at z=20 — one connected solid despite the slot in its middle.
void test_to_next_seated_run_connected_around_a_slot() {
    const TopoDS_Shape block =
        BRepPrimAPI_MakeBox(gp_Pnt(-12, -1, 0), 14.0, 12.0, 20.0).Shape();
    const TopoDS_Shape slot =
        BRepPrimAPI_MakeBox(gp_Pnt(-9, -2, -5), 8.0, 14.0, 15.0).Shape();
    BRepAlgoAPI_Cut cut(block, slot);
    cut.Build();
    check(cut.IsDone() && !cut.Shape().IsNull(), "toNext U slot: target built");
    // 14·12·20 − 8·12·10 = 3360 − 960: the slot spans the block in y and reaches
    // z=10, so the block keeps its full-width bridge above.
    check_near(onecad::session::shape_volume(cut.Shape()), 2400.0, 1.0e-6,
               "toNext U slot: fixture is a U opening downward");

    const double span = to_next_span(rect_sketch("sk1", 10, 10), cut.Shape(),
                                     "toNext U slot");
    check_near(span, 20.0, 1.0e-6,
               "toNext U slot: the legs and the bridge are ONE run ⇒ exit z=20, not "
               "the leg tops z=10");
}

// An ANNULAR profile must sweep its ring only. A boss hangs off the plate's underside
// down to z=4, but it sits wholly inside the profile's HOLE (x∈[-15,-5], y∈[5,15] with
// 2mm clearance), so the ring never meets it: a filled-profile probe would answer 4.
void test_to_next_annular_profile_ignores_material_inside_its_hole() {
    const json sketch = annular_sketch("sk1", 0, 0, 20, 20, 5, 5, 10, 10);
    const std::string ring = holed_region_id(sketch);
    check(!ring.empty(), "toNext annular: annular region id derived");
    if (ring.empty()) return;

    const TopoDS_Shape plate =
        BRepPrimAPI_MakeBox(gp_Pnt(-22, -2, 12), 24.0, 24.0, 4.0).Shape();
    const TopoDS_Shape boss =
        BRepPrimAPI_MakeBox(gp_Pnt(-13, 7, 4), 6.0, 6.0, 8.0).Shape();
    BRepAlgoAPI_Fuse fuse(plate, boss);
    fuse.Build();
    check(fuse.IsDone() && !fuse.Shape().IsNull(), "toNext annular: target built");

    const ToNextRun run = run_to_next(sketch, ring, fuse.Shape());
    check(run.outcome.status == ops::OpOutcome::Status::Ok,
          "toNext annular: Ok (got '" + run.outcome.error_message + "')");
    check_near(run.span, 12.0, 1.0e-6,
               "toNext annular: the ring reaches the plate at z=12; the boss at z=4 "
               "lies inside the hole and is not swept");
    // Ring area (20·20 − 10·10 = 300) × the 12 mm termination: proves the hole
    // survived into the swept profile rather than the inner rect being selected.
    check_near(run.volume, 3600.0, 1.0e-6,
               "toNext annular: the swept profile is the 300 mm² ring");
}

// Two SEPARATED plates under one profile: the intersection is two runs.
//   * from free space, "next" is the first run's ENTRY (z=10);
//   * seated on the near plate, "next" is that FIRST run's EXIT (z=6) — never the
//     second plate.
void test_to_next_across_two_separated_runs() {
    const TopoDS_Shape lower =
        BRepPrimAPI_MakeBox(gp_Pnt(-12, -1, 10), 14.0, 12.0, 2.0).Shape();
    const TopoDS_Shape upper =
        BRepPrimAPI_MakeBox(gp_Pnt(-12, -1, 20), 14.0, 12.0, 2.0).Shape();
    BRepAlgoAPI_Fuse pair(lower, upper);
    pair.Build();
    check(pair.IsDone() && !pair.Shape().IsNull(), "toNext two runs: target built");
    const double free_span = to_next_span(rect_sketch("sk1", 10, 10), pair.Shape(),
                                          "toNext two runs (free space)");
    check_near(free_span, 10.0, 1.0e-6,
               "toNext two runs: first contact is the near plate's bottom z=10");

    const TopoDS_Shape seat =
        BRepPrimAPI_MakeBox(gp_Pnt(-12, -1, 0), 14.0, 12.0, 6.0).Shape();
    const TopoDS_Shape beyond =
        BRepPrimAPI_MakeBox(gp_Pnt(-12, -1, 12), 14.0, 12.0, 2.0).Shape();
    BRepAlgoAPI_Fuse seated_pair(seat, beyond);
    seated_pair.Build();
    check(seated_pair.IsDone() && !seated_pair.Shape().IsNull(),
          "toNext two runs seated: target built");
    const double seated_span = to_next_span(rect_sketch("sk1", 10, 10), seated_pair.Shape(),
                                            "toNext two runs (seated)");
    check_near(seated_span, 6.0, 1.0e-6,
               "toNext two runs seated: exits the seated plate at z=6, never jumps to "
               "the second plate at z=12");
}

// NEAR-SEAT tangency. The plate's bottom face stands 1e-7 above the sketch plane —
// inside `kToNextContactEpsilon` (1e-4), the start-exclusion the sweep is lifted by.
// The spec calls that the profile's own seat, so the deterministic answer is the
// SEATED one: terminate at the plate's far face, not 1e-7 away.
void test_to_next_near_seat_tangent_contact() {
    const TopoDS_Shape plate =
        BRepPrimAPI_MakeBox(gp_Pnt(-12, -1, 1.0e-7), 14.0, 12.0, 10.0 - 1.0e-7).Shape();
    const double span = to_next_span(rect_sketch("sk1", 10, 10), plate,
                                     "toNext near-seat");
    check_near(span, 10.0, 1.0e-6,
               "toNext near-seat: a 1e-7 gap is under the 1e-4 seat epsilon ⇒ seated "
               "semantics ⇒ exit z=10");
}

// SCALE. The `test_to_next` fixture rescaled by s: an s×s profile under an s-cube
// whose bottom face is at 2s. The termination is 2s at every scale.
void test_to_next_scale_extremes() {
    const double small = 0.01;  // 0.01 mm — the small end of the supported range
    const TopoDS_Shape small_target =
        BRepPrimAPI_MakeBox(gp_Pnt(-small, 0, 2 * small), small, small, small).Shape();
    const double small_span = to_next_span(rect_sketch("sk1", small, small), small_target,
                                           "toNext scale 0.01mm");
    check_near(small_span, 0.02, 2.0e-8,
               "toNext scale 0.01mm: nearest face at 2s = 0.02");

    const double large = 10000.0;  // 10 m — the large end
    const TopoDS_Shape large_target =
        BRepPrimAPI_MakeBox(gp_Pnt(-large, 0, 2 * large), large, large, large).Shape();
    const double large_span = to_next_span(rect_sketch("sk1", large, large), large_target,
                                           "toNext scale 10m");
    check_near(large_span, 20000.0, 2.0e-2,
               "toNext scale 10m: nearest face at 2s = 20000");
}

// FAR FROM ORIGIN. The same fixture translated +1e6 mm along world Y (sketch u). The
// answer is a 20 mm termination measured between coordinates of magnitude 1e6.
void test_to_next_far_from_origin() {
    const TopoDS_Shape target =
        BRepPrimAPI_MakeBox(gp_Pnt(-10, 1.0e6, 20), 10.0, 10.0, 10.0).Shape();
    const double span = to_next_span(rect_sketch_at("sk1", 1.0e6, 0, 10, 10), target,
                                     "toNext far from origin");
    check_near(span, 20.0, 1.0e-6,
               "toNext far from origin: nearest face z=20 at y≈1e6");
}

// ELEVATED TOLERANCE. Imported geometry arrives with B-Rep tolerances orders above
// authoring resolution; the termination is a property of the SURFACES, so inflating
// every subshape's tolerance to 1e-3 must not move it.
void test_to_next_with_inflated_target_tolerance() {
    const TopoDS_Shape target =
        BRepPrimAPI_MakeBox(gp_Pnt(-10, 0, 20), 10.0, 10.0, 10.0).Shape();
    ShapeFix_ShapeTolerance().SetTolerance(target, 1.0e-3);
    double worst = 0.0;
    for (TopExp_Explorer exp(target, TopAbs_VERTEX); exp.More(); exp.Next())
        worst = std::max(worst, BRep_Tool::Tolerance(TopoDS::Vertex(exp.Current())));
    check(worst >= 1.0e-3,
          "toNext inflated tolerance: fixture tolerance actually raised to 1e-3");

    const double span = to_next_span(rect_sketch("sk1", 10, 10), target,
                                     "toNext inflated tolerance");
    check_near(span, 20.0, 1.0e-6,
               "toNext inflated tolerance: the exact extremum is unchanged by "
               "tolerance metadata ⇒ z=20");
}

// Draft: a 10×10 profile extruded 10mm with a 10° draft tapers the side faces
// inward ⇒ volume strictly below the 1000 straight prism.
void test_draft() {
    BodyStore bodies;
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", rect_sketch("sk1", 10, 10)});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json op = {{"opType", "Extrude"}, {"opId", "ope"},
               {"params", {{"sketchId", "sk1"}, {"distance", 10.0}, {"draftAngleDeg", 10.0},
                           {"extrudeMode", "Blind"}, {"booleanMode", "NewBody"}}}};
    ops::OpOutcome oc = ops::execute_extrude(ctx, op, "ope");
    check(oc.status == ops::OpOutcome::Status::Ok, "draft: Ok");
    if (bodies.contains("body_ope")) {
        const double v = onecad::session::shape_volume(bodies.get("body_ope")->geom);
        check(v < 990.0 && v > 500.0, "draft: tapered volume clearly below the 1000 straight prism");
    }
}

// input_body() picks up a whole-BODY inputs[0] ref as the boolean target when
// `targetBodyId` is absent (the valid fallback): Add fuses into body_target.
void test_boolean_target_from_body_ref() {
    const TopoDS_Shape target = BRepPrimAPI_MakeBox(gp_Pnt(-10, 0, 0), 10.0, 10.0, 10.0).Shape();
    BodyStore bodies;
    bodies.create("body_target", "op_t", target);  // world z∈[0,10], vol 1000
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", rect_sketch("sk1", 10, 10)});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json op = {{"opType", "Extrude"},
               {"opId", "ope"},
               {"inputs", json::array({json{{"primary",
                                             {{"bodyId", "body_target"},
                                              {"elementId", "body_target"},
                                              {"kind", "body"}}}}})},
               {"params",
                {{"sketchId", "sk1"}, {"distance", 20.0}, {"extrudeMode", "Blind"},
                 {"booleanMode", "Add"}}}};
    ops::OpOutcome oc = ops::execute_extrude(ctx, op, "ope");
    check(oc.status == ops::OpOutcome::Status::Ok, "boolean body-ref fallback: Ok");
    if (bodies.contains("body_target"))
        check_near(onecad::session::shape_volume(bodies.get("body_target")->geom), 2000.0, 1.0,
                   "boolean body-ref fallback: Add fused into body_target (z∈[0,20] → vol 2000)");
}

// A FACE ref at inputs[0] (e.g. a ToFace targetFace) must NOT be bound as the boolean
// target when `targetBodyId` is absent (hazard 6): the op fails cleanly instead of
// silently cutting the ToFace target's body.
void test_boolean_ignores_face_ref_at_input0() {
    const TopoDS_Shape target = BRepPrimAPI_MakeBox(gp_Pnt(-10, 0, 0), 10.0, 10.0, 10.0).Shape();
    BodyStore bodies;
    bodies.create("body_target", "op_t", target);  // vol 1000
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", rect_sketch("sk1", 10, 10)});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json op = {{"opType", "Extrude"},
               {"opId", "ope"},
               {"inputs", json::array({json{{"primary",
                                             {{"bodyId", "body_target"},
                                              {"elementId", "el_face"},
                                              {"kind", "face"}}}}})},
               {"params",
                {{"sketchId", "sk1"}, {"distance", 5.0}, {"extrudeMode", "Blind"},
                 {"booleanMode", "Cut"}}}};
    ops::OpOutcome oc = ops::execute_extrude(ctx, op, "ope");
    check(oc.status == ops::OpOutcome::Status::Failed,
          "hazard 6: a face-ref inputs[0] is not bound as boolean target → clean failure");
    if (bodies.get("body_target") != nullptr)
        check_near(onecad::session::shape_volume(bodies.get("body_target")->geom), 1000.0, 1.0,
                   "hazard 6: the face ref's body is left unmodified (no silent cut)");
}

}  // namespace

int main() {
    test_to_face();
    test_to_face_refuses_a_laterally_disjoint_bounded_face();
    test_to_face_refuses_tilted_surface_until_exact_termination_exists();
    test_to_face_unresolved_needs_repair();
    test_to_next();
    test_to_next_miss_fails();
    test_to_next_skips_missed_nearer_face();
    test_to_next_finds_a_tiny_interior_first_contact();
    test_to_next_seated_profile_stops_at_the_material_exit();
    test_to_next_curved_face_interior_contact();
    test_to_next_curved_edge_interior_contact();
    test_to_next_seated_over_a_closed_internal_void();
    test_to_next_partial_cavity_under_the_profile();
    test_to_next_seated_run_connected_around_a_slot();
    test_to_next_annular_profile_ignores_material_inside_its_hole();
    test_to_next_across_two_separated_runs();
    test_to_next_near_seat_tangent_contact();
    test_to_next_scale_extremes();
    test_to_next_far_from_origin();
    test_to_next_with_inflated_target_tolerance();
    test_draft();
    test_boolean_target_from_body_ref();
    test_boolean_ignores_face_ref_at_input0();
    if (g_failures == 0) std::fprintf(stderr, "wp6_extrude: OK\n");
    return g_failures;
}
