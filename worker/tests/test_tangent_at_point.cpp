#include <cmath>
#include <algorithm>
#include <cstdio>
#include <numbers>
#include <string>

#include "nlohmann/json.hpp"
#include "sketch/ResidualGate.h"
#include "sketch/SketchPose.h"
#include "sketch/constraints/Constraints.h"
#include "protocol/SolverLane.h"
#include "protocol/Dispatcher.h"
#include "session/SketchStore.h"
#include "sketch/Sketch.h"
#include "sketch/WireSketch.h"

using nlohmann::json;
namespace sk = onecad::core::sketch;
namespace {
int failures = 0;
void check(bool condition, const std::string& message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message.c_str());
        ++failures;
    }
}
json relation(const std::string& id, const std::string& type, const std::string& a,
              const std::string& b, const std::string& role_a, const std::string& role_b) {
    return {{"id", id}, {"type", type}, {"entities", {a, b}}, {"positions", {role_a, role_b}}};
}
json corner(bool reverse = false, double perturbation = 0.0, double length = 20.0) {
    json first = {{"id", "a"},
                  {"type", "Line"},
                  {"p0", {length * std::sin(perturbation), 5 + length * std::cos(perturbation)}},
                  {"p1", {0, 5}}};
    if (reverse)
        std::swap(first["p0"], first["p1"]);
    return {{"sketchId", "t1"},
            {"plane", {{"kind", "XY"}}},
            {"entities",
             {first,
              {{"id", "b"}, {"type", "Line"}, {"p0", {5, 0}}, {"p1", {25, 0}}},
              {{"id", "arc"},
               {"type", "Arc"},
               {"center", {5, 5}},
               {"radius", 5},
               {"startAngle", std::numbers::pi},
               {"endAngle", 1.5 * std::numbers::pi}}}},
            {"constraints",
             {relation("w1", "Coincident", "a", "arc", reverse ? "Start" : "End", "Start"),
              relation("w2", "Coincident", "b", "arc", "Start", "End")}}};
}
void rank_sequence(json input, const std::vector<json>& tangents,
                   const std::vector<int>& expected) {
    for (std::size_t count = 0; count <= tangents.size(); ++count) {
        if (count)
            input["constraints"].push_back(tangents[count - 1]);
        auto translated = onecad::wire::translate(input);
        check(translated.ok,
              "translate rank stage " + std::to_string(count) + ": " + translated.error);
        if (!translated.ok)
            continue;
        const auto solved = translated.sketch->solve();
        check(solved.success && solved.truth.publishable(),
              "solve rank stage " + std::to_string(count));
        check(translated.sketch->getDegreesOfFreedom() == expected[count],
              "DOF stage " + std::to_string(count) + " expected " +
                  std::to_string(expected[count]) + " got " +
                  std::to_string(translated.sketch->getDegreesOfFreedom()));
        check(!translated.sketch->hasRedundantConstraints(), "independent endpoint row");
        check(solved.truth.residuals.violations.empty(), "true angular residual accepted");
    }
}
void t1_rank_reversal_and_perturbation() {
    for (bool reverse_first : {false, true})
        for (bool reverse_second : {false, true})
            for (bool swap_arc : {false, true})
                for (double perturbation : {0.0, -0.001, 0.001}) {
                    auto input = corner(reverse_first, perturbation);
                    if (reverse_second)
                        std::swap(input["entities"][1]["p0"], input["entities"][1]["p1"]);
                    if (swap_arc)
                        std::swap(input["entities"][2]["startAngle"],
                                  input["entities"][2]["endAngle"]);
                    const std::string first = reverse_first ? "Start" : "End";
                    const std::string second = reverse_second ? "End" : "Start";
                    const std::string arc_first = swap_arc ? "End" : "Start";
                    const std::string arc_second = swap_arc ? "Start" : "End";
                    input["constraints"][0]["positions"] = {first, arc_first};
                    input["constraints"][1]["positions"] = {second, arc_second};
                    rank_sequence(
                        input,
                        {
                            relation("t1", "TangentAtPoint", "a", "arc", first, arc_first),
                            relation("t2", "TangentAtPoint", "b", "arc", second, arc_second),
                            {{"id", "r"},
                             {"type", "Radius"},
                             {"entities", {"arc"}},
                             {"value", 5.0}},
                        },
                        {9, 8, 7, 6});
                }
}
void t5_slot() {
    json input = {{"sketchId", "slot"},
                  {"plane", {{"kind", "XY"}}},
                  {"entities",
                   {{{"id", "top"}, {"type", "Line"}, {"p0", {-50, 20}}, {"p1", {50, 20}}},
                    {{"id", "bottom"}, {"type", "Line"}, {"p0", {-50, -20}}, {"p1", {50, -20}}},
                    {{"id", "right"},
                     {"type", "Arc"},
                     {"center", {50, 0}},
                     {"radius", 20},
                     {"startAngle", -std::numbers::pi / 2},
                     {"endAngle", std::numbers::pi / 2}},
                    {{"id", "left"},
                     {"type", "Arc"},
                     {"center", {-50, 0}},
                     {"radius", 20},
                     {"startAngle", std::numbers::pi / 2},
                     {"endAngle", 1.5 * std::numbers::pi}}}},
                  {"constraints",
                   {relation("w1", "Coincident", "top", "right", "End", "End"),
                    relation("w2", "Coincident", "bottom", "right", "End", "Start"),
                    relation("w3", "Coincident", "top", "left", "Start", "Start"),
                    relation("w4", "Coincident", "bottom", "left", "Start", "End"),
                    {{"id", "equal"}, {"type", "Equal"}, {"entities", {"right", "left"}}}}}};
    rank_sequence(input,
                  {
                      relation("t1", "TangentAtPoint", "top", "right", "End", "End"),
                      relation("t2", "TangentAtPoint", "bottom", "right", "End", "Start"),
                      relation("t3", "TangentAtPoint", "top", "left", "Start", "Start"),
                      relation("t4", "TangentAtPoint", "bottom", "left", "Start", "End"),
                  },
                  {9, 8, 7, 6, 5});
}
void additional_rank_vectors() {
    rank_sequence(corner(false, 0.0, 0.01),
                  {relation("t1", "TangentAtPoint", "a", "arc", "End", "Start"),
                   relation("t2", "TangentAtPoint", "b", "arc", "Start", "End")},
                  {9, 8, 7});
    auto tee = corner();
    tee["entities"][1]["p0"] = {0, 5};
    tee["entities"][1]["p1"] = {0, -15};
    tee["constraints"][1]["positions"] = {"Start", "Start"};
    rank_sequence(tee,
                  {relation("t1", "TangentAtPoint", "a", "arc", "End", "Start"),
                   relation("t2", "TangentAtPoint", "b", "arc", "Start", "Start")},
                  {9, 8, 7});
    json arcs = {{"sketchId", "arcs"},
                 {"plane", {{"kind", "XY"}}},
                 {"entities",
                  {{{"id", "a"},
                    {"type", "Arc"},
                    {"center", {-5, 0}},
                    {"radius", 5},
                    {"startAngle", 0},
                    {"endAngle", std::numbers::pi / 2}},
                   {{"id", "b"},
                    {"type", "Arc"},
                    {"center", {5, 0}},
                    {"radius", 5},
                    {"startAngle", std::numbers::pi / 2},
                    {"endAngle", std::numbers::pi}}}},
                 {"constraints", {relation("w", "Coincident", "a", "b", "Start", "End")}}};
    rank_sequence(arcs, {relation("t", "TangentAtPoint", "a", "b", "Start", "End")}, {8, 7});
    json lines = {{"sketchId", "lines"},
                  {"plane", {{"kind", "XY"}}},
                  {"entities",
                   {{{"id", "a"}, {"type", "Line"}, {"p0", {-5, 0}}, {"p1", {0, 0}}},
                    {{"id", "b"}, {"type", "Line"}, {"p0", {0, 0}}, {"p1", {5, 0}}}}},
                  {"constraints", {relation("w", "Coincident", "a", "b", "End", "Start")}}};
    rank_sequence(lines, {relation("t", "TangentAtPoint", "a", "b", "End", "Start")}, {6, 5});
    json shared = {{"sketchId", "shared"},
                   {"plane", {{"kind", "XY"}}},
                   {"entities",
                    {{{"id", "p0"}, {"type", "Point"}, {"at", {-5, 0}}},
                     {{"id", "joint"}, {"type", "Point"}, {"at", {0, 0}}},
                     {{"id", "p1"}, {"type", "Point"}, {"at", {5, 0}}},
                     {{"id", "a"}, {"type", "Line"}, {"p0Ref", "p0"}, {"p1Ref", "joint"}},
                     {{"id", "b"}, {"type", "Line"}, {"p0Ref", "joint"}, {"p1Ref", "p1"}}}},
                   {"constraints", json::array()}};
    rank_sequence(shared, {relation("t", "TangentAtPoint", "a", "b", "End", "Start")}, {6, 5});
}
void malformed_inputs_fail_by_name() {
    for (int bad = 0; bad < 9; ++bad) {
        auto input = corner();
        auto tangent = relation("t", "TangentAtPoint", "a", "arc", "End", "Start");
        std::string expected = "TANGENT_ENDPOINT_UNRESOLVED";
        if (bad == 0)
            tangent.erase("positions");
        if (bad == 1)
            tangent["positions"] = {"End"};
        if (bad == 2)
            tangent["positions"] = {"Center", "Start"};
        if (bad == 3)
            tangent["entities"] = "malformed";
        if (bad == 4) {
            input["constraints"] = json::array();
            expected = "TANGENT_ENDPOINT_NOT_WELDED";
        }
        if (bad == 5) {
            input["entities"][0]["p0"] = {0, 5};
            expected = "TANGENT_DEGENERATE_OPERAND";
        }
        if (bad == 6) {
            input["entities"][0]["p0"] = {20, 5};
            expected = "TANGENT_BRANCH_AMBIGUOUS";
        }
        if (bad == 7) {
            input["entities"][2]["radius"] = 0;
            expected = "TANGENT_DEGENERATE_OPERAND";
        }
        if (bad == 8) {
            input["entities"].push_back({{"id", "mid"}, {"type", "Point"}, {"at", {0, 5}}});
            input["constraints"][0] = relation("hop1", "Coincident", "a", "mid", "End", "");
            input["constraints"].push_back(
                relation("hop2", "Coincident", "mid", "arc", "", "Start"));
            expected = "TANGENT_ENDPOINT_NOT_WELDED";
        }
        input["constraints"].push_back(tangent);
        const auto result = onecad::wire::translate(input);
        check(!result.ok && result.error.starts_with(expected),
              "named atomic refusal " + expected + ": " + result.error);
    }
}
void fixed_branch_rotations_and_collapse() {
    auto input = corner();
    input["constraints"].push_back(relation("t", "TangentAtPoint", "a", "arc", "End", "Start"));
    auto result = onecad::wire::translate(input);
    check(result.ok, "branch fixture translates");
    if (!result.ok)
        return;
    auto& sketch = *result.sketch;
    check(sketch.solve().success, "branch fixture solves");
    const double rotation = 2.2;
    for (const auto& entity : sketch.getAllEntities()) {
        if (auto* point = dynamic_cast<sk::SketchPoint*>(entity.get())) {
            const double x = point->x(), y = point->y();
            point->setPosition(x * std::cos(rotation) - y * std::sin(rotation),
                               x * std::sin(rotation) + y * std::cos(rotation));
        } else if (auto* arc = dynamic_cast<sk::SketchArc*>(entity.get())) {
            arc->startAngle() += rotation;
            arc->endAngle() += rotation;
        }
    }
    check(sk::checkResiduals(sketch).violations.empty(),
          "joint rotation >90 degrees keeps fixed relative branch");
    const sk::SketchPose before(sketch);
    const auto joint = result.index.resolve_point("a", "end"),
               free = result.index.resolve_point("a", "start");
    auto* p = sketch.getEntityAs<sk::SketchPoint>(free);
    const auto* t = sketch.getEntityAs<sk::SketchPoint>(joint);
    p->setPosition(2 * t->x() - p->x(), 2 * t->y() - p->y());
    auto issues = before.integrity(sketch);
    check(
        std::any_of(issues.begin(), issues.end(),
                    [](const auto& issue) { return issue.reason == "TANGENT_RESIDUAL_EXCEEDED"; }),
        "relative branch reversal refused");
    before.restore(sketch);
    p->setPosition(t->position());
    issues = before.integrity(sketch);
    check(
        std::any_of(issues.begin(), issues.end(),
                    [](const auto& issue) { return issue.reason == "TANGENT_DEGENERATE_OPERAND"; }),
        "collapsed leg explicitly refused");
    before.restore(sketch);
    check(sk::checkResiduals(sketch).violations.empty(), "complete pose restored after refusal");
}
struct Lane {
    onecad::session::SketchStore store;
    onecad::protocol::Dispatcher dispatcher;
    onecad::protocol::SolverLane solver;
    std::uint64_t next = 1;
    Lane() : solver(store) { solver.register_verbs(dispatcher); }
    onecad::protocol::Envelope call(const std::string& verb, json args) {
        return dispatcher.dispatch_once(
            onecad::protocol::Envelope::request(next++, verb, std::move(args)));
    }
};
void drag_and_cache_refusal() {
    Lane lane;
    auto input = corner();
    input["constraints"].push_back(relation("t1", "TangentAtPoint", "a", "arc", "End", "Start"));
    input["constraints"].push_back(relation("t2", "TangentAtPoint", "b", "arc", "Start", "End"));
    auto upsert = lane.call("SketchUpsert", input);
    check(upsert.ok.value_or(false), "upsert endpoint tangent");
    if (!upsert.ok.value_or(false))
        return;
    const auto revision = upsert.result["sketchRevision"];
    auto begin = lane.call("BeginGesture", {{"sketchId", "t1"},
                                            {"sketchRevision", revision},
                                            {"gestureId", 99},
                                            {"pointId", "arc.start"},
                                            {"drag", {{"pointId", "arc.start"}}}});
    check(begin.ok.value_or(false), "begin welded endpoint drag");
    if (!begin.ok.value_or(false))
        return;
    const auto drag = lane.call(
        "SolveDrag", {{"gestureId", 99}, {"seq", 1}, {"pointId", "arc.start"}, {"target", {1, 6}}});
    check(drag.ok.value_or(false), "drag welded endpoint");
    const auto end = lane.call("EndGesture", {{"gestureId", 99}});
    check(end.ok.value_or(false), "end endpoint drag");
    const auto saved = lane.store.snapshot("t1");
    const auto& solved_entities = saved->wire_args["entities"];
    const auto& arc = solved_entities[2];
    for (int side : {0, 1}) {
        const auto& line = solved_entities[side];
        const double dx = line["p1"][0].get<double>() - line["p0"][0].get<double>();
        const double dy = line["p1"][1].get<double>() - line["p0"][1].get<double>();
        const double angle = arc[side == 0 ? "startAngle" : "endAngle"];
        const double radius = arc["radius"], length = std::hypot(dx, dy);
        const double residual = std::asin(
            std::clamp(std::abs((dx * std::cos(angle) + dy * std::sin(angle)) / length), 0.0, 1.0));
        check(residual <= 1e-4 / std::max(radius, length),
              "independent radian endpoint angle after End");
        const auto& endpoint = line[side == 0 ? "p1" : "p0"];
        const double ex = arc["center"][0].get<double>() + radius * std::cos(angle);
        const double ey = arc["center"][1].get<double>() + radius * std::sin(angle);
        check(std::hypot(endpoint[0].get<double>() - ex, endpoint[1].get<double>() - ey) <= 1e-4,
              "independent endpoint weld after End");
    }
    auto retained = onecad::wire::translate(saved->wire_args);
    check(retained.ok, "retained drag geometry translates");
    if (retained.ok) {
        const auto residuals = sk::checkResiduals(*retained.sketch);
        check(residuals.unassessed.empty() && residuals.violations.empty(),
              "independent released angular residual and weld truth");
    }
    auto bad = input;
    bad["constraints"][2].erase("positions");
    const auto refused = lane.call("SketchUpsert", bad);
    check(!refused.ok.value_or(false), "malformed endpoint upsert refused");
    check(refused.error && refused.error->detail &&
              (*refused.error->detail)["diagnostics"][0]["reasonCode"] ==
                  "TANGENT_ENDPOINT_UNRESOLVED",
          "structured named endpoint refusal");
    check(lane.store.snapshot("t1")->wire_args == saved->wire_args &&
              lane.store.snapshot("t1")->revision == saved->revision,
          "refusal leaves worker store byte-equivalent");
}

} // namespace
int main() {
    t1_rank_reversal_and_perturbation();
    t5_slot();
    additional_rank_vectors();
    malformed_inputs_fail_by_name();
    fixed_branch_rotations_and_collapse();
    drag_and_cache_refusal();
    return failures ? 1 : 0;
}
