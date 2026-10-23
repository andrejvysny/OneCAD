// test_preview_plan.cpp — ONECAD-369 `PreviewPlan` (SCHEMA §7.6): a full-replay
// edit preview that publishes nothing.
//
// The load-bearing claim is INVISIBILITY: after any terminal — ok, error,
// cancelled — the head bodies, `historyPrefixHash`, `snapshotId`,
// `documentRevision`, `workerEpoch`, `hasScratch` and `hasRestoredBase` are
// unchanged, and a held prepared scratch still accepts. It is asserted after
// every preview here. Also pinned: the read-only fence (STALE_PREVIEW on snapshot
// AND epoch), the mesh scope (changed/new bodies only), the per-step signatures
// equal a from-0 ExecutePlan of the same ops, cooperative cancel, the
// `baseCheckpoint` refusal and the shared mesh budget's structured refusal.
//
// No test framework: exit code == failure count.
#include <chrono>
#include <cmath>
#include <cstdio>
#include <string>
#include <thread>
#include <vector>

#include "io/Checkpoint.h"
#include "nlohmann/json.hpp"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "protocol/Limits.h"
#include "session/PlanExecutor.h"
#include "session/PreviewMesh.h"
#include "session/PreviewPlan.h"
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include "elementmap/ElementMapPartition.h"
#include "session/Session.h"
#include "session/Tessellate.h"
#include "tess/Tessellate.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::CancelToken;
using onecad::protocol::Envelope;
using onecad::protocol::HandlerContext;
using onecad::session::Session;
using onecad::session::WorkerHead;

