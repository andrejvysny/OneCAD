// test_hole_extrude.cpp — closes the untested-claim gap: a region-with-hole
// extrude, in-process via execute_extrude (real OCCT), mirroring the direct-op
// harness in test_wp6_extrude.cpp. No framework: exit code == failure count.
//
// Pipeline under test: WireSketch::translate (Circle entity) -> LoopDetector
// (buildFaceHierarchy nests the circle loop as an innerLoop of the rectangle
// loop) -> FaceBuilder (adds the hole wire to the TopoDS_Face, FaceBuilder.cpp
// ~552-570) -> build_profile_face (OpCommon.cpp) -> execute_extrude (prism of a
// face-with-hole => a through-hole solid).
#include <cmath>
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include "nlohmann/json.hpp"
#include "ops/ExtrudeOp.h"
#include "ops/OpTypes.h"
#include "session/BodyStore.h"
#include "session/ShapeMetrics.h"
#include "util/Cancel.h"

using nlohmann::json;
namespace ops = onecad::ops;
namespace em = onecad::elementmap;
using onecad::session::BodyStore;

namespace {
int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) { std::fprintf(stderr, "FAIL: %s\n", msg.c_str()); ++g_failures; }
}
void check_near(double got, double want, double tol, const std::string& msg) {
    if (std::abs(got - want) > tol) {
        std::fprintf(stderr, "FAIL: %s (got %.6f want %.6f tol %.4f)\n", msg.c_str(), got, want, tol);
        ++g_failures;
    }
}

// Outer w×h rectangle on the XY plane, optionally with a circular hole of
// radius `r` centered at (w/2, h/2) — well clear of every edge for w=40,h=20,
// r=5 (10mm margin to the nearest side).
json rect_with_optional_hole(const std::string& sid, double w, double h, bool with_hole, double r) {
    json sk;
    sk["sketchId"] = sid;
    sk["plane"] = json{{"kind", "XY"}};
    json entities = json::array({
        json{{"id", "e1"}, {"type", "Line"}, {"p0", {0, 0}}, {"p1", {w, 0}}},
        json{{"id", "e2"}, {"type", "Line"}, {"p0", {w, 0}}, {"p1", {w, h}}},
        json{{"id", "e3"}, {"type", "Line"}, {"p0", {w, h}}, {"p1", {0, h}}},
        json{{"id", "e4"}, {"type", "Line"}, {"p0", {0, h}}, {"p1", {0, 0}}},
    });
    if (with_hole) {
        entities.push_back(json{{"id", "c1"}, {"type", "Circle"}, {"center", {w / 2.0, h / 2.0}}, {"radius", r}});
    }
    sk["entities"] = entities;
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

constexpr double kW = 40.0, kH = 20.0, kR = 5.0, kDepth = 10.0;

// Positive case: 40x20 rectangle with a centered r=5 hole (not touching any
// edge -> LoopDetector must yield ONE region with ONE innerLoop), extruded
// depth 10, NewBody. Volume must equal (w*h - pi*r^2) * depth; the profile
// face-with-hole prism should produce 7 faces (4 planar sides from the outer
// rectangle + top/bottom annular planar faces + 1 cylindrical hole-wall face).
void test_hole_extrude_volume_and_faces() {
    BodyStore bodies;
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", rect_with_optional_hole("sk1", kW, kH, /*with_hole=*/true, kR)});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json op = {{"opType", "Extrude"}, {"opId", "ope"},
               {"params", {{"sketchId", "sk1"}, {"distance", kDepth}, {"extrudeMode", "Blind"},
                           {"booleanMode", "NewBody"}}}};
    ops::OpOutcome oc = ops::execute_extrude(ctx, op, "ope");
    check(oc.status == ops::OpOutcome::Status::Ok, "hole extrude: Ok");
    check(bodies.contains("body_ope"), "hole extrude: NewBody created");
    if (!bodies.contains("body_ope")) return;

    const TopoDS_Shape& shape = bodies.get("body_ope")->geom;
    const double want_volume = (kW * kH - M_PI * kR * kR) * kDepth;
    const double got_volume = onecad::session::shape_volume(shape);
    // OCCT tessellation/quadrature of the cylindrical hole wall carries some
    // numeric error; 1e-3 relative (matched to want_volume) is comfortably
    // above BRepGProp's actual error for this size of solid.
    check_near(got_volume, want_volume, want_volume * 1e-3, "hole extrude: volume == (w*h - pi*r^2)*depth");

    const onecad::session::ShapeMetrics m = onecad::session::compute_shape_metrics(shape);
    check(m.face_count == 7,
          "hole extrude: face count == 7 (4 outer sides + top + bottom + 1 cylindrical hole wall), got " +
              std::to_string(m.face_count));
}

// Negative control: the SAME outer rectangle, no circle. Proves the volume
// deficit above is really the hole (not an unrelated systematic error) — a
// solid rectangular prism has the full w*h*depth volume and 6 faces.
void test_no_hole_control() {
    BodyStore bodies;
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", rect_with_optional_hole("sk1", kW, kH, /*with_hole=*/false, kR)});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json op = {{"opType", "Extrude"}, {"opId", "ope"},
               {"params", {{"sketchId", "sk1"}, {"distance", kDepth}, {"extrudeMode", "Blind"},
                           {"booleanMode", "NewBody"}}}};
    ops::OpOutcome oc = ops::execute_extrude(ctx, op, "ope");
    check(oc.status == ops::OpOutcome::Status::Ok, "no-hole control: Ok");
    check(bodies.contains("body_ope"), "no-hole control: NewBody created");
    if (!bodies.contains("body_ope")) return;

    const TopoDS_Shape& shape = bodies.get("body_ope")->geom;
    const double want_volume = kW * kH * kDepth;
    const double got_volume = onecad::session::shape_volume(shape);
    check_near(got_volume, want_volume, want_volume * 1e-3,
               "no-hole control: volume == w*h*depth (no subtraction)");

    const onecad::session::ShapeMetrics m = onecad::session::compute_shape_metrics(shape);
    check(m.face_count == 6, "no-hole control: plain box has 6 faces, got " + std::to_string(m.face_count));
}

}  // namespace

int main() {
    test_hole_extrude_volume_and_faces();
    test_no_hole_control();
    if (g_failures == 0) std::fprintf(stderr, "hole_extrude: OK\n");
    return g_failures;
}
