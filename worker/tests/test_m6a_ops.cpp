// test_m6a_ops.cpp — M6a breadth-op numerics (Shell / Linear+Circular pattern /
// MirrorBody), in-process via the op executors (real OCCT). Exact box arithmetic +
// the resolution paths (ladder + partition-tracked) + recoverable guards. No
// framework: exit code == failure count.
#include <algorithm>
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Shape.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "ops/MirrorOp.h"
#include "ops/OpCommon.h"
#include "ops/OpTypes.h"
#include "ops/PatternOp.h"
#include "ops/ShellOp.h"
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

double vol(const TopoDS_Shape& s) { return onecad::session::shape_volume(s); }

double polar_angle_deg(const TopoDS_Shape& shape, double axis_x, double axis_y) {
    GProp_GProps props;
    BRepGProp::VolumeProperties(shape, props);
    const gp_Pnt center = props.CentreOfMass();
    double angle = std::atan2(center.Y() - axis_y, center.X() - axis_x) * 180.0 / M_PI;
    if (angle < 0.0) angle += 360.0;
    return angle;
}
std::size_t face_count(const TopoDS_Shape& s) {
    return onecad::session::compute_shape_metrics(s).face_count;
}

// The FACE whose descriptor centre is nearest (cx,cy,cz).
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

// A face input ref carrying the face's frozen descriptor + a world-point anchor.
json face_input(const std::string& body_id, const std::string& elem_id, const TopoDS_Shape& face,
                double ax, double ay, double az) {
    return json{{"primary", {{"bodyId", body_id}, {"elementId", elem_id}, {"kind", "face"}}},
                {"intent", {{"kind", "face"},
                            {"descriptor", em::ElementMapPartition::descriptor_to_json(
                                               em::ElementMapPartition::describe(face))}}},
                {"anchor", {{"worldPoint", {ax, ay, az}}}}};
}

struct Ctx {
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;
    ops::OpContext make(BodyStore& bodies, em::ElementMapPartition& part) {
        return ops::OpContext{bodies, &sketches, part, &last_sketch, false, json::object(), &cancel};
    }
};

