// Boolean Intersect standalone fixtures: overlap, containment, identity, and
// touching cases. Verifies the worker op consumes the tool only when a real
// solid intersection exists and preserves the target BodyId.
#include <cmath>
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include <BRepPrimAPI_MakeBox.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "ops/BooleanOp.h"
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

void check(bool condition, const std::string& message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message.c_str());
        ++g_failures;
    }
}

struct Fixture {
    BodyStore bodies;
    em::ElementMapPartition partition;
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;

    ops::OpContext context() {
        return {bodies, &sketches, partition, &last_sketch, false, json::object(), &cancel};
    }
};

ops::OpOutcome run_intersect(Fixture& fixture, const TopoDS_Shape& target,
                             const TopoDS_Shape& tool) {
    fixture.bodies.create("body_target", "seed_target", target);
    fixture.bodies.create("body_tool", "seed_tool", tool);
    const json op = {{"opType", "Boolean"}, {"opId", "op_intersect"},
                     {"params", {{"operation", "Intersect"}, {"targetBodyId", "body_target"},
                                 {"toolBodyId", "body_tool"}}}};
    ops::OpContext context = fixture.context();
    return ops::execute_boolean(context, op, "op_intersect");
}

void test_overlap_intersect() {
    Fixture fixture;
    const TopoDS_Shape target = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    const TopoDS_Shape tool = BRepPrimAPI_MakeBox(gp_Pnt(5.0, 0.0, 0.0), 10.0, 10.0, 10.0).Shape();

    const ops::OpOutcome outcome = run_intersect(fixture, target, tool);
    check(outcome.status == ops::OpOutcome::Status::Ok, "overlap Intersect succeeds");
    check(fixture.bodies.contains("body_target"), "overlap Intersect preserves target");
    check(!fixture.bodies.contains("body_tool"), "overlap Intersect consumes tool");

    const double volume = onecad::session::shape_volume(fixture.bodies.get("body_target")->geom);
    check(std::abs(volume - 500.0) < 1e-6, "overlap Intersect volume is 500");
}

void test_containment_intersect() {
    Fixture fixture;
    const TopoDS_Shape target = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    const TopoDS_Shape tool = BRepPrimAPI_MakeBox(gp_Pnt(2.5, 2.5, 2.5), 5.0, 5.0, 5.0).Shape();

    const ops::OpOutcome outcome = run_intersect(fixture, target, tool);
    check(outcome.status == ops::OpOutcome::Status::Ok, "containment Intersect succeeds");
    check(fixture.bodies.contains("body_target"), "containment Intersect preserves target");
    check(!fixture.bodies.contains("body_tool"), "containment Intersect consumes tool");

    const double volume = onecad::session::shape_volume(fixture.bodies.get("body_target")->geom);
    check(std::abs(volume - 125.0) < 1e-6, "containment Intersect volume is 125");
}

void test_identity_intersect() {
    Fixture fixture;
    const TopoDS_Shape target = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();

    const ops::OpOutcome outcome = run_intersect(fixture, target, target);
    check(outcome.status == ops::OpOutcome::Status::Ok, "identity Intersect succeeds");
    check(fixture.bodies.contains("body_target"), "identity Intersect preserves target");
    check(!fixture.bodies.contains("body_tool"), "identity Intersect consumes tool");

    const double volume = onecad::session::shape_volume(fixture.bodies.get("body_target")->geom);
    check(std::abs(volume - 1000.0) < 1e-6, "identity Intersect volume is 1000");
}

void test_touching_intersect(const char* name, const gp_Pnt& tool_origin) {
    Fixture fixture;
    const TopoDS_Shape target = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    const TopoDS_Shape tool = BRepPrimAPI_MakeBox(tool_origin, 10.0, 10.0, 10.0).Shape();
    const double target_volume = onecad::session::shape_volume(target);
    const double tool_volume = onecad::session::shape_volume(tool);

    const ops::OpOutcome outcome = run_intersect(fixture, target, tool);
    check(outcome.status == ops::OpOutcome::Status::Failed,
          std::string(name) + " Intersect is refused");
    check(outcome.error_code == "OP_FAILED",
          std::string(name) + " Intersect refusal is OP_FAILED");
    check(fixture.bodies.contains("body_target") && fixture.bodies.contains("body_tool"),
          std::string(name) + " Intersect preserves both bodies");
    check(std::abs(onecad::session::shape_volume(fixture.bodies.get("body_target")->geom) -
                   target_volume) < 1e-9,
          std::string(name) + " Intersect leaves target geometry unchanged");
    check(std::abs(onecad::session::shape_volume(fixture.bodies.get("body_tool")->geom) -
                   tool_volume) < 1e-9,
          std::string(name) + " Intersect leaves tool geometry unchanged");
}
}  // namespace

int main() {
    test_overlap_intersect();
    test_containment_intersect();
    test_identity_intersect();
    test_touching_intersect("face-touching", gp_Pnt(10.0, 0.0, 0.0));
    test_touching_intersect("edge-touching", gp_Pnt(10.0, 10.0, 0.0));
    test_touching_intersect("vertex-touching", gp_Pnt(10.0, 10.0, 10.0));
    if (g_failures == 0) std::fprintf(stderr, "boolean_intersect: OK\n");
    return g_failures;
}
