// PlanExecutor.cpp — see PlanExecutor.h. REAL OCCT ops (W-WP5).
#include "session/PlanExecutor.h"

#include <chrono>
#include <cstdint>
#include <cstdlib>  // std::abort
#include <memory>
#include <optional>
#include <set>
#include <string>
#include <thread>
#include <vector>

#include <map>
#include <stdexcept>
#include <utility>

#include <Standard_Failure.hxx>

#include "elementmap/ElementMapPartition.h"
#include "elementmap/Ladder.h"
#include "ops/BooleanOp.h"
#include "ops/ComponentOp.h"
#include "ops/ExtrudeOp.h"
#include "ops/FilletChamferOp.h"
#include "ops/GearOp.h"
#include "ops/HoleOp.h"
#include "ops/ImportOp.h"
#include "ops/MirrorOp.h"
#include "ops/OffsetFaceOp.h"
#include "ops/OpCommon.h"
#include "ops/TransformOp.h"
#include "ops/OpTypes.h"
#include "ops/PatternOp.h"
#include "ops/RevolveOp.h"
#include "ops/ShellOp.h"
#include "protocol/Limits.h"
#include "session/ClassifyElement.h"
#include "session/FeaturePattern.h"
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
// SCHEMA §7.3 gear referenceability (WP-I): the gear info for `body_id`, from
// the map the fence cloned (a gear body this plan INHERITED) or from the plan
// itself (one this plan CREATES). `nullopt` ⇒ an ordinary body, no filtering.
std::optional<ops::GearBodyInfo> gear_info_for(const ScratchJob& job, const std::string& body_id,
                                               const TopoDS_Shape& body_shape) {
    const auto tracked = job.gear_bodies.find(body_id);
    if (tracked != job.gear_bodies.end()) return tracked->second;
    return ops::gear_body_info(job.plan, body_id, body_shape);
}

// The step diagnostic that rides a gear-referenceability halt, in the SAME
// evidence shape `BindElementIds` uses (SCHEMA §7.3): a FACE reports its
// `surfaceType`, an EDGE or VERTEX reports `kind` in its place. `topo_key` and
// the geometry field are empty when the ref never named a concrete sub-element
// (the descriptor rung, where nothing was bound to inspect).
json gear_element_diagnostic(const std::string& body_id, const ops::GearBodyInfo& info,
                             const std::string& element_id, em::km::ElementKind kind,
                             const std::string& topo_key, const std::string& surface_type) {
    const bool is_face = kind == em::km::ElementKind::Face;
    const std::string noun = em::ElementMapPartition::kind_name(kind);
    json evidence = {{"bodyId", body_id}, {"topoKey", topo_key}, {"gearOpId", info.gear_op_id}};
    if (is_face) {
        evidence["surfaceType"] = surface_type;
    } else {
        evidence["kind"] = noun;
    }
    const std::string article = (noun == "edge") ? "an " : "a ";
    const std::string message =
        "ref " + element_id + " names " + article + noun + " of gear body " + body_id +
        " that is not referenceable (only the two caps, a bore below the root radius, and the "
        "edges and vertices whose every adjacent face is one of those, carry identity a "
        "tooth-count edit preserves)";
    return json{{"severity", "warning"},
                {"code", "REF_UNRESOLVED"},
                {"message", message},
                {"stage", "input-resolution"},
                {"reasonCode", "GEAR_FACE_NOT_REFERENCEABLE"},
                {"evidence", std::move(evidence)}};
}

// A §9 item for a STORED id that tracks a gear sub-element the §7.3 rule refuses.
// It is a RECORD DEFECT, not a resolution problem, so it halts here with an
// EMPTY candidate list rather than going to the ladder: the narrowed pool would
// otherwise be free to auto-bind the id to a cap or a bore, silently REPOINTING
// a tooth reference at different geometry — the wrong bind §10 exists to
// prevent, arrived at from the other direction. The partition entry is left
// exactly as it was; only the user's repair may move it.
json refused_gear_element_repair(const em::LadderRef& ref, const std::string& noun) {
    return json{{"refId", ref.ref_id},
                {"elementId", ref.element_id},
                {"ladderFailed", "descriptor"},
                {"reason", "no-candidates"},
                {"scoringVersion", em::kResolverVersion},
                {"candidates", json::array()},
                {"anchor", ref.anchor_json.is_null() ? json::object() : ref.anchor_json},
                {"uiLabel", "stored reference names a gear " + noun +
                                " that is not referenceable; re-pick it on a cap, a bore, or an "
                                "edge whose every adjacent face is one of those"}};
}

