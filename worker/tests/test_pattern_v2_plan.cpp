// Pattern V2 ExecutePlan lifecycle: source is instance zero, no source event.
#include <cstdio>
#include <string>
#include <vector>

#include "nlohmann/json.hpp"
#include "protocol/Envelope.h"
#include "session/PlanExecutor.h"
#include "session/Session.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::CancelToken;
using onecad::protocol::Envelope;
using onecad::protocol::HandlerContext;
using onecad::session::Session;

namespace {
constexpr const char* kEmpty =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
int g_failures = 0;

void check(bool condition, const std::string& message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message.c_str());
        ++g_failures;
    }
}

json rectangle() {
    return json::array({
        {{"id", "e1"}, {"type", "Line"}, {"p0", {0, 0}}, {"p1", {20, 0}}},
        {{"id", "e2"}, {"type", "Line"}, {"p0", {20, 0}}, {"p1", {20, 20}}},
        {{"id", "e3"}, {"type", "Line"}, {"p0", {20, 20}}, {"p1", {0, 20}}},
        {{"id", "e4"}, {"type", "Line"}, {"p0", {0, 20}}, {"p1", {0, 0}}},
    });
}

json pattern_plan() {
    return json::array({
        {{"opType", "Sketch"}, {"opId", "op0"}, {"stepIndex", 0},
         {"params", {{"sketchId", "sk0"}, {"plane", {{"kind", "XY"}}},
                     {"entities", rectangle()}, {"constraints", json::array()}}}},
        {{"opType", "Extrude"}, {"opId", "op1"}, {"stepIndex", 1},
         {"params", {{"sketchId", "sk0"}, {"distance", 25.0},
                     {"extrudeMode", "Blind"}, {"booleanMode", "NewBody"}}}},
        {{"opType", "LinearPattern"}, {"opId", "op2"}, {"stepIndex", 2},
         {"params", {{"sourceBodyId", "body_op1"}, {"direction", {1, 0, 0}},
                     {"spacing", 40.0}, {"count", 4}, {"fuseResult", false},
                     {"resultPolicyVersion", 2}}}},
    });
}

std::vector<json> execute(Session& session) {
    session.open("doc", 0, 3, "determinism");
    std::vector<json> steps;
    CancelToken token;
    HandlerContext context{token, [](int) {}, [&steps](Envelope& event) {
                               if (event.event_name == "planStep") steps.push_back(event.result);
                           }};
    const json ops = pattern_plan();
    const json args = {{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3},
                       {"expectedBaseHash", kEmpty},
                       {"prefixHashes", json::array({"h0", "h1", "h2"})},
                       {"targetStep", 2}, {"ops", ops}};
    onecad::session::handle_execute_plan(session, Envelope::request(1, "ExecutePlan", args),
                                         context);
    onecad::session::handle_accept_prepared(
        session, Envelope::request(1, "AcceptPrepared",
                                   json{{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    return steps;
}
}  // namespace

int main() {
    Session session;
    const std::vector<json> steps = execute(session);
    check(steps.size() == 3, "v2 plan: streams three successful steps");
    if (steps.size() == 3) {
        const json& events = steps[2]["bodyEvents"];
        check(events == json::array({json{{"kind", "created"}, {"bodyId", "body_op2:0"}},
                                     json{{"kind", "created"}, {"bodyId", "body_op2:1"}},
                                     json{{"kind", "created"}, {"bodyId", "body_op2:2"}}}),
              "v2 plan: Pattern creates ordinal children only");
        for (const json& event : events) {
            check(event.value("bodyId", "") != "body_op1",
                  "v2 plan: source emits neither modified nor deleted lifecycle event");
        }
    }
    const onecad::session::BodyStore bodies = session.bodies_copy();
    check(bodies.contains("body_op1"), "v2 plan: source survives at head");
    check(bodies.contains("body_op2:0") && bodies.contains("body_op2:1") &&
              bodies.contains("body_op2:2"),
          "v2 plan: all children survive at head");
    check(!bodies.contains("body_op2"), "v2 plan: no legacy aggregate body");
    if (g_failures == 0) std::fprintf(stderr, "pattern_v2_plan: OK\n");
    return g_failures;
}
