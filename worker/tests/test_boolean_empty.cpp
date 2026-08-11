// Empty standalone Boolean results are refusals, never body deletion by accident.
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

void check_refusal(const char* name, const TopoDS_Shape& target, const TopoDS_Shape& tool,
                   const char* operation) {
    Fixture fixture;
    fixture.bodies.create("body_target", "seed_target", target);
    fixture.bodies.create("body_tool", "seed_tool", tool);
    const double target_volume = onecad::session::shape_volume(target);
    const double tool_volume = onecad::session::shape_volume(tool);
    const json op = {{"opType", "Boolean"}, {"opId", "op_empty"},
                     {"params", {{"operation", operation}, {"targetBodyId", "body_target"},
                                 {"toolBodyId", "body_tool"}}}};

    ops::OpContext context = fixture.context();
    const ops::OpOutcome outcome = ops::execute_boolean(context, op, "op_empty");

    check(outcome.status == ops::OpOutcome::Status::Failed,
          std::string(name) + ": empty boolean is recoverably refused");
    check(outcome.error_code == "OP_FAILED", std::string(name) + ": refusal is OP_FAILED");
    check(outcome.body_events.empty() && outcome.body_ids.empty() && outcome.delta.empty(),
          std::string(name) + ": refusal emits no lifecycle or element-map delta");
    check(fixture.bodies.contains("body_target") && fixture.bodies.contains("body_tool"),
          std::string(name) + ": target and tool both remain");
    check(std::abs(onecad::session::shape_volume(fixture.bodies.get("body_target")->geom) -
                   target_volume) < 1e-9,
          std::string(name) + ": target geometry is unchanged");
    check(std::abs(onecad::session::shape_volume(fixture.bodies.get("body_tool")->geom) -
                   tool_volume) < 1e-9,
          std::string(name) + ": tool geometry is unchanged");
}

void test_disjoint_intersect() {
    check_refusal("disjoint Intersect", BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape(),
                  BRepPrimAPI_MakeBox(gp_Pnt(20.0, 0.0, 0.0), 10.0, 10.0, 10.0).Shape(),
                  "Intersect");
}

void test_complete_cut() {
    check_refusal("containing Cut", BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape(),
                  BRepPrimAPI_MakeBox(gp_Pnt(-5.0, -5.0, -5.0), 20.0, 20.0, 20.0).Shape(),
                  "Cut");
}
}  // namespace

int main() {
    test_disjoint_intersect();
    test_complete_cut();
    if (g_failures == 0) std::fprintf(stderr, "boolean_empty: OK\n");
    return g_failures;
}
