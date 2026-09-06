// test_executor_hazards.cpp — WP-H H0 probes (e) and (f) for `PlanExecutor`.
//
//   (e) LATE CANCEL — the cancel token is polled only at step iteration
//       (`PlanExecutor.cpp` execute_ops) and never re-checked before
//       `session.store_prepared(...)`. A cancel that lands after the LAST step's
//       `planStep` therefore strands a scratch: the terminal resp says
//       PlanPrepared and `hasScratch` stays true, so the next ExecutePlan with a
//       different jobId is refused until DiscardPrepared/restart. The cancel is
//       injected from the `HandlerContext::emit` callback — the production
//       emit-point for the last planStep — so no production code is touched.
//
//   (f) TESSELLATION ARTIFACT — `attach_tessellate` runs outside any try/catch, so
//       a throwing `tess::tessellate_body` (a pathological body in the base the plan
//       inherited) propagates out of `handle_execute_plan`. An artifact failure must
//       be a `warning` diagnostic on an Ok plan, never a thrown/failed plan.
//
// No test framework: exit code == failure count.
#include <cstdio>
#include <exception>
#include <limits>
#include <string>
#include <vector>

#include <BRep_Builder.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <Standard_Failure.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Face.hxx>

#include "nlohmann/json.hpp"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "session/PlanExecutor.h"
#include "session/ScratchJob.h"
#include "session/Session.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::CancelToken;
using onecad::protocol::Envelope;
using onecad::protocol::HandlerContext;
using onecad::session::ScratchJob;
using onecad::session::Session;

