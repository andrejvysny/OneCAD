// test_gear_transformed_referenceability.cpp — independent review NEW-01.
// SCHEMA §7.3 gear referenceability classifies a gear face against the GEAR AXIS.
// `gear_body_info` read that axis from the Gear op's authored frame and ignored a
// later `TransformBody`, so after rotating the gear 90° about world X the real cap
// (normal now ±Y) was refused at BindElementIds with GEAR_FACE_NOT_REFERENCEABLE.
//
// Driven through the real ExecutePlan → AcceptPrepared → BindElementIds verbs:
//   * unrotated control: cap and bore bind, flank refused;
//   * rotated + translated in the SAME plan (cold replay shape): cap and bore
//     bind, flank refused;
//   * rotated by an INCREMENTAL plan that does not name the Gear op: same;
//   * a face-placed gear rotated afterwards: cap binds, flank refused.
// Faces are located by geometry (normal / axis / radius), never by ordinal.
// No framework: exit code == failure count.
#include <cmath>
#include <cstdio>
#include <array>
#include <string>
#include <vector>

#include <BRepPrimAPI_MakeBox.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Shape.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "session/ClassifyElement.h"
#include "session/ElementIdentity.h"
#include "session/PlanExecutor.h"
#include "session/Session.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::CancelToken;
using onecad::protocol::Envelope;
using onecad::protocol::HandlerContext;
using onecad::session::Session;
namespace em = onecad::elementmap;

namespace {
int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) { std::fprintf(stderr, "FAIL: %s\n", msg.c_str()); ++g_failures; }
}
constexpr const char* kEmpty =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
constexpr double kRootRadius = 17.5;  // m=2, z=20: root diameter 35

json involute_block() {
    return {{"teeth", 20}, {"module", 2.0}, {"height", 5.0}, {"pressureAngleDeg", 20.0},
            {"shift", 0.0}, {"helixAngleDeg", 0.0}, {"doubleHelix", false},
            {"propertiesFromTool", false}, {"undercut", false}, {"backlash", 0.0},
            {"clearance", 0.25}, {"head", 0.0}, {"sampleCount", 12}, {"axleHole", true},
            {"axleHoleDiameter", 10.0}};
}

json gear_frame_op(int step) {
    return json{{"opType", "Gear"}, {"opId", "opgear"}, {"stepIndex", step},
                {"inputs", json::array()},
                {"params", {{"recipe", "involuteExternal"},
                            {"placement", {{"face", nullptr},
                                           {"frame", {{"origin", {0.0, 0.0, 0.0}},
                                                      {"axis", {0.0, 0.0, 1.0}},
                                                      {"xDir", {1.0, 0.0, 0.0}}}},
                                           {"point", {0.0, 0.0, 0.0}}}},
                            {"involuteExternal", involute_block()}}}};
}

// Box [0,20]×[0,20]×[0,25] (XY sketch maps (u,v) → (−v,u,0)) and a Gear seated on
// its top face at (10,10,25); the face ref is described from an identical box.
std::vector<json> face_gear_ops() {
    const json rect = json::array({
        json{{"id", "b1"}, {"type", "Line"}, {"p0", {0.0, -20.0}}, {"p1", {20.0, -20.0}}},
        json{{"id", "b2"}, {"type", "Line"}, {"p0", {20.0, -20.0}}, {"p1", {20.0, 0.0}}},
        json{{"id", "b3"}, {"type", "Line"}, {"p0", {20.0, 0.0}}, {"p1", {0.0, 0.0}}},
        json{{"id", "b4"}, {"type", "Line"}, {"p0", {0.0, 0.0}}, {"p1", {0.0, -20.0}}}});
    const TopoDS_Shape twin = BRepPrimAPI_MakeBox(20.0, 20.0, 25.0).Shape();
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(twin, TopAbs_FACE, faces);
    json top_ref;
    for (int i = 1; i <= faces.Extent(); ++i) {
        const auto d = em::ElementMapPartition::describe(faces(i));
        if (std::abs(d.center.Z() - 25.0) > 1e-9) continue;
        top_ref = {{"primary", {{"bodyId", "body_op1"}, {"elementId", "el_top"}, {"kind", "face"}}},
                   {"intent", {{"kind", "face"},
                               {"descriptor", em::ElementMapPartition::descriptor_to_json(d)}}},
                   {"anchor", {{"worldPoint", {10.0, 10.0, 25.0}}}}};
    }
    return {json{{"opType", "Sketch"}, {"opId", "op0"}, {"stepIndex", 0},
                 {"params", {{"sketchId", "sk"}, {"plane", {{"kind", "XY"}}}, {"entities", rect},
                             {"constraints", json::array()}}}},
            json{{"opType", "Extrude"}, {"opId", "op1"}, {"stepIndex", 1},
                 {"params", {{"sketchId", "sk"}, {"distance", 25.0}, {"extrudeMode", "Blind"},
                             {"booleanMode", "NewBody"}}}},
            json{{"opType", "Gear"}, {"opId", "opgear"}, {"stepIndex", 2},
                 {"inputs", json::array({top_ref})},
                 {"params", {{"recipe", "involuteExternal"},
                             {"placement", {{"face", top_ref}, {"frame", nullptr},
                                            {"point", {10.0, 10.0, 25.0}}}},
                             {"involuteExternal", involute_block()}}}}};
}

