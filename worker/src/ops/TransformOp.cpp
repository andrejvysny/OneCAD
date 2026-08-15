// TransformOp.cpp — see TransformOp.h.
#include "ops/TransformOp.h"

#include <cmath>
#include <string>
#include <vector>

#include <BRepBuilderAPI_Transform.hxx>
#include <Standard_Failure.hxx>
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

constexpr double kPi = 3.14159265358979323846;
// Below this an axis is treated as degenerate (squared length; matches MirrorOp).
constexpr double kMinAxisLen2 = 1e-20;
constexpr std::size_t kMaxTransformTargets = 128;

bool read_scalar_value(const json& value, double& result) {
    if (value.is_number()) {
        result = value.get<double>();
    } else if (value.is_object() && value.contains("value") && value["value"].is_number()) {
        result = value["value"].get<double>();
    } else {
        return false;
    }
    return std::isfinite(result);
}

bool read_translate(const json& params, double& x, double& y, double& z, std::string& err) {
    if (!params.contains("translate")) return true;
    const json& values = params["translate"];
    if (!values.is_array() || values.size() != 3 || !read_scalar_value(values[0], x) ||
        !read_scalar_value(values[1], y) || !read_scalar_value(values[2], z)) {
        err = "TransformBody translate must be a finite 3-vector";
        return false;
    }
    return true;
}

bool read_vec3(const json& holder, const char* key, double& x, double& y, double& z,
               std::string& err) {
    if (!holder.contains(key)) return true;
    const json& v = holder[key];
    if (!v.is_array() || v.size() != 3 || !v[0].is_number() || !v[1].is_number() ||
        !v[2].is_number()) {
        err = std::string("TransformBody rotate.") + key + " must be a finite 3-vector";
        return false;
    }
    x = v[0].get<double>();
    y = v[1].get<double>();
    z = v[2].get<double>();
    if (!std::isfinite(x) || !std::isfinite(y) || !std::isfinite(z)) {
        err = std::string("TransformBody rotate.") + key + " must be a finite 3-vector";
        return false;
    }
    return true;
}

// The parsed op params.
struct Placement {
    std::vector<std::string> targets;
    gp_Trsf trsf;
    bool copy = false;
};

// Parses + validates `params` into a `Placement`. On failure fills `err` with the
// §8 message (always recoverable OP_FAILED — a bad placement is user data).
bool parse_placement(const json& params, Placement& out, std::string& err) {
    if (!params.contains("targets") || !params["targets"].is_array() ||
        params["targets"].empty()) {
        err = "TransformBody requires a non-empty targets array";
        return false;
    }
    if (params["targets"].size() > kMaxTransformTargets) {
        err = "TransformBody targets exceed maximum of " + std::to_string(kMaxTransformTargets);
        return false;
    }
    for (const json& t : params["targets"]) {
        if (!t.is_string() || t.get<std::string>().empty()) {
            err = "TransformBody targets must be non-empty body id strings";
            return false;
        }
        const std::string id = t.get<std::string>();
        for (const std::string& seen : out.targets) {
            if (seen == id) {
                err = "TransformBody targets contain a duplicate body " + id;
                return false;
            }
        }
        out.targets.push_back(id);
    }

    double tx = 0.0, ty = 0.0, tz = 0.0;
    if (!read_translate(params, tx, ty, tz, err)) return false;

    double cx = 0.0, cy = 0.0, cz = 0.0;
    double ax = 0.0, ay = 0.0, az = 1.0;
    double angle_deg = 0.0;
    if (params.contains("rotate")) {
        if (!params["rotate"].is_object()) {
            err = "TransformBody rotate must be an object";
            return false;
        }
        const json& rot = params["rotate"];
        if (!read_vec3(rot, "center", cx, cy, cz, err) ||
            !read_vec3(rot, "axis", ax, ay, az, err) ||
            !read_scalar_strict(rot, "angleDeg", 0.0, angle_deg, err)) {
            if (err.rfind("angleDeg", 0) == 0) err = "TransformBody rotate." + err;
            return false;
        }
    }
    const double axis_len2 = ax * ax + ay * ay + az * az;
    if (angle_deg != 0.0 && axis_len2 < kMinAxisLen2) {
        err = "TransformBody rotate.axis must be non-zero when angleDeg != 0";
        return false;
    }

    // NORMATIVE ORDER: X' = T ∘ R. gp_Trsf composition applies the RIGHT operand
    // first, so `translation * rotation` rotates about the pivot, then translates.
    gp_Trsf rotation;
    if (angle_deg != 0.0) {
        rotation.SetRotation(gp_Ax1(gp_Pnt(cx, cy, cz), gp_Dir(ax, ay, az)),
                             angle_deg * kPi / 180.0);
    }
    gp_Trsf translation;
    translation.SetTranslation(gp_Vec(tx, ty, tz));
    out.trsf = translation * rotation;

    return read_bool_strict(params, "copy", false, out.copy, err);
}