// True when the ALREADY-TRACKED entry for `element_id` binds a gear sub-element
// the §7.3 rule refuses — a binding that predates this build (nothing can mint
// one now). The caller then halts on the item above; it must NOT hand the ref to
// the ladder, because a narrowed pool that auto-binds would repoint the id.
bool tracked_binding_is_refused_gear_element(const ScratchJob& job, const std::string& body_id,
                                             const std::string& element_id,
                                             std::vector<json>& diagnostics) {
    const BodyRecord* rec = job.bodies.get(body_id);
    if (rec == nullptr) return false;
    const em::PartitionEntry* entry = job.partition.find(element_id);
    if (entry == nullptr || entry->body_id != body_id) return false;
    const std::optional<ops::GearBodyInfo> gear = gear_info_for(job, body_id, rec->geom);
    if (!gear) return false;
    const TopoDS_Shape shape =
        em::ElementMapPartition::shape_for_topokey(rec->geom, entry->topo_key);
    if (shape.IsNull()) return false;
    if (ops::gear_element_referenceable(*gear, rec->geom, shape)) return false;
    diagnostics.push_back(gear_element_diagnostic(
        body_id, *gear, element_id, entry->kind, entry->topo_key,
        classify_shape(shape).value("surfaceType", std::string("other"))));
    return true;
}