namespace {
int g_failures = 0;
constexpr const char* kEmpty =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#define CHECK(cond, ...)                                                          \
    do {                                                                          \
        if (!(cond)) {                                                            \
            std::fprintf(stderr, "FAIL %s:%d: %s | ", __FILE__, __LINE__, #cond); \
            std::fprintf(stderr, __VA_ARGS__);                                    \
            std::fprintf(stderr, "\n");                                           \
            ++g_failures;                                                         \
        }                                                                         \
    } while (0)

json line_ent(const std::string& id, double x0, double y0, double x1, double y1) {
    return json{{"id", id},
                {"type", "Line"},
                {"p0", json::array({x0, y0})},
                {"p1", json::array({x1, y1})}};
}

json rect_sketch(const std::string& op_id, const std::string& sid, double w, double h) {
    json params;
    params["sketchId"] = sid;
    params["plane"] = json{{"kind", "XY"}};
    params["entities"] = json::array({line_ent("e1", 0, 0, w, 0), line_ent("e2", w, 0, w, h),
                                      line_ent("e3", w, h, 0, h), line_ent("e4", 0, h, 0, 0)});
    params["constraints"] = json::array();
    return json{{"opType", "Sketch"}, {"opId", op_id}, {"stepIndex", 0}, {"params", params}};
}

json extrude_op(const std::string& op_id, const std::string& sid, std::uint64_t step, double d) {
    return json{{"opType", "Extrude"},
                {"opId", op_id},
                {"stepIndex", step},
                {"params",
                 {{"sketchId", sid},
                  {"distance", d},
                  {"extrudeMode", "Blind"},
                  {"booleanMode", "NewBody"}}}};
}

// ─────────────────────────────────────────────────────────────────────────────
// (e) a cancel that lands after the LAST planStep must not strand a scratch
// ─────────────────────────────────────────────────────────────────────────────
void test_late_cancel_does_not_strand_scratch() {
    Session s;
    s.open("doc", 0, 3, "determinism");

    const json ops =
        json::array({rect_sketch("op0", "sk1", 10, 10), extrude_op("op1", "sk1", 1, 20.0)});

    CancelToken tok;
    std::size_t emitted = 0;
    // Cancel the moment the LAST step's planStep is emitted — after the loop's only
    // cancel poll for that iteration, before the terminal is built.
    HandlerContext ctx{tok, [](int) {}, [&](Envelope&) {
                           ++emitted;
                           if (emitted == 2) tok.cancel();
                       }};

    json args = {{"jobId", 1},
                 {"documentRevision", 0},
                 {"workerEpoch", 3},
                 {"expectedBaseHash", kEmpty},
                 {"prefixHashes", json::array({"h0", "h1"})},
                 {"targetStep", ops.size()},
                 {"ops", ops}};
    Envelope resp = onecad::session::handle_execute_plan(
        s, Envelope::request(1, "ExecutePlan", args), ctx);

    const bool ok = resp.ok.value_or(false);
    const std::string code = resp.error.has_value() ? resp.error->code : std::string("<none>");
    const bool scratch = s.has_scratch();
    std::fprintf(stderr,
                 "late-cancel: planSteps emitted=%zu, resp ok=%d code=%s, hasScratch=%d\n",
                 emitted, ok ? 1 : 0, code.c_str(), scratch ? 1 : 0);

    CHECK(emitted == 2, "both steps must emit a planStep (got %zu)", emitted);
    // THE PROBE: a cancel observed before the scratch is installed must leave the
    // session intact (SCHEMA §8 CANCELLED: "the session is untouched").
    CHECK(!scratch,
          "a cancel that landed before store_prepared left hasScratch=%d — the scratch is "
          "stranded and the next ExecutePlan is refused until DiscardPrepared",
          scratch ? 1 : 0);
    CHECK(!ok && code == "CANCELLED",
          "a plan cancelled before the scratch was stored must terminate CANCELLED, got ok=%d "
          "code=%s",
          ok ? 1 : 0, code.c_str());
}

// ─────────────────────────────────────────────────────────────────────────────
// (f) a throwing tessellation artifact must be a warning, not a thrown plan
// ─────────────────────────────────────────────────────────────────────────────

// Two pathological bodies, tried in order:
//   * an UNBOUNDED planar face (`BRepBuilderAPI_MakeFace` from a bare `gp_Pln`) —
//     an infinite surface has no finite bbox, so `BRepMesh_IncrementalMesh` has
//     nothing to discretize;
//   * a face with no underlying surface at all, wrapped in a compound so the
//     top-level shape is not itself null (`tessellate_body` returns early on a
//     null shape).
TopoDS_Shape unbounded_face_shape() {
    return BRepBuilderAPI_MakeFace(gp_Pln(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1))).Face();
}

TopoDS_Shape surfaceless_face_shape() {
    BRep_Builder b;
    TopoDS_Face empty_face;
    b.MakeFace(empty_face);
    TopoDS_Compound comp;
    b.MakeCompound(comp);
    b.Add(comp, empty_face);
    return comp;
}

// Returns true when this body actually made the tessellator throw (i.e. the probe
// was meaningful and is recorded RED).
bool test_attach_tessellate_failure_is_a_warning(const char* label,
                                                 const TopoDS_Shape& bad) {
    Session s;
    s.open("doc", 0, 3, "determinism");

    // Seat the pathological body in the HEAD so an incremental plan inherits it.
    ScratchJob seed;
    seed.job_id = 7;
    seed.plan_document_revision = 0;
    seed.bodies.create("body_bad", "op_bad", bad);
    seed.history_prefix_hash = "h_head";
    seed.prepared_snapshot_id = 1;
    s.store_prepared(std::move(seed));
    onecad::session::AcceptOutcome acc = s.accept_prepared(7, 0, 3);
    CHECK(acc.ok, "seeding the head with the pathological body must accept");

    CancelToken tok;
    HandlerContext ctx{tok, [](int) {}, [](Envelope&) {}};
    json args = {{"jobId", 8},
                 {"documentRevision", 0},
                 {"workerEpoch", 3},
                 {"expectedBaseHash", "h_head"},  // incremental: clones the head
                 {"prefixHashes", json::array()},
                 {"targetStep", 0},
                 {"ops", json::array()},
                 {"artifacts", {{"tessellate", {{"lod", "coarse"}, {"includeEdges", true}}}}}};

    bool threw = false;
    std::string what;
    Envelope resp;
    try {
        resp = onecad::session::handle_execute_plan(
            s, Envelope::request(8, "ExecutePlan", args), ctx);
    } catch (const Standard_Failure&) {
        threw = true;
        what = "Standard_Failure (OCCT)";
    } catch (const std::exception& ex) {
        threw = true;
        what = std::string("std::exception: ") + ex.what();
    } catch (...) {
        threw = true;
        what = "unknown exception";
    }

    if (!threw) {
        std::fprintf(stderr,
                     "attach-tessellate[%s]: NOT REPRODUCIBLE — this body did not make "
                     "tess::tessellate_body throw (resp ok=%d, stoppedReason=%s).\n",
                     label, resp.ok.value_or(false) ? 1 : 0,
                     resp.result.value("stoppedReason", std::string("<none>")).c_str());
        return false;
    }

    std::fprintf(stderr, "attach-tessellate[%s]: threw out of handle_execute_plan — %s\n", label,
                 what.c_str());
    // THE PROBE: an artifact failure is advisory. The plan is prepared and the
    // failure rides as a diagnostic; it never escapes the handler.
    CHECK(!threw,
          "attach_tessellate propagated an exception out of handle_execute_plan (%s) — the "
          "whole plan fails instead of preparing with a warning diagnostic",
          what.c_str());
    return true;
}

// (f, continued) — WP-H: the ARTIFACT injection point. `tessellate_body` returns
// ok=false rather than throwing on every pathological body above, so the guard
// around `attach_tessellate` is exercised through the documented in-band hook
// `artifacts.tessellate.__testThrow` (house style, cf. the `__crash` / `__slow` /
// `__fail` op ids). This case WAS red before the guard landed: the forced throw
// propagated straight out of `handle_execute_plan`.
void test_attach_tessellate_hook_is_a_warning() {
    Session s;
    s.open("doc", 0, 3, "determinism");

    const json ops =
        json::array({rect_sketch("op0", "sk1", 10, 10), extrude_op("op1", "sk1", 1, 20.0)});
    CancelToken tok;
    HandlerContext ctx{tok, [](int) {}, [](Envelope&) {}};
    json args = {{"jobId", 21},
                 {"documentRevision", 0},
                 {"workerEpoch", 3},
                 {"expectedBaseHash", kEmpty},
                 {"prefixHashes", json::array({"h0", "h1"})},
                 {"targetStep", ops.size()},
                 {"ops", ops},
                 {"artifacts",
                  {{"tessellate", {{"lod", "coarse"}, {"includeEdges", true}, {"__testThrow", true}}}}}};

    bool threw = false;
    std::string what;
    Envelope resp;
    try {
        resp = onecad::session::handle_execute_plan(
            s, Envelope::request(21, "ExecutePlan", args), ctx);
    } catch (const std::exception& ex) {
        threw = true;
        what = ex.what();
    } catch (...) {
        threw = true;
        what = "unknown exception";
    }
    CHECK(!threw, "a throwing tessellate artifact escaped handle_execute_plan (%s)", what.c_str());
    if (threw) return;

    const json result = resp.result;
    std::fprintf(stderr, "attach-tessellate[hook]: ok=%d result=%s bin=%zu sections=%zu\n",
                 resp.ok.value_or(false) ? 1 : 0, result.dump().c_str(), resp.out_bin.size(),
                 resp.bin.size());
    CHECK(resp.ok.value_or(false) && result.value("planPrepared", false),
          "the plan must still PREPARE when only the artifact failed");
    CHECK(s.has_scratch(), "the scratch must be stored");
    CHECK(!result.contains("artifacts"), "no artifact reference when the attachment failed");
    CHECK(resp.out_bin.empty() && resp.bin.empty(),
          "the partially built tail must be dropped with the artifact (got %zu bytes / %zu sections)",
          resp.out_bin.size(), resp.bin.size());
    const json per_step = result.value("perStepResults", json::array());
    bool warned = false;
    if (!per_step.empty()) {
        for (const json& d : per_step.back().value("diagnostics", json::array())) {
            warned = warned || (d.value("code", std::string{}) == "ARTIFACT_TESSELLATE_FAILED" &&
                                d.value("severity", std::string{}) == "warning" &&
                                d.value("stage", std::string{}) == "artifact" &&
                                !d.value("message", std::string{}).empty());
        }
    }
    CHECK(warned, "the LAST step must carry the ARTIFACT_TESSELLATE_FAILED warning, got %s",
          per_step.dump().c_str());
}

}  // namespace

int main() {
    test_late_cancel_does_not_strand_scratch();
    test_attach_tessellate_hook_is_a_warning();
    const bool a = test_attach_tessellate_failure_is_a_warning("unbounded-face",
                                                              unbounded_face_shape());
    const bool b = test_attach_tessellate_failure_is_a_warning("surfaceless-face",
                                                              surfaceless_face_shape());
    if (!a && !b) {
        std::fprintf(stderr,
                     "attach-tessellate: UNPROBED — no pathological body available to this test "
                     "makes tess::tessellate_body throw, so the missing try/catch in "
                     "attach_tessellate cannot be shown RED without a production hook.\n");
    }
    if (g_failures == 0) std::fprintf(stderr, "executor_hazards PASS\n");
    return g_failures;
}
