// HoleOp.cpp — see HoleOp.h.
#include "ops/HoleOp.h"

#include <cmath>
#include <memory>
#include <optional>
#include <string>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepClass_FaceClassifier.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRep_Tool.hxx>
#include <GeomAPI_ProjectPointOnSurf.hxx>
#include <Geom_Surface.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_State.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>

#include "elementmap/ElementMapPartition.h"
#include "elementmap/Ladder.h"
#include "kernel/validation/GeometryPrecision.h"
#include "modeling/BooleanMode.h"
#include "ops/ComponentGenerators.h"
#include "ops/HoleTool.h"
#include "ops/OpCommon.h"

namespace onecad::ops {

using nlohmann::json;
namespace em = onecad::elementmap;

namespace {

// The dimension floor now comes from the precision context —
// `GeometryPrecisionContext::authoring_resolution()`. The value is unchanged
// (1e-3 mm, RegenerationEngine.cpp:61 kMinValue); it is no longer restated here.
// SCHEMA §7.3: the frozen `point` is re-projected onto the resolved face and the
// op "fails loudly past 1e-3 mm".
constexpr double kPointPlaneFence = 1e-3;
// Face-boundary classification tolerance (BooleanOperation.cpp:116 precedent).
constexpr double kClassifyTol = 1e-7;
constexpr int kResultPolicyVersion2 = 2;

enum class HoleResultPolicy { Legacy, V2 };

bool read_result_policy(const json& params, HoleResultPolicy& policy,
                        std::string& error) {
    policy = HoleResultPolicy::Legacy;
    if (!params.contains("resultPolicyVersion")) return true;
    const json& value = params["resultPolicyVersion"];
    if (!value.is_number_integer() || value.get<int>() != kResultPolicyVersion2) {
        error = "UNSUPPORTED_HOLE_RESULT_POLICY_VERSION";
        return false;
    }
    policy = HoleResultPolicy::V2;
    return true;
}

// SCHEMA §7.3: `csAngleDeg ∈ {82, 90, 100, 120}`. Re-checked here because the
// worker is an independent trust boundary — a hand-authored plan never passes
// through the Rust session's validator.
constexpr double kCsAngles[] = {82.0, 90.0, 100.0, 120.0};

// A `thread.detail` refusal: a RECOVERABLE `OP_FAILED` whose fine-grained
// reason rides `detail.diagnostics[].reasonCode` (SCHEMA §8) — the SAME
// WP-C `ComponentOp.cpp::profile_refusal` diagnostic shape, not the bare
// {severity,code,message,stage} local shape `read_result_policy` uses above.
OpOutcome thread_detail_refusal(const char* reason_code, const std::string& message) {
    OpOutcome failure = OpOutcome::fail("OP_FAILED", message);
    failure.diagnostics.push_back({{"severity", "error"},
                                   {"code", "OP_FAILED"},
                                   {"message", message},
                                   {"stage", "preflight"},
                                   {"reasonCode", reason_code}});
    return failure;
}

// SCHEMA §7.3 `Hole.thread` (WP-T1): a PRESENT thread's `detail` is the ONLY
// thread field this worker reads — every other field is resolved-fact
// provenance it never consumes (SCHEMA §7.3 the `PlaceComponent`
// `source.params` precedent). "cosmetic" is implemented; "simplified" /
// "modeled" are refused BY NAME rather than silently answered with a plain
// hole, and an unknown or absent `detail` beside a present `thread` is
// likewise refused (never guessed). Reuses `parse_thread_detail`
// (ComponentGenerators.h) — one vocabulary, one parser. Absent `thread` ⇒
// nothing to check (an unthreaded hole; this worker never sees the block).
std::optional<OpOutcome> read_thread_detail(const json& params) {
    if (!params.contains("thread") || !params["thread"].is_object()) return std::nullopt;
    const json& thread = params["thread"];
    const std::string detail_str = read_str(thread, "detail");
    ThreadDetail detail;
    if (!parse_thread_detail(detail_str, detail)) {
        return thread_detail_refusal("HOLE_THREAD_DETAIL_UNKNOWN",
                                     "Hole thread.detail '" + detail_str + "' is unknown");
    }
    if (detail != ThreadDetail::Cosmetic) {
        return thread_detail_refusal("HOLE_THREAD_DETAIL_UNSUPPORTED",
                                     "Hole thread.detail '" + detail_str +
                                         "' is not implemented (only cosmetic)");
    }
    return std::nullopt;
}

// The host body id: params.targetBodyId, else the first whole-BODY input ref.
// A face/edge primary is NOT an acceptable fallback (the ExtrudeOp `input_body`
// hazard: binding a sub-element's body would cut a body the user never named).
std::string target_body_of(const json& op, const json& params) {
    const std::string pid = read_str(params, "targetBodyId");
    if (!pid.empty()) return pid;
    if (op.contains("inputs") && op["inputs"].is_array()) {
        for (const json& in : op["inputs"]) {
            if (in.is_object() && in.contains("primary") && in["primary"].is_object()) {
                if (read_str(in["primary"], "kind") != "body") continue;
                const std::string bid = read_str(in["primary"], "bodyId");
                if (!bid.empty()) return bid;
            }
        }
    }
    return "";
}

// Whether `params` carries a readable scalar under `key` — a bare number OR a
// `{value}` object (SCHEMA §4). `read_scalar`'s default cannot tell an absent
// field from one legitimately set to that default, and `depth` is `null` for
// THROUGH-ALL, so presence is asked separately.
bool has_scalar(const json& params, const char* key) {
    if (!params.is_object() || !params.contains(key)) return false;
    const json& v = params[key];
    return v.is_number() || (v.is_object() && v.contains("value") && v["value"].is_number());
}

// The op's host-FACE semantic ref: the `inputs[]` entry of kind `face`, else the
// typed `params.face` (a hand-authored plan may carry only the latter).
std::optional<em::LadderRef> face_ref_of(const json& op, const json& params,
                                         const std::string& op_id) {
    if (op.contains("inputs") && op["inputs"].is_array()) {
        std::size_t i = 0;
        for (const json& in : op["inputs"]) {
            em::LadderRef r = em::ladder_ref_from_input(in, op_id + ".input" + std::to_string(i));
            ++i;
            if (r.kind == em::km::ElementKind::Face) return r;
        }
    }
    if (params.contains("face") && params["face"].is_object()) {
        em::LadderRef r = em::ladder_ref_from_input(params["face"], op_id + ".face");
        // `params.face` is the host face by construction; a payload that omitted
        // `primary.kind` still means "face" here.
        if (r.kind == em::km::ElementKind::Unknown) r.kind = em::km::ElementKind::Face;
        if (r.kind == em::km::ElementKind::Face) return r;
    }
    return std::nullopt;
}

// Resolve the host face on the predecessor snapshot: partition-tracked first
// (the production path — PlanExecutor's `resolve_input_refs` minted it), then the
// descriptor+anchor ladder. `nr` receives the NeedsRepair payload when neither
// rung binds; the returned face is then null.
TopoDS_Face resolve_host_face(OpContext& ctx, const TopoDS_Shape& target_shape,
                              const std::string& target_id, em::LadderRef ref, json& nr) {
    // The tracked rung is valid ONLY for an entry that belongs to THIS body: a
    // TopoKey is an ordinal in one body's map, so a foreign entry's key would
    // silently seat the drill on the target's Nth face (VF-M7). "" ⇒ fall through.
    const std::string topo_key = em::ElementMapPartition::topokey_for_element_in_body(
        ctx.partition, ref.element_id, target_id);
    if (!topo_key.empty()) {
        const TopoDS_Shape sub =
            em::ElementMapPartition::shape_for_topokey(target_shape, topo_key);
        if (!sub.IsNull() && sub.ShapeType() == TopAbs_FACE) return TopoDS::Face(sub);
    }
    std::vector<em::LadderRef> refs{std::move(ref)};
    const std::vector<em::LadderResolution> res = em::resolve_descriptor_stage(
        target_shape, target_id, refs, em::LadderEditContext{ctx.post_upstream_edit, ctx.from_zero_replay});
    if (!res.empty() && res[0].outcome == em::LadderOutcome::AutoBind &&
        !res[0].bound_shape.IsNull() && res[0].bound_shape.ShapeType() == TopAbs_FACE) {
        return TopoDS::Face(res[0].bound_shape);
    }
    nr = res.empty() ? json::object() : res[0].to_needs_repair_json();
    return {};
}

// Read + range-check every dimensional param for `hole_type`. Returns the reason
// on violation (SCHEMA §7.3 conditional blocks); `dims.depth` is left 0 for
// THROUGH-ALL so the caller can substitute the ray extent.
std::string read_dims(const json& params, const std::string& hole_type, HoleDims& dims) {
    // Dimension validation runs before the target body is resolved, and
    // `authoring_resolution()` is scale-independent in v1 (GeometryPrecision.h), so
    // the floor-only context answers exactly what a measured one would.
    const double min_value =
        kernel::validation::GeometryPrecisionContext{}.authoring_resolution();
    dims.diameter = read_scalar(params, "diameter", 0.0);
    if (dims.diameter < min_value) return "Hole diameter too small";

    // `depth` absent OR json null ⇒ through-all (SCHEMA §7.3).
    if (has_scalar(params, "depth")) {
        dims.depth = read_scalar(params, "depth", 0.0);
        if (dims.depth < min_value) return "Hole depth too small";
    }

    if (hole_type == "counterbore") {
        if (!has_scalar(params, "cbDiameter") || !has_scalar(params, "cbDepth")) {
            return "Hole counterbore requires cbDiameter and cbDepth";
        }
        dims.cb_diameter = read_scalar(params, "cbDiameter", 0.0);
        dims.cb_depth = read_scalar(params, "cbDepth", 0.0);
        if (dims.cb_depth < min_value) return "Hole cbDepth too small";
        if (dims.cb_diameter <= dims.diameter) {
            return "Hole cbDiameter must exceed diameter";
        }
        return "";
    }

    if (hole_type == "countersink") {
        if (!has_scalar(params, "csDiameter") || !has_scalar(params, "csAngleDeg")) {
            return "Hole countersink requires csDiameter and csAngleDeg";
        }
        dims.cs_diameter = read_scalar(params, "csDiameter", 0.0);
        dims.cs_angle_deg = read_scalar(params, "csAngleDeg", 0.0);
        if (dims.cs_diameter <= dims.diameter) {
            return "Hole csDiameter must exceed diameter";
        }
        bool standard = false;
        for (const double a : kCsAngles) {
            if (std::abs(dims.cs_angle_deg - a) < 1e-9) standard = true;
        }
        if (!standard) return "Hole csAngleDeg must be one of 82, 90, 100, 120";
        return "";
    }

    if (hole_type != "simple") {
        return "Hole holeType must be simple, counterbore or countersink";
    }
    return "";
}

}  // namespace

OpOutcome execute_hole(OpContext& ctx, const json& op, const std::string& op_id) {
    const json params =
        (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();
    HoleResultPolicy result_policy;
    std::string policy_error;
    if (!read_result_policy(params, result_policy, policy_error)) {
        const std::string message =
            "Hole resultPolicyVersion is unsupported";
        OpOutcome failure = OpOutcome::fail("OP_FAILED", message);
        failure.diagnostics.push_back({{"severity", "error"},
                                       {"code", policy_error},
                                       {"message", message},
                                       {"stage", "preflight"}});
        return failure;
    }

    // WP-T1: `thread.detail` is a preflight, same rung as `resultPolicyVersion`
    // above — checked before ownership/body resolution, independent of any
    // geometry this op will build.
    if (std::optional<OpOutcome> refusal = read_thread_detail(params)) {
        return std::move(*refusal);
    }

    if (std::vector<json> repairs = operation_ref_ownership_repairs(op, op_id); !repairs.empty()) {
        OpOutcome out;
        out.needs_repair = std::move(repairs);
        return out;
    }

    // --- host body (predecessor snapshot — Invariant 3) ---
    const std::string target_id = target_body_of(op, params);
    if (target_id.empty()) {
        return OpOutcome::fail("OP_FAILED", "Hole requires body input");
    }
    const session::BodyRecord* target_rec = ctx.bodies.get(target_id);
    if (!target_rec) {
        return OpOutcome::fail("REF_UNRESOLVED", "Hole target body not found: " + target_id);
    }
    const TopoDS_Shape target_shape = target_rec->geom;
    if (auto invalid = validate_modeling_body(*target_rec, "Hole", "target")) return *invalid;

    // --- dimensional params (worker is an independent trust boundary) ---
    const std::string hole_type = read_str(params, "holeType", "simple");
    HoleDims dims;
    if (const std::string reason = read_dims(params, hole_type, dims); !reason.empty()) {
        return OpOutcome::fail("OP_FAILED", reason);
    }

    // --- host face ---
    std::optional<em::LadderRef> ref = face_ref_of(op, params, op_id);
    if (!ref) {
        return OpOutcome::fail("OP_FAILED", "Hole requires a face input");
    }
    OpOutcome out;
    json nr;
    const TopoDS_Face face = resolve_host_face(ctx, target_shape, target_id, std::move(*ref), nr);
    if (face.IsNull()) {
        // Host face no longer resolves ⇒ NeedsRepair STATE (never a wrong bind).
        out.needs_repair.push_back(nr);
        return out;  // status Ok + needsRepair ⇒ PlanExecutor prepares m−1
    }

    gp_Pln plane;
    gp_Dir outward;
    if (!planar_face_plane_normal(face, plane, outward)) {
        return OpOutcome::fail("OP_FAILED", "Hole face is not planar (a hole needs a planar seat)");
    }

    // --- re-project the frozen point onto the face, then fence it ---
    if (!params.contains("point") || !params["point"].is_array() || params["point"].size() != 3) {
        return OpOutcome::fail("OP_FAILED", "Hole point must be a [x, y, z] world point");
    }
    for (const json& c : params["point"]) {
        if (!c.is_number()) return OpOutcome::fail("OP_FAILED", "Hole point must be numeric");
    }
    const gp_Pnt frozen(params["point"][0].get<double>(), params["point"][1].get<double>(),
                        params["point"][2].get<double>());

    gp_Pnt origin;
    try {
        const Handle(Geom_Surface) surf = BRep_Tool::Surface(face);
        GeomAPI_ProjectPointOnSurf proj(frozen, surf);
        if (!proj.IsDone() || proj.NbPoints() < 1) {
            return OpOutcome::fail("OP_FAILED", "Hole point could not be projected onto the face");
        }
        if (proj.LowerDistance() > kPointPlaneFence) {
            return OpOutcome::fail("OP_FAILED",
                                   "Hole point is off the resolved face plane (fence 1e-3 mm)");
        }
        origin = proj.NearestPoint();
        Standard_Real u = 0.0, v = 0.0;
        proj.LowerDistanceParameters(u, v);
        // Re-projecting onto the PLANE is not enough: the point must land inside
        // the face's boundary, else the drill would start beside the material.
        BRepClass_FaceClassifier classifier(face, gp_Pnt2d(u, v), kClassifyTol);
        if (classifier.State() == TopAbs_OUT) {
            return OpOutcome::fail("OP_FAILED",
                                   "Hole point lies outside the resolved face boundary");
        }
    } catch (const Standard_Failure& f) {
        return OpOutcome::fail("OP_FAILED",
                               std::string("Hole point could not be resolved on the face: ") +
                                   (f.GetMessageString() ? f.GetMessageString() : "OCCT"));
    }

    // SCHEMA §7.3: axis = the face's INWARD normal at `point`.
    const gp_Dir axis = outward.Reversed();

    // THROUGH-ALL: bound the drill by the host's own extent (clamped = through).
    // A blind depth that already overshoots the extent is FINE (SCHEMA §7.3) —
    // it simply becomes a through hole; nothing here clamps it back.
    if (dims.depth <= 0.0) {
        dims.depth = through_all_length(origin, axis, target_shape);
    }

    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    // --- tool solid (drill + the conditional cb cylinder / cs cone), fused ---
    std::string tool_err;
    const TopoDS_Shape tool = build_hole_tool(origin, axis, dims, tool_err);
    if (tool.IsNull()) {
        return OpOutcome::fail("OP_FAILED", tool_err.empty() ? "Hole tool solid is null" : tool_err);
    }

    // --- ONE cut against the host, builder kept alive for history ---
    std::shared_ptr<BRepBuilderAPI_MakeShape> builder;
    BooleanResult br = checked_boolean(target_shape, tool, app::BooleanMode::Cut, ctx.parallel,
                                       ctx.occt_options, ctx.cancel, builder);
    if (br.error_code == "CANCELLED") return OpOutcome::cancelled();
    if (!br.error_code.empty()) {
        return OpOutcome::fail(br.error_code, "Hole cut failed: " + br.error_message);
    }
    const kernel::validation::PublicationTier validation_tier =
        result_validation_tier(ctx, kernel::validation::PublicationTier::TierB);
    kernel::validation::PublicationPolicy policy;
    if (result_policy == HoleResultPolicy::V2) {
        policy = kernel::validation::single_solid_policy("Hole", validation_tier);
    } else {
        policy.name = "Hole legacy";
        policy.allowed_top_level_shapes =
            kernel::validation::TopLevelShapePolicy::SolidSet;
        policy.max_solid_count = -1;
        policy.tier = validation_tier;
        policy.require_closed_manifold =
            validation_tier == kernel::validation::PublicationTier::TierB;
    }
    // WP-E: tolerance budget grown from the host, as Extrude's boolean does.
    {
        const auto prec = kernel::validation::precision_of(target_shape);
        policy.maximum_tolerance = prec.tolerance_ceiling(prec.input_tolerance, 2.0, 1.0e-6);
    }
    const kernel::validation::PublicationDecision decision =
        publication_decision(br.shape, policy, ctx.cancel);
    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
    if (!decision.publishable()) {
        const std::size_t solid_count = ordered_solids(br.shape).size();
        if (solid_count == 0) {
            return OpOutcome::fail("OP_FAILED", "Hole removed the entire host body");
        }
        if (result_policy == HoleResultPolicy::V2 && solid_count > 1) {
            const std::string message =
                "Hole V2 would disconnect the host into " +
                std::to_string(solid_count) + " solids";
            OpOutcome failure = OpOutcome::fail("OP_FAILED", message);
            failure.diagnostics.push_back({{"severity", "error"},
                                           {"code", "HOLE_DISJOINT_RESULT"},
                                           {"message", message},
                                           {"stage", "publication"},
                                           {"evidence", {{"solidCount", solid_count}}}});
            return failure;
        }
        return publication_refusal(decision, "publication");
    }

    // --- publish the MODIFIED host (id preserved) + rebind partition via history ---
    // Deliberately NOT `publish_boolean_result`: SCHEMA §7.3 pins Hole to modify
    // lineage with "nothing minted". Even in the pathological case where a through
    // hole severs a bridge, the result stays ONE body — splitting it would mint
    // `body_<opId>:<k>` children the contract forbids.
    ctx.bodies.create(target_id, op_id, br.shape);
    if (builder) {
        ctx.partition.apply_history(target_id, br.shape, *builder, out.delta, &out.needs_repair);
    }
    out.body_events.push_back({"modified", target_id, {}});
    out.body_ids.push_back(target_id);
    return out;
}

}  // namespace onecad::ops