// 90° about world X through the origin, then a translation: X' = T ∘ R.
json rotate_gear_op(int step) {
    return json{{"opType", "TransformBody"}, {"opId", "opmove"}, {"stepIndex", step},
                {"inputs", json::array({json{{"primary", {{"bodyId", "body_opgear"},
                                                          {"elementId", "body_opgear"},
                                                          {"kind", "body"}}}}})},
                {"params", {{"targets", json::array({"body_opgear"})},
                            {"translate", {5.0, 7.0, 9.0}},
                            {"rotate", {{"center", {0.0, 0.0, 0.0}}, {"axis", {1.0, 0.0, 0.0}},
                                        {"angleDeg", 90.0}}}}}};
}

// One ExecutePlan + AcceptPrepared. `base` is the head prefix hash the plan builds on.
bool execute(Session& s, int job, const std::vector<json>& ops, const std::string& base,
             int first_step) {
    json hashes = json::array();
    for (std::size_t i = 0; i < ops.size(); ++i) hashes.push_back("h" + std::to_string(first_step + i));
    CancelToken tok;
    HandlerContext ctx{tok, [](int) {}, [](Envelope&) {}};
    const json args = {{"jobId", job}, {"documentRevision", job}, {"workerEpoch", 3},
                       {"expectedBaseHash", base}, {"prefixHashes", hashes},
                       {"targetStep", first_step + static_cast<int>(ops.size()) - 1},
                       {"ops", ops}};
    const Envelope r = onecad::session::handle_execute_plan(s, Envelope::request(job, "ExecutePlan", args), ctx);
    const bool completed = r.ok.value_or(false) && r.result.value("stoppedReason", "") == "completed";
    check(completed, "plan " + std::to_string(job) + " completed: " + r.result.dump().substr(0, 400));
    const Envelope a = onecad::session::handle_accept_prepared(
        s, Envelope::request(job, "AcceptPrepared",
                             json{{"jobId", job}, {"documentRevision", job}, {"workerEpoch", 3}}));
    check(a.ok.value_or(false), "plan " + std::to_string(job) + " accepted");
    return completed && a.ok.value_or(false);
}

struct Faces {
    std::string cap, bore, flank;
};

// Located by GEOMETRY against the expected (transformed) axis direction `ax`.
Faces locate(const TopoDS_Shape& gear, const std::array<double, 3>& ax) {
    Faces out;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(gear, TopAbs_FACE, faces);
    auto parallel = [&](const json& v) {
        return std::abs(v[0].get<double>() * ax[0] + v[1].get<double>() * ax[1] +
                        v[2].get<double>() * ax[2]) > 0.999;
    };
    for (int i = 1; i <= faces.Extent(); ++i) {
        const std::string key = "f:" + std::to_string(i);
        const json c = onecad::session::classify_shape(faces(i));
        const std::string type = c.value("surfaceType", "");
        if (type == "plane" && c.contains("frame") && parallel(c["frame"]["normal"])) {
            if (out.cap.empty()) out.cap = key;
        } else if (type == "cylinder" && c.contains("frame") && parallel(c["frame"]["axis"]) &&
                   c["frame"].value("radius", 0.0) < kRootRadius) {
            if (out.bore.empty()) out.bore = key;
        } else if (type != "plane" && type != "cylinder" && out.flank.empty()) {
            out.flank = key;
        }
    }
    return out;
}

