#include <cmath>
#include <cstdio>
#include <string>

#include "nlohmann/json.hpp"
#include "protocol/Dispatcher.h"
#include "protocol/SolverLane.h"
#include "session/SketchStore.h"

using nlohmann::json;
using onecad::protocol::Envelope;

namespace {
int failures = 0;

void check(bool value, const char* message) {
    if (!value) {
        ++failures;
        std::fprintf(stderr, "FAIL: %s\n", message);
    }
}

void tiny_point_delta() {
    onecad::session::SketchStore store;
    onecad::protocol::Dispatcher dispatcher;
    onecad::protocol::SolverLane lane(store);
    lane.register_verbs(dispatcher);
    const json args = {
        {"sketchId", "tiny-point"}, {"plane", {{"kind", "XY"}}},
        {"entities", json::array({
            {{"id", "a"}, {"type", "Point"}, {"at", {0.0, 0.0}}},
            {{"id", "b"}, {"type", "Point"}, {"at", {10.0, 5e-8}}},
            {{"id", "line"}, {"type", "Line"}, {"p0Ref", "a"}, {"p1Ref", "b"}},
        })},
        {"constraints", json::array({
            {{"id", "fixed"}, {"type", "Fixed"}, {"entities", {"a"}}},
            {{"id", "horizontal"}, {"type", "Horizontal"}, {"entities", {"line"}}},
        })},
    };
    const auto result = dispatcher.dispatch_once(Envelope::request(1, "SketchUpsert", args));
    check(result.ok.value_or(false), "tiny point upsert succeeds");
    check(result.result.at("positions").contains("b"), "5e-8 mm correction is not dropped");
    check(result.result.at("positions").at("b") == json::array({10.0, 0.0}), "tiny point pose is exact");
    check(store.snapshot("tiny-point")->wire_args.at("entities").at(1).at("at") ==
          result.result.at("positions").at("b"), "reported pose equals stored pose");
}

void raw_curve_scalar_baselines() {
    onecad::session::SketchStore store;
    onecad::protocol::Dispatcher dispatcher;
    onecad::protocol::SolverLane lane(store);
    lane.register_verbs(dispatcher);
    const json circle = {
        {"sketchId", "tiny-radius"}, {"plane", {{"kind", "XY"}}},
        {"entities", json::array({{{"id", "circle"}, {"type", "Circle"}, {"center", {0.0, 0.0}}, {"radius", 4.0}}})},
        {"constraints", json::array({{{"id", "radius"}, {"type", "Radius"}, {"entities", {"circle"}}, {"value", 4.00000005}}})},
    };
    const auto radius = dispatcher.dispatch_once(Envelope::request(1, "SketchUpsert", circle));
    check(radius.ok.value_or(false), "tiny radius upsert succeeds");
    check(radius.result.at("curves").at("circle").at("radius") == 4.00000005,
          "5e-8 mm radius correction is exact");
    const json arc = {
        {"sketchId", "normalized-arc"}, {"plane", {{"kind", "XY"}}},
        {"entities", json::array({{{"id", "arc"}, {"type", "Arc"}, {"center", {0.0, 0.0}},
            {"radius", 5.0}, {"startAngle", -7.0}, {"endAngle", 7.0}}})},
        {"constraints", json::array()},
    };
    const auto normalized = dispatcher.dispatch_once(Envelope::request(2, "SketchUpsert", arc));
    check(normalized.ok.value_or(false), "noncanonical arc angle upsert succeeds");
    const auto& scalars = normalized.result.at("curves").at("arc");
    const auto stored = store.snapshot("normalized-arc")->wire_args.at("entities").at(0);
    check(scalars.at("startAngle") == stored.at("startAngle"), "normalized start angle reported from raw request baseline");
    check(scalars.at("endAngle") == stored.at("endAngle"), "normalized end angle reported from raw request baseline");
    check(scalars.at("startAngle") != -7.0 && scalars.at("endAngle") != 7.0,
          "translation normalization cannot disappear from the correction payload");
}
}

int main() {
    tiny_point_delta();
    raw_curve_scalar_baselines();
    return failures;
}
