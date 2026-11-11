// test_shell_openings.cpp — kernel audit 2026-11-11 finding 9 and the independent
// review's Shell hypothesis.
//
// 1. "A real shell always adds inner-wall faces" is not an invariant. Opening FIVE
//    faces of a box leaves one wall thickened inward: a valid plate with the input's
//    six faces. OCCT builds it exactly; the face-count guard refused it as "left the
//    body unchanged". The guard is now the actual invariant — an inward shell
//    removes material — so the ONECAD-243 no-op stays refused (test_m6a_ops).
// 2. A box with a Ø12 through hole cannot be shelled by MakeThickSolidByJoin on the
//    pinned kernel (no join/intersection configuration builds it; measured). That
//    is an open capability limitation: it must stay an honest refusal with the
//    predecessor intact — never a published wrong solid. With a fillet as well the
//    kernel's construction failure now carries SHELL_CONSTRUCTION_FAILED.
// Box 40×20×25 at the origin, thickness 2. No framework.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepPrimAPI_MakeBox.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "ops/OpTypes.h"
#include "ops/ShellOp.h"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "session/PlanExecutor.h"
#include "session/Session.h"
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
constexpr double kPi = 3.14159265358979323846;
constexpr double L = 40.0, W = 20.0, H = 25.0, T = 2.0;

double vol(const TopoDS_Shape& s) { return onecad::session::shape_volume(s); }

TopoDS_Shape face_at(const TopoDS_Shape& shape, double x, double y, double z) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    TopoDS_Shape best;
    double best_d2 = -1.0;
    for (int i = 1; i <= faces.Extent(); ++i) {
        const auto d = em::ElementMapPartition::describe(faces(i));
        const double d2 = d.center.SquareDistance(gp_Pnt(x, y, z));
        if (best_d2 < 0.0 || d2 < best_d2) { best_d2 = d2; best = faces(i); }
    }
    return best;
}

struct Result {
    ops::OpOutcome outcome;
    double before = 0.0, after = 0.0;
    int faces = 0;
};

// Shells `body` with the faces nearest `centres` opened.
Result shell(const TopoDS_Shape& body, const std::vector<gp_Pnt>& centres) {
    BodyStore bodies;
    bodies.create("body_1", "op0", body);
    em::ElementMapPartition part;
    json inputs = json::array(), ids = json::array();
    for (std::size_t i = 0; i < centres.size(); ++i) {
        const gp_Pnt& c = centres[i];
        const std::string id = "el_" + std::to_string(i);
        const TopoDS_Shape face = face_at(body, c.X(), c.Y(), c.Z());
        inputs.push_back({{"primary", {{"bodyId", "body_1"}, {"elementId", id}, {"kind", "face"}}},
                          {"intent", {{"kind", "face"},
                                      {"descriptor", em::ElementMapPartition::descriptor_to_json(
                                                         em::ElementMapPartition::describe(face))}}},
                          {"anchor", {{"worldPoint", {c.X(), c.Y(), c.Z()}}}}});
        ids.push_back(id);
    }
    const json op = {{"opType", "Shell"}, {"opId", "opsh"}, {"inputs", inputs},
                     {"params", {{"thickness", T}, {"targetBodyId", "body_1"}, {"openFaces", ids}}}};
    std::vector<std::pair<std::string, json>> sketches;
    std::string last;
    onecad::CancelToken cancel;
    ops::OpContext ctx{bodies, &sketches, part, &last, false, json::object(), &cancel};
    Result r;
    r.before = vol(body);
    r.outcome = ops::execute_shell(ctx, op, "opsh");
    const TopoDS_Shape after = bodies.get("body_1")->geom;
    r.after = vol(after);
    r.faces = static_cast<int>(onecad::session::compute_shape_metrics(after).face_count);
    return r;
}

const gp_Pnt kTop(L / 2, W / 2, H), kBottom(L / 2, W / 2, 0), kX0(0, W / 2, H / 2),
    kX1(L, W / 2, H / 2), kY0(L / 2, 0, H / 2), kY1(L / 2, W, H / 2);

void expect_ok(const char* label, const Result& r, double want) {
    check(r.outcome.status == ops::OpOutcome::Status::Ok,
          std::string(label) + ": Ok (" + r.outcome.error_message + ")");
    check(std::abs(r.after - want) < 1e-9 * want,
          std::string(label) + ": analytic volume " + std::to_string(want) + " (got " +
              std::to_string(r.after) + ")");
}

void test_multi_opening() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(L, W, H).Shape();
    expect_ok("two openings (tube)", shell(box, {kTop, kBottom}), L * W * H - (L - 2 * T) * (W - 2 * T) * H);
    const Result keep_bottom = shell(box, {kTop, kX0, kX1, kY0, kY1});
    expect_ok("five openings, bottom wall kept", keep_bottom, L * W * T);
    check(keep_bottom.faces == 6, "five openings: the plate has the input's six faces");
    expect_ok("five openings, x0 wall kept", shell(box, {kTop, kBottom, kX1, kY0, kY1}), W * H * T);
}

// The audit's p9 plan through the real ExecutePlan path: box (XY rect 40×20 →
// world x∈[−20,0], y∈[0,40], z∈[0,25]), a Ø12 cut through it at world (−10, 20),
// optionally an r=3 fillet on the x=0 / z=25 edge, then Shell t=2 open bottom.
json ref(const char* id, const char* kind, double x, double y, double z) {
    return {{"primary", {{"bodyId", "body_op1"}, {"elementId", id}, {"kind", kind}}},
            {"anchor", {{"worldPoint", {x, y, z}}}}};
}
json region(const char* sid) {
    return json::array({{{"primary", {{"bodyId", ""}, {"elementId", std::string(sid) + ".region.r0"},
                                      {"kind", "face"}}}}});
}

