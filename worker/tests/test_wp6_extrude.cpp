// test_wp6_extrude.cpp — W-WP6 extrude completion (scope C): ToFace / ToNext end
// conditions (typed targetFace ref via the ladder) + draft angle. In-process via
// execute_extrude with real OCCT. No framework: exit code == failure count.
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax1.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "ops/ExtrudeOp.h"
#include "ops/OpTypes.h"
#include "session/BodyStore.h"
#include "session/ShapeMetrics.h"
#include "util/Cancel.h"

using nlohmann::json;
namespace ops = onecad::ops;
namespace em = onecad::elementmap;
namespace km = onecad::kernel::elementmap;
using onecad::session::BodyStore;

namespace {
int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) { std::fprintf(stderr, "FAIL: %s\n", msg.c_str()); ++g_failures; }
}
void check_near(double got, double want, double tol, const std::string& msg) {
    if (std::abs(got - want) > tol) {
        std::fprintf(stderr, "FAIL: %s (got %.4f want %.4f)\n", msg.c_str(), got, want);
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
    return {{"primary", {{"bodyId", body_id}, {"elementId", element_id}, {"kind", "face"}}},
            {"intent", {{"kind", "face"},
                         {"descriptor", em::ElementMapPartition::descriptor_to_json(
                                            em::ElementMapPartition::describe(face))}}},
            {"anchor", {{"worldPoint", {0.0, 0.0, 0.0}}}}};
}

// Sketch params: a w×h rectangle on the XY plane. World map (u,v)->(-v,u,0), so a
// w(u)×h(v) rect → world x∈[-h,0], y∈[0,w], z=0 (area w·h).
json rect_sketch(const std::string& sid, double w, double h) {
    json sk;
    sk["sketchId"] = sid;
    sk["plane"] = json{{"kind", "XY"}};
    sk["entities"] = json::array({
        json{{"id", "e1"}, {"type", "Line"}, {"p0", {0, 0}}, {"p1", {w, 0}}},
        json{{"id", "e2"}, {"type", "Line"}, {"p0", {w, 0}}, {"p1", {w, h}}},
        json{{"id", "e3"}, {"type", "Line"}, {"p0", {w, h}}, {"p1", {0, h}}},
        json{{"id", "e4"}, {"type", "Line"}, {"p0", {0, h}}, {"p1", {0, 0}}},
    });
    sk["constraints"] = json::array();
    return sk;
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
    test_draft();
    test_boolean_target_from_body_ref();
    test_boolean_ignores_face_ref_at_input0();
    if (g_failures == 0) std::fprintf(stderr, "wp6_extrude: OK\n");
    return g_failures;
}
