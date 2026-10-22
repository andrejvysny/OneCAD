// PreviewPlan.cpp — see PreviewPlan.h.
#include "session/PreviewPlan.h"

#include <cstdint>
#include <optional>
#include <string>
#include <utility>
#include <vector>

#include <BRepBuilderAPI_Copy.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>

#include "io/BrepCodec.h"
#include "io/OcctBusy.h"
#include "session/PlanExecutor.h"
#include "session/PreviewMesh.h"
#include "session/ScratchJob.h"
#include "util/Log.h"

namespace onecad::session {

using nlohmann::json;
using protocol::Envelope;
using protocol::ErrorInfo;
using protocol::HandlerContext;

namespace {

struct PreviewPlanArgs {
    std::uint64_t job_id = 0;
    std::uint64_t worker_epoch = 0;
    std::uint64_t expected_snapshot = 0;
    std::optional<std::uint64_t> edited_from;
    std::string lod = "coarse";
    json ops = json::array();
};

Envelope fail(const Envelope& req, std::uint64_t job_id, const char* code,
              const std::string& message, std::optional<json> detail = std::nullopt) {
    Envelope r = Envelope::error_response(
        req.id, ErrorInfo{code, message, /*retriable=*/false, std::move(detail)});
    r.stamp.job_id = job_id;
    return r;
}

bool read_u64(const json& args, const char* key, std::uint64_t& out) {
    if (!args.contains(key) || !args[key].is_number_unsigned()) return false;
    out = args[key].get<std::uint64_t>();
    return true;
}

// Parses the request; on a malformed one fills `error` (PROTOCOL_ERROR).
bool parse_args(const Envelope& req, PreviewPlanArgs& out, std::string& error) {
    const json& args = req.args;
    if (!args.is_object()) {
        error = "PreviewPlan: args must be an object";
        return false;
    }
    if (args.contains("jobId")) read_u64(args, "jobId", out.job_id);
    if (!read_u64(args, "workerEpoch", out.worker_epoch)) {
        error = "PreviewPlan: workerEpoch is required (u64)";
        return false;
    }
    if (!read_u64(args, "expectedSnapshotId", out.expected_snapshot)) {
        error = "PreviewPlan: expectedSnapshotId is required (u64)";
        return false;
    }
    // A preview replays from an EMPTY base only; it never claims the restored-base
    // slot regen owns (SCHEMA §7.6).
    if (args.contains("baseCheckpoint")) {
        error = "PreviewPlan: baseCheckpoint is not accepted (from-0 replay only)";
        return false;
    }
    if (!args.contains("ops") || !args["ops"].is_array()) {
        error = "PreviewPlan: ops must be an array";
        return false;
    }
    out.ops = args["ops"];
    if (args.contains("lod")) {
        if (!args["lod"].is_string()) {
            error = "PreviewPlan: lod must be a string";
            return false;
        }
        out.lod = args["lod"].get<std::string>();
    }
    if (out.lod != "coarse" && out.lod != "medium" && out.lod != "fine") {
        error = "PreviewPlan: lod must be coarse, medium, or fine";
        return false;
    }
    // Same tolerate-malformed rule as ExecutePlan's `editedFrom` (§4).
    if (args.contains("editedFrom") && args["editedFrom"].is_number_unsigned()) {
        out.edited_from = args["editedFrom"].get<std::uint64_t>();
    }
    return true;
}

// The B-rep bytes of a mesh-free geometry copy of `shape` with its cache flags
// cleared. Meshing marks the head's faces `Checked` (every commit tessellates
// its bodies for the inline artifact), and BinTools serializes that flag, so
// without the normalization the head never compares equal to a fresh replay.
// `Checked`/`Modified` record validation bookkeeping, not geometry.
bool mesh_free_brep(const TopoDS_Shape& shape, std::vector<std::uint8_t>& out) {
    try {
        TopoDS_Shape copy =
            BRepBuilderAPI_Copy(shape, /*copyGeom=*/true, /*copyMesh=*/false).Shape();
        TopTools_IndexedMapOfShape all;
        TopExp::MapShapes(copy, all);
        for (int i = 1; i <= all.Extent(); ++i) {
            TopoDS_Shape sub = all(i);
            sub.Checked(false);
            sub.Modified(false);
        }
        copy.Checked(false);
        copy.Modified(false);
        return io::write_brep_shape(copy, out).empty();
    } catch (const Standard_Failure&) {
        return false;
    }
}

// True iff the candidate body would DISPLAY exactly as the head body with the
// same id. Compared on mesh-free B-rep bytes: a metric summary (counts, bbox,
// volume) cannot see a feature that moved inside the body. Any doubt reads as
// changed, which only costs a mesh.
bool unchanged_from_head(const BodyStore& head, const std::string& body_id,
                         const BodyRecord& candidate) {
    const BodyRecord* prior = head.get(body_id);
    if (prior == nullptr || prior->health != candidate.health ||
        prior->face_colors != candidate.face_colors) {
        return false;
    }
    std::vector<std::uint8_t> a;
    std::vector<std::uint8_t> b;
    return mesh_free_brep(prior->geom, a) && mesh_free_brep(candidate.geom, b) && a == b;
}

json mesh_failure_detail(const PreviewMeshFailure& failure) {
    const bool budget = failure.kind == PreviewMeshFailure::Kind::Budget;
    json diagnostic = {
        {"severity", "error"},
        {"code", "UNSUPPORTED"},
        {"message", failure.message},
        {"stage", "preview-mesh"},
        {"reasonCode", budget ? "PREVIEW_MESH_BUDGET_EXCEEDED" : "PREVIEW_TESSELLATE_FAILED"},
    };
    json evidence = {{"bodyId", failure.body_id}};
    if (budget) {
        evidence["bytes"] = failure.bytes;
        evidence["limit"] = failure.limit;
    }
    diagnostic["evidence"] = std::move(evidence);
    return json{{"diagnostics", json::array({std::move(diagnostic)})}};
}

// Meshes every final body that is new or differs from the head into `response`.
// Returns the failure terminal, if any.
std::optional<Envelope> attach_meshes(const Envelope& req, const PreviewPlanArgs& in,
                                      const ScratchJob& job, const BodyStore& head,
                                      HandlerContext& ctx, Envelope& response,
                                      json& meshes, json& bodies) {
    for (const auto& [body_id, record] : job.bodies.all()) {
        if (ctx.cancel.cancelled()) {
            return fail(req, in.job_id, "CANCELLED", "preview cancelled");
        }
        bodies.push_back(body_id);
        if (unchanged_from_head(head, body_id, record)) continue;
        if (const auto failure =
                append_preview_mesh("PreviewPlan", body_id, record, in.lod,
                                    in.expected_snapshot, job.partition, response, meshes)) {
            return fail(req, in.job_id, "UNSUPPORTED", failure->message,
                        mesh_failure_detail(*failure));
        }
    }
    return std::nullopt;
}

}  // namespace

Envelope handle_preview_plan(Session& session, const Envelope& req, HandlerContext& ctx) {
    PreviewPlanArgs in;
    std::string parse_error;
    if (!parse_args(req, in, parse_error)) {
        return fail(req, in.job_id, "PROTOCOL_ERROR", parse_error);
    }
    if (!session.is_open()) {
        return fail(req, in.job_id, "PROTOCOL_ERROR", "PreviewPlan: no open session");
    }
    // Read-only fence. A mismatched epoch is STALE_PREVIEW here, not the D4
    // PROTOCOL_ERROR: a preview publishes nothing, so a stale one is merely late.
    // Epoch, snapshot and bodies come from ONE critical section.
    std::uint64_t head_snapshot = 0;
    std::uint64_t head_epoch = 0;
    std::optional<PublishedStateSnapshot> published =
        session.published_state_at(in.expected_snapshot, &head_snapshot, &head_epoch);
    if (head_epoch != in.worker_epoch) published.reset();
    if (!published) {
        return fail(req, in.job_id, "STALE_PREVIEW",
                    "PreviewPlan: stale head; expected epoch " +
                        std::to_string(in.worker_epoch) + " snapshot " +
                        std::to_string(in.expected_snapshot),
                    json{{"requested", in.expected_snapshot},
                         {"head", head_snapshot},
                         {"workerEpoch", head_epoch}});
    }
    if (ctx.cancel.cancelled()) return fail(req, in.job_id, "CANCELLED", "preview cancelled");

    // An EMPTY base (the D5 from-0 shape), never the fence-and-clone path.
    ScratchJob job;
    job.job_id = in.job_id;
    job.plan = json{{"ops", in.ops}};
    job.edited_from = in.edited_from;
    const ExecResult exec = execute_plan_ops(job, in.ops, in.job_id, req.id, ctx,
                                             ops::ValidationMode::PreviewInteractive);
    if (exec.status == ExecStatus::Cancelled) {
        return fail(req, in.job_id, "CANCELLED", "preview cancelled");
    }
    if (exec.status == ExecStatus::Busy) {
        Envelope r = io::busy_response(req.id, exec.busy_message);
        r.stamp.job_id = in.job_id;
        return r;
    }

    Envelope response = Envelope::ok_response(req.id, json::object());
    response.stamp.job_id = in.job_id;
    json meshes = json::array();
    json bodies = json::array();
    if (auto failure =
            attach_meshes(req, in, job, published->bodies, ctx, response, meshes, bodies)) {
        return std::move(*failure);
    }
    if (ctx.cancel.cancelled()) return fail(req, in.job_id, "CANCELLED", "preview cancelled");
    WLOG_DEBUG("PreviewPlan job %llu: %zu steps, %zu bodies, %zu meshed",
               static_cast<unsigned long long>(in.job_id), job.per_step.size(), bodies.size(),
               meshes.size());
    response.result = {
        {"planPreviewed", true},
        {"snapshotId", in.expected_snapshot},
        {"lastValidStep",
         job.last_valid_step.has_value() ? json(*job.last_valid_step) : json(nullptr)},
        {"stoppedReason", job.stopped_reason},
        {"perStepResults", per_step_json(job)},
        {"bodies", std::move(bodies)},
        {"meshes", std::move(meshes)},
    };
    return response;
}

}  // namespace onecad::session