Envelope bind_face(Session& s, const std::string& topo_key, const std::string& element_id) {
    const std::uint64_t snapshot = s.current_snapshot_id();
    return onecad::session::handle_bind_element_ids(
        s, Envelope::request(8, "BindElementIds",
                             json{{"snapshotId", snapshot},
                                  {"bindings", json::array({json{{"bodyId", "body_opgear"},
                                                                 {"topoKey", topo_key},
                                                                 {"elementId", element_id},
                                                                 {"kind", "face"}}})}}));
}

bool refused_as_gear(const Envelope& r) {
    if (r.ok.value_or(false) || !r.error || !r.error->detail ||
        !r.error->detail->contains("diagnostics")) {
        return false;
    }
    const json& d = (*r.error->detail)["diagnostics"];
    return !d.empty() && d[0].value("reasonCode", "") == "GEAR_FACE_NOT_REFERENCEABLE";
}

std::string message_of(const Envelope& r) { return r.error ? r.error->message : std::string{}; }

void expect_classification(Session& s, const std::array<double, 3>& ax, bool with_bore,
                           const std::string& label) {
    const onecad::session::BodyStore bodies = s.bodies_copy();
    const onecad::session::BodyRecord* rec = bodies.get("body_opgear");
    check(rec != nullptr, label + ": gear body published");
    if (rec == nullptr) return;
    const Faces f = locate(rec->geom, ax);
    check(!f.cap.empty() && !f.flank.empty() && (!with_bore || !f.bore.empty()),
          label + ": cap/bore/flank located by geometry");
    const Envelope cap = bind_face(s, f.cap, "el_cap");
    check(cap.ok.value_or(false), label + ": the real cap binds (" + message_of(cap) + ")");
    if (with_bore) {
        const Envelope bore = bind_face(s, f.bore, "el_bore");
        check(bore.ok.value_or(false), label + ": the bore binds (" + message_of(bore) + ")");
    }
    check(refused_as_gear(bind_face(s, f.flank, "el_flank")), label + ": a tooth flank is still refused");
}

}  // namespace

int main() {
    try {
        {
            Session s;
            s.open("doc", 0, 3, "determinism");
            execute(s, 1, {gear_frame_op(0)}, kEmpty, 0);
            expect_classification(s, {0.0, 0.0, 1.0}, true, "unrotated control");
        }
        {
            Session s;
            s.open("doc", 0, 3, "determinism");
            execute(s, 1, {gear_frame_op(0), rotate_gear_op(1)}, kEmpty, 0);
            expect_classification(s, {0.0, 1.0, 0.0}, true, "rotated in the same plan");
        }
        {
            Session s;
            s.open("doc", 0, 3, "determinism");
            execute(s, 1, {gear_frame_op(0)}, kEmpty, 0);
            execute(s, 2, {rotate_gear_op(1)}, "h0", 1);
            expect_classification(s, {0.0, 1.0, 0.0}, true, "rotated by an incremental plan");
        }
        {
            Session s;
            s.open("doc", 0, 3, "determinism");
            std::vector<json> ops = face_gear_ops();
            ops.push_back(rotate_gear_op(3));
            execute(s, 1, ops, kEmpty, 0);
            expect_classification(s, {0.0, 1.0, 0.0}, false, "face-placed gear rotated");
        }
    } catch (const std::exception& e) {
        std::fprintf(stderr, "FAIL: exception %s\n", e.what());
        ++g_failures;
    }
    if (g_failures == 0) std::fprintf(stderr, "test_gear_transformed_referenceability: all checks passed\n");
    return g_failures == 0 ? 0 : 1;
}