namespace {
int g_failures = 0;

#define CHECK(cond, ...)                                                          \
    do {                                                                          \
        if (!(cond)) {                                                            \
            std::fprintf(stderr, "FAIL %s:%d: %s | ", __FILE__, __LINE__, #cond); \
            std::fprintf(stderr, __VA_ARGS__);                                    \
            std::fprintf(stderr, "\n");                                           \
            ++g_failures;                                                         \
        }                                                                         \
    } while (0)

constexpr const char* kEmpty =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
constexpr std::uint64_t kEpoch = 3;

json rect(double x0, double w, double h) {
    return json::array(
        {{{"id", "e1"}, {"type", "Line"}, {"p0", {x0, 0}}, {"p1", {x0 + w, 0}}},
         {{"id", "e2"}, {"type", "Line"}, {"p0", {x0 + w, 0}}, {"p1", {x0 + w, h}}},
         {{"id", "e3"}, {"type", "Line"}, {"p0", {x0 + w, h}}, {"p1", {x0, h}}},
         {{"id", "e4"}, {"type", "Line"}, {"p0", {x0, h}}, {"p1", {x0, 0}}}});
}

json sketch_op(const std::string& op_id, const std::string& sid, std::uint64_t step, double x0) {
    return json{{"opType", "Sketch"},
                {"opId", op_id},
                {"stepIndex", step},
                {"params",
                 {{"sketchId", sid}, {"plane", {{"kind", "XY"}}}, {"entities", rect(x0, 20, 10)},
                  {"constraints", json::array()}}}};
}

json extrude_op(const std::string& op_id, const std::string& sid, std::uint64_t step, double d) {
    return json{{"opType", "Extrude"},
                {"opId", op_id},
                {"stepIndex", step},
                {"params",
                 {{"sketchId", sid}, {"distance", d}, {"extrudeMode", "Blind"},
                  {"booleanMode", "NewBody"}}}};
}

// Two independent parts: A (steps 0,1) and B (steps 2,3).
json two_parts(double a_depth, const std::string& a_extrude_id = "opA") {
    return json::array({sketch_op("opSA", "skA", 0, 0), extrude_op(a_extrude_id, "skA", 1, a_depth),
                        sketch_op("opSB", "skB", 2, 60), extrude_op("opB", "skB", 3, 8)});
}

struct Captured {
    std::vector<json> steps;
};

Envelope run_plan(Session& s, std::uint64_t job_id, const json& ops, Captured* cap = nullptr) {
    CancelToken tok;
    HandlerContext ctx{tok, [](int) {}, [cap](Envelope& ev) {
                           if (cap) cap->steps.push_back(ev.result);
                       }};
    json args = {{"jobId", job_id},
                 {"documentRevision", 1},
                 {"workerEpoch", kEpoch},
                 {"expectedBaseHash", kEmpty},
                 {"prefixHashes", json::array({"h0", "h1", "h2", "h3"})},
                 {"targetStep", ops.size() - 1},
                 {"ops", ops}};
    return onecad::session::handle_execute_plan(s, Envelope::request(job_id, "ExecutePlan", args),
                                                ctx);
}

void accept(Session& s, std::uint64_t job_id) {
    const Envelope a = onecad::session::handle_accept_prepared(
        s, Envelope::request(job_id, "AcceptPrepared",
                             json{{"jobId", job_id}, {"documentRevision", 1}, {"workerEpoch", kEpoch}}));
    CHECK(a.ok.value_or(false), "AcceptPrepared(%llu) failed",
          static_cast<unsigned long long>(job_id));
}

json preview_args(const json& ops, std::uint64_t snapshot, std::uint64_t epoch = kEpoch) {
    return json{{"jobId", 900},          {"workerEpoch", epoch}, {"expectedSnapshotId", snapshot},
                {"editedFrom", 1},       {"targetStep", ops.size() - 1},
                {"lod", "coarse"},       {"ops", ops}};
}

Envelope preview(Session& s, const json& args, CancelToken& tok, Captured* cap = nullptr) {
    HandlerContext ctx{tok, [](int) {}, [cap](Envelope& ev) {
                           if (cap) cap->steps.push_back(ev.result);
                       }};
    return onecad::session::handle_preview_plan(s, Envelope::request(77, "PreviewPlan", args), ctx);
}

Envelope preview(Session& s, const json& args, Captured* cap = nullptr) {
    CancelToken tok;
    return preview(s, args, tok, cap);
}

bool same_head(const WorkerHead& a, const WorkerHead& b) {
    return a.document_revision == b.document_revision && a.worker_epoch == b.worker_epoch &&
           a.snapshot_id == b.snapshot_id && a.history_prefix_hash == b.history_prefix_hash &&
           a.has_scratch == b.has_scratch && a.has_restored_base == b.has_restored_base;
}

std::string code_of(const Envelope& r) { return r.error.has_value() ? r.error->code : "<ok>"; }

std::vector<std::string> meshed(const Envelope& r) {
    std::vector<std::string> out;
    for (const json& m : r.result.value("meshes", json::array())) out.push_back(m["bodyId"]);
    return out;
}

void edit_meshes_only_changed_bodies(Session& s) {
    const WorkerHead before = s.head();
    const std::string sig_before = onecad::session::geometry_signature(s.bodies_copy());
    const Envelope r = preview(s, preview_args(two_parts(5), before.snapshot_id));
    CHECK(r.ok.value_or(false), "preview failed: %s", code_of(r).c_str());
    CHECK(r.result.value("planPreviewed", false), "planPreviewed");
    CHECK(r.result.value("snapshotId", std::uint64_t{0}) == before.snapshot_id, "head echo");
    CHECK(r.result.value("stoppedReason", std::string{}) == "completed", "completed");
    CHECK((r.result["bodies"] == json::array({"body_opA", "body_opB"})), "bodies %s",
          r.result["bodies"].dump().c_str());
    CHECK((meshed(r) == std::vector<std::string>{"body_opA"}),
          "only the edited part is meshed: %s", r.result["meshes"].dump().c_str());
    CHECK(same_head(before, s.head()), "head moved after an ok preview");
    CHECK(onecad::session::geometry_signature(s.bodies_copy()) == sig_before, "head bodies moved");

    // An unchanged candidate meshes nothing at all — even after the head was
    // tessellated (a commit's inline artifact does exactly that), which caches a
    // triangulation on the head shapes and marks their faces `Checked`.
    onecad::session::handle_tessellate(
        s, Envelope::request(5, "Tessellate",
                             json{{"bodyIds", "all"}, {"lod", "fine"}, {"includeEdges", true}}));
    const Envelope same = preview(s, preview_args(two_parts(10), before.snapshot_id));
    CHECK(same.ok.value_or(false) && meshed(same).empty(), "unchanged replay meshed %s",
          same.result["meshes"].dump().c_str());
}

void signatures_equal_execute_plan(Session& s) {
    Captured via_preview;
    preview(s, preview_args(two_parts(5), s.head().snapshot_id), &via_preview);
    Session other;
    other.open("doc2", 0, kEpoch, "determinism");
    Captured via_plan;
    run_plan(other, 50, two_parts(5), &via_plan);
    CHECK(via_preview.steps.size() == 4 && via_plan.steps.size() == 4, "step counts %zu/%zu",
          via_preview.steps.size(), via_plan.steps.size());
    for (std::size_t i = 0; i < via_plan.steps.size() && i < via_preview.steps.size(); ++i) {
        CHECK(via_preview.steps[i]["signatures"] == via_plan.steps[i]["signatures"],
              "step %zu signatures differ", i);
        CHECK(via_preview.steps[i]["bodyEvents"] == via_plan.steps[i]["bodyEvents"],
              "step %zu bodyEvents differ", i);
    }
}

void operand_context_reads_only_the_prefix(Session& s) {
    const WorkerHead before = s.head();
    const auto bodies = s.bodies_copy();
    const auto* body = bodies.get("body_opA");
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(body->geom, TopAbs_FACE, faces);
    const auto descriptor = onecad::elementmap::ElementMapPartition::describe(faces(1), body->geom);
    const json input{{"primary", {{"bodyId", "body_opA"}, {"elementId", "el_read"}, {"kind", "face"}}},
                     {"intent", {{"version", 4}, {"kind", "face"},
                                 {"descriptor", onecad::elementmap::ElementMapPartition::descriptor_to_json(descriptor)}}},
                     {"anchor", {{"worldPoint", {descriptor.center.X(), descriptor.center.Y(), descriptor.center.Z()}}}}};
    json args = preview_args(json::array({sketch_op("opSA", "skA", 0, 0), extrude_op("opA", "skA", 1, 10)}), before.snapshot_id);
    args.erase("editedFrom");
    args["editOperandContext"] = {{"opId", "readShell"}, {"opType", "Shell"},
        {"inputs", json::array({input})}, {"params", {{"targetBodyId", "body_opA"}, {"openFaces", {"el_read"}}, {"thickness", 1}}}};
    const Envelope result = preview(s, args);
    CHECK(result.ok.value_or(false), "prefix operand query failed: %s", code_of(result).c_str());
    if (result.ok.value_or(false)) {
        const json context = result.result["editOperandContext"];
        CHECK(context["eligibleBodies"] == json::array({"body_opA"}), "downstream body leaked into prefix");
        CHECK(context["inputs"].size() == 1 && !context["inputs"][0].contains("refusal"), "operand unresolved: %s", context.dump().c_str());
        CHECK(context["inputs"][0]["classification"]["surfaceType"] == "plane", "analytic surface missing");
        CHECK(result.result["meshes"].empty() && result.out_bin.empty(), "context query must not mesh");
    }
    json too_many = args;
    too_many["editOperandContext"]["inputs"] = json::array();
    for (int i = 0; i < 65; ++i) too_many["editOperandContext"]["inputs"].push_back(input);
    CHECK(code_of(preview(s, too_many)) == "PROTOCOL_ERROR", "65 inputs must fail before replay");
    json self = args;
    self["editOperandContext"]["opId"] = "opA";
    CHECK(code_of(preview(s, self)) == "PROTOCOL_ERROR", "queried feature must not execute in prefix");
    json body_query = args;
    body_query["editOperandContext"] = {{"opId", "readBody"}, {"opType", "TransformBody"},
        {"params", {{"targets", {"body_opA"}}}},
        {"inputs", json::array({{{"primary", {{"bodyId", "body_opA"},
            {"elementId", "body_opA"}, {"kind", "body"}}}}})}};
    const auto body_result = preview(s, body_query);
    CHECK(body_result.ok.value_or(false), "body context failed");
    if (body_result.ok.value_or(false)) {
        const auto measured = body_result.result["editOperandContext"]["inputs"][0];
        CHECK(measured.contains("bounds") && std::abs(measured["bounds"]["max"][2].get<double>() - 10.0) < 1e-5,
            "body bounds must come from replayed prefix, not final head");
        CHECK(measured.contains("bounds") && !measured.contains("point") &&
            !measured.contains("direction") && !measured.contains("classification"),
            "body requires bounds without fabricated analytic geometry: %s", measured.dump().c_str());
    }
    json empty = args;
    empty["ops"] = json::array();
    empty["editOperandContext"] = {{"opId", "firstFeature"}, {"opType", "Sketch"},
        {"params", json::object()}, {"inputs", json::array()}};
    const auto first = preview(s, empty);
    CHECK(first.ok.value_or(false) && first.result["bodies"].empty() &&
        first.result["editOperandContext"]["inputs"].empty(), "k0 no-input context failed");
    json malformed = args;
    malformed["editOperandContext"]["inputs"][0]["primary"]["kind"] = 9;
    CHECK(code_of(preview(s, malformed)) == "PROTOCOL_ERROR", "malformed primary must fail before replay");
    malformed = args;
    malformed["editedFrom"] = 0;
    CHECK(code_of(preview(s, malformed)) == "PROTOCOL_ERROR", "context must not claim edited operation execution");
    CHECK(same_head(before, s.head()), "operand read changed worker head");
}

void fence_refusals(Session& s) {
    const WorkerHead before = s.head();
    const Envelope stale = preview(s, preview_args(two_parts(5), before.snapshot_id + 9));
    CHECK(code_of(stale) == "STALE_PREVIEW", "snapshot mismatch: %s", code_of(stale).c_str());
    const Envelope epoch = preview(s, preview_args(two_parts(5), before.snapshot_id, kEpoch + 1));
    CHECK(code_of(epoch) == "STALE_PREVIEW", "epoch mismatch: %s", code_of(epoch).c_str());
    json with_base = preview_args(two_parts(5), before.snapshot_id);
    with_base["baseCheckpoint"] = json{{"stepIndex", 1}, {"checkpointId", "x"}};
    CHECK(code_of(preview(s, with_base)) == "PROTOCOL_ERROR", "baseCheckpoint must be refused");
    json no_snapshot = preview_args(two_parts(5), before.snapshot_id);
    no_snapshot.erase("expectedSnapshotId");
    CHECK(code_of(preview(s, no_snapshot)) == "PROTOCOL_ERROR", "expectedSnapshotId required");
    CHECK(same_head(before, s.head()), "head moved after refusals");
}

void edited_failure_stops_at_k(Session& s) {
    const WorkerHead before = s.head();
    const Envelope r = preview(s, preview_args(two_parts(5, "opA__fail"), before.snapshot_id));
    CHECK(r.ok.value_or(false), "a failed step is a successful preview terminal");
    CHECK(r.result.value("stoppedReason", std::string{}) == "opFailed", "opFailed");
    CHECK(r.result["lastValidStep"] == json(0), "lastValidStep %s",
          r.result["lastValidStep"].dump().c_str());
    CHECK(r.result["perStepResults"].back().value("stepIndex", 99) == 1, "halt at step 1");
    CHECK(same_head(before, s.head()), "head moved after a failed preview");
}

void cancel_mid_flight(Session& s) {
    const WorkerHead before = s.head();
    json ops = two_parts(5);
    ops[1]["opId"] = "opA__slow";
    CancelToken tok;
    std::thread canceller([&tok] {
        std::this_thread::sleep_for(std::chrono::milliseconds(60));
        tok.cancel();
    });
    const Envelope r = preview(s, preview_args(ops, before.snapshot_id), tok);
    canceller.join();
    CHECK(code_of(r) == "CANCELLED", "cancel: %s", code_of(r).c_str());
    CHECK(same_head(before, s.head()), "head moved after a cancelled preview");
}

void held_scratch_and_restored_base_survive(Session& s) {
    const Envelope save = onecad::io::handle_save_checkpoint(
        s, Envelope::request(5, "SaveCheckpoint", json{{"stepIndex", 3}}));
    CHECK(save.ok.value_or(false), "SaveCheckpoint");
    const Envelope restore = onecad::io::handle_restore_checkpoint(
        s, Envelope::request(6, "RestoreCheckpoint",
                             json{{"stepIndex", 3},
                                  {"checkpointId", "ck"},
                                  {"expectedHistoryPrefixHash",
                                   save.result.value("historyPrefixHash", std::string{})},
                                  {"workerEpoch", kEpoch}}));
    CHECK(s.head().has_restored_base, "precondition: restored base parked");
    WorkerHead before = s.head();
    CHECK(preview(s, preview_args(two_parts(5), before.snapshot_id)).ok.value_or(false), "ok");
    CHECK(same_head(before, s.head()), "restored base disturbed");

    const Envelope held = run_plan(s, 60, two_parts(12));
    CHECK(held.ok.value_or(false), "held plan prepared");
    before = s.head();
    CHECK(before.has_scratch, "precondition: scratch held");
    CHECK(preview(s, preview_args(two_parts(5), before.snapshot_id)).ok.value_or(false), "ok");
    CHECK(same_head(before, s.head()), "held scratch disturbed");
    accept(s, 60);
}

void mesh_budget_refuses_with_evidence(Session& s) {
    const onecad::session::BodyStore bodies = s.bodies_copy();
    const onecad::session::BodyRecord* body = bodies.get("body_opA");
    CHECK(body != nullptr, "body_opA");
    if (body == nullptr) return;
    const onecad::elementmap::ElementMapPartition partition;
    const auto size_at = [&](const char* lod) {
        return onecad::tess::tessellate_body(body->geom, "body_opA", lod, true, &partition,
                                             &body->face_colors)
            .blob.size();
    };
    // Coarse only: OCCT keeps a finer cached triangulation over a coarser request,
    // so meshing `fine` first would make every later size read the fine one.
    const std::uint64_t coarse = size_at("coarse");
    Envelope resp = Envelope::ok_response(1);
    json meshes = json::array();
    resp.out_bin.assign(onecad::protocol::kInitialBulkCredit - coarse, 0);
    const auto fits = onecad::session::append_preview_mesh(
        "PreviewPlan", "body_opA", *body, "coarse", 1, partition, resp, meshes);
    CHECK(!fits && meshes.size() == 1 && meshes[0]["lod"] == "coarse",
          "an exact fit must be accepted: %s", meshes.dump().c_str());
    resp.out_bin.assign(onecad::protocol::kInitialBulkCredit - coarse + 1, 0);
    resp.bin.clear();
    meshes = json::array();
    const auto refused = onecad::session::append_preview_mesh(
        "PreviewPlan", "body_opA", *body, "coarse", 1, partition, resp, meshes);
    CHECK(refused && refused->kind == onecad::session::PreviewMeshFailure::Kind::Budget &&
              refused->bytes == coarse && refused->limit == coarse - 1,
          "over budget at coarse must refuse with evidence");
}

}  // namespace

int main() {
    Session s;
    s.open("doc", 0, kEpoch, "determinism");
    const Envelope seed = run_plan(s, 1, two_parts(10));
    CHECK(seed.ok.value_or(false), "seed plan prepared");
    accept(s, 1);

    edit_meshes_only_changed_bodies(s);
    signatures_equal_execute_plan(s);
    fence_refusals(s);
    operand_context_reads_only_the_prefix(s);
    edited_failure_stops_at_k(s);
    cancel_mid_flight(s);
    held_scratch_and_restored_base_survive(s);
    mesh_budget_refuses_with_evidence(s);

    std::fprintf(stderr, "test_preview_plan: %d failure(s)\n", g_failures);
    return g_failures;
}