struct PlanResult {
    std::string stopped;
    json last_step;
    double box_volume = -1.0;
};

PlanResult p9_plan(bool with_fillet) {
    const json rect = json::array({
        {{"id", "e1"}, {"type", "Line"}, {"p0", {0, 0}}, {"p1", {40, 0}}},
        {{"id", "e2"}, {"type", "Line"}, {"p0", {40, 0}}, {"p1", {40, 20}}},
        {{"id", "e3"}, {"type", "Line"}, {"p0", {40, 20}}, {"p1", {0, 20}}},
        {{"id", "e4"}, {"type", "Line"}, {"p0", {0, 20}}, {"p1", {0, 0}}}});
    const json circle = json::array({{{"id", "c1"}, {"type", "Circle"}, {"center", {20, 10}}, {"radius", 6.0}}});
    auto sketch = [](const char* sid, const json& ents) {
        return json{{"sketchId", sid}, {"plane", {{"kind", "XY"}}}, {"entities", ents},
                    {"constraints", json::array()}};
    };
    std::vector<json> ops = {
        {{"opType", "Sketch"}, {"params", sketch("a", rect)}},
        {{"opType", "Extrude"}, {"inputs", region("a")},
         {"params", {{"sketchId", "a"}, {"distance", 25.0}, {"extrudeMode", "Blind"}, {"booleanMode", "NewBody"}}}},
        {{"opType", "Sketch"}, {"params", sketch("c", circle)}},
        {{"opType", "Extrude"}, {"inputs", region("c")},
         {"params", {{"sketchId", "c"}, {"distance", 40.0}, {"extrudeMode", "Blind"}, {"booleanMode", "Cut"},
                     {"targetBodyId", "body_op1"}}}}};
    if (with_fillet) {
        ops.push_back({{"opType", "Fillet"}, {"inputs", json::array({ref("el_f", "edge", 0, 20, 25)})},
                       {"params", {{"mode", "Fillet"}, {"radius", 3.0}, {"edgeIds", {"el_f"}},
                                   {"chainTangentEdges", true}, {"tangentClosureVersion", 1},
                                   {"targetBodyId", "body_op1"}}}});
    }
    ops.push_back({{"opType", "Shell"}, {"inputs", json::array({ref("el_b", "face", -10, 2, 0)})},
                   {"params", {{"thickness", T}, {"targetBodyId", "body_op1"}, {"openFaces", {"el_b"}}}}});
    json wire = json::array(), hashes = json::array();
    for (std::size_t i = 0; i < ops.size(); ++i) {
        json op = ops[i];
        op["opId"] = "op" + std::to_string(i);
        op["stepIndex"] = i;
        wire.push_back(op);
        hashes.push_back("h" + std::to_string(i));
    }
    onecad::session::Session s;
    s.open("doc", 0, 3, "determinism");
    onecad::CancelToken tok;
    onecad::protocol::HandlerContext ctx{tok, [](int) {}, [](onecad::protocol::Envelope&) {}};
    const json args = {{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3},
                       {"expectedBaseHash", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"},
                       {"prefixHashes", hashes}, {"targetStep", ops.size() - 1}, {"ops", wire}};
    const auto r = onecad::session::handle_execute_plan(
        s, onecad::protocol::Envelope::request(1, "ExecutePlan", args), ctx);
    onecad::session::handle_accept_prepared(
        s, onecad::protocol::Envelope::request(1, "AcceptPrepared",
                                               json{{"jobId", 1}, {"documentRevision", 0}, {"workerEpoch", 3}}));
    PlanResult out;
    out.stopped = r.result.value("stoppedReason", "?");
    if (r.result.contains("perStepResults") && !r.result["perStepResults"].empty()) {
        out.last_step = r.result["perStepResults"].back();
    }
    const BodyStore bodies = s.bodies_copy();
    if (const auto* rec = bodies.get("body_op1")) out.box_volume = vol(rec->geom);
    return out;
}

void test_through_hole_limitation() {
    const double holed = L * W * H - kPi * 36 * H;
    const PlanResult plain = p9_plan(false);
    if (plain.stopped == "completed") {
        const double analytic = holed - ((L - 2 * T) * (W - 2 * T) * (H - T) - kPi * 64 * (H - T));
        check(std::abs(plain.box_volume - analytic) < 1e-6 * analytic, "through hole: a published shell is the analytic one");
    } else {
        check(plain.stopped == "opFailed", "through hole: honest recoverable refusal");
        check(std::abs(plain.box_volume - holed) < 1e-6, "through hole: the holed predecessor is intact");
    }

    const PlanResult filleted = p9_plan(true);
    check(filleted.stopped == "opFailed", "hole + fillet: refused on this kernel");
    const json diags = filleted.last_step.value("diagnostics", json::array());
    check(!diags.empty() && diags[0].value("code", "") == "SHELL_CONSTRUCTION_FAILED",
          "hole + fillet: the kernel construction failure carries SHELL_CONSTRUCTION_FAILED (got " +
              filleted.last_step.dump().substr(0, 300) + ")");
}
}  // namespace

int main() {
    try {
        test_multi_opening();
        test_through_hole_limitation();
    } catch (const std::exception& e) {
        std::fprintf(stderr, "FAIL: exception %s\n", e.what());
        ++g_failures;
    }
    if (g_failures == 0) std::fprintf(stderr, "test_shell_openings: all checks passed\n");
    return g_failures == 0 ? 0 : 1;
}
