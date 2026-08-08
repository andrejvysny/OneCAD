// PreviewOp.cpp — see PreviewOp.h.
#include "session/PreviewOp.h"

#include <optional>
#include <set>
#include <string>
#include <utility>
#include <vector>

#include "session/PlanExecutor.h"
#include "session/ScratchJob.h"
#include "tess/MeshHandle.h"
#include "tess/Tessellate.h"
#include "util/Hashing.h"

namespace onecad::session {

using nlohmann::json;
using protocol::Envelope;

namespace {

struct PreviewRequest {
    json op;
    std::string lod;
    std::uint64_t snapshot_id = 0;
};

Envelope err(const Envelope& req, const char* code, const std::string& msg,
             json diagnostics = json::array()) {
    // A preview never mutates anything, so every failure is recoverable by
    // definition — the session is untouched whatever happened.
    std::optional<json> detail;
    if (!diagnostics.empty()) detail = json{{"diagnostics", std::move(diagnostics)}};
    return Envelope::error_response(
        req.id, protocol::ErrorInfo{code, msg, /*retriable=*/false, std::move(detail)});
}

json empty_result(std::uint64_t snapshot_id, json needs_repair = json::array()) {
    return json{
        {"snapshotId", snapshot_id},
        {"bodyEvents", json::array()},
        {"changedBodies", json::array()},
        {"deletedBodies", json::array()},
        {"needsRepair", std::move(needs_repair)},
        {"meshes", json::array()},
    };
}

bool valid_operation_shape(const json& op) {
    if (!op.contains("opType") || !op["opType"].is_string() ||
        (op.contains("opId") && !op["opId"].is_string()) ||
        (op.contains("params") && !op["params"].is_object())) {
        return false;
    }
    if (!op.contains("params")) return true;
    const json& params = op["params"];
    return !params.contains("sketchId") || params["sketchId"].is_string();
}

bool parse_request(Session& session, const Envelope& req,
                   const onecad::CancelToken& cancel, PreviewRequest& out,
                   Envelope& error) {
    const json& args = req.args;
    if (!args.is_object() || !args.contains("op") || !args["op"].is_object()) {
        error = err(req, "PROTOCOL_ERROR",
                    "PreviewOp: args.op must be an operation object");
        return false;
    }
    if ((args.contains("lod") && !args["lod"].is_string()) ||
        (args.contains("sketchId") && !args["sketchId"].is_string())) {
        error = err(req, "PROTOCOL_ERROR", "PreviewOp: malformed string argument");
        return false;
    }
    out.op = args["op"];
    if (!valid_operation_shape(out.op)) {
        error = err(req, "PROTOCOL_ERROR",
                    "PreviewOp: malformed operation object");
        return false;
    }
    out.lod = args.value("lod", std::string("coarse"));
    if (out.lod != "coarse" && out.lod != "medium" && out.lod != "fine") {
        error = err(req, "PROTOCOL_ERROR",
                    "PreviewOp: lod must be coarse, medium, or fine");
        return false;
    }
    if (cancel.cancelled()) {
        error = err(req, "CANCELLED", "preview cancelled");
        return false;
    }
    out.snapshot_id = session.current_snapshot_id();
    if (!args.contains("expectedSnapshotId")) return true;
    if (!args["expectedSnapshotId"].is_number_unsigned()) {
        error = err(req, "PROTOCOL_ERROR",
                    "PreviewOp: expectedSnapshotId must be a u64");
        return false;
    }
    const std::uint64_t expected =
        args["expectedSnapshotId"].get<std::uint64_t>();
    if (expected == out.snapshot_id) return true;
    // Structured code (SCHEMA §7.6/§8): Rust maps STALE_PREVIEW without
    // sniffing the message text, so rewording this can never downgrade the
    // recoverable stale case to a generic failure.
    error = err(req, "STALE_PREVIEW",
                "PreviewOp: stale snapshot; expected " +
                    std::to_string(expected) + ", current " +
                    std::to_string(out.snapshot_id));
    return false;
}

std::optional<std::string> profile_sketch_id(const Envelope& req, const json& op,
                                             std::string& error) {
    const std::string arg_id = req.args.value("sketchId", std::string{});
    const json params =
        (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();
    const std::string op_id = params.value("sketchId", std::string{});
    if (!arg_id.empty() && !op_id.empty() && arg_id != op_id) {
        error = "PreviewOp: args.sketchId does not match op.params.sketchId";
        return std::nullopt;
    }
    return !op_id.empty() ? op_id : arg_id;
}

bool seed_profile_sketch(Session& session, const Envelope& req, const json& op,
                         ScratchJob& job, std::string& last_sketch_id,
                         Envelope& error) {
    std::string sketch_error;
    const std::optional<std::string> sketch_id =
        profile_sketch_id(req, op, sketch_error);
    if (!sketch_id) {
        error = err(req, "PROTOCOL_ERROR", sketch_error);
        return false;
    }
    if (sketch_id->empty()) return true;
    std::optional<StoredSketch> stored = session.sketches().snapshot(*sketch_id);
    if (!stored) {
        error = err(req, "REF_UNRESOLVED",
                    "PreviewOp: profile sketch not found: " + *sketch_id);
        return false;
    }
    json params = stored->wire_args;
    params["sketchId"] = *sketch_id;
    job.sketches.emplace_back(*sketch_id, std::move(params));
    last_sketch_id = *sketch_id;
    return true;
}

json body_events_json(const std::vector<BodyEvent>& events) {
    json out = json::array();
    for (const BodyEvent& event : events) {
        out.push_back({{"kind", event.kind}, {"bodyId", event.body_id}});
    }
    return out;
}

void classify_body_events(const std::vector<BodyEvent>& events,
                          std::set<std::string>& changed,
                          std::set<std::string>& deleted) {
    for (const BodyEvent& event : events) {
        if (event.kind == "deleted") {
            deleted.insert(event.body_id);
            changed.erase(event.body_id);
        } else if ((event.kind == "created" || event.kind == "modified") &&
                   deleted.count(event.body_id) == 0) {
            changed.insert(event.body_id);
        }
    }
}

std::string append_mesh(const std::string& body_id, const BodyRecord& body,
                        const std::string& lod, std::uint64_t snapshot_id,
                        const elementmap::ElementMapPartition& partition,
                        Envelope& response, json& meshes) {
    tess::BodyMesh mesh = tess::tessellate_body(
        body.geom, body_id, lod, /*include_edges=*/true, &partition, &body.face_colors);
    if (!mesh.ok || mesh.triangle_count == 0) {
        return "PreviewOp: changed body produced no preview mesh: " + body_id;
    }
    const std::uint64_t offset = response.out_bin.size();
    response.out_bin.insert(response.out_bin.end(), mesh.blob.begin(),
                            mesh.blob.end());
    const std::string section = "mesh:" + body_id;
    response.bin.push_back(
        protocol::BinSection{section, offset, mesh.blob.size()});
    meshes.push_back(tess::mesh_handle_json(
        body_id, section, lod, mesh.blob.size(), mesh.triangle_count,
        hashing::sha256_hex(mesh.blob.data(), mesh.blob.size()), snapshot_id));
    return {};
}

Envelope build_response(const Envelope& req, const PreviewRequest& input,
                        const ScratchJob& job, const CandidateResult& outcome) {
    Envelope response = Envelope::ok_response(req.id, json::object());
    json meshes = json::array();
    std::set<std::string> changed_ids;
    std::set<std::string> deleted_ids;
    classify_body_events(outcome.body_events, changed_ids, deleted_ids);
    for (const std::string& body_id : changed_ids) {
        const BodyRecord* body = job.bodies.get(body_id);
        if (!body) {
            return err(req, "OP_FAILED",
                       "PreviewOp: changed body missing from candidate state: " +
                           body_id);
        }
        const std::string mesh_error =
            append_mesh(body_id, *body, input.lod, input.snapshot_id,
                        job.partition, response, meshes);
        if (!mesh_error.empty()) {
            return err(req, "GEOMETRY_INVALID", mesh_error);
        }
    }
    response.result = {
        {"snapshotId", input.snapshot_id},
        {"bodyEvents", body_events_json(outcome.body_events)},
        {"changedBodies", std::vector<std::string>(changed_ids.begin(),
                                                   changed_ids.end())},
        {"deletedBodies", std::vector<std::string>(deleted_ids.begin(),
                                                   deleted_ids.end())},
        {"needsRepair", json::array()},
        {"meshes", std::move(meshes)},
    };
    return response;
}

}  // namespace

Envelope handle_preview_op(Session& session, const Envelope& req,
                           const onecad::CancelToken& cancel) {
    PreviewRequest input;
    Envelope error;
    if (!parse_request(session, req, cancel, input, error)) return error;

    // Fencing-free, throwaway head copy. Never prepare or advance a snapshot.
    ScratchJob job;
    job.bodies = session.bodies_copy();
    job.partition = session.partition_copy();
    std::string last_sketch_id;
    if (!seed_profile_sketch(session, req, input.op, job, last_sketch_id,
                             error)) {
        return error;
    }

    const std::string op_id =
        input.op.value("opId", std::string("preview"));
    CandidateResult outcome = execute_candidate_op(
        job, input.op, op_id, last_sketch_id, cancel);
    if (outcome.status == CandidateResult::Status::Cancelled) {
        return err(req, "CANCELLED", "preview cancelled");
    }
    if (outcome.status == CandidateResult::Status::NeedsRepair) {
        return Envelope::ok_response(
            req.id,
            empty_result(input.snapshot_id, std::move(outcome.needs_repair)));
    }
    if (outcome.status != CandidateResult::Status::Ok) {
        const std::string code =
            outcome.error_code.empty() ? "OP_FAILED" : outcome.error_code;
        return err(req, code.c_str(),
                   outcome.error_message.empty() ? "PreviewOp: op failed"
                                                 : outcome.error_message,
                   candidate_diagnostics(outcome));
    }
    return build_response(req, input, job, outcome);
}

Envelope handle_preview_op(Session& session, const Envelope& req) {
    const onecad::CancelToken cancel;
    return handle_preview_op(session, req, cancel);
}

}  // namespace onecad::session
