// PatternOp.cpp — see PatternOp.h. Ports buildLinearPattern / buildCircularPattern.
#include "ops/PatternOp.h"

#include <cmath>
#include <functional>

#include <BRep_Builder.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <Standard_Failure.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax1.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include "ops/OpCommon.h"

namespace onecad::ops {

using nlohmann::json;

namespace {

constexpr int kMaxPatternCount = 128;
constexpr int kResultPolicyVersion2 = 2;

enum class PatternResultPolicy { Legacy, V2 };

// Read a Vec3 param serialized as the JSON array `[x, y, z]` (core `Vec3` wire form).
bool read_vec3(const json& params, const char* key, double& x, double& y, double& z) {
    if (!params.is_object() || !params.contains(key)) return false;
    const json& v = params[key];
    if (!v.is_array() || v.size() != 3) return false;
    if (!v[0].is_number() || !v[1].is_number() || !v[2].is_number()) return false;
    x = v[0].get<double>();
    y = v[1].get<double>();
    z = v[2].get<double>();
    return std::isfinite(x) && std::isfinite(y) && std::isfinite(z);
}

bool read_count(const json& params, int& count, std::string& error) {
    double value = 0.0;
    if (!read_scalar_strict(params, "count", 0.0, value, error)) return false;
    if (std::floor(value) != value || value < 2.0 || value > kMaxPatternCount) {
        error = "Pattern count must be an integer from 2 to " + std::to_string(kMaxPatternCount);
        return false;
    }
    count = static_cast<int>(value);
    return true;
}

bool read_result_policy(const json& params, PatternResultPolicy& policy, std::string& error) {
    policy = PatternResultPolicy::Legacy;
    if (!params.contains("resultPolicyVersion")) return true;
    const json& value = params["resultPolicyVersion"];
    if (!value.is_number_integer() || value.get<int>() != kResultPolicyVersion2) {
        error = "Pattern resultPolicyVersion must be 2";
        return false;
    }
    policy = PatternResultPolicy::V2;
    return true;
}

// The shared pattern replay: source ⊕ (count−1) transformed instances, fused into one
// solid or gathered into a compound. `xform(i)` is the gp_Trsf for instance i∈[1,count).
OpOutcome build_pattern(OpContext& ctx, const json& op, const std::string& op_id,
                        const char* op_name, int count, bool fuse_result,
                        PatternResultPolicy result_policy,
                        const std::function<gp_Trsf(int)>& xform) {
    const json params =
        (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();

    const std::string source_id = read_str(params, "sourceBodyId");
    if (source_id.empty()) {
        return OpOutcome::fail("OP_FAILED", std::string(op_name) + " requires a source body");
    }
    const session::BodyRecord* source_rec = ctx.bodies.get(source_id);
    if (!source_rec || source_rec->geom.IsNull()) {
        return OpOutcome::fail("REF_UNRESOLVED",
                               std::string(op_name) + " source body not found: " + source_id);
    }
    const TopoDS_Shape source = source_rec->geom;
    if (result_policy == PatternResultPolicy::V2) {
        const kernel::validation::PublicationDecision source_decision = publication_decision(
            source, kernel::validation::single_solid_policy(
                        std::string(op_name) + " v2 source",
                        kernel::validation::PublicationTier::TierA));
        if (!source_decision.publishable()) {
            return OpOutcome::fail(source_decision.code, source_decision.message);
        }
    }

    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
    OpOutcome out;

    if (result_policy == PatternResultPolicy::V2 && !fuse_result) {
        for (int i = 1; i < count; ++i) {
            if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
            BRepBuilderAPI_Transform xf(source, xform(i), Standard_True);
            if (!xf.IsDone() || xf.Shape().IsNull()) {
                return OpOutcome::fail("OP_FAILED", std::string(op_name) +
                                                        " transform failed at instance " +
                                                        std::to_string(i));
            }
            const kernel::validation::PublicationDecision decision = publication_decision(
                xf.Shape(), kernel::validation::single_solid_policy(
                                std::string(op_name) + " v2 child",
                                kernel::validation::PublicationTier::TierA));
            if (!decision.publishable()) return OpOutcome::fail(decision.code, decision.message);
            const std::string bid = "body_" + op_id + ":" + std::to_string(i - 1);
            ctx.bodies.create(bid, op_id, xf.Shape());
            out.body_events.push_back({"created", bid});
            out.body_ids.push_back(bid);
        }
        return out;
    }

    TopoDS_Shape result;
    try {
        result = source;  // legacy: the result INCLUDES the source geometry
        TopoDS_Compound compound;
        BRep_Builder cbuilder;
        if (!fuse_result) {
            cbuilder.MakeCompound(compound);
            cbuilder.Add(compound, source);
        }
        for (int i = 1; i < count; ++i) {
            if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
            BRepBuilderAPI_Transform xf(source, xform(i), Standard_True);
            if (!xf.IsDone() || xf.Shape().IsNull()) {
                return OpOutcome::fail("OP_FAILED", std::string(op_name) +
                                                        " transform failed at instance " +
                                                        std::to_string(i));
            }
            if (fuse_result) {
                BRepAlgoAPI_Fuse fuse(result, xf.Shape());
                fuse.Build();
                if (!fuse.IsDone() || fuse.Shape().IsNull()) {
                    return OpOutcome::fail("OP_FAILED", std::string(op_name) +
                                                           " fuse failed at instance " +
                                                           std::to_string(i));
                }
                result = fuse.Shape();
                if (result_policy == PatternResultPolicy::V2) {
                    ctx.partition.apply_history(source_id, result, fuse, out.delta,
                                                &out.needs_repair);
                }
            } else {
                cbuilder.Add(compound, xf.Shape());
            }
        }
        if (!fuse_result) result = compound;
    } catch (const Standard_Failure& f) {
        return OpOutcome::fail("OP_FAILED",
                               std::string(op_name) + " raised: " +
                                   (f.GetMessageString() ? f.GetMessageString() : "OCCT"));
    } catch (...) {
        return OpOutcome::fail("OP_FAILED", std::string(op_name) + " raised an unknown exception");
    }

    if (result.IsNull()) {
        return OpOutcome::fail("GEOMETRY_INVALID", std::string(op_name) + " produced null shape");
    }
    if (fuse_result) {
        const kernel::validation::PublicationDecision decision = publication_decision(
            result, kernel::validation::single_solid_policy(
                        std::string(op_name) + " fused result",
                        kernel::validation::PublicationTier::TierB));
        if (!decision.publishable()) {
            if (ordered_solids(result).size() > 1) {
                return OpOutcome::fail("PATTERN_DISJOINT_RESULT",
                                       std::string(op_name) + " fused result is disconnected");
            }
            return OpOutcome::fail(decision.code, decision.message);
        }
    }

    if (result_policy == PatternResultPolicy::V2) {
        // V2 fused lineage: the source is instance zero, so it is MODIFIED in place.
        // Its per-face colors cannot be mapped through a composed fuse history; drop
        // them rather than attach source face indices to unrelated result faces.
        ctx.bodies.create(source_id, op_id, result);
        out.body_events.push_back({"modified", source_id});
        out.body_ids.push_back(source_id);
        return out;
    }
    const std::string bid = "body_" + op_id;
    ctx.bodies.create(bid, op_id, result);
    out.body_events.push_back({"created", bid});
    out.body_ids.push_back(bid);
    return out;
}

}  // namespace

OpOutcome execute_linear_pattern(OpContext& ctx, const json& op, const std::string& op_id) {
    const json params =
        (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();

    int count = 0;
    std::string error;
    if (!read_count(params, count, error)) return OpOutcome::fail("OP_FAILED", error);
    PatternResultPolicy result_policy;
    if (!read_result_policy(params, result_policy, error)) return OpOutcome::fail("OP_FAILED", error);
    double spacing = 0.0;
    if (!read_scalar_strict(params, "spacing", 0.0, spacing, error)) {
        return OpOutcome::fail("OP_FAILED", error);
    }
    bool fuse_result = true;
    if (!read_bool_strict(params, "fuseResult", true, fuse_result, error)) {
        return OpOutcome::fail("OP_FAILED", error);
    }

    double dx = 0.0, dy = 0.0, dz = 0.0;
    if (!read_vec3(params, "direction", dx, dy, dz)) {
        return OpOutcome::fail("OP_FAILED", "LinearPattern: missing direction vector");
    }
    if (std::abs(spacing) < 1e-9) {
        return OpOutcome::fail("OP_FAILED", "LinearPattern spacing must be non-zero");
    }
    const double len = std::sqrt(dx * dx + dy * dy + dz * dz);
    if (len < 1e-10) {
        return OpOutcome::fail("OP_FAILED", "LinearPattern direction vector is zero");
    }
    const double nx = dx / len, ny = dy / len, nz = dz / len;

    return build_pattern(ctx, op, op_id, "LinearPattern", count, fuse_result, result_policy,
                         [&](int i) {
        gp_Trsf t;
        t.SetTranslation(gp_Vec(nx * spacing * i, ny * spacing * i, nz * spacing * i));
        return t;
    });
}

OpOutcome execute_circular_pattern(OpContext& ctx, const json& op, const std::string& op_id) {
    const json params =
        (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();

    int count = 0;
    std::string error;
    if (!read_count(params, count, error)) return OpOutcome::fail("OP_FAILED", error);
    PatternResultPolicy result_policy;
    if (!read_result_policy(params, result_policy, error)) return OpOutcome::fail("OP_FAILED", error);
    double angle_deg = 360.0;
    if (!read_scalar_strict(params, "angleDeg", 360.0, angle_deg, error)) {
        return OpOutcome::fail("OP_FAILED", error);
    }
    if (std::abs(angle_deg) < 1e-9 || std::abs(angle_deg) > 360.0) {
        return OpOutcome::fail("OP_FAILED",
                               "CircularPattern angleDeg must be non-zero and within [-360, 360]");
    }
    bool fuse_result = true;
    if (!read_bool_strict(params, "fuseResult", true, fuse_result, error)) {
        return OpOutcome::fail("OP_FAILED", error);
    }

    double ox = 0.0, oy = 0.0, oz = 0.0;
    double ax = 0.0, ay = 0.0, az = 1.0;
    if (!read_vec3(params, "axisOrigin", ox, oy, oz) ||
        !read_vec3(params, "axisDirection", ax, ay, az)) {
        return OpOutcome::fail("OP_FAILED", "CircularPattern: missing axis origin/direction");
    }
    if (ax * ax + ay * ay + az * az < 1e-20) {
        return OpOutcome::fail("OP_FAILED", "CircularPattern axis direction is zero");
    }
    gp_Ax1 axis(gp_Pnt(ox, oy, oz), gp_Dir(ax, ay, az));
    // Legacy stepAngle = (angleDeg / count) — divides by count (not count−1); parity.
    const double step_rad = (angle_deg / count) * M_PI / 180.0;

    return build_pattern(ctx, op, op_id, "CircularPattern", count, fuse_result, result_policy,
                         [&](int i) {
        gp_Trsf t;
        t.SetRotation(axis, step_rad * i);
        return t;
    });
}

}  // namespace onecad::ops
