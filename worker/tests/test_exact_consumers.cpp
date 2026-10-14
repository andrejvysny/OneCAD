#include <cassert>
#include <numbers>

#include "ops/OpCommon.h"
#include "sketch/Sketch.h"
#include "sketch/WireSketch.h"
#include "ops/ExtrudeOp.h"
#include "ops/RevolveOp.h"
#include "protocol/Dispatcher.h"
#include "protocol/SolverLane.h"
#include "session/PlanExecutor.h"
#include "session/SketchStore.h"
#include "session/Session.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::protocol::Envelope;

json near_sketch(const std::string& id) {
    return {{"sketchId", id}, {"plane", {{"kind", "XY"}}},
        {"entities", json::array({{{"id", "a"}, {"type", "Point"}, {"at", {0, 0}}},
            {{"id", "b"}, {"type", "Point"}, {"at", {10, 0}}},
            {{"id", "axis"}, {"type", "Line"}, {"p0Ref", "a"}, {"p1Ref", "b"}}})},
        {"constraints", json::array({{{"id", "ten"}, {"type", "Distance"}, {"entities", {"a", "b"}}, {"value", 10}},
            {{"id", "near"}, {"type", "Distance"}, {"entities", {"a", "b"}}, {"value", 10.001}}})}};
}

void successful_violating_candidate_keeps_real_correction() {
    auto input = near_sketch("retained");
    input["entities"].push_back({{"id", "circle"}, {"type", "Circle"}, {"center", {30, 30}}, {"radius", 4}});
    input["constraints"].push_back({{"id", "radius"}, {"type", "Radius"}, {"entities", {"circle"}}, {"value", 5}});
    auto translated = onecad::wire::translate(input);
    assert(translated.ok);
    const auto solved = translated.sketch->solve();
    assert(solved.success && solved.truth.publishable() && !solved.truth.admissible());
    const auto* circle = translated.sketch->getEntityAs<onecad::core::sketch::SketchCircle>(
        translated.index.wire_to_internal.at("circle"));
    assert(circle && circle->radius() == 5);
}

void negative_radii_refuse_before_store_mutation() {
    onecad::session::SketchStore store;
    onecad::protocol::Dispatcher dispatcher;
    onecad::protocol::SolverLane lane(store);
    lane.register_verbs(dispatcher);
    json input = {{"sketchId", "shape"}, {"plane", {{"kind", "XY"}}},
        {"entities", json::array({{{"id", "curve"}, {"type", "Circle"}, {"center", {0, 0}}, {"radius", 4}}})},
        {"constraints", json::array()}};
    assert(dispatcher.dispatch_once(Envelope::request(1, "SketchUpsert", input)).ok.value_or(false));
    const auto prior = store.snapshot("shape");
    for (const std::string type : {"Circle", "Arc", "Ellipse", "EllipseMinor"}) {
        auto bad = input;
        bad["entities"][0]["type"] = type == "EllipseMinor" ? "Ellipse" : type;
        bad["entities"][0]["radius"] = -1;
        bad["entities"][0]["majorR"] = type == "EllipseMinor" ? 2 : -1;
        bad["entities"][0]["minorR"] = type == "EllipseMinor" ? -1 : 2;
        const auto refused = dispatcher.dispatch_once(Envelope::request(2, "SketchUpsert", bad));
        assert(!refused.ok.value_or(true) && refused.error->code == "OP_FAILED");
        assert(refused.error->detail->at("diagnostics")[0]["reasonCode"] == "internalCouplingFailure");
        assert(store.snapshot("shape")->wire_args == prior->wire_args);
        assert(store.snapshot("shape")->revision == prior->revision);
    }
    input["constraints"].push_back({{"id", "radius"}, {"type", "Radius"}, {"entities", {"curve"}}, {"value", -1}});
    const auto solved = dispatcher.dispatch_once(Envelope::request(3, "SketchUpsert", input));
    assert(!solved.ok.value_or(true) && solved.error->detail->at("diagnostics")[0]["reasonCode"] == "internalCouplingFailure");
    assert(store.snapshot("shape")->wire_args == prior->wire_args);
    assert(store.snapshot("shape")->revision == prior->revision);
    input["constraints"] = json::array(); input["entities"][0]["radius"] = 0;
    assert(dispatcher.dispatch_once(Envelope::request(4, "SketchUpsert", input)).ok.value_or(false));
}

