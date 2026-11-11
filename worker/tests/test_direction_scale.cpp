// test_direction_scale.cpp — independent review NEW-03 and its siblings.
// A user direction is normalized by its length; `sqrt(x² + y² + z²)` (and OCCT's
// `gp_Dir(x, y, z)`, which does the same) overflows for a finite direction such
// as [1e200, 0, 0], the length becomes +inf and every unit component 0. Measured
// on the unfixed worker: LinearPattern published its copy COINCIDENT with the
// source; CircularPattern and TransformBody spun at 100 % CPU; MirrorBody refused
// with a misleading GEOMETRY_INVALID.
//
// Invariant pinned here: multiplying a nonzero direction by a positive scalar
// does not change the result — every body's volume and centroid match the
// unit-direction control exactly, and a pattern copy is never coincident.
// Ops: LinearPattern, CircularPattern, MirrorBody, TransformBody rotate,
// FeaturePattern Linear/Circular, Gear frame axis. No framework.
#include <algorithm>
#include <array>
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>

#include "nlohmann/json.hpp"
#include "session/PlanExecutor.h"
#include "session/ScratchJob.h"
#include "util/Cancel.h"

using nlohmann::json;
namespace session = onecad::session;

namespace {
int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) { std::fprintf(stderr, "FAIL: %s\n", msg.c_str()); ++g_failures; }
}

json sketch_op() {
    return {{"opType", "Sketch"}, {"opId", "sk_op"},
            {"params", {{"sketchId", "sk"}, {"plane", {{"kind", "XY"}}},
                        {"entities", json::array({
                            {{"id", "a"}, {"type", "Line"}, {"p0", {0, 0}}, {"p1", {2, 0}}},
                            {{"id", "b"}, {"type", "Line"}, {"p0", {2, 0}}, {"p1", {2, 2}}},
                            {{"id", "c"}, {"type", "Line"}, {"p0", {2, 2}}, {"p1", {0, 2}}},
                            {{"id", "d"}, {"type", "Line"}, {"p0", {0, 2}}, {"p1", {0, 0}}}})},
                        {"constraints", json::array()}}}};
}
json extrude_op() {
    return {{"opType", "Extrude"}, {"opId", "ex"},
            {"params", {{"sketchId", "sk"}, {"distance", 2.0}, {"extrudeMode", "Blind"},
                        {"booleanMode", "NewBody"}}}};
}
json body_input() {
    return json::array({{{"primary", {{"bodyId", "body_ex"}, {"elementId", "body_ex"}, {"kind", "body"}}}}});
}

using Signature = std::vector<std::array<double, 4>>;  // per body: volume + centroid, sorted

// Runs the 2×2×2 seed box (unless `seed` is false) then `op`; the signature of
// every body afterwards, or empty when the op did not succeed.
Signature run(const json& op, bool seed = true) {
    session::ScratchJob job;
    std::string last;
    onecad::CancelToken cancel;
    if (seed) {
        session::execute_candidate_op(job, sketch_op(), "sk_op", last, cancel);
        session::execute_candidate_op(job, extrude_op(), "ex", last, cancel);
    }
    const auto r = session::execute_candidate_op(job, op, op.value("opId", "op"), last, cancel);
    if (r.status != session::CandidateResult::Status::Ok) {
        std::fprintf(stderr, "  %s refused: %s\n", op.value("opType", "").c_str(), r.error_message.c_str());
        return {};
    }
    Signature out;
    for (const auto& [id, rec] : job.bodies.all()) {
        GProp_GProps props;
        BRepGProp::VolumeProperties(rec.geom, props);
        const gp_Pnt c = props.CentreOfMass();
        out.push_back({props.Mass(), c.X(), c.Y(), c.Z()});
    }
    std::sort(out.begin(), out.end());
    return out;
}

void expect_same(const char* label, const Signature& unit, const Signature& scaled,
                 std::size_t bodies) {
    check(unit.size() == bodies, std::string(label) + ": unit control publishes " +
                                     std::to_string(bodies) + " bodies (got " +
                                     std::to_string(unit.size()) + ")");
    check(scaled.size() == unit.size(), std::string(label) + ": scaled direction publishes as many bodies");
    for (std::size_t i = 0; i < std::min(unit.size(), scaled.size()); ++i) {
        for (int k = 0; k < 4; ++k) {
            if (std::abs(unit[i][k] - scaled[i][k]) > 1e-9) {
                check(false, std::string(label) + ": body " + std::to_string(i) + " component " +
                                 std::to_string(k) + " " + std::to_string(scaled[i][k]) +
                                 " != unit control " + std::to_string(unit[i][k]));
            }
        }
    }
    // No two bodies of a pattern/copy share a centroid.
    for (std::size_t i = 1; i < scaled.size(); ++i) {
        const double d = std::hypot(scaled[i][1] - scaled[i - 1][1], scaled[i][2] - scaled[i - 1][2],
                                    scaled[i][3] - scaled[i - 1][3]);
        check(d > 1.0, std::string(label) + ": no coincident copy");
    }
}

json with(json op, const std::string& key, const json& value) {
    op["params"][key] = value;
    return op;
}

