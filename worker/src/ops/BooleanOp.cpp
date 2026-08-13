// BooleanOp.cpp — see BooleanOp.h. Ports RegenerationEngine.cpp buildBoolean.
#include "ops/BooleanOp.h"

#include <memory>
#include <optional>

#include <BRepBuilderAPI_MakeShape.hxx>
#include <TopoDS_Shape.hxx>

#include "modeling/BooleanMode.h"
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
}  // namespace

OpOutcome execute_boolean(OpContext& ctx, const json& op, const std::string& op_id) {
    const json params =
        (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();

    const std::string target_id = read_str(params, "targetBodyId");
    const std::string tool_id = read_str(params, "toolBodyId");
    if (params.contains("operation") && !params["operation"].is_string()) {
        return OpOutcome::fail("OP_FAILED", "Boolean operation must be a string");
    }
    const std::optional<app::BooleanMode> mode = mode_of(read_str(params, "operation", "Union"));
    if (!mode) return OpOutcome::fail("OP_FAILED", "Boolean operation is unsupported");

    const session::BodyRecord* target_rec = ctx.bodies.get(target_id);
    if (!target_rec) {
        return OpOutcome::fail("REF_UNRESOLVED", "Boolean target body not found: " + target_id);
    }
    const session::BodyRecord* tool_rec = ctx.bodies.get(tool_id);
    if (!tool_rec || tool_id == target_id) {
        return OpOutcome::fail("REF_UNRESOLVED", "Boolean tool body not found: " + tool_id);
    }

    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    const TopoDS_Shape old_target = target_rec->geom;
    const TopoDS_Shape tool_shape = tool_rec->geom;
    std::shared_ptr<BRepBuilderAPI_MakeShape> builder;
    BooleanResult br = checked_boolean(old_target, tool_shape, *mode, ctx.parallel, ctx.occt_options,
                                       ctx.cancel, builder);
    if (br.error_code == "CANCELLED") return OpOutcome::cancelled();
    if (!br.error_code.empty()) return OpOutcome::fail(br.error_code, br.error_message);
    kernel::validation::PublicationPolicy policy;
    policy.name = "Boolean";
    policy.max_solid_count = -1;
    policy.tier = kernel::validation::PublicationTier::TierB;
    policy.allow_empty_lifecycle = true;
    const kernel::validation::PublicationDecision decision = publication_decision(br.shape, policy);
    if (!decision.publishable() && !decision.lifecycle_only()) {
        return OpOutcome::fail(decision.code, decision.message);
    }

    OpOutcome out;
    // Publish the successor of the target: a single-solid result MODIFIES it in place
    // (BodyId preserved — corpus c invariant); a multi-solid result SPLITS into
    // deterministic children `body_<opId>:<k>` (SCHEMA §2, D1).
    if (publish_boolean_result(ctx, op_id, target_id, br.shape, builder.get(), out) ==
        BooleanPublishResult::Empty) {
        // A recoverable refusal (SCHEMA §7.3): target and tool stay intact and no
        // lifecycle event is emitted. It carries its own diagnostic code because
        // the §8 top-level code is the generic `OP_FAILED` shared with every other
        // Boolean failure, and "your Cut consumed the target completely" is the one
        // the user can actually act on.
        OpOutcome failure = OpOutcome::fail("OP_FAILED", "Boolean produced no solids");
        failure.diagnostics.push_back(
            {{"severity", "error"},
             {"code", "BOOLEAN_EMPTY_RESULT"},
             {"message", "Boolean produced no solids"},
             {"stage", "publish"},
             {"evidence",
              {{"boolean",
                {{"operation", read_str(params, "operation", "Union")},
                 {"targetBodyId", target_id},
                 {"toolBodyId", tool_id},
                 {"solidCount", 0}}}}}});
        return failure;
    }

    // The tool is consumed by the operation: drop its body + partition entries.
    ctx.bodies.erase(tool_id);
    ctx.partition.remove_body(tool_id, out.delta);
    out.body_events.push_back({"deleted", tool_id});

    return out;
}

}  // namespace onecad::ops
