// GearOp.cpp — see GearOp.h.

#include "ops/GearOp.h"

#include <cmath>
#include <optional>
#include <vector>

#include <BRep_Tool.hxx>
#include <BRepClass_FaceClassifier.hxx>
#include <GeomAPI_ProjectPointOnSurf.hxx>
#include <Geom_Surface.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_State.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <gp_Ax2.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>

#include "elementmap/ElementMapPartition.h"
#include "elementmap/Ladder.h"
#include "kernel/validation/ShapeAudit.h"
#include "ops/OpCommon.h"
#include "ops/gear/GearTool.h"

namespace onecad::ops {

using nlohmann::json;
namespace em = onecad::elementmap;

namespace {

constexpr double kPi = 3.14159265358979323846;
// SCHEMA §7.3: the frozen `point` is re-projected onto the resolved face and
// the op fails loudly past 1e-3 mm. Same constant, same reason as HoleOp.
constexpr double kPointPlaneFence = 1e-3;
// Face-boundary classification tolerance (HoleOp.cpp precedent).
constexpr double kClassifyTol = 1e-7;

double deg_to_rad(double d) { return d * kPi / 180.0; }

/// `params.<key>` as a double, accepting BOTH the plain-number and the
/// `{value, expr}` scalar forms SCHEMA §7.3's header allows. Absent/malformed
/// yields `dflt`.
double read_scalar(const json& o, const char* key, double dflt) {
    if (!o.contains(key)) return dflt;
    const json& v = o[key];
    if (v.is_number()) return v.get<double>();
    if (v.is_object() && v.contains("value") && v["value"].is_number()) {
        return v["value"].get<double>();
    }
    return dflt;
}

bool read_flag(const json& o, const char* key, bool dflt) {
    if (!o.contains(key) || !o[key].is_boolean()) return dflt;
    return o[key].get<bool>();
}

int read_int(const json& o, const char* key, int dflt) {
    if (!o.contains(key)) return dflt;
    const json& v = o[key];
    if (v.is_number_integer()) return v.get<int>();
    if (v.is_number()) return static_cast<int>(v.get<double>());
    return dflt;
}

/// A `[x, y, z]` array. Returns false when absent or malformed.
bool read_vec3(const json& o, const char* key, gp_Pnt& out) {
    if (!o.contains(key) || !o[key].is_array() || o[key].size() != 3) return false;
    for (const json& c : o[key]) {
        if (!c.is_number()) return false;
    }
    out = gp_Pnt(o[key][0].get<double>(), o[key][1].get<double>(), o[key][2].get<double>());
    return true;
}

/// Translate the wire's recipe block into the builder's spec. The worker is an
/// INDEPENDENT trust boundary (SCHEMA §7.3), so it re-reads and re-validates
/// every parameter rather than assuming the Rust session validated it — a
/// hand-authored plan never passes through that validator at all.
std::string read_involute_spec(const json& params, gear::GearBuildSpec& spec) {
    if (!params.contains("involuteExternal") || !params["involuteExternal"].is_object()) {
        return "Gear involuteExternal params are required for an involuteExternal recipe";
    }
    const json& b = params["involuteExternal"];

    spec.involute.numTeeth = read_int(b, "teeth", 0);
    spec.involute.module = read_scalar(b, "module", 0.0);
    spec.involute.pressureAngle = deg_to_rad(read_scalar(b, "pressureAngleDeg", 0.0));
    spec.involute.clearance = read_scalar(b, "clearance", 0.25);
    spec.involute.shift = read_scalar(b, "shift", 0.0);
    spec.involute.helixAngle = deg_to_rad(read_scalar(b, "helixAngleDeg", 0.0));
    spec.involute.undercut = read_flag(b, "undercut", false);
    spec.involute.backlash = read_scalar(b, "backlash", 0.0);
    spec.involute.head = read_scalar(b, "head", 0.0);
    spec.involute.propertiesFromTool = read_flag(b, "propertiesFromTool", false);
    spec.sampleCount = read_int(b, "sampleCount", 20);
    spec.height = read_scalar(b, "height", 0.0);

    // Helical is UNSUPPORTED in this version (SCHEMA §7.3). Refused by name
    // rather than flattened to a spur gear — quietly returning a body that is
    // not the one asked for is the worse failure.
    if (read_scalar(b, "helixAngleDeg", 0.0) != 0.0) {
        return "Gear helixAngleDeg must be 0 in this version (helical gears are not yet supported)";
    }
    if (read_flag(b, "doubleHelix", false)) {
        return "Gear doubleHelix must be false in this version (herringbone gears are not yet "
               "supported)";
    }

    spec.axleHole = read_flag(b, "axleHole", false);
    if (spec.axleHole) {
        spec.axleHoleDiameter = read_scalar(b, "axleHoleDiameter", 0.0);
        if (!(spec.axleHoleDiameter > 0.0)) {
            return "Gear axleHoleDiameter must be positive when axleHole is on";
        }
    }
    spec.offsetHole = read_flag(b, "offsetHole", false);
    if (spec.offsetHole) {
        spec.offsetHoleDiameter = read_scalar(b, "offsetHoleDiameter", 0.0);
        spec.offsetHoleOffset = read_scalar(b, "offsetHoleOffset", 0.0);
        if (!(spec.offsetHoleDiameter > 0.0)) {
            return "Gear offsetHoleDiameter must be positive when offsetHole is on";
        }
        if (!(spec.offsetHoleOffset > 0.0)) {
            return "Gear offsetHoleOffset must be positive when offsetHole is on";
        }
    }
    return {};
}

/// The gear's placement face ref, when the payload carries one.
std::optional<em::LadderRef> placement_face_ref(const json& op, const json& params,
                                                const std::string& op_id) {
    if (params.contains("placement") && params["placement"].is_object()) {
        const json& pl = params["placement"];
        if (pl.contains("face") && pl["face"].is_object()) {
            em::LadderRef r = em::ladder_ref_from_input(pl["face"], op_id + ".placement.face");
            // `placement.face` is a face by construction; a payload that omitted
            // `primary.kind` still means "face" here (the HoleOp precedent).
            if (r.kind == em::km::ElementKind::Unknown) r.kind = em::km::ElementKind::Face;
            if (r.kind == em::km::ElementKind::Face) return r;
        }
    }
    if (op.contains("inputs") && op["inputs"].is_array()) {
        std::size_t i = 0;
        for (const json& in : op["inputs"]) {
            em::LadderRef r = em::ladder_ref_from_input(in, op_id + ".input" + std::to_string(i));
            ++i;
            if (r.kind == em::km::ElementKind::Face) return r;
        }
    }
    return std::nullopt;
}

/// The body a placement ref names, from its `primary.bodyId`.
std::string placement_body_of(const json& params) {
    if (!params.contains("placement") || !params["placement"].is_object()) return {};
    const json& pl = params["placement"];
    if (!pl.contains("face") || !pl["face"].is_object()) return {};
    const json& face = pl["face"];
    if (!face.contains("primary") || !face["primary"].is_object()) return {};
    return read_str(face["primary"], "bodyId");
}

/// Resolve the placement face on `host_shape` — the tracked rung first, then
/// the descriptor ladder. Mirrors HoleOp's `resolve_host_face`, including the
/// rule that a TopoKey is only valid for an entry belonging to THIS body.
TopoDS_Face resolve_placement_face(OpContext& ctx, const TopoDS_Shape& host_shape,
                                   const std::string& host_id, em::LadderRef ref, json& nr) {
    const std::string topo_key = em::ElementMapPartition::topokey_for_element_in_body(
        ctx.partition, ref.element_id, host_id);
    if (!topo_key.empty()) {
        const TopoDS_Shape sub = em::ElementMapPartition::shape_for_topokey(host_shape, topo_key);
        if (!sub.IsNull() && sub.ShapeType() == TopAbs_FACE) return TopoDS::Face(sub);
    }
    std::vector<em::LadderRef> refs{std::move(ref)};
    const std::vector<em::LadderResolution> res =
        em::resolve_descriptor_stage(host_shape, host_id, refs,
                                     em::LadderEditContext{ctx.post_upstream_edit,
                                                           ctx.from_zero_replay});
    if (!res.empty() && res[0].outcome == em::LadderOutcome::AutoBind &&
        !res[0].bound_shape.IsNull() && res[0].bound_shape.ShapeType() == TopAbs_FACE) {
        return TopoDS::Face(res[0].bound_shape);
    }
    nr = res.empty() ? json::object() : res[0].to_needs_repair_json();
    return {};
}

}  // namespace

std::set<std::string> gear_body_op_ids(const json& plan) {
    std::set<std::string> out;
    if (!plan.is_object() || !plan.contains("ops") || !plan["ops"].is_array()) return out;
    for (const json& op : plan["ops"]) {
        if (!op.is_object()) continue;
        if (read_str(op, "opType") != "Gear") continue;
        const std::string id = read_str(op, "opId");
        if (!id.empty()) out.insert(id);
    }
    return out;
}

bool is_generated_gear_body(const json& plan, const std::string& body_id) {
    // D1: a Gear mints exactly `body_<opId>`, so the mapping is total and needs
    // no stored state to survive a reopen.
    static constexpr const char* kPrefix = "body_";
    if (body_id.rfind(kPrefix, 0) != 0) return false;
    const std::string op_id = body_id.substr(std::string(kPrefix).size());
    return gear_body_op_ids(plan).count(op_id) > 0;
}

OpOutcome execute_gear(OpContext& ctx, const json& op, const std::string& op_id) {
    const json params =
        (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();

    if (std::vector<json> repairs = operation_ref_ownership_repairs(op, op_id); !repairs.empty()) {
        OpOutcome out;
        out.needs_repair = std::move(repairs);
        return out;
    }

    // --- recipe dispatch (worker is an independent trust boundary) ----------
    const std::string recipe = read_str(params, "recipe");
    if (recipe.empty()) {
        return OpOutcome::fail("OP_FAILED", "Gear requires a recipe");
    }
    if (recipe != "involuteExternal") {
        return OpOutcome::fail("UNSUPPORTED", "Gear recipe '" + recipe + "' is not supported");
    }
    gear::GearBuildSpec spec;
    if (const std::string reason = read_involute_spec(params, spec); !reason.empty()) {
        return OpOutcome::fail("OP_FAILED", reason);
    }

    // --- placement ---------------------------------------------------------
    if (!params.contains("placement") || !params["placement"].is_object()) {
        return OpOutcome::fail("OP_FAILED", "Gear requires a placement");
    }
    const json& placement = params["placement"];
    const bool has_face = placement.contains("face") && placement["face"].is_object();
    const bool has_frame = placement.contains("frame") && placement["frame"].is_object();
    if (has_face && has_frame) {
        return OpOutcome::fail(
            "OP_FAILED", "Gear placement carries BOTH a face and a frame; exactly one is allowed");
    }
    if (!has_face && !has_frame) {
        return OpOutcome::fail(
            "OP_FAILED",
            "Gear placement carries neither a face nor a frame; exactly one is required");
    }

    gp_Pnt frozen;
    if (!read_vec3(placement, "point", frozen)) {
        return OpOutcome::fail("OP_FAILED", "Gear placement point must be a [x, y, z] world point");
    }

    OpOutcome out;
    gp_Ax2 frame;

    if (has_frame) {
        // Explicit frozen frame: no refs, no reprojection, no host.
        const json& fr = placement["frame"];
        gp_Pnt origin;
        gp_Pnt axis_pt;
        gp_Pnt xdir_pt;
        if (!read_vec3(fr, "origin", origin) || !read_vec3(fr, "axis", axis_pt) ||
            !read_vec3(fr, "xDir", xdir_pt)) {
            return OpOutcome::fail("OP_FAILED",
                                   "Gear placement frame needs origin, axis and xDir as [x,y,z]");
        }
        const gp_Vec axis_v(axis_pt.X(), axis_pt.Y(), axis_pt.Z());
        const gp_Vec xdir_v(xdir_pt.X(), xdir_pt.Y(), xdir_pt.Z());
        if (axis_v.Magnitude() <= 0.0) {
            return OpOutcome::fail("OP_FAILED", "Gear placement frame axis is degenerate");
        }
        if (xdir_v.Magnitude() <= 0.0) {
            return OpOutcome::fail("OP_FAILED", "Gear placement frame xDir is degenerate");
        }
        if (origin.Distance(frozen) > 1e-9) {
            return OpOutcome::fail("OP_FAILED",
                                   "Gear placement point must equal the frame origin");
        }
        if (axis_v.IsParallel(xdir_v, 1e-9)) {
            return OpOutcome::fail("OP_FAILED",
                                   "Gear placement frame xDir is parallel to the axis");
        }
        try {
            frame = gp_Ax2(origin, gp_Dir(axis_v), gp_Dir(xdir_v));
        } catch (const Standard_Failure& f) {
            return OpOutcome::fail("OP_FAILED",
                                   std::string("Gear placement frame is not buildable: ") +
                                       (f.what() ? f.what() : "OCCT"));
        }
    } else {
        // Face placement: Hole's semantics, reused verbatim.
        const std::string host_id = placement_body_of(params);
        if (host_id.empty()) {
            return OpOutcome::fail("OP_FAILED",
                                   "Gear placement face must name its body (primary.bodyId)");
        }
        const session::BodyRecord* host_rec = ctx.bodies.get(host_id);
        if (!host_rec) {
            return OpOutcome::fail("REF_UNRESOLVED",
                                   "Gear placement body not found: " + host_id);
        }
        const TopoDS_Shape host_shape = host_rec->geom;

        std::optional<em::LadderRef> ref = placement_face_ref(op, params, op_id);
        if (!ref) {
            return OpOutcome::fail("OP_FAILED", "Gear placement face could not be read");
        }
        json nr;
        const TopoDS_Face face =
            resolve_placement_face(ctx, host_shape, host_id, std::move(*ref), nr);
        if (face.IsNull()) {
            // Placement no longer resolves ⇒ NeedsRepair STATE, never a wrong
            // bind (SCHEMA §10). The gear is not built at all this regen.
            out.needs_repair.push_back(nr);
            return out;
        }

        gp_Pln plane;
        gp_Dir outward;
        if (!planar_face_plane_normal(face, plane, outward)) {
            return OpOutcome::fail("OP_FAILED",
                                   "Gear placement face is not planar (a gear needs a planar seat)");
        }

        gp_Pnt origin;
        try {
            const Handle(Geom_Surface) surf = BRep_Tool::Surface(face);
            GeomAPI_ProjectPointOnSurf proj(frozen, surf);
            if (!proj.IsDone() || proj.NbPoints() < 1) {
                return OpOutcome::fail("OP_FAILED",
                                       "Gear placement point could not be projected onto the face");
            }
            if (proj.LowerDistance() > kPointPlaneFence) {
                return OpOutcome::fail(
                    "OP_FAILED", "Gear placement point is off the resolved face plane (fence 1e-3 mm)");
            }
            origin = proj.NearestPoint();
            double u = 0.0;
            double v = 0.0;
            proj.LowerDistanceParameters(u, v);
            BRepClass_FaceClassifier classifier(face, gp_Pnt2d(u, v), kClassifyTol);
            if (classifier.State() == TopAbs_OUT) {
                return OpOutcome::fail("OP_FAILED",
                                       "Gear placement point lies outside the resolved face boundary");
            }
        } catch (const Standard_Failure& f) {
            return OpOutcome::fail(
                "OP_FAILED", std::string("Gear placement point could not be resolved on the face: ") +
                                 (f.what() ? f.what() : "OCCT"));
        }

        // SCHEMA §7.3: axis = the face's INWARD normal, so the gear grows into
        // material rather than away from the part it is seated on.
        try {
            frame = gp_Ax2(origin, outward.Reversed());
        } catch (const Standard_Failure& f) {
            return OpOutcome::fail("OP_FAILED",
                                   std::string("Gear placement frame is not buildable: ") +
                                       (f.what() ? f.what() : "OCCT"));
        }
    }

    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    // --- build -------------------------------------------------------------
    const gear::GearBuildResult built = gear::build_gear_solid(spec, frame);
    if (!built.ok) {
        return OpOutcome::fail("OP_FAILED", built.error.empty() ? "Gear body could not be built"
                                                                : built.error);
    }

    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    // --- publication: the FULL audit (SCHEMA §7.3) -------------------------
    const kernel::validation::PublicationDecision decision = publication_decision(
        built.shape,
        kernel::validation::single_solid_policy("Gear", kernel::validation::PublicationTier::TierB));
    if (!decision.publishable()) {
        return OpOutcome::fail(decision.code, decision.message);
    }

    // --- mint (D1) ---------------------------------------------------------
    // NewBody lineage: a fresh `body_<opId>`, one `created` event, and an EMPTY
    // delta — a gear tracks no pre-existing element, so `apply_history` has
    // nothing to rebind. Tooth faces are deliberately NOT minted into the
    // element map; see GearOp.h on referenceability.
    const std::string bid = "body_" + op_id;
    ctx.bodies.create(bid, op_id, built.shape);
    out.body_events.push_back({"created", bid, {}});
    out.body_ids.push_back(bid);
    return out;
}

}  // namespace onecad::ops
