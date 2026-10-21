// BooleanOp.cpp — see BooleanOp.h. Ports RegenerationEngine.cpp buildBoolean.
#include "ops/BooleanOp.h"

#include <algorithm>
#include <memory>
#include <optional>
#include <set>
#include <vector>

#include <BRepBuilderAPI_MakeShape.hxx>
#include <TopoDS_Shape.hxx>

#include "modeling/BooleanMode.h"
#include "kernel/validation/GeometryPrecision.h"
#include "ops/BooleanParticipation.h"
#include "ops/OpCommon.h"

namespace onecad::ops {

using nlohmann::json;

namespace {
std::optional<app::BooleanMode> mode_of(const std::string& op) {
    if (op == "Cut") return app::BooleanMode::Cut;
    if (op == "Intersect") return app::BooleanMode::Intersect;
    if (op == "Union") return app::BooleanMode::Add;
    return std::nullopt;
}

// The authored tool set. Legacy (`toolBodyId`) is one consumed tool; the array
// form (`toolBodyIds` + optional `keepTools`) is everything else (SCHEMA §7.3).
struct ToolSet {
    std::vector<std::string> ids;
    bool keep = false;
    bool array_form = false;
};

std::optional<OpOutcome> parse_tools(const json& params, const std::string& target_id,
                                     ToolSet& out) {
    const bool has_single = params.contains("toolBodyId");
    if (!params.contains("toolBodyIds")) {
        // Legacy, byte-preserved: a missing or self tool resolves as before.
        if (params.contains("keepTools")) {
            return OpOutcome::fail("OP_FAILED", "Boolean keepTools requires toolBodyIds");
        }
        out.ids = {read_str(params, "toolBodyId")};
        return std::nullopt;
    }
    if (has_single) {
        return OpOutcome::fail("OP_FAILED", "Boolean carries both toolBodyId and toolBodyIds");
    }
    out.array_form = true;
    std::string error;
    if (!read_string_array_strict(params, "toolBodyIds", out.ids, error) ||
        !read_bool_strict(params, "keepTools", false, out.keep, error)) {
        return OpOutcome::fail("OP_FAILED", "Boolean " + error);
    }
    if (out.ids.empty()) {
        return OpOutcome::fail("OP_FAILED", "Boolean requires at least one tool body");
    }
    if (out.ids.size() > kBooleanMaxTools) {
        return OpOutcome::fail("OP_FAILED", "Boolean supports at most " +
                                                std::to_string(kBooleanMaxTools) + " tool bodies");
    }
    std::set<std::string> seen;
    for (const std::string& id : out.ids) {
        if (id == target_id) return OpOutcome::fail("OP_FAILED", "Boolean tool " + id + " is the target body");
        if (!seen.insert(id).second) {
            return OpOutcome::fail("OP_FAILED", "Boolean tools contain a duplicate body " + id);
        }
    }
    return std::nullopt;
}

// Evidence keys naming the tool set: legacy `toolBodyId`, else the array + flag.
json tools_evidence(const ToolSet& tools) {
    if (!tools.array_form) return json{{"toolBodyId", tools.ids.front()}};
    return json{{"toolBodyIds", tools.ids}, {"keepTools", tools.keep}};
}

OpOutcome boolean_refusal(const char* code, const std::string& message, json evidence) {
    OpOutcome failure = OpOutcome::fail("OP_FAILED", message);
    failure.diagnostics.push_back({{"severity", "error"},
                                   {"code", code},
                                   {"message", message},
                                   {"stage", "publish"},
                                   {"evidence", json{{"boolean", std::move(evidence)}}}});
    return failure;
}

// target ∩ t0 ∩ t1 ∩ … : OCCT's group COMMON is target ∩ (∪ tools), so the tools
// are pre-combined in authored order (aux builders discarded) and the caller runs
// ONE final builder for the target history. `combined` is left solid-free when
// the tools share no common solid (the caller refuses).
std::optional<OpOutcome> precombine_intersect(const OpContext& ctx,
                                              const std::vector<TopoDS_Shape>& tools,
                                              TopoDS_Shape& combined) {
    combined = tools.front();
    for (std::size_t i = 1; i < tools.size(); ++i) {
        std::shared_ptr<BRepBuilderAPI_MakeShape> aux;
        BooleanResult step = checked_boolean(combined, tools[i], app::BooleanMode::Intersect,
                                             ctx.parallel, ctx.occt_options, ctx.cancel, aux);
        if (step.error_code == "CANCELLED") return OpOutcome::cancelled();
        if (!step.error_code.empty()) return OpOutcome::fail(step.error_code, step.error_message);
        combined = step.shape;
        if (ordered_solids(combined).empty()) break;
    }
    return std::nullopt;
}

// Array-form Union: every result solid must hold target material (SCHEMA §7.3).
std::optional<OpOutcome> union_disjoint_policy(BRepBuilderAPI_MakeShape& builder,
                                               const TopoDS_Shape& target,
                                               const TopoDS_Shape& result,
                                               const std::vector<TopoDS_Shape>& tool_shapes,
                                               const ToolSet& tools, const std::string& target_id) {
    const UnionAnchoring anchoring = union_anchoring(builder, result, target, tool_shapes);
    if (anchoring.unanchored_solids == 0) return std::nullopt;
    json evidence = tools_evidence(tools);
    json disjoint = json::array();
    for (std::size_t t : anchoring.disjoint_tools) disjoint.push_back(tools.ids[t]);
    evidence["operation"] = "Add";
    evidence["targetBodyId"] = target_id;
    evidence["targetSolids"] = static_cast<int>(ordered_solids(target).size());
    evidence["solidCount"] = static_cast<int>(ordered_solids(result).size());
    evidence["disjointToolBodyIds"] = std::move(disjoint);
    return boolean_refusal("BOOLEAN_DISJOINT_RESULT",
                           "Boolean Add: a tool does not touch the target body", evidence);
}

// Array-form Union/Cut: a tool that leaves no face of its own in the result is
// refused whole-op, never consumed silently (SCHEMA §7.3 BOOLEAN_TOOL_NO_EFFECT).
std::optional<OpOutcome> tool_effect_policy(BRepBuilderAPI_MakeShape& builder,
                                            const TopoDS_Shape& target, const TopoDS_Shape& result,
                                            const std::vector<TopoDS_Shape>& tool_shapes,
                                            const ToolSet& tools, const std::string& target_id,
                                            const std::string& operation) {
    const std::optional<std::size_t> idle =
        first_tool_without_effect(builder, result, target, tool_shapes);
    if (!idle) return std::nullopt;
    json evidence = tools_evidence(tools);
    evidence["operation"] = operation;
    evidence["targetBodyId"] = target_id;
    evidence["noEffectToolBodyId"] = tools.ids[*idle];
    evidence["toolIndex"] = static_cast<int>(*idle);
    return boolean_refusal("BOOLEAN_TOOL_NO_EFFECT",
                           "Boolean " + operation + ": tool " + tools.ids[*idle] +
                               " has no effect on the result",
                           evidence);
}

double tool_group_tolerance(const TopoDS_Shape& target, const std::vector<TopoDS_Shape>& tools) {
    // WP-E: the budget grows from the WORST input; a committed tool may carry an
    // imported tolerance (vendor STEP solids sit at 1e-3..1e-2 mm).
    const auto prec = kernel::validation::precision_of(target);
    double worst = prec.input_tolerance;
    for (const TopoDS_Shape& tool : tools) {
        worst = std::max(worst, kernel::validation::precision_of(tool).input_tolerance);
    }
    return prec.tolerance_ceiling(worst, 2.0, 1.0e-6);
}
}  // namespace

OpOutcome execute_boolean(OpContext& ctx, const json& op, const std::string& op_id) {
    const json params =
        (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();

    const std::string target_id = read_str(params, "targetBodyId");
    if (params.contains("operation") && !params["operation"].is_string()) {
        return OpOutcome::fail("OP_FAILED", "Boolean operation must be a string");
    }
    const std::string operation = read_str(params, "operation", "Union");
    const std::optional<app::BooleanMode> mode = mode_of(operation);
    if (!mode) return OpOutcome::fail("OP_FAILED", "Boolean operation is unsupported");
    ToolSet tools;
    if (auto invalid = parse_tools(params, target_id, tools)) return *invalid;

    const session::BodyRecord* target_rec = ctx.bodies.get(target_id);
    if (!target_rec) {
        return OpOutcome::fail("REF_UNRESOLVED", "Boolean target body not found: " + target_id);
    }
    std::vector<const session::BodyRecord*> tool_recs;
    for (const std::string& tool_id : tools.ids) {
        const session::BodyRecord* rec = ctx.bodies.get(tool_id);
        if (!rec || tool_id == target_id) {
            return OpOutcome::fail("REF_UNRESOLVED", "Boolean tool body not found: " + tool_id);
        }
        tool_recs.push_back(rec);
    }

    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    const TopoDS_Shape old_target = target_rec->geom;
    if (auto invalid = validate_modeling_body(*target_rec, "Boolean", "target")) return *invalid;
    std::vector<TopoDS_Shape> tool_shapes;
    for (const session::BodyRecord* rec : tool_recs) {
        if (auto invalid = validate_modeling_body(*rec, "Boolean", "tool")) return *invalid;
        tool_shapes.push_back(rec->geom);
    }
    const json evidence_tools = tools_evidence(tools);

    std::vector<TopoDS_Shape> kernel_tools = tool_shapes;
    if (*mode == app::BooleanMode::Intersect && tool_shapes.size() > 1) {
        TopoDS_Shape combined;
        if (auto failure = precombine_intersect(ctx, tool_shapes, combined)) return *failure;
        if (ordered_solids(combined).empty()) {
            json evidence = evidence_tools;
            evidence["operation"] = operation;
            evidence["targetBodyId"] = target_id;
            evidence["solidCount"] = 0;
            evidence["precombine"] = true;
            return boolean_refusal("BOOLEAN_EMPTY_RESULT",
                                   "Boolean Intersect: the tools share no common solid", evidence);
        }
        kernel_tools = {combined};
    }
    std::shared_ptr<BRepBuilderAPI_MakeShape> builder;
    BooleanResult br = checked_boolean(old_target, kernel_tools, *mode, ctx.parallel,
                                       ctx.occt_options, ctx.cancel, builder);
    if (br.error_code == "CANCELLED") return OpOutcome::cancelled();
    if (!br.error_code.empty()) return OpOutcome::fail(br.error_code, br.error_message);
    // A Union whose tool never touched the target is a REFUSAL (WP-C), never a
    // split that retires the target id. The empty Cut/Intersect case keeps its
    // fixture-pinned BOOLEAN_EMPTY_RESULT shape below.
    if (*mode == app::BooleanMode::Add) {
        auto refusal = tools.array_form
                           ? union_disjoint_policy(*builder, old_target, br.shape, tool_shapes,
                                                   tools, target_id)
                           : boolean_result_policy(*mode, old_target, br.shape, "Boolean",
                                                   target_id, "BOOLEAN_DISJOINT_RESULT",
                                                   "BOOLEAN_EMPTY_RESULT", evidence_tools);
        if (refusal) return *refusal;
    }
    kernel::validation::PublicationPolicy policy;
    policy.name = "Boolean";
    policy.allowed_top_level_shapes = kernel::validation::TopLevelShapePolicy::SolidSet;
    policy.max_solid_count = -1;
    policy.tier = result_validation_tier(
        ctx, kernel::validation::PublicationTier::TierB);
    policy.require_closed_manifold =
        policy.tier == kernel::validation::PublicationTier::TierB;
    policy.maximum_tolerance = tool_group_tolerance(old_target, tool_shapes);
    policy.allow_empty_lifecycle = true;
    const kernel::validation::PublicationDecision decision =
        publication_decision(br.shape, policy, ctx.cancel);
    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
    if (!decision.publishable() && !decision.lifecycle_only()) {
        return publication_refusal(decision, "publication");
    }

    // UX-2026-09-14 WP-1 (D2/A-3): see `ExtrudeOp.cpp` — every subtracting tail
    // shares the one predicate, after the publication decision for the same reason.
    // Scoped to `Cut` inside the policy, so Union / Intersect are untouched.
    if (auto refusal = cut_effect_policy(*mode, old_target, br.shape, "Boolean", target_id,
                                         evidence_tools)) {
        return *refusal;
    }
    if (tools.array_form && *mode != app::BooleanMode::Intersect) {
        if (auto refusal = tool_effect_policy(*builder, old_target, br.shape, tool_shapes, tools,
                                              target_id, operation)) {
            return *refusal;
        }
    }

    OpOutcome out;
    // Publish the successor of the target: a single-solid result MODIFIES it in place
    // (BodyId preserved — corpus c invariant); a multi-solid result SPLITS into
    // deterministic children `body_<opId>:<k>` (SCHEMA §2, D1).
    if (publish_boolean_result(ctx, op_id, target_id, br.shape, builder.get(), out) ==
        BooleanPublishResult::Empty) {
        // A recoverable refusal (SCHEMA §7.3): target and tools stay intact and no
        // lifecycle event is emitted. It carries its own diagnostic code because
        // the §8 top-level code is the generic `OP_FAILED` shared with every other
        // Boolean failure, and "your Cut consumed the target completely" is the one
        // the user can actually act on.
        json evidence = evidence_tools;
        evidence["operation"] = operation;
        evidence["targetBodyId"] = target_id;
        evidence["solidCount"] = 0;
        return boolean_refusal("BOOLEAN_EMPTY_RESULT", "Boolean produced no solids", evidence);
    }

    // Consumed tools drop their body + partition entries, in authored order. A
    // retained tool set emits nothing: no event, no partition change.
    if (!tools.keep) {
        for (const std::string& tool_id : tools.ids) {
            ctx.bodies.erase(tool_id);
            ctx.partition.remove_body(tool_id, out.delta);
            out.body_events.push_back({"deleted", tool_id});
        }
    }
    return out;
}

}  // namespace onecad::ops
