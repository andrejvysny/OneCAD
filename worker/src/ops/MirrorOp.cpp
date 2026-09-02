// MirrorOp.cpp — see MirrorOp.h. Ports RegenerationEngine.cpp buildMirrorBody.
#include "ops/MirrorOp.h"

#include <cmath>
#include <string>

#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <Standard_Failure.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax2.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>

#include "ops/OpCommon.h"

namespace onecad::ops {

using nlohmann::json;

namespace {

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

}  // namespace

OpOutcome execute_mirror_body(OpContext& ctx, const json& op, const std::string& op_id) {
    const json params =
        (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();

    const std::string source_id = read_str(params, "sourceBodyId");
    if (source_id.empty()) {
        return OpOutcome::fail("OP_FAILED", "MirrorBody requires a source body");
    }
    const session::BodyRecord* source_rec = ctx.bodies.get(source_id);
    if (!source_rec || source_rec->geom.IsNull()) {
        return OpOutcome::fail("REF_UNRESOLVED", "MirrorBody source body not found: " + source_id);
    }
    const TopoDS_Shape source = source_rec->geom;
    if (auto invalid = validate_modeling_body(*source_rec, "MirrorBody", "source")) return *invalid;

    double px = 0.0, py = 0.0, pz = 0.0;
    double nx = 0.0, ny = 0.0, nz = 1.0;
    if (!read_vec3(params, "planePoint", px, py, pz) ||
        !read_vec3(params, "planeNormal", nx, ny, nz)) {
        return OpOutcome::fail("OP_FAILED", "MirrorBody: missing plane point/normal");
    }
    if (nx * nx + ny * ny + nz * nz < 1e-20) {
        return OpOutcome::fail("OP_FAILED", "MirrorBody plane normal is zero");
    }

    bool fuse_with_original = false;
    std::string bool_error;
    if (!read_bool_strict(params, "fuseWithOriginal", false, fuse_with_original, bool_error)) {
        return OpOutcome::fail("OP_FAILED", bool_error);
    }

    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    TopoDS_Shape result;
    try {
        gp_Trsf mirror_trsf;
        mirror_trsf.SetMirror(gp_Ax2(gp_Pnt(px, py, pz), gp_Dir(nx, ny, nz)));
        BRepBuilderAPI_Transform mirror(source, mirror_trsf, Standard_True);
        if (!mirror.IsDone() || mirror.Shape().IsNull()) {
            return OpOutcome::fail("OP_FAILED", "MirrorBody transform failed");
        }
        result = mirror.Shape();

        if (fuse_with_original) {
            BRepAlgoAPI_Fuse fuse;
            stage_boolean(fuse, source, result);
            fuse.Build();
            if (!fuse.IsDone() || fuse.Shape().IsNull()) {
                return OpOutcome::fail("OP_FAILED", "MirrorBody fuse failed");
            }
            result = fuse.Shape();
        }
    } catch (const Standard_Failure& f) {
        return OpOutcome::fail("OP_FAILED", std::string("MirrorBody raised: ") +
                                                (f.GetMessageString() ? f.GetMessageString()
                                                                      : "OCCT"));
    } catch (...) {
        return OpOutcome::fail("OP_FAILED", "MirrorBody raised an unknown exception");
    }

    if (result.IsNull()) {
        return OpOutcome::fail("GEOMETRY_INVALID", "MirrorBody produced null shape");
    }
    // WP-E: a FUSED mirror is new boolean geometry and commits at Tier B (preview
    // Tier A). An unfused mirror is a reflection of an already-published body —
    // closed-manifold, BRepCheck and self-interference are invariant under an
    // isometry, so re-running Tier B would cost the full BOP self-interference pass
    // (minutes on a threaded fastener) and prove nothing new: Tier A.
    const kernel::validation::PublicationTier tier =
        fuse_with_original ? result_validation_tier(ctx, kernel::validation::PublicationTier::TierB)
                           : kernel::validation::PublicationTier::TierA;
    const kernel::validation::PublicationDecision decision = publication_decision(
        result,
        kernel::validation::single_solid_policy(
            fuse_with_original ? "MirrorBody fused result" : "MirrorBody result", tier),
        ctx.cancel);
    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
    if (!decision.publishable()) {
        // A mirror image that never touches its source fuses into a compound of two
        // solids. That is the RECOVERABLE "move the plane" case, named like
        // PatternOp's PATTERN_DISJOINT_RESULT (WP-C) so a caller never has to read
        // it as a kernel validity failure; the publication `reasonCode` + evidence
        // still ride the diagnostic (SCHEMA §7.2), so the fixture pin stays honest.
        if (fuse_with_original && decision.evidence.solid_count > 1) {
            const std::string message = "MirrorBody fused result is disconnected";
            OpOutcome failure = OpOutcome::fail("OP_FAILED", message);
            failure.diagnostics.push_back({{"severity", "error"},
                                           {"code", "MIRROR_DISJOINT_RESULT"},
                                           {"message", message},
                                           {"stage", "publication"},
                                           {"reasonCode", decision.reason_code},
                                           {"evidence", decision.evidence.to_json()}});
            return failure;
        }
        return publication_refusal(decision, "publication");
    }

    // NewBody lineage: fresh body `body_<opId>` (D1); the source body is preserved.
    OpOutcome out;
    const std::string bid = "body_" + op_id;
    ctx.bodies.create(bid, op_id, result);
    out.body_events.push_back({"created", bid});
    out.body_ids.push_back(bid);
    return out;  // no pre-existing tracked elements → empty delta
}

}  // namespace onecad::ops