void profile_refuses_successful_but_violating_pose() {
    std::string error;
    std::vector<json> diagnostics;
    const auto face = onecad::ops::build_profile_face(near_sketch("profile"), "", std::nullopt,
                                                     error, &diagnostics);
    assert(!face && !diagnostics.empty());
    assert(diagnostics[0]["reasonCode"] == "residualViolation");
}

void independent_revolve_axis_and_extrude_refuse_typed_conflicts() {
    onecad::session::BodyStore bodies;
    onecad::elementmap::ElementMapPartition partition;
    const json valid = {{"sketchId", "profile"}, {"plane", {{"kind", "XY"}}},
        {"entities", json::array({{{"id", "circle"}, {"type", "Circle"},
            {"center", {30, 0}}, {"radius", 5}}})}, {"constraints", json::array()}};
    std::vector<std::pair<std::string, json>> sketches = {{"profile", valid}, {"axis", near_sketch("axis")}};
    std::string last = "axis";
    onecad::ops::OpContext context{bodies, &sketches, partition, &last};
    const json revolve = {{"params", {{"sketchId", "profile"}, {"angleDeg", 360},
        {"axis", {{"kind", "sketchLine"}, {"sketchId", "axis"}, {"lineId", "axis"}}}}}};
    const auto refused = onecad::ops::execute_revolve(context, revolve, "revolve");
    assert(refused.status == onecad::ops::OpOutcome::Status::Failed);
    assert(refused.error_message.find("axis sketch exact truth") != std::string::npos);
    assert(!refused.diagnostics.empty() && refused.diagnostics[0]["reasonCode"] == "residualViolation");
    const auto extrude = onecad::ops::execute_extrude(context,
        {{"params", {{"sketchId", "axis"}, {"distance", 10}, {"extrudeMode", "Blind"}}}}, "extrude");
    assert(extrude.status == onecad::ops::OpOutcome::Status::Failed);
    assert(!extrude.diagnostics.empty() && extrude.diagnostics[0]["reasonCode"] == "residualViolation");
    assert(bodies.all().empty());
}

void redundant_final_target_refusal_is_partial_with_released_truth() {
    onecad::session::SketchStore store;
    onecad::protocol::Dispatcher dispatcher;
    onecad::protocol::SolverLane lane(store);
    lane.register_verbs(dispatcher);
    const json fixed = {{"id", "fixed"}, {"type", "Fixed"}, {"entities", {"arc"}}, {"positions", {"Start"}}};
    const json args = {{"sketchId", "arc"}, {"plane", {{"kind", "XY"}}},
        {"entities", json::array({{{"id", "arc"}, {"type", "Arc"}, {"center", {0, 0}},
            {"radius", 20}, {"startAngle", 0}, {"endAngle", std::numbers::pi / 2}}})},
        {"constraints", json::array({fixed, {{"id", "duplicate"}, {"type", "Fixed"},
            {"entities", {"arc"}}, {"positions", {"Start"}}}})}};
    const auto upsert = dispatcher.dispatch_once(Envelope::request(1, "SketchUpsert", args));
    assert(upsert.ok.value_or(false) && upsert.result["state"] == "OverConstrained");
    const auto begin = dispatcher.dispatch_once(Envelope::request(2, "BeginGesture", {
        {"sketchId", "arc"}, {"sketchRevision", 1}, {"gestureId", 5},
        {"drag", {{"kind", "arcEnd"}, {"entity", "arc"}, {"role", "end"}}}}));
    assert(begin.ok.value_or(false));
    const auto end = dispatcher.dispatch_once(Envelope::request(3, "EndGesture", {
        {"gestureId", 5}, {"commit", {{"finalTarget", {20, 0}}}}}));
    assert(end.ok.value_or(false) && end.result["status"] == "partial");
    assert(end.result["state"] == "OverConstrained" && end.result["conflicting"].empty());
    assert(!end.result["diagnostics"].empty());
    assert(end.result["entityStates"] == begin.result["entityStates"]);
    assert(store.snapshot("arc")->wire_args["entities"][0]["endAngle"] == std::numbers::pi / 2);
}

int main() {
    negative_radii_refuse_before_store_mutation();
    successful_violating_candidate_keeps_real_correction();
    profile_refuses_successful_but_violating_pose();
    independent_revolve_axis_and_extrude_refuse_typed_conflicts();
    redundant_final_target_refusal_is_partial_with_released_truth();
}