// The §2 minted id of the `copy: true` result for target index `k` of `n`.
std::string copy_body_id(const std::string& op_id, std::size_t k, std::size_t n) {
    return n == 1 ? "body_" + op_id : "body_" + op_id + ":" + std::to_string(k);
}

}  // namespace

OpOutcome execute_transform_body(OpContext& ctx, const json& op, const std::string& op_id) {
    const json params =
        (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();

    Placement placement;
    std::string err;
    if (!parse_placement(params, placement, err)) {
        return OpOutcome::fail("OP_FAILED", err);
    }

    // Resolve every target on the PREDECESSOR snapshot before mutating anything —
    // a half-applied multi-target placement would be worse than a clean refusal.
    std::vector<TopoDS_Shape> sources;
    sources.reserve(placement.targets.size());
    for (const std::string& id : placement.targets) {
        const session::BodyRecord* rec = ctx.bodies.get(id);
        if (!rec || rec->geom.IsNull()) {
            return OpOutcome::fail("REF_UNRESOLVED",
                                   "TransformBody target body not found: " + id);
        }
        if (auto invalid = validate_modeling_body(*rec, "TransformBody", "target")) {
            return *invalid;
        }
        sources.push_back(rec->geom);
    }

    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    OpOutcome out;
    const std::size_t n = placement.targets.size();
    for (std::size_t k = 0; k < n; ++k) {
        if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
        const std::string& target_id = placement.targets[k];
        TopoDS_Shape result;
        BRepBuilderAPI_Transform builder(placement.trsf);
        try {
            // `copy: false` keeps the geometry shared and only re-locates it (an
            // isometry with determinant 1), which is both cheap and what makes
            // `Modified()` return exactly ONE image per sub-shape — the level-1
            // rebind with zero descriptor scoring. `copy: true` must deep-copy: the
            // source stays in the document alongside its copy.
            builder.Perform(sources[k], placement.copy ? Standard_True : Standard_False);
            if (!builder.IsDone() || builder.Shape().IsNull()) {
                return OpOutcome::fail("OP_FAILED",
                                       "TransformBody failed on body " + target_id);
            }
            result = builder.Shape();
        } catch (const Standard_Failure& f) {
            return OpOutcome::fail("OP_FAILED",
                                   std::string("TransformBody raised: ") +
                                       (f.GetMessageString() ? f.GetMessageString() : "OCCT"));
        } catch (...) {
            return OpOutcome::fail("OP_FAILED", "TransformBody raised an unknown exception");
        }

        const kernel::validation::PublicationDecision decision = publication_decision(
            result,
            kernel::validation::single_solid_policy(
                "TransformBody", kernel::validation::PublicationTier::TierA));
        if (!decision.publishable()) {
            return OpOutcome::fail(decision.code, decision.message + " on body " + target_id);
        }

        if (placement.copy) {
            // NewBody lineage: a fresh body per target; the source is preserved and
            // emits NO event. Nothing is tracked on a brand-new body, so its
            // partition starts empty (ID-on-demand).
            const std::string bid = copy_body_id(op_id, k, n);
            ctx.bodies.create(bid, op_id, result);
            out.body_events.push_back({"created", bid});
            out.body_ids.push_back(bid);
        } else {
            // MODIFY lineage: the BodyId is preserved (corpus invariant) and the
            // element map follows the geometry — history first (shape/topoKey/
            // descriptor), then the rigid anchor migration.
            ctx.bodies.create(target_id, op_id, result);
            ctx.partition.apply_history(target_id, result, builder, out.delta,
                                        &out.needs_repair);
            ctx.partition.apply_placement(target_id, placement.trsf);
            out.body_events.push_back({"modified", target_id});
            out.body_ids.push_back(target_id);
        }
    }
    return out;
}

}  // namespace onecad::ops
