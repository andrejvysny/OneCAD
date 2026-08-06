// PlanExecutor.cpp — see PlanExecutor.h. REAL OCCT ops (W-WP5).
#include "session/PlanExecutor.h"

#include <chrono>
#include <cstdint>
#include <cstdlib>  // std::abort
#include <optional>
#include <string>
#include <thread>
#include <vector>

#include <map>
#include <utility>

#include "elementmap/ElementMapPartition.h"
#include "elementmap/Ladder.h"
#include "ops/BooleanOp.h"
#include "ops/ExtrudeOp.h"
#include "ops/FilletChamferOp.h"
#include "ops/HoleOp.h"
#include "ops/ImportOp.h"
#include "ops/MirrorOp.h"
#include "ops/OffsetFaceOp.h"
#include "ops/TransformOp.h"
#include "ops/OpTypes.h"
#include "ops/PatternOp.h"
#include "ops/RevolveOp.h"
#include "ops/ShellOp.h"
#include "session/Signatures.h"
#include "tess/MeshHandle.h"
#include "tess/Tessellate.h"
#include "util/Hashing.h"
#include "util/Log.h"

namespace onecad::session {

using nlohmann::json;
using protocol::Envelope;
using protocol::ErrorInfo;
using protocol::HandlerContext;
namespace em = onecad::elementmap;

namespace {

std::uint64_t read_u64(const json& o, const char* key) {
    if (o.is_object() && o.contains(key) && o[key].is_number()) return o[key].get<std::uint64_t>();
    return 0;
}

std::string get_str(const json& o, const char* key, const std::string& dflt = "") {
    if (o.is_object() && o.contains(key) && o[key].is_string()) return o[key].get<std::string>();
    return dflt;
}

// (refId → ElementId) bindings a step resolved for its inputs (referencedBinding
// signature). refId = "<opId>.input<i>", elementId echoed from the ref.
std::vector<RefBinding> collect_ref_bindings(const json& op, const std::string& op_id) {
    std::vector<RefBinding> out;
    if (!op.contains("inputs") || !op["inputs"].is_array()) return out;
    std::size_t i = 0;
    for (const json& in : op["inputs"]) {
        RefBinding b;
        b.ref_id = op_id + ".input" + std::to_string(i);
        if (in.is_object() && in.contains("primary") && in["primary"].is_object()) {
            b.element_id = get_str(in["primary"], "elementId");
        }
        out.push_back(std::move(b));
        ++i;
    }
    return out;
}

// A minimal §9 NeedsRepair (STATE) for a ref whose owning body is gone.
json missing_body_repair(const em::LadderRef& ref, const std::string& body_id) {
    return json{{"refId", ref.ref_id},
                {"elementId", ref.element_id},
                {"ladderFailed", "descriptor"},
                {"reason", "no-candidates"},
                {"scoringVersion", em::kResolverVersion},
                {"candidates", json::array()},
                {"anchor", ref.anchor_json.is_null() ? json::object() : ref.anchor_json},
                {"uiLabel", "referenced body not found: " + body_id}};
}

// Resolve + mint referenced sub-element inputs through the resolution ladder
// (descriptor + anchor, SCHEMA §10 level 2). Runs BEFORE the op, on the PREDECESSOR
// snapshot (Invariant 3). A confident unique match auto-binds → `delta.added`; a ref
// that does not resolve ⇒ NeedsRepair (appended to `needs_repair`) and the caller
// stops before running the op (prepare m−1). Replaces the interim primary.topoKey
// shortcut (D3 — the field is gone). Body/region refs (no sub-element) are skipped.
void resolve_input_refs(ScratchJob& job, const json& op, const std::string& op_id,
                        const em::LadderEditContext& edit, em::ElementMapDelta& delta,
                        std::vector<json>& needs_repair) {
    if (!op.contains("inputs") || !op["inputs"].is_array()) return;

    // Group sub-element refs by owning body (assignment/scoring is per-body pool).
    std::map<std::string, std::vector<em::LadderRef>> by_body;
    std::size_t i = 0;
    for (const json& in : op["inputs"]) {
        em::LadderRef r = em::ladder_ref_from_input(in, op_id + ".input" + std::to_string(i++));
        const std::string bid = (in.is_object() && in.contains("primary") && in["primary"].is_object())
                                    ? get_str(in["primary"], "bodyId")
                                    : "";
        if (bid.empty() || r.element_id.empty()) continue;
        if (r.kind != em::km::ElementKind::Face && r.kind != em::km::ElementKind::Edge &&
            r.kind != em::km::ElementKind::Vertex) {
            continue;
        }
        if (job.partition.contains(r.element_id)) continue;  // already tracked
        by_body[bid].push_back(std::move(r));
    }

    for (auto& [bid, refs] : by_body) {
        const BodyRecord* rec = job.bodies.get(bid);
        if (!rec) {
            for (const em::LadderRef& r : refs) needs_repair.push_back(missing_body_repair(r, bid));
            continue;
        }
        const std::vector<em::LadderResolution> resolutions =
            em::resolve_descriptor_stage(rec->geom, bid, refs, edit);
        for (std::size_t k = 0; k < resolutions.size(); ++k) {
            const em::LadderResolution& res = resolutions[k];
            if (res.outcome == em::LadderOutcome::AutoBind && !res.bound_shape.IsNull()) {
                json anchor = refs[k].anchor_json;
                delta.added.push_back(job.partition.mint(bid, res.element_id, res.kind,
                                                         res.bound_shape, rec->geom, std::move(anchor)));
            } else {
                needs_repair.push_back(res.to_needs_repair_json());
            }
        }
    }
}

// A fabricated §9 NeedsRepair item (STATE, not error) for the __needsrepair hook.
json make_needs_repair(const json& op, const std::string& op_id) {
    json anchor = {{"worldPoint", {12.0, 3.5, 0.0}}, {"surfaceUv", {0.25, 0.75}}};
    std::string element_id = "el_stub";
    if (op.contains("inputs") && op["inputs"].is_array() && !op["inputs"].empty()) {
        const json& in0 = op["inputs"][0];
        if (in0.is_object()) {
            if (in0.contains("anchor") && in0["anchor"].is_object()) anchor = in0["anchor"];
            if (in0.contains("primary") && in0["primary"].is_object())
                element_id = get_str(in0["primary"], "elementId", element_id);
        }
    }
    return json{
        {"refId", op_id + ".input0"},
        {"elementId", element_id},
        {"ladderFailed", "descriptor"},
        {"reason", "ambiguous"},
        {"scoringVersion", em::kResolverVersion},
        {"candidates",
         json::array(
             {json{{"topoKey", "f:31"}, {"score", 0.91}, {"margin", 0.0}, {"worldPos", {12.0, 3.5, 0.0}},
                   {"summary", "planar face, area~120mm2"},
                   {"featureContributions",
                    {{"surfaceType", 0.2}, {"area", 0.25}, {"normal", 0.2}, {"adjacency", 0.15}, {"anchor", 0.11}}}},
              json{{"topoKey", "f:44"}, {"score", 0.91}, {"margin", 0.0}, {"worldPos", {12.0, -3.5, 0.0}},
                   {"summary", "planar face, area~120mm2"}, {"featureContributions", json::object()}}})},
        {"anchor", anchor},
        {"uiLabel", "stub repair candidate for " + op_id}};
}

json signatures_json(const BodyStore& bodies, const std::vector<BodyEvent>& events,
                     const std::vector<RefBinding>& bindings) {
    return json{{"geometry", geometry_signature(bodies)},
                {"bodyLifecycle", body_lifecycle_signature(events)},
                {"referencedBinding", referenced_binding_signature(bindings)}};
}

void emit_plan_step(HandlerContext& ctx, std::uint64_t req_id, std::uint64_t job_id,
                    std::uint64_t step_index, const std::vector<BodyEvent>& events,
                    const json& element_map_delta, const json& needs_repair, const json& signatures,
                    const json& diagnostics) {
    json body_events = json::array();
    for (const auto& e : events) {
        json be = {{"kind", e.kind}, {"bodyId", e.body_id}};
        // VF-B6: `rankKey` is emitted ONLY where the ordinal genuinely IS a geometric
        // rank. Absence = "no claim" (SCHEMA §7.2), so a step that never ranks its
        // bodies stays byte-identical to the pre-VF-B6 wire.
        if (e.rank_key) be["rankKey"] = *e.rank_key;
        body_events.push_back(std::move(be));
    }
    json payload = {
        {"stepIndex", step_index},
        {"bodyEvents", std::move(body_events)},
        {"elementMapDelta", element_map_delta},
        {"needsRepair", needs_repair},
        {"signatures", signatures},
        {"diagnostics", diagnostics},
    };
    Envelope ev = Envelope::event(req_id, "planStep", step_index, std::move(payload));
    ev.stamp.job_id = job_id;
    if (ctx.emit) ctx.emit(ev);
}

json fail_diagnostic(const std::string& code, const std::string& message) {
    return json{{"severity", "error"}, {"code", code}, {"message", message}};
}

// Determinism policy for one op: parallel flag + occtOptions (SCHEMA §7.3). Rust
// sets parallel=false in determinism mode, so reading the field satisfies
// "SetRunParallel(false) in determinism mode".
struct OpDeterminism {
    bool parallel = false;
    json occt_options = json::object();
};
OpDeterminism read_determinism(const json& op) {
    OpDeterminism d;
    if (op.contains("determinism") && op["determinism"].is_object()) {
        const json& det = op["determinism"];
        if (det.contains("parallel") && det["parallel"].is_boolean()) d.parallel = det["parallel"].get<bool>();
        if (det.contains("occtOptions") && det["occtOptions"].is_object()) d.occt_options = det["occtOptions"];
    }
    return d;
}

enum class ExecStatus { Completed, Cancelled };

struct ExecResult {
    ExecStatus status = ExecStatus::Completed;
    std::optional<std::size_t> last_ok_exec_idx;  // execution-order index for prefixHashes
};

// Dispatch one op to its real executor. Sketch materializes into the plan; Extrude
// / Boolean run OCCT; other verbs are UNSUPPORTED this WP.
ops::OpOutcome run_single_op(ScratchJob& job, const json& op, const std::string& op_id,
                             std::string& last_sketch_id, const onecad::CancelToken& cancel,
                             bool post_upstream_edit) {
    const std::string op_type = get_str(op, "opType");
    const json params = (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();

    if (op_type == "Sketch") {
        const std::string sid = get_str(params, "sketchId", "sk_" + op_id);
        job.sketches.emplace_back(sid, params);  // raw Sketch op params (profile source)
        last_sketch_id = sid;
        return ops::OpOutcome::ok();  // materializes a sketch; no body, empty delta
    }

    const OpDeterminism det = read_determinism(op);
    ops::OpContext octx{job.bodies,       &job.sketches,    job.partition, &last_sketch_id,
                        det.parallel,     det.occt_options, &cancel,       post_upstream_edit};

    if (op_type == "Extrude") return ops::execute_extrude(octx, op, op_id);
    if (op_type == "Boolean") return ops::execute_boolean(octx, op, op_id);
    if (op_type == "Revolve") return ops::execute_revolve(octx, op, op_id);
    if (op_type == "Fillet") return ops::execute_fillet(octx, op, op_id);
    if (op_type == "Chamfer") return ops::execute_chamfer(octx, op, op_id);
    if (op_type == "Shell") return ops::execute_shell(octx, op, op_id);
    if (op_type == "OffsetFace") return ops::execute_offset_face(octx, op, op_id);
    if (op_type == "Hole") return ops::execute_hole(octx, op, op_id);
    if (op_type == "LinearPattern") return ops::execute_linear_pattern(octx, op, op_id);
    if (op_type == "CircularPattern") return ops::execute_circular_pattern(octx, op, op_id);
    if (op_type == "MirrorBody") return ops::execute_mirror_body(octx, op, op_id);
    if (op_type == "ImportStep") return ops::execute_import_step(octx, op, op_id);
    if (op_type == "TransformBody") return ops::execute_transform_body(octx, op, op_id);

    // Loft / Sweep remain UNSUPPORTED (SCHEMA §8) — Rust freezes the node.
    return ops::OpOutcome::unsupported("unsupported opType: " + op_type);
}

struct CandidateSnapshot {
    BodyStore bodies;
    em::ElementMapPartition partition;
    std::size_t sketch_count = 0;
    std::string last_sketch_id;
};

CandidateSnapshot snapshot_candidate(const ScratchJob& job,
                                     const std::string& last_sketch_id) {
    return CandidateSnapshot{job.bodies, job.partition, job.sketches.size(),
                             last_sketch_id};
}

void merge_outcome(CandidateResult& result, ops::OpOutcome outcome) {
    result.error_code = std::move(outcome.error_code);
    result.error_message = std::move(outcome.error_message);
    result.body_events = std::move(outcome.body_events);
    result.body_ids = std::move(outcome.body_ids);
    for (auto& entry : outcome.delta.added) result.delta.added.push_back(std::move(entry));
    for (auto& entry : outcome.delta.relabeled) result.delta.relabeled.push_back(std::move(entry));
    for (auto& id : outcome.delta.removed) result.delta.removed.push_back(std::move(id));
    for (auto& repair : outcome.needs_repair) {
        result.needs_repair.push_back(std::move(repair));
    }
    for (auto& diag : outcome.diagnostics) result.diagnostics.push_back(std::move(diag));
    if (outcome.status == ops::OpOutcome::Status::Ok) {
        result.status = result.needs_repair.empty()
                            ? CandidateResult::Status::Ok
                            : CandidateResult::Status::NeedsRepair;
    } else if (outcome.status == ops::OpOutcome::Status::Failed) {
        result.status = CandidateResult::Status::Failed;
    } else if (outcome.status == ops::OpOutcome::Status::Unsupported) {
        result.status = CandidateResult::Status::Unsupported;
    } else {
        result.status = CandidateResult::Status::Cancelled;
    }
}

void rollback_candidate(ScratchJob& job, std::string& last_sketch_id,
                        CandidateResult& result, CandidateSnapshot snapshot) {
    job.bodies = std::move(snapshot.bodies);
    job.partition = std::move(snapshot.partition);
    job.sketches.resize(snapshot.sketch_count);
    last_sketch_id = std::move(snapshot.last_sketch_id);
    result.body_events.clear();
    result.body_ids.clear();
    result.delta = em::ElementMapDelta{};
}

}  // namespace

// Whether this op's refs resolve against geometry an UPSTREAM edit moved — the
// gate on the SCHEMA §10 descriptor-tie veto. True iff the plan carried
// `editedFrom = k` and this op's `stepIndex` is strictly greater than k.
//
// `> k`, not `>= k`: step k is the edited op itself, and its own refs were
// re-authored by the very edit that dirtied it (Rust restamps the descriptor and
// anchor on the record it just wrote), so they are FRESH, not stale. Everything
// after k inherits geometry that moved under it.
//
// A `ScratchJob` with no `edited_from` (preview, and every no-edit replay lane)
// yields false ⇒ resolverVersion-1 behaviour, unchanged.
namespace {
bool step_is_post_edit(const ScratchJob& job, const json& op) {
    if (!job.edited_from) return false;
    if (!op.contains("stepIndex") || !op["stepIndex"].is_number()) return false;
    return op["stepIndex"].get<std::uint64_t>() > *job.edited_from;
}
}  // namespace

CandidateResult execute_candidate_op(ScratchJob& job, const json& op,
                                     const std::string& op_id,
                                     std::string& last_sketch_id,
                                     const onecad::CancelToken& cancel) {
    CandidateResult result;
    result.ref_bindings = collect_ref_bindings(op, op_id);
    if (cancel.cancelled()) {
        result.status = CandidateResult::Status::Cancelled;
        return result;
    }

    CandidateSnapshot snapshot = snapshot_candidate(job, last_sketch_id);
    const bool post_edit = step_is_post_edit(job, op);

    resolve_input_refs(job, op, op_id, em::LadderEditContext{post_edit}, result.delta,
                       result.needs_repair);
    if (result.needs_repair.empty()) {
        merge_outcome(
            result, run_single_op(job, op, op_id, last_sketch_id, cancel, post_edit));
    } else {
        result.status = CandidateResult::Status::NeedsRepair;
    }

    if (result.status != CandidateResult::Status::Ok) {
        rollback_candidate(job, last_sketch_id, result, std::move(snapshot));
    }
    return result;
}

namespace {

// Drive the ordered op slice into `job`, streaming one planStep per executed step
// and stopping at the first failure / NeedsRepair (SCHEMA §7.2).
ExecResult execute_ops(ScratchJob& job, const json& ops, std::uint64_t job_id, std::uint64_t req_id,
                       HandlerContext& ctx) {
    std::string last_sketch_id;
    ExecResult res;
    std::optional<std::uint64_t> last_ok_step;
    std::size_t exec_idx = 0;

    for (const json& op : ops) {
        const std::uint64_t step_index = (op.contains("stepIndex") && op["stepIndex"].is_number())
                                             ? op["stepIndex"].get<std::uint64_t>()
                                             : exec_idx;
        const std::string op_id = get_str(op, "opId", "op_" + std::to_string(step_index));

        // --- test hooks (documented; harmless in production) ---
        if (op_id.find("__crash") != std::string::npos) {
            WLOG_ERROR("ExecutePlan: __crash hook at op '%s' — aborting", op_id.c_str());
            std::abort();  // chaos drill: no terminal frame
        }
        if (op_id.find("__slow") != std::string::npos) {
            for (int i = 0; i < 50; ++i) {  // ~500 ms in 10 ms cancellation slices
                if (ctx.cancel.cancelled()) { res.status = ExecStatus::Cancelled; return res; }
                std::this_thread::sleep_for(std::chrono::milliseconds(10));
            }
        }
        if (ctx.cancel.cancelled()) { res.status = ExecStatus::Cancelled; return res; }

        json diagnostics = json::array();
        CandidateResult candidate;

        if (op_id.find("__fail") != std::string::npos) {
            candidate.status = CandidateResult::Status::Failed;
            candidate.error_code = "STUB_FORCED_FAIL";
            candidate.error_message = "forced op failure (__fail hook)";
            candidate.ref_bindings = collect_ref_bindings(op, op_id);
        } else if (op_id.find("__needsrepair") != std::string::npos) {
            candidate.status = CandidateResult::Status::NeedsRepair;
            candidate.needs_repair.push_back(make_needs_repair(op, op_id));
            candidate.ref_bindings = collect_ref_bindings(op, op_id);
        } else {
            candidate = execute_candidate_op(job, op, op_id, last_sketch_id, ctx.cancel);
        }

        if (candidate.status == CandidateResult::Status::Cancelled) {
            res.status = ExecStatus::Cancelled;
            return res;
        }
        // Advisory op diagnostics first, so a failure diagnostic stays LAST — the
        // `opFailed` branch below reads `diagnostics.back()` as the step message.
        for (json& d : candidate.diagnostics) diagnostics.push_back(std::move(d));
        if (candidate.status == CandidateResult::Status::Failed ||
            candidate.status == CandidateResult::Status::Unsupported) {
            diagnostics.push_back(
                fail_diagnostic(candidate.error_code, candidate.error_message));
        }

        if (candidate.status == CandidateResult::Status::Ok) {
            emit_plan_step(ctx, req_id, job_id, step_index, candidate.body_events,
                           candidate.delta.to_json(), candidate.needs_repair,
                           signatures_json(job.bodies, candidate.body_events,
                                           candidate.ref_bindings),
                           diagnostics);
            StepResult r;
            r.step_index = step_index;
            r.status = "ok";
            r.body_ids = std::move(candidate.body_ids);
            job.per_step.push_back(std::move(r));
            last_ok_step = step_index;
            res.last_ok_exec_idx = exec_idx;
        } else if (candidate.status == CandidateResult::Status::NeedsRepair) {
            emit_plan_step(ctx, req_id, job_id, step_index, /*events=*/{},
                           em::ElementMapDelta{}.to_json(), candidate.needs_repair,
                           signatures_json(job.bodies, /*events=*/{},
                                           candidate.ref_bindings),
                           diagnostics);
            StepResult r;
            r.step_index = step_index;
            r.status = "needsRepair";
            r.ref_count = candidate.needs_repair.size();
            job.per_step.push_back(std::move(r));
            job.stopped_reason = "needsRepair";
            job.last_valid_step = last_ok_step;  // prepare m−1 (SCHEMA §8)
            return res;
        } else {  // Failed — revert to m−1; NO planStep event for the failed step.
            StepResult r;
            r.step_index = step_index;
            r.status = "opFailed";
            // Carry the op's §8 message into perStepResults (the failed step emits no
            // planStep, so this is the only channel to Rust — see the emit below).
            if (!diagnostics.empty()) r.message = diagnostics.back().value("message", "");
            job.per_step.push_back(std::move(r));
            job.stopped_reason = "opFailed";
            job.last_valid_step = last_ok_step;  // publish ≤ m-1 (Invariant 6)
            return res;
        }
        ++exec_idx;
    }

    job.stopped_reason = "completed";
    job.last_valid_step = last_ok_step;
    return res;
}

// Inline tessellation artifact on ExecutePlan (SCHEMA §7.2 artifacts.tessellate):
// tessellate every prepared body into a MESH1 blob attached to the terminal resp's
// binary tail (small → inlined per §5.2), referenced by result.artifacts.tessellate.
json attach_tessellate(const ScratchJob& job, const json& artifacts, Envelope& resp) {
    if (!artifacts.is_object() || !artifacts.contains("tessellate") ||
        !artifacts["tessellate"].is_object()) {
        return json();
    }
    const json& t = artifacts["tessellate"];
    const std::string lod = t.value("lod", std::string("coarse"));
    const bool include_edges = t.value("includeEdges", true);

    json meshes = json::array();
    for (const auto& [bid, rec] : job.bodies.all()) {
        tess::BodyMesh bm = tess::tessellate_body(rec.geom, bid, lod, include_edges, &job.partition,
                                                  &rec.face_colors);
        if (!bm.ok) continue;
        const std::uint64_t off = resp.out_bin.size();
        resp.out_bin.insert(resp.out_bin.end(), bm.blob.begin(), bm.blob.end());
        const std::string section = "mesh:" + bid;
        resp.bin.push_back(protocol::BinSection{section, off, bm.blob.size()});
        // Shared §7.6 handle builder (identical shape as the Tessellate verb —
        // MeshHandle.h). `snapshotId` is the prepared scratch snapshot the artifact
        // belongs to (reconciled to the §7.6 superset; was previously omitted here).
        meshes.push_back(tess::mesh_handle_json(
            bid, section, lod, bm.blob.size(), bm.triangle_count,
            hashing::sha256_hex(bm.blob.data(), bm.blob.size()), job.prepared_snapshot_id));
    }
    return json{{"meshes", std::move(meshes)}};
}

}  // namespace

Envelope handle_execute_plan(Session& session, const Envelope& req, HandlerContext& ctx) {
    const json& args = req.args;
    const std::uint64_t job_id = read_u64(args, "jobId");
    const std::uint64_t doc_rev = read_u64(args, "documentRevision");
    const std::uint64_t epoch = read_u64(args, "workerEpoch");
    const std::string expected_base_hash = get_str(args, "expectedBaseHash");
    const json ops = (args.contains("ops") && args["ops"].is_array()) ? args["ops"] : json::array();
    const json prefix_hashes =
        (args.contains("prefixHashes") && args["prefixHashes"].is_array()) ? args["prefixHashes"]
                                                                           : json::array();
    const json artifacts =
        (args.contains("artifacts") && args["artifacts"].is_object()) ? args["artifacts"] : json::object();

    FenceOutcome fence = session.fence_and_clone(job_id, doc_rev, epoch, expected_base_hash);
    if (fence.status == FenceOutcome::Status::Error) {
        Envelope r = Envelope::error_response(req.id, fence.error);
        r.stamp.job_id = job_id;
        return r;
    }
    if (fence.status == FenceOutcome::Status::IdempotentPrepared) {
        // Same jobId re-sent while prepared → re-return the cached PlanPrepared.
        Envelope r = Envelope::ok_response(req.id, fence.idempotent_result);
        r.stamp.job_id = job_id;
        return r;
    }

    ScratchJob job;
    job.job_id = job_id;
    job.plan_document_revision = doc_rev;  // D4: adopted as head documentRevision at accept
    job.bodies = std::move(fence.cloned_bodies);
    job.partition = std::move(fence.cloned_partition);
    job.prepared_snapshot_id = fence.prepared_snapshot_id;
    // OPTIONAL `editedFrom` (SCHEMA §7.2). Absence = "no edit context" = no claim;
    // a non-integer is treated as absent rather than as an error, per §4's
    // tolerate-unknown/ignore-malformed-optional reader rule. See §10 for what it
    // gates (the descriptor-tie veto) — nothing else in the plan depends on it.
    if (args.contains("editedFrom") && args["editedFrom"].is_number_unsigned()) {
        job.edited_from = args["editedFrom"].get<std::uint64_t>();
    }

    const ExecResult exec = execute_ops(job, ops, job_id, req.id, ctx);
    if (exec.status == ExecStatus::Cancelled) {
        // The scratch was never stored, so the session head is unchanged and
        // hasScratch stays false (SCHEMA §8 CANCELLED: session intact).
        Envelope r = Envelope::error_response(
            req.id, ErrorInfo{"CANCELLED", "plan cancelled", /*retriable=*/false});
        r.stamp.job_id = job_id;
        return r;
    }

    // Prepared opaque head token: prefixHashes[lastExecutedIdx] — the token AFTER
    // the last executed op — or expectedBaseHash when only the base is valid. The
    // worker NEVER computes it (HistoryHash.h opaque-token contract).
    if (exec.last_ok_exec_idx.has_value() &&
        *exec.last_ok_exec_idx < prefix_hashes.size() &&
        prefix_hashes[*exec.last_ok_exec_idx].is_string()) {
        job.history_prefix_hash = prefix_hashes[*exec.last_ok_exec_idx].get<std::string>();
    } else {
        job.history_prefix_hash = expected_base_hash;  // base-only prepare (or missing tokens)
    }

    json per_step = json::array();
    for (const StepResult& ps : job.per_step) {
        json e = {{"stepIndex", ps.step_index}, {"status", ps.status}};
        if (!ps.body_ids.empty()) e["bodyIds"] = ps.body_ids;
        if (ps.ref_count.has_value()) e["refCount"] = *ps.ref_count;
        // Surface a failed op's recoverable message (§8) so Rust can report WHY —
        // a failed step emits no planStep event, so its diagnostic would otherwise be
        // worker-local. Additive (readers ignore unknown keys; §4).
        if (!ps.message.empty()) e["message"] = ps.message;
        per_step.push_back(std::move(e));
    }
    json last_valid = job.last_valid_step.has_value() ? json(*job.last_valid_step) : json(nullptr);
    json result = {
        {"planPrepared", true},
        {"preparedSnapshotId", job.prepared_snapshot_id},
        {"lastValidStep", last_valid},
        {"stoppedReason", job.stopped_reason},
        {"perStepResults", std::move(per_step)},
        {"historyPrefixHash", job.history_prefix_hash},
    };

    // Build the terminal resp first (so tessellation can attach binary sections),
    // then cache the JSON result for idempotent re-return.
    Envelope r = Envelope::ok_response(req.id, result);
    r.stamp.job_id = job_id;

    // Cache the PlanPrepared JSON for idempotent re-return WITHOUT the tessellate
    // artifacts: the artifact bytes ride in THIS resp's binary tail only, so a
    // re-sent jobId (which returns the cached JSON with no bin) must not reference
    // dangling `mesh:*` sections. The idempotency contract pins
    // preparedSnapshotId/historyPrefixHash/perStepResults — meshes are re-fetchable
    // via Tessellate. The artifact reference is attached to the live resp only.
    job.prepared_result = result;
    json tess = attach_tessellate(job, artifacts, r);
    if (!tess.is_null()) {
        result["artifacts"] = json{{"tessellate", tess}};
        r.result = std::move(result);  // live resp references the inlined sections
    }

    session.store_prepared(std::move(job));
    return r;
}

Envelope handle_accept_prepared(Session& session, const Envelope& req) {
    const json& args = req.args;
    const std::uint64_t job_id = read_u64(args, "jobId");
    const std::uint64_t doc_rev = read_u64(args, "documentRevision");
    const std::uint64_t epoch = read_u64(args, "workerEpoch");

    AcceptOutcome a = session.accept_prepared(job_id, doc_rev, epoch);
    if (!a.ok) {
        Envelope r = Envelope::error_response(req.id, a.error);
        r.stamp.job_id = job_id;
        return r;
    }
    json result = {{"accepted", true}, {"snapshotId", a.snapshot_id}, {"documentRevision", a.document_revision}};
    Envelope r = Envelope::ok_response(req.id, std::move(result));
    r.stamp.job_id = job_id;
    return r;
}

Envelope handle_discard_prepared(Session& session, const Envelope& req) {
    const std::uint64_t job_id = read_u64(req.args, "jobId");
    session.discard_prepared(job_id);
    Envelope r = Envelope::ok_response(req.id, json{{"discarded", true}});
    r.stamp.job_id = job_id;
    return r;
}

}  // namespace onecad::session