// ── Shell: box 20×20×25 (vol 10000), open top face, t=2 → cup of vol 4112. ──────
// Outer 10000 − inner cavity 16×16×23 = 5888 ⇒ 4112 (exact box arithmetic).
void test_shell_ladder() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 20.0, 25.0).Shape();  // vol 10000
    BodyStore bodies;
    bodies.create("body_1", "op0", box);
    em::ElementMapPartition part;

    const TopoDS_Shape top = face_by_center(box, 10, 10, 25);  // +Z cap
    json op = {{"opType", "Shell"}, {"opId", "opsh"},
               {"inputs", json::array({face_input("body_1", "el_top", top, 10, 10, 25)})},
               {"params", {{"thickness", 2.0}, {"targetBodyId", "body_1"},
                           {"openFaces", json::array({"el_top"})}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_shell(ctx, op, "opsh");
    check(oc.status == ops::OpOutcome::Status::Ok, "shell(ladder): Ok");
    check(oc.needs_repair.empty(), "shell(ladder): no NeedsRepair (top face resolves)");
    check(oc.body_events.size() == 1 && oc.body_events[0].kind == "modified",
          "shell(ladder): body modified (id preserved)");
    check_near(vol(bodies.get("body_1")->geom), 4112.0, 1.0,
               "shell(ladder): 10000 − 16·16·23 = 4112");
    check(face_count(bodies.get("body_1")->geom) > 6, "shell(ladder): inner walls add faces");
}

// ── Shell: partition-TRACKED open face (bare ref, no descriptor/anchor) — the
// production path where PlanExecutor already minted the elementId. ─────────────
void test_shell_partition_tracked() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 20.0, 25.0).Shape();
    BodyStore bodies;
    bodies.create("body_1", "op0", box);
    em::ElementMapPartition part;
    const TopoDS_Shape top = face_by_center(box, 10, 10, 25);
    part.mint("body_1", "el_top", km::ElementKind::Face, top, box, json::object());

    // BARE ref (no intent/anchor) — resolves ONLY via the tracked partition entry.
    json op = {{"opType", "Shell"}, {"opId", "opsh2"},
               {"inputs", json::array({json{{"primary", {{"bodyId", "body_1"},
                                                         {"elementId", "el_top"},
                                                         {"kind", "face"}}}}})},
               {"params", {{"thickness", 2.0}, {"targetBodyId", "body_1"},
                           {"openFaces", json::array({"el_top"})}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_shell(ctx, op, "opsh2");
    check(oc.status == ops::OpOutcome::Status::Ok, "shell(tracked): Ok on a bare ref");
    check(oc.needs_repair.empty(), "shell(tracked): no NeedsRepair (partition binding)");
    check_near(vol(bodies.get("body_1")->geom), 4112.0, 1.0, "shell(tracked): vol 4112");
}

// ── Shell: ambiguous open face (top/bottom symmetric tie) ⇒ NeedsRepair, no build. ─
void test_shell_ambiguous_needs_repair() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 20.0, 25.0).Shape();
    BodyStore bodies;
    bodies.create("body_1", "op0", box);
    em::ElementMapPartition part;
    const TopoDS_Shape top = face_by_center(box, 10, 10, 25);
    // Intent = the top cap, anchor at the body CENTRE → top & bottom caps tie
    // (equal area, |normal·normal|=1, equidistant anchor) ⇒ ambiguous.
    json op = {{"opType", "Shell"}, {"opId", "opsh3"},
               {"inputs", json::array({face_input("body_1", "el_x", top, 10, 10, 12.5)})},
               {"params", {{"thickness", 2.0}, {"targetBodyId", "body_1"},
                           {"openFaces", json::array({"el_x"})}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_shell(ctx, op, "opsh3");
    check(oc.status == ops::OpOutcome::Status::Ok, "shell ambiguous: state, not error");
    check(!oc.needs_repair.empty(), "shell ambiguous: NeedsRepair emitted");
    check(oc.body_events.empty(), "shell ambiguous: body NOT modified (never a wrong bind)");
    check_near(vol(bodies.get("body_1")->geom), 10000.0, 1e-6, "shell ambiguous: box unchanged");
}

// ── Shell: thickness below kMinValue → recoverable OP_FAILED. ──────────────────
void test_shell_thickness_too_small() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 20.0, 25.0).Shape();
    BodyStore bodies;
    bodies.create("body_1", "op0", box);
    em::ElementMapPartition part;
    const TopoDS_Shape top = face_by_center(box, 10, 10, 25);
    json op = {{"opType", "Shell"}, {"opId", "opsh4"},
               {"inputs", json::array({face_input("body_1", "el_top", top, 10, 10, 25)})},
               {"params", {{"thickness", 0.0}, {"targetBodyId", "body_1"},
                           {"openFaces", json::array({"el_top"})}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_shell(ctx, op, "opsh4");
    check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "OP_FAILED",
          "shell: thickness too small → OP_FAILED (recoverable)");
}

// ── LinearPattern: touching copies fuse into one 60×20×25 solid. ───────
void test_linear_pattern() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 20.0, 25.0).Shape();  // vol 10000
    BodyStore bodies;
    bodies.create("body_src", "op0", box);
    em::ElementMapPartition part;
    json op = {{"opType", "LinearPattern"}, {"opId", "oplp"},
               {"params", {{"sourceBodyId", "body_src"}, {"direction", {1, 0, 0}},
                           {"spacing", 20.0}, {"count", 3}, {"fuseResult", true}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_linear_pattern(ctx, op, "oplp");
    check(oc.status == ops::OpOutcome::Status::Ok, "linpat: Ok");
    check(oc.body_events.size() == 1 && oc.body_events[0].kind == "created", "linpat: NewBody created");
    check(bodies.contains("body_oplp"), "linpat: NewBody id body_oplp (D1)");
    check(oc.delta.empty(), "linpat: empty delta (ID-on-demand NewBody)");
    check(bodies.contains("body_src"), "linpat: source body preserved");
    if (bodies.contains("body_oplp")) {
        check_near(vol(bodies.get("body_oplp")->geom), 30000.0, 1.0, "linpat: fused volume 30000");
    }
}

// ── LinearPattern: fuseResult=false → compound of 3, same 30000 total. ─────────
void test_linear_pattern_compound() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 20.0, 25.0).Shape();
    BodyStore bodies;
    bodies.create("body_src", "op0", box);
    em::ElementMapPartition part;
    json op = {{"opType", "LinearPattern"}, {"opId", "oplp2"},
               {"params", {{"sourceBodyId", "body_src"}, {"direction", {0, 1, 0}},
                           {"spacing", 40.0}, {"count", 3}, {"fuseResult", false}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_linear_pattern(ctx, op, "oplp2");
    check(oc.status == ops::OpOutcome::Status::Ok, "linpat(compound): Ok");
    if (bodies.contains("body_oplp2")) {
        check_near(vol(bodies.get("body_oplp2")->geom), 30000.0, 1.0, "linpat(compound): 30000");
    }
}

// ── LinearPattern V2 non-fuse: source is instance 0; children are instances 1..N-1. ──
void test_linear_pattern_v2_preserves_source() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 20.0, 25.0).Shape();
    BodyStore bodies;
    bodies.create("body_src", "op0", box);
    em::ElementMapPartition part;
    json op = {{"opType", "LinearPattern"}, {"opId", "oplpv2"},
               {"params", {{"sourceBodyId", "body_src"}, {"direction", {1, 0, 0}},
                           {"spacing", 40.0}, {"count", 4}, {"fuseResult", false},
                           {"resultPolicyVersion", 2}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_linear_pattern(ctx, op, "oplpv2");
    check(oc.status == ops::OpOutcome::Status::Ok, "linpat(v2): Ok");
    check(oc.body_events.size() == 3 && oc.body_ids.size() == 3,
          "linpat(v2): creates count - 1 children only");
    check(bodies.contains("body_src") && !bodies.contains("body_oplpv2"),
          "linpat(v2): source survives; no legacy aggregate body");
    for (int k = 0; k < 3; ++k) {
        const std::string child = "body_oplpv2:" + std::to_string(k);
        check(bodies.contains(child), "linpat(v2): deterministic child id");
        if (bodies.contains(child)) {
            check_near(vol(bodies.get(child)->geom), 10000.0, 1.0,
                       "linpat(v2): child is one source instance");
            check_near(onecad::session::compute_shape_metrics(bodies.get(child)->geom).bbox_min[0],
                       40.0 * (k + 1), 1e-6, "linpat(v2): child ordinal matches instance");
            check(bodies.get(child)->face_colors.empty(),
                  "linpat(v2): child does not inherit source face identities/colors");
        }
    }
    for (const auto& event : oc.body_events) {
        check(event.body_id != "body_src", "linpat(v2): source emits no lifecycle event");
    }
}

void test_linear_pattern_v2_fused_modifies_source() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 20.0, 25.0).Shape();
    BodyStore bodies;
    bodies.create("body_src", "op0", box);
    em::ElementMapPartition part;
    json op = {{"opType", "LinearPattern"}, {"opId", "oplpv2f"},
               {"params", {{"sourceBodyId", "body_src"}, {"direction", {1, 0, 0}},
                           {"spacing", 20.0}, {"count", 3}, {"fuseResult", true},
                           {"resultPolicyVersion", 2}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_linear_pattern(ctx, op, "oplpv2f");
    check(oc.status == ops::OpOutcome::Status::Ok, "linpat(v2 fused): Ok");
    check(oc.body_events.size() == 1 && oc.body_events[0].kind == "modified" &&
              oc.body_events[0].body_id == "body_src",
          "linpat(v2 fused): source modified in place");
    check(!bodies.contains("body_oplpv2f"), "linpat(v2 fused): no duplicate result body");
    check_near(vol(bodies.get("body_src")->geom), 30000.0, 1.0,
               "linpat(v2 fused): source holds fused result");
}

// ── LinearPattern guards. ─────────────────────────────────────────────────────
void test_linear_pattern_guards() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 20.0, 25.0).Shape();
    BodyStore bodies;
    bodies.create("body_src", "op0", box);
    em::ElementMapPartition part;
    Ctx c;
    {
        json op = {{"opType", "LinearPattern"}, {"opId", "oplpA"},
                   {"params", {{"sourceBodyId", "body_src"}, {"direction", {1, 0, 0}},
                               {"spacing", 40.0}, {"count", 1}, {"fuseResult", true}}}};
        ops::OpContext ctx = c.make(bodies, part);
        ops::OpOutcome oc = ops::execute_linear_pattern(ctx, op, "oplpA");
        check(oc.status == ops::OpOutcome::Status::Failed, "linpat: count<2 → OP_FAILED");
    }
    {
        json op = {{"opType", "LinearPattern"}, {"opId", "oplpB"},
                   {"params", {{"sourceBodyId", "body_src"}, {"direction", {1, 0, 0}},
                               {"spacing", 0.0}, {"count", 3}, {"fuseResult", true}}}};
        ops::OpContext ctx = c.make(bodies, part);
        ops::OpOutcome oc = ops::execute_linear_pattern(ctx, op, "oplpB");
        check(oc.status == ops::OpOutcome::Status::Failed, "linpat: spacing 0 → OP_FAILED");
    }
    {
        json op = {{"opType", "LinearPattern"}, {"opId", "oplpC"},
                   {"params", {{"sourceBodyId", "no_such"}, {"direction", {1, 0, 0}},
                               {"spacing", 40.0}, {"count", 3}, {"fuseResult", true}}}};
        ops::OpContext ctx = c.make(bodies, part);
        ops::OpOutcome oc = ops::execute_linear_pattern(ctx, op, "oplpC");
        check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "REF_UNRESOLVED",
              "linpat: missing source → REF_UNRESOLVED");
    }
    {
        json op = {{"opType", "LinearPattern"}, {"opId", "oplpD"},
                   {"params", {{"sourceBodyId", "body_src"}, {"direction", {1, 0}},
                               {"spacing", 40.0}, {"count", "3"}, {"fuseResult", true}}}};
        ops::OpContext ctx = c.make(bodies, part);
        ops::OpOutcome oc = ops::execute_linear_pattern(ctx, op, "oplpD");
        check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "OP_FAILED",
              "linpat: malformed present fields → OP_FAILED");
    }
    {
        json op = {{"opType", "LinearPattern"}, {"opId", "oplpE"},
                   {"params", {{"sourceBodyId", "body_src"}, {"direction", {1, 0, 0}},
                               {"spacing", 40.0}, {"count", 3}, {"fuseResult", "true"}}}};
        ops::OpContext ctx = c.make(bodies, part);
        ops::OpOutcome oc = ops::execute_linear_pattern(ctx, op, "oplpE");
        check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "OP_FAILED",
              "linpat: non-boolean fuseResult → OP_FAILED");
    }
    {
        json op = {{"opType", "LinearPattern"}, {"opId", "oplpF"},
                   {"params", {{"sourceBodyId", "body_src"}, {"direction", {1, 0, 0}},
                               {"spacing", 40.0}, {"count", 3}, {"fuseResult", true}}}};
        ops::OpContext ctx = c.make(bodies, part);
        ops::OpOutcome oc = ops::execute_linear_pattern(ctx, op, "oplpF");
        check(oc.status == ops::OpOutcome::Status::Failed &&
                  oc.error_code == "PATTERN_DISJOINT_RESULT",
              "linpat: fused disjoint result → PATTERN_DISJOINT_RESULT");
        check(!bodies.contains("body_oplpF"), "linpat: refusal does not mint a body");
    }
}

// ── CircularPattern: 3× box around a Z-axis far away → 3 disjoint ⇒ 30000. ─────
void test_circular_pattern() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 20.0, 25.0).Shape();
    BodyStore bodies;
    bodies.create("body_src", "op0", box);
    em::ElementMapPartition part;
    json op = {{"opType", "CircularPattern"}, {"opId", "opcp"},
               {"params", {{"sourceBodyId", "body_src"}, {"axisOrigin", {0, -100, 0}},
                           {"axisDirection", {0, 0, 1}}, {"angleDeg", 360.0}, {"count", 3},
                           {"fuseResult", false}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_circular_pattern(ctx, op, "opcp");
    check(oc.status == ops::OpOutcome::Status::Ok, "circpat(legacy compound): Ok");
    check(oc.body_events.size() == 1 && oc.body_events[0].kind == "created", "circpat: NewBody created");
    if (bodies.contains("body_opcp")) {
        check_near(vol(bodies.get("body_opcp")->geom), 30000.0, 2.0, "circpat: 3 × 10000 (disjoint)");
    }
}

// ── CircularPattern partial sweep: same `angleDeg / count` formula as SCHEMA/FE. ──
void test_circular_pattern_partial_sweep() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 20.0, 25.0).Shape();
    BodyStore bodies;
    bodies.create("body_src", "op0", box);
    em::ElementMapPartition part;
    json op = {{"opType", "CircularPattern"}, {"opId", "opcp_partial"},
               {"params", {{"sourceBodyId", "body_src"}, {"axisOrigin", {0, -100, 0}},
                           {"axisDirection", {0, 0, 1}}, {"angleDeg", 180.0}, {"count", 3},
                           {"fuseResult", false}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_circular_pattern(ctx, op, "opcp_partial");
    check(oc.status == ops::OpOutcome::Status::Ok, "circpat(partial): Ok");
    if (!bodies.contains("body_opcp_partial")) return;

    std::vector<double> angles;
    for (const TopoDS_Shape& solid : ops::ordered_solids(bodies.get("body_opcp_partial")->geom)) {
        angles.push_back(polar_angle_deg(solid, 0.0, -100.0));
    }
    std::sort(angles.begin(), angles.end());
    check(angles.size() == 3, "circpat(partial): source plus two cloned solids");
    if (angles.size() != 3) return;
    check_near(angles[1] - angles[0], 60.0, 1e-6,
               "circpat(partial): second instance is 180 / 3 = 60 degrees away");
    check_near(angles[2] - angles[1], 60.0, 1e-6,
               "circpat(partial): third instance is 120 degrees, not terminal 180");
}

// ── MirrorBody: mirror the box about x=0 (no fuse) → mirrored 10000 on −X. ──────
void test_mirror_no_fuse() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 20.0, 25.0).Shape();  // [0,20]³-ish
    BodyStore bodies;
    bodies.create("body_src", "op0", box);
    em::ElementMapPartition part;
    json op = {{"opType", "MirrorBody"}, {"opId", "opmir"},
               {"params", {{"sourceBodyId", "body_src"}, {"planePoint", {0, 0, 0}},
                           {"planeNormal", {1, 0, 0}}, {"fuseWithOriginal", false}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_mirror_body(ctx, op, "opmir");
    check(oc.status == ops::OpOutcome::Status::Ok, "mirror: Ok");
    check(bodies.contains("body_opmir"), "mirror: NewBody id body_opmir");
    if (bodies.contains("body_opmir")) {
        const auto m = onecad::session::compute_shape_metrics(bodies.get("body_opmir")->geom);
        check_near(m.volume, 10000.0, 1.0, "mirror(no fuse): mirrored vol 10000");
        check_near(m.bbox_min[0], -20.0, 1e-4, "mirror(no fuse): xmin −20 (reflected to −X)");
        check_near(m.bbox_max[0], 0.0, 1e-4, "mirror(no fuse): xmax 0");
    }
}

// ── MirrorBody: fuse source + mirror about x=0 → merged 40×20×25 = 20000. ──────
void test_mirror_fuse() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 20.0, 25.0).Shape();
    BodyStore bodies;
    bodies.create("body_src", "op0", box);
    em::ElementMapPartition part;
    json op = {{"opType", "MirrorBody"}, {"opId", "opmir2"},
               {"params", {{"sourceBodyId", "body_src"}, {"planePoint", {0, 0, 0}},
                           {"planeNormal", {1, 0, 0}}, {"fuseWithOriginal", true}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_mirror_body(ctx, op, "opmir2");
    check(oc.status == ops::OpOutcome::Status::Ok, "mirror(fuse): Ok");
    if (bodies.contains("body_opmir2")) {
        check_near(vol(bodies.get("body_opmir2")->geom), 20000.0, 1.0,
                   "mirror(fuse): 2 × 10000 merged across x=0");
    }
}

void test_mirror_strict_params() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 20.0, 25.0).Shape();
    BodyStore bodies;
    bodies.create("body_src", "op0", box);
    em::ElementMapPartition part;
    json op = {{"opType", "MirrorBody"}, {"opId", "opmir_bad"},
               {"params", {{"sourceBodyId", "body_src"}, {"planePoint", {0, 0, 0}},
                           {"planeNormal", {1, 0, 0}}, {"fuseWithOriginal", 1}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_mirror_body(ctx, op, "opmir_bad");
    check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "OP_FAILED",
          "mirror: non-boolean fuseWithOriginal → OP_FAILED");
}

}  // namespace

int main() {
    test_shell_ladder();
    test_shell_partition_tracked();
    test_shell_ambiguous_needs_repair();
    test_shell_thickness_too_small();
    test_linear_pattern();
    test_linear_pattern_compound();
    test_linear_pattern_v2_preserves_source();
    test_linear_pattern_v2_fused_modifies_source();
    test_linear_pattern_guards();
    test_circular_pattern();
    test_circular_pattern_partial_sweep();
    test_mirror_no_fuse();
    test_mirror_fuse();
    test_mirror_strict_params();
    if (g_failures == 0) std::fprintf(stderr, "m6a_ops: OK\n");
    return g_failures;
}