void test_body_ops() {
    const json linear = {{"opType", "LinearPattern"}, {"opId", "pat"}, {"inputs", body_input()},
                         {"params", {{"sourceBodyId", "body_ex"}, {"spacing", 10.0}, {"count", 2},
                                     {"fuseResult", false}, {"resultPolicyVersion", 2}}}};
    expect_same("LinearPattern [1e200,0,0]", run(with(linear, "direction", {1.0, 0.0, 0.0})),
                run(with(linear, "direction", {1e200, 0.0, 0.0})), 2);
    expect_same("LinearPattern [3e200,4e200,0]", run(with(linear, "direction", {3.0, 4.0, 0.0})),
                run(with(linear, "direction", {3e200, 4e200, 0.0})), 2);

    const json circular = {{"opType", "CircularPattern"}, {"opId", "pat"}, {"inputs", body_input()},
                           {"params", {{"sourceBodyId", "body_ex"}, {"axisOrigin", {10.0, 0.0, 0.0}},
                                       {"angleDeg", 180.0}, {"count", 2}, {"fuseResult", false},
                                       {"resultPolicyVersion", 2}}}};
    expect_same("CircularPattern axis [0,0,1e200]", run(with(circular, "axisDirection", {0.0, 0.0, 1.0})),
                run(with(circular, "axisDirection", {0.0, 0.0, 1e200})), 2);

    const json mirror = {{"opType", "MirrorBody"}, {"opId", "mir"}, {"inputs", body_input()},
                         {"params", {{"sourceBodyId", "body_ex"}, {"planePoint", {5.0, 0.0, 0.0}},
                                     {"fuseWithOriginal", false}}}};
    expect_same("MirrorBody normal [1e200,0,0]", run(with(mirror, "planeNormal", {1.0, 0.0, 0.0})),
                run(with(mirror, "planeNormal", {1e200, 0.0, 0.0})), 2);

    const json rotate = {{"opType", "TransformBody"}, {"opId", "mv"}, {"inputs", body_input()},
                         {"params", {{"targets", {"body_ex"}}, {"copy", true}}}};
    auto rotation = [](double scale) {
        return json{{"center", {10.0, 0.0, 0.0}}, {"axis", {0.0, 0.0, scale}}, {"angleDeg", 90.0}};
    };
    expect_same("TransformBody rotate.axis [0,0,1e200]", run(with(rotate, "rotate", rotation(1.0))),
                run(with(rotate, "rotate", rotation(1e200))), 2);
}

void test_feature_pattern() {
    json sketch = sketch_op();
    json extrude = extrude_op();
    sketch["sourceRecordId"] = "sk_op";
    extrude["sourceRecordId"] = "ex";
    auto pattern = [&](const json& layout) {
        return json{{"opType", "FeaturePattern"}, {"opId", "fp"},
                    {"params", {{"semanticsVersion", 1}, {"count", 2},
                                {"sourceRecordIds", {"sk_op", "ex"}}, {"layout", layout},
                                {"sourceOps", json::array({sketch, extrude})}}}};
    };
    auto linear = [](double scale) {
        return json{{"kind", "Linear"}, {"direction", {scale, 0.0, 0.0}}, {"spacing", 20.0}};
    };
    expect_same("FeaturePattern Linear [1e200,0,0]", run(pattern(linear(1.0))),
                run(pattern(linear(1e200))), 2);
    auto circular = [](double scale) {
        return json{{"kind", "Circular"}, {"axisOrigin", {10.0, 0.0, 0.0}},
                    {"axisDirection", {0.0, 0.0, scale}}, {"angleDeg", 180.0}};
    };
    expect_same("FeaturePattern Circular axis [0,0,1e200]", run(pattern(circular(1.0))),
                run(pattern(circular(1e200))), 2);
}

void test_gear_frame() {
    auto gear = [](double scale) {
        const json block = {{"teeth", 12}, {"module", 1.0}, {"height", 3.0}, {"pressureAngleDeg", 20.0},
                            {"shift", 0.0}, {"helixAngleDeg", 0.0}, {"doubleHelix", false},
                            {"propertiesFromTool", false}, {"undercut", false}, {"backlash", 0.0},
                            {"clearance", 0.25}, {"head", 0.0}, {"sampleCount", 8}};
        return json{{"opType", "Gear"}, {"opId", "g"}, {"inputs", json::array()},
                    {"params", {{"recipe", "involuteExternal"},
                                {"placement", {{"face", nullptr},
                                               {"frame", {{"origin", {0.0, 0.0, 0.0}},
                                                          {"axis", {scale, 0.0, 0.0}},
                                                          {"xDir", {0.0, scale, 0.0}}}},
                                               {"point", {0.0, 0.0, 0.0}}}},
                                {"involuteExternal", block}}}};
    };
    expect_same("Gear frame axis/xDir ×1e200", run(gear(1.0), false), run(gear(1e200), false), 1);
}

}  // namespace

int main() {
    try {
        test_body_ops();
        test_feature_pattern();
        test_gear_frame();
    } catch (const std::exception& e) {
        std::fprintf(stderr, "FAIL: exception %s\n", e.what());
        ++g_failures;
    }
    if (g_failures == 0) std::fprintf(stderr, "test_direction_scale: all checks passed\n");
    return g_failures == 0 ? 0 : 1;
}