void resolve_input_refs(ScratchJob& job, const json& op, const std::string& op_id,
                        const em::LadderEditContext& edit, em::ElementMapDelta& delta,
                        std::vector<json>& needs_repair, std::vector<json>& diagnostics) {
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
        // Tracked rung. WP-I: a tracked entry that binds a REFUSED gear
        // sub-element is a record defect — halt on it directly and leave the
        // entry alone. Sending it to the (narrowed) ladder instead would let an
        // AutoBind repoint the stored id at a cap or a bore.
        if (job.partition.contains(r.element_id)) {
            if (tracked_binding_is_refused_gear_element(job, bid, r.element_id, diagnostics)) {
                needs_repair.push_back(refused_gear_element_repair(
                    r, em::ElementMapPartition::kind_name(r.kind)));
            }
            continue;
        }
        by_body[bid].push_back(std::move(r));
    }

    for (auto& [bid, refs] : by_body) {
        const BodyRecord* rec = job.bodies.get(bid);
        if (!rec) {
            for (const em::LadderRef& r : refs) needs_repair.push_back(missing_body_repair(r, bid));
            continue;
        }
        // SCHEMA §7.3 (WP-I): on a gear body the refused faces are REMOVED FROM
        // THE CANDIDATE POOL before the §10 ladder runs, so a stored ref that
        // names one halts with the ladder's own `no-candidates` — never a bind,
        // and never a synthesized op-built item either.
        const std::optional<ops::GearBodyInfo> gear = gear_info_for(job, bid, rec->geom);
        em::CandidateFilter admissible;
        if (gear) {
            const ops::GearBodyInfo info = *gear;
            const TopoDS_Shape body_shape = rec->geom;
            // ONE adjacency map for the whole pool: the edge/vertex rule needs a
            // sub-element's adjacent faces, and rebuilding that per candidate is
            // O(candidates x body size) — on a 400-tooth gear (the §7.3 bound,
            // itself a COST bound) that is thousands of traversals per step.
            const auto adjacency = std::make_shared<ops::GearAdjacency>(body_shape);
            admissible = [info, body_shape, adjacency](const TopoDS_Shape& candidate) {
                return ops::gear_element_referenceable(info, body_shape, candidate,
                                                       adjacency.get());
            };
        }
        const std::vector<em::LadderResolution> resolutions =
            em::resolve_descriptor_stage(rec->geom, bid, refs, edit, admissible);
        for (std::size_t k = 0; k < resolutions.size(); ++k) {
            const em::LadderResolution& res = resolutions[k];
            if (res.outcome == em::LadderOutcome::AutoBind && !res.bound_shape.IsNull()) {
                json anchor = refs[k].anchor_json;
                delta.added.push_back(job.partition.mint(bid, res.element_id, res.kind,
                                                         res.bound_shape, rec->geom, std::move(anchor)));
            } else {
                needs_repair.push_back(res.to_needs_repair_json());
                // Say WHY a ref on a gear body could not bind: the pool it was
                // scored against excludes tooth geometry by rule.
                if (gear && (refs[k].kind == em::km::ElementKind::Face ||
                             refs[k].kind == em::km::ElementKind::Edge ||
                             refs[k].kind == em::km::ElementKind::Vertex)) {
                    diagnostics.push_back(gear_element_diagnostic(bid, *gear, refs[k].element_id,
                                                                  refs[k].kind, std::string(),
                                                                  std::string()));
                }
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

// Returns TRUE when the event went out VERBATIM. False means the Dispatcher had
// to send the §3.4 substitute, which carries none of this step's `bodyEvents` or
// `elementMapDelta` — the caller must then fail the step (SCHEMA §3.4/§7.2).
bool emit_plan_step(HandlerContext& ctx, std::uint64_t req_id, std::uint64_t job_id,
                    std::uint64_t step_index, const std::vector<BodyEvent>& events,
                    const json& element_map_delta, const json& needs_repair, const json& signatures,
                    const json& diagnostics,
                    const std::optional<json>& mate_placement = std::nullopt,
                    const std::optional<json>& mate_resolved = std::nullopt) {
    json body_events = json::array();
    for (const auto& e : events) {
        json be = {{"kind", e.kind}, {"bodyId", e.body_id}};
        // VF-B6: `rankKey` is emitted ONLY where the ordinal genuinely IS a geometric
        // rank. Absence = "no claim" (SCHEMA §7.2), so a step that never ranks its
        // bodies stays byte-identical to the pre-VF-B6 wire.
        if (e.rank_key) be["rankKey"] = *e.rank_key;
        if (e.health) be["health"] = *e.health;
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
    // Component Library P3 WP-3.1 (SCHEMA §7.2, additive): present ONLY on a
    // step that actually reseated a mate — absence keeps every other step
    // byte-identical to the pre-WP-3.1 wire.
    if (mate_placement) payload["matePlacement"] = *mate_placement;
    // Kernel-hardening WP-I (SCHEMA §7.2, additive): the resolved cylindrical
    // mate's axis (and sidedness when measurable) on EVERY such step, seat
    // moved or not — absence keeps every other step byte-identical.
    if (mate_resolved) payload["mateResolved"] = *mate_resolved;
    // §4/§3.4 (WP-H): sanitise at the PRODUCER. A degenerate measurement (an
    // evidence number over a zero-area face, a ratio with a vanishing denominator)
    // must not reach the encoder as NaN/±Inf: there it would fail serialization and
    // cost the step its whole planStep, where `null` says "not measured" and every
    // other field survives. The encoder's rejection stays the backstop for a bug.
    protocol::sanitize_non_finite(payload);
    Envelope ev = Envelope::event(req_id, "planStep", step_index, std::move(payload));
    ev.stamp.job_id = job_id;
    if (ctx.emit) ctx.emit(ev);
    // No reporter (an in-process driver, or a lane that streams nothing) reads as
    // "went out verbatim": those callers own the frame and nothing was replaced.
    return !(ctx.last_emit_substituted && ctx.last_emit_substituted());
}

json fail_diagnostic(const std::string& code, const std::string& message) {
    return json{{"severity", "error"},
                {"code", code.size() <= 128 ? code : "OP_FAILED"},
                {"message", !message.empty() && message.size() <= 4096
                                ? message
                                : "Operation failed"},
                {"stage", "build"}};
}

std::optional<json> bounded_diagnostic(const json& value) {
    if (!value.is_object() || !value.contains("severity") ||
        !value["severity"].is_string() || !value.contains("code") ||
        !value["code"].is_string() || !value.contains("message") ||
        !value["message"].is_string()) {
        return std::nullopt;
    }
    const std::string severity = value["severity"].get<std::string>();
    const std::string code = value["code"].get<std::string>();
    const std::string message = value["message"].get<std::string>();
    if ((severity != "info" && severity != "warning" && severity != "error") ||
        code.empty() || code.size() > 128 || message.empty() || message.size() > 4096) {
        return std::nullopt;
    }
    json bounded = {{"severity", severity}, {"code", code}, {"message", message}};
    if (value.contains("stage") && value["stage"].is_string()) {
        const std::string stage = value["stage"].get<std::string>();
        if (stage.size() <= 64) bounded["stage"] = stage;
    }
    // SCHEMA §7.2 `diagnostics[].reasonCode` — the fine-grained publication
    // refusal reason. This allowlist REBUILDS the diagnostic, so anything not
    // named here is dropped before framing; a malformed or oversized value is
    // dropped alone and never invalidates the diagnostic.
    if (value.contains("reasonCode") && value["reasonCode"].is_string()) {
        const std::string reason_code = value["reasonCode"].get<std::string>();
        if (!reason_code.empty() && reason_code.size() <= 64)
            bounded["reasonCode"] = reason_code;
    }
    if (value.contains("evidence") && value["evidence"].is_object() &&
        value["evidence"].dump().size() <= 65'536) {
        bounded["evidence"] = value["evidence"];
    }
    return bounded;
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
                             bool post_upstream_edit, bool from_zero_replay,
                             ops::ValidationMode validation_mode) {
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
                        det.parallel,     det.occt_options, &cancel,       post_upstream_edit,
                        from_zero_replay, validation_mode};

    if (op_type == "Extrude") return ops::execute_extrude(octx, op, op_id);
    if (op_type == "Boolean") return ops::execute_boolean(octx, op, op_id);
    if (op_type == "Revolve") return ops::execute_revolve(octx, op, op_id);
    if (op_type == "Fillet") return ops::execute_fillet(octx, op, op_id);
    if (op_type == "Chamfer") return ops::execute_chamfer(octx, op, op_id);
    if (op_type == "Shell") return ops::execute_shell(octx, op, op_id);
    if (op_type == "OffsetFace") return ops::execute_offset_face(octx, op, op_id);
    if (op_type == "Hole") return ops::execute_hole(octx, op, op_id);
    if (op_type == "Gear") return ops::execute_gear(octx, op, op_id);
    if (op_type == "LinearPattern") return ops::execute_linear_pattern(octx, op, op_id);
    if (op_type == "CircularPattern") return ops::execute_circular_pattern(octx, op, op_id);
    if (op_type == "MirrorBody") return ops::execute_mirror_body(octx, op, op_id);
    if (op_type == "ImportStep") return ops::execute_import_step(octx, op, op_id);
    if (op_type == "TransformBody") return ops::execute_transform_body(octx, op, op_id);
    if (op_type == "PlaceComponent") return ops::execute_place_component(octx, op, op_id);
    if (op_type == "DetachComponent") return ops::execute_detach_component(octx, op, op_id);
    if (op_type == "FeaturePattern") {
        return execute_feature_pattern(job, op, op_id, last_sketch_id, cancel,
                                       validation_mode);
    }

    // SetComponentParams / ReplaceComponent (P2/P3 — in-place edits of
    // PlaceComponentParams at the Rust layer, no distinct wire op) and
    // Loft / Sweep remain UNSUPPORTED (SCHEMA §8) — Rust freezes the node.
    return ops::OpOutcome::unsupported("unsupported opType: " + op_type);
}

struct CandidateSnapshot {
    BodyStore bodies;
    em::ElementMapPartition partition;
    ResolvedInputEvidenceLedger input_evidence;
    TopologyOwnerLedger topology_owners;
    std::size_t sketch_count = 0;
    std::string last_sketch_id;
};

CandidateSnapshot snapshot_candidate(const ScratchJob& job,
                                     const std::string& last_sketch_id) {
    return CandidateSnapshot{job.bodies, job.partition, job.resolved_input_evidence,
                             job.topology_owners, job.sketches.size(),
                             last_sketch_id};
}

void capture_resolved_input_evidence(ScratchJob& job, const json& op,
                                     const std::string& op_id) {
    ResolvedOpEvidence captured;
    std::size_t eligible = 0;
    captured.effective_hash = feature_pattern_effective_source_hash(op);
    if (op.contains("inputs") && op["inputs"].is_array()) {
        for (std::size_t i = 0; i < op["inputs"].size(); ++i) {
            const json& input = op["inputs"][i];
            const json primary = input.value("primary", json::object());
            const std::string kind = primary.value("kind", std::string());
            if (kind != "edge" && kind != "face" && kind != "vertex") continue;
            const std::string element_id = primary.value("elementId", std::string());
            const std::string body_id = primary.value("bodyId", std::string());
            if (element_id.empty() || body_id.empty()) continue;
            ++eligible;
            const em::PartitionEntry* entry = job.partition.find(element_id);
            const BodyRecord* body = job.bodies.get(body_id);
            if (!entry || !body || entry->body_id != body_id || entry->kind !=
                    em::ElementMapPartition::kind_from_name(kind) || entry->shape.IsNull()) continue;
            const OriginClaim origin = job.topology_owners.lookup(body_id, entry->shape);
            captured.inputs.push_back(ResolvedInputEvidence{
                i, element_id, body_id, kind,
                em::ElementMapPartition::descriptor_to_json(
                    em::ElementMapPartition::describe(entry->shape, body->geom)),
                entry->anchor.is_null() ? input.value("anchor", json::object()) : entry->anchor,
                origin.state, origin.producer_record_id});
        }
    }
    if (captured.inputs.empty() || captured.inputs.size() != eligible) {
        job.resolved_input_evidence.erase(op_id);
    } else {
        job.resolved_input_evidence[op_id] = std::move(captured);
    }
}

void apply_topology_history(ScratchJob& job, const std::string& op_id,
                            const std::vector<BodyEvent>& events,
                            const std::vector<TopologyBodyHistory>& histories) {
    std::vector<std::string> touched;
    touched.reserve(events.size());
    for (const BodyEvent& event : events) touched.push_back(event.body_id);
    session::apply_topology_history(job.topology_owners, op_id, touched, histories);
    for (const std::string& body_id : touched) {
        if (const BodyRecord* body = job.bodies.get(body_id)) {
            job.topology_owners.retain_body_members(body_id, body->geom);
        }
    }
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
    if (outcome.mate_placement) result.mate_placement = std::move(outcome.mate_placement);
    if (outcome.mate_resolved) result.mate_resolved = std::move(outcome.mate_resolved);
    if (outcome.status == ops::OpOutcome::Status::Ok) {
        // A step that PUBLISHED geometry stays Ok even when it ALSO carries
        // NeedsRepair evidence (Component Library P3 WP-3.1: a mated
        // `PlaceComponent` publishes at its frozen `placement` AND flags a
        // stale mate simultaneously — spec §5.5 "never drop it, never
        // silently move it"). Every OTHER op's needs_repair path returns
        // BEFORE building any geometry (Hole/Fillet/Chamfer/Shell/
        // OffsetFace all early-return on an unresolved ref, e.g.
        // `HoleOp.cpp`'s `if (face.IsNull()) { out.needs_repair...; return
        // out; }` before the tool solid is ever built) — `result.
        // body_events` is empty there, so this branch is unreachable for
        // them and their existing "needsRepair ⇒ prepare m−1, no geometry"
        // behavior is unchanged.
        result.status = (result.needs_repair.empty() || !result.body_events.empty())
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
    job.resolved_input_evidence = std::move(snapshot.input_evidence);
    job.topology_owners = std::move(snapshot.topology_owners);
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

std::optional<ops::OpOutcome> validate_operation_body_inputs(
    const ScratchJob& job, const json& op) {
    if (!op.contains("inputs") || !op["inputs"].is_array()) return std::nullopt;
    for (const json& input : op["inputs"]) {
        if (!input.is_object() || !input.contains("primary") ||
            !input["primary"].is_object()) {
            continue;
        }
        const std::string body_id = get_str(input["primary"], "bodyId");
        const session::BodyRecord* body =
            body_id.empty() ? nullptr : job.bodies.get(body_id);
        if (body == nullptr || body->modeling_eligible()) continue;
        const std::string message = "Operation cannot reference quarantined body " +
                                    body_id +
                                    (body->health_reason.empty()
                                         ? std::string{}
                                         : ": " + body->health_reason);
        ops::OpOutcome failure =
            ops::OpOutcome::fail("OP_FAILED", message);
        failure.diagnostics.push_back({{"severity", "error"},
                                       {"code", "QUARANTINED_MODELING_INPUT"},
                                       {"message", message},
                                       {"stage", "input-validation"},
                                       {"bodyId", body_id}});
        return failure;
    }
    return std::nullopt;
}

std::optional<ops::OpOutcome> validate_published_bodies(
    const ScratchJob& job, const json& op, const ops::OpOutcome& outcome) {
    const std::string op_type = get_str(op, "opType");
    // Quarantined imports and the versioned legacy aggregate contracts are explicit
    // compatibility exceptions. Every healthy published Body is held to the global
    // exactly-one-connected-solid invariant here, after the operation-local checks
    // and before scratch state can become a successful candidate.
    const json params = op.contains("params") && op["params"].is_object()
                            ? op["params"]
                            : json::object();
    const bool legacy_aggregate =
        (op_type == "Hole" && !params.contains("resultPolicyVersion")) ||
        ((op_type == "LinearPattern" || op_type == "CircularPattern") &&
         !params.contains("resultPolicyVersion"));

    std::set<std::string> published;
    for (const session::BodyEvent& event : outcome.body_events) {
        if (event.kind == "created" || event.kind == "modified")
            published.insert(event.body_id);
    }
    for (const std::string& body_id : published) {
        const session::BodyRecord* body = job.bodies.get(body_id);
        if (!body) {
            return ops::OpOutcome::fail(
                "GEOMETRY_INVALID", "Published body is missing from scratch state: " + body_id);
        }
        if (op_type == "ImportStep" && !body->modeling_eligible()) continue;
        kernel::validation::PublicationPolicy policy;
        policy.name = op_type + " body " + body_id;
        policy.allowed_top_level_shapes = legacy_aggregate
            ? kernel::validation::TopLevelShapePolicy::SolidSet
            : kernel::validation::TopLevelShapePolicy::SingleBody;
        policy.max_solid_count = legacy_aggregate ? -1 : 1;
        policy.tier = kernel::validation::PublicationTier::TierA;
        const kernel::validation::PublicationDecision decision =
            ops::publication_decision(body->geom, policy);
        if (!decision.publishable()) {
            ops::OpOutcome failure = ops::OpOutcome::fail(decision.code, decision.message);
            failure.diagnostics.push_back({{"severity", "error"},
                                           {"code", decision.code},
                                           {"message", decision.message},
                                           {"stage", "publication-invariant"},
                                           {"reasonCode", decision.reason_code},
                                           {"bodyId", body_id},
                                           {"evidence", decision.evidence.to_json()}});
            return failure;
        }
    }
    return std::nullopt;
}
}  // namespace

CandidateResult execute_candidate_op(ScratchJob& job, const json& op,
                                     const std::string& op_id,
                                     std::string& last_sketch_id,
                                     const onecad::CancelToken& cancel,
                                     ops::ValidationMode validation_mode) {
    CandidateResult result;
    result.ref_bindings = collect_ref_bindings(op, op_id);
    if (cancel.cancelled()) {
        result.status = CandidateResult::Status::Cancelled;
        return result;
    }
    if (const auto failure = validate_operation_body_inputs(job, op)) {
        merge_outcome(result, *failure);
        return result;
    }

    CandidateSnapshot snapshot = snapshot_candidate(job, last_sketch_id);
    const bool post_edit = step_is_post_edit(job, op);

    // Generic resolution is intentionally body-agnostic: a ref normally resolves
    // against the body it names. Target-bound operations first reject a typed ref
    // that claims another body, so descriptor fallback cannot retarget it.
    result.needs_repair = ops::operation_ref_ownership_repairs(op, op_id);
    if (result.needs_repair.empty()) {
        resolve_input_refs(job, op, op_id,
                           em::LadderEditContext{post_edit, job.from_zero_replay}, result.delta,
                           result.needs_repair, result.diagnostics);
    }
    if (result.needs_repair.empty()) {
        capture_resolved_input_evidence(job, op, op_id);
        ops::OpOutcome outcome =
            run_single_op(job, op, op_id, last_sketch_id, cancel, post_edit,
                          job.from_zero_replay, validation_mode);
        if (outcome.status == ops::OpOutcome::Status::Ok) {
            if (const auto invariant_failure =
                    validate_published_bodies(job, op, outcome)) {
                outcome = *invariant_failure;
            } else if (outcome.topology_history_mode ==
                       ops::TopologyHistoryMode::CompositeAdopted) {
                if (op.value("opType", std::string()) != "FeaturePattern") {
                    outcome = ops::OpOutcome::fail(
                        "OP_FAILED", "composite topology history is reserved for FeaturePattern");
                }
            } else {
                apply_topology_history(job, op_id, outcome.body_events,
                                       outcome.topology_history);
            }
        }
        merge_outcome(result, std::move(outcome));
    } else {
        result.status = CandidateResult::Status::NeedsRepair;
    }

    if (result.status != CandidateResult::Status::Ok) {
        rollback_candidate(job, last_sketch_id, result, std::move(snapshot));
    }
    return result;
}

json candidate_diagnostics(const CandidateResult& candidate) {
    json diagnostics = json::array();
    const bool failed = candidate.status == CandidateResult::Status::Failed ||
                        candidate.status == CandidateResult::Status::Unsupported;
    const std::size_t advisory_limit = failed ? 63 : 64;
    bool has_error_diagnostic = false;
    for (const json& diagnostic : candidate.diagnostics) {
        if (diagnostics.size() >= advisory_limit) break;
        if (const auto bounded = bounded_diagnostic(diagnostic)) {
            has_error_diagnostic = has_error_diagnostic || (*bounded)["severity"] == "error";
            diagnostics.push_back(*bounded);
        }
    }
    if (failed && !has_error_diagnostic) {
        diagnostics.push_back(
            fail_diagnostic(candidate.error_code, candidate.error_message));
    }
    return diagnostics;
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

        // SCHEMA §3.4/§7.2 (WP-H): a step's `bodyEvents` and `elementMapDelta`
        // reach Rust through its `planStep` event and NOWHERE else, so a step
        // whose event had to be replaced by the §3.4 substitute cannot be
        // reported Ok — Rust would publish a snapshot whose lineage it never
        // received. Undoing such a step needs a pre-step copy, and only a lane
        // that can actually substitute needs it (an in-process driver owns its
        // frames and never replaces one), so the copy is taken only there.
        const bool can_substitute = static_cast<bool>(ctx.last_emit_substituted);
        CandidateSnapshot pre_step;
        if (can_substitute) pre_step = snapshot_candidate(job, last_sketch_id);

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
        const json diagnostics = candidate_diagnostics(candidate);

        if (candidate.status == CandidateResult::Status::Ok) {
            const bool verbatim =
                emit_plan_step(ctx, req_id, job_id, step_index, candidate.body_events,
                               candidate.delta.to_json(), candidate.needs_repair,
                               signatures_json(job.bodies, candidate.body_events,
                                               candidate.ref_bindings),
                               diagnostics, candidate.mate_placement, candidate.mate_resolved);
            if (!verbatim) {
                // The step RAN, but its evidence never left the process. Undo it
                // and end the plan here, exactly as a failed op does: prepare
                // m−1, report this step failed with the §3.4 diagnostic, execute
                // nothing after it. (A NeedsRepair step needs no such rule: it
                // already stops the plan, publishes nothing of its own, and its
                // outcome rides `perStepResults`, not the event.)
                WLOG_ERROR(
                    "ExecutePlan job %llu step %llu: planStep was substituted (§3.4); failing the "
                    "step and stopping the plan",
                    static_cast<unsigned long long>(job_id),
                    static_cast<unsigned long long>(step_index));
                rollback_candidate(job, last_sketch_id, candidate, std::move(pre_step));
                const std::string message =
                    "planStep event could not be serialized; this step's bodyEvents and "
                    "elementMapDelta never reached the caller";
                StepResult r;
                r.step_index = step_index;
                r.status = "opFailed";
                r.message = message;
                r.diagnostics = json::array({json{{"severity", "error"},
                                                  {"code", "EVENT_SERIALIZATION_FAILED"},
                                                  {"stage", "wire"},
                                                  {"message", message}}});
                job.per_step.push_back(std::move(r));
                job.stopped_reason = "opFailed";
                job.last_valid_step = last_ok_step;  // prepare m−1 (Invariant 6)
                return res;
            }
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
            r.diagnostics = diagnostics;
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

// D11 / WP09 F1: `tessellate_body` now returns `ok = false` when a NONDEGENERATE
// face of a body carried no triangles, and `diagnostic` names those faces. The
// §7.2 tessellation artifact is ADVISORY, so the plan still prepares and every
// other body still attaches — but the failure must not be silent, or the user
// sees one body simply not arrive. It rides the existing per-step diagnostics
// channel as a warning under the existing §7.2 code, so no wire field changes.
// Unlike `note_artifact_failure` this does NOT drop the tail: the meshes already
// attached are still referenced by their handles and are still correct.
void note_incomplete_body(ScratchJob& job, const std::string& body_id, const std::string& why) {
    WLOG_WARN("ExecutePlan job %llu: body %s tessellated incompletely (%s); attaching the other "
              "bodies without it",
              static_cast<unsigned long long>(job.job_id), body_id.c_str(), why.c_str());
    if (job.per_step.empty()) return;  // base-only prepare: the stderr line is the record
    json& diagnostics = job.per_step.back().diagnostics;
    if (!diagnostics.is_array()) diagnostics = json::array();
    if (diagnostics.size() >= 64) return;  // the §7.2 per-step diagnostic budget
    const std::string message = "body " + body_id + ": " + why;
    diagnostics.push_back(
        json{{"severity", "warning"},
             {"code", "ARTIFACT_TESSELLATE_FAILED"},
             {"stage", "artifact"},
             {"message", message.size() <= 4096 ? message : message.substr(0, 4096)}});
}

// Inline tessellation artifact on ExecutePlan (SCHEMA §7.2 artifacts.tessellate):
// tessellate every prepared body into a MESH1 blob attached to the terminal resp's
// binary tail when it fits the transport limits advertised in hello. Larger meshes are
// omitted: Rust then uses Tessellate, keeping control responses bounded.
json attach_tessellate(ScratchJob& job, const json& artifacts, Envelope& resp) {
    if (!artifacts.is_object() || !artifacts.contains("tessellate") ||
        !artifacts["tessellate"].is_object()) {
        return json();
    }
    const json& t = artifacts["tessellate"];
    const std::string lod = t.value("lod", std::string("coarse"));
    const bool include_edges = t.value("includeEdges", true);
    // Test hook, house style (cf. the `__crash` / `__slow` / `__fail` op ids in
    // execute_ops): force the artifact attachment to throw. It exists because NO
    // pathological body makes `tess::tessellate_body` throw — it returns ok=false —
    // so without it the §7.2 ARTIFACT_TESSELLATE_FAILED guard could not be shown red
    // (see test_executor_hazards.cpp). Rust never sends this key.
    const bool test_throw = t.is_object() && t.value("__testThrow", false);
    auto throw_if_hooked = [test_throw](const char* where) {
        if (test_throw) {
            throw std::runtime_error(std::string("__testThrow: forced tessellation artifact "
                                                 "failure (") +
                                     where + ")");
        }
    };
    json meshes = json::array();
    for (const auto& [bid, rec] : job.bodies.all()) {
        tess::BodyMesh bm = tess::tessellate_body(rec.geom, bid, lod, include_edges, &job.partition,
                                                  &rec.face_colors);
        if (!bm.ok) {
            note_incomplete_body(job, bid,
                                 bm.diagnostic.empty()
                                     ? std::string("the body produced no triangulation")
                                     : bm.diagnostic);
            continue;
        }
        if (bm.blob.size() > protocol::kChunkSize ||
            resp.out_bin.size() + bm.blob.size() > protocol::kInitialBulkCredit) {
            continue;
        }
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
        throw_if_hooked("after a partial attach");  // exercises the tail cleanup
    }
    throw_if_hooked("no attachable body");
    return json{{"meshes", std::move(meshes)}};
}

// SCHEMA §7.2 `ARTIFACT_TESSELLATE_FAILED`: the artifact is advisory, so a failure
// is recorded as a WARNING on the last executed step and the plan still prepares.
// The partially built tail is dropped with it — the surviving `mesh:*` sections
// would otherwise be bytes no result references (§5.1: the tail is addressed by
// the table only). Rust reads "no mesh attached" as "not cached" and calls
// Tessellate later.
void note_artifact_failure(ScratchJob& job, const std::string& what, Envelope& resp) {
    WLOG_WARN("ExecutePlan job %llu: tessellate artifact failed (%s); preparing without it",
              static_cast<unsigned long long>(job.job_id), what.c_str());
    resp.out_bin.clear();
    resp.bin.clear();
    if (job.per_step.empty()) {
        // A base-only prepare has no step to carry the warning; the stderr line
        // above is the whole record, and the plan still prepares.
        return;
    }
    json& diagnostics = job.per_step.back().diagnostics;
    if (!diagnostics.is_array()) diagnostics = json::array();
    if (diagnostics.size() >= 64) return;  // the §7.2 per-step diagnostic budget
    diagnostics.push_back(json{{"severity", "warning"},
                               {"code", "ARTIFACT_TESSELLATE_FAILED"},
                               {"stage", "artifact"},
                               {"message", what.size() <= 4096 ? what : what.substr(0, 4096)}});
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

    // OPTIONAL `baseCheckpoint` (SCHEMA §7.2, WP-H): the plan's claim on the
    // restored-base slot. Read with the same tolerate-malformed rule as
    // `editedFrom` (§4) — a non-object is treated as absent, and absent means
    // "fence against the head / from 0" AND drops any pending restored base.
    std::optional<BaseCheckpointRef> base;
    if (args.contains("baseCheckpoint") && args["baseCheckpoint"].is_object()) {
        BaseCheckpointRef ref;
        ref.step_index = read_u64(args["baseCheckpoint"], "stepIndex");
        ref.checkpoint_id = get_str(args["baseCheckpoint"], "checkpointId");
        base = std::move(ref);
    }

    FenceOutcome fence = session.fence_and_clone(job_id, doc_rev, epoch, expected_base_hash,
                                                 base.has_value() ? &*base : nullptr);
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
    job.resolved_input_evidence = std::move(fence.cloned_input_evidence);
    job.topology_owners = std::move(fence.cloned_topology_owners);
    job.gear_bodies = std::move(fence.cloned_gear_bodies);
    job.prepared_snapshot_id = fence.prepared_snapshot_id;
    // The fence only returns Ok for a `baseCheckpoint` plan when the slot MATCHED,
    // so recording the claim here is recording what was actually consumed (§7.2).
    if (base.has_value()) {
        job.from_restored_base = true;
        job.base_checkpoint_id = base->checkpoint_id;
        job.base_checkpoint_step = base->step_index;
    }
    // SCHEMA §7.3 gear referenceability (WP-I): the plan itself is what makes a
    // body a gear body (`body_<opId>` names a `Gear` op — D1), so the step-input
    // resolution below needs it. Stored, not re-derived: an accept rebuilds the
    // session's map from exactly this object.
    job.plan = json{{"ops", ops}};
    // OPTIONAL `editedFrom` (SCHEMA §7.2). Absence = "no edit context" = no claim;
    // a non-integer is treated as absent rather than as an error, per §4's
    // tolerate-unknown/ignore-malformed-optional reader rule. See §10 for what it
    // gates (the descriptor-tie veto) — nothing else in the plan depends on it.
    if (args.contains("editedFrom") && args["editedFrom"].is_number_unsigned()) {
        job.edited_from = args["editedFrom"].get<std::uint64_t>();
    }
    // OPTIONAL `checkpointFallbackReplay` (SCHEMA §7.2), read with the same
    // tolerate-malformed rule as `editedFrom` above (§4): a non-boolean is treated
    // as absent, and absent means false.
    //
    // The VF-M5 gate. Only Rust can know this: the hazard is a replay rebuilt on a
    // basis a checkpoint restore did NOT reproduce, and such a plan is byte-identical
    // to an ordinary replay-from-0. Deriving it here is what failed before —
    // `partition.size() == 0` is true of EVERY ordinary regen (D5), so the gate
    // degenerated to `editedFrom.is_some()` and turned the shipped edit lane into
    // NeedsRepair (`src-tauri/tests/topology_rebind.rs` H6a).
    //
    // What it gates is exactly one thing (§10): the anchor-exact carve-out in the
    // descriptor-tie veto, which would otherwise bless a congruent decoy parked at
    // the stale anchor. On the ORDINARY edit lane the carve-out stays on and that
    // teleport remains the accepted, documented residual (H6a).
    if (args.contains("checkpointFallbackReplay") &&
        args["checkpointFallbackReplay"].is_boolean()) {
        job.from_zero_replay = args["checkpointFallbackReplay"].get<bool>();
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

    // The tessellation artifact is ADVISORY (SCHEMA §7.2 ARTIFACT_TESSELLATE_FAILED,
    // WP-H): it is attached HERE, before `perStepResults` is serialized, so that a
    // failure can ride as a warning on the last step instead of failing the whole
    // prepared plan. Before WP-H this ran outside any try/catch after the result was
    // built, so one throwing body cost the user a plan that had already succeeded.
    Envelope r = Envelope::ok_response(req.id, json::object());
    r.stamp.job_id = job_id;
    json tess;
    try {
        tess = attach_tessellate(job, artifacts, r);
    } catch (const Standard_Failure& f) {
        const char* msg = f.GetMessageString();  // OCCT 7.9/8.0 common accessor
        note_artifact_failure(job, (msg && *msg) ? msg : "Standard_Failure", r);
        tess = json();
    } catch (const std::exception& ex) {
        note_artifact_failure(job, ex.what(), r);
        tess = json();
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
        if (!ps.diagnostics.empty()) e["diagnostics"] = ps.diagnostics;
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

    // Cache the PlanPrepared JSON for idempotent re-return WITHOUT the tessellate
    // artifacts: the artifact bytes ride in THIS resp's binary tail only, so a
    // re-sent jobId (which returns the cached JSON with no bin) must not reference
    // dangling `mesh:*` sections. The idempotency contract pins
    // preparedSnapshotId/historyPrefixHash/perStepResults — meshes are re-fetchable
    // via Tessellate. The artifact reference is attached to the live resp only.
    job.prepared_result = result;
    if (!tess.is_null()) result["artifacts"] = json{{"tessellate", tess}};
    r.result = std::move(result);  // the live resp references the inlined sections

    // LATE CANCEL (SCHEMA §7.2/§8, WP-H). The op loop polls the token per step, so a
    // cancel raised AFTER the last step — while the terminal was being assembled —
    // used to be ignored: the scratch was stored behind an ok:true PlanPrepared that
    // Rust, having cancelled, never accepted or discarded, and every later
    // ExecutePlan was refused until restart (finding session-executor-4). Re-check
    // here, at the last instant before the scratch is installed, and end with the
    // EXISTING cancelled terminal: nothing stored, session untouched.
    if (ctx.cancel.cancelled()) {
        WLOG_WARN("ExecutePlan job %llu cancelled before store_prepared; nothing stored",
                  static_cast<unsigned long long>(job_id));
        Envelope cancelled = Envelope::error_response(
            req.id, ErrorInfo{"CANCELLED", "plan cancelled", /*retriable=*/false});
        cancelled.stamp.job_id = job_id;
        return cancelled;
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
