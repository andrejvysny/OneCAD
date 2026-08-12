// ComponentOp.cpp — see ComponentOp.h.
#include "ops/ComponentOp.h"

#include <cmath>
#include <map>
#include <memory>
#include <sstream>
#include <string>

#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_MakeShape.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <Standard_Failure.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include "kernel/validation/ShapeAudit.h"
#include "modeling/BooleanMode.h"
#include "ops/OpCommon.h"

namespace onecad::ops {

using nlohmann::json;

namespace {

constexpr double kPi = 3.14159265358979323846;
constexpr double kMinAxisLen2 = 1e-12;

// Default free params — chosen to reproduce P0/P1's old hardcoded M6×20
// EXACTLY, so every existing caller (nothing sends `source.params.thread`/
// `.length` yet — see WP-2.1's plan doc) stays byte-identical.
constexpr const char* kDefaultThread = "M6";
constexpr double kDefaultLengthMm = 20.0;

// ISO 4762 / DIN 912 socket-head cap screw dimensions (mm), keyed by thread
// designation — BOLTS-seeded (spec §6.3): `data/hex_socket.blt`, class
// `hexsocketheadcap`, github.com/boltsparts/BOLTS_archive, retrieved
// 2026-08-12. LGPL 2.1+, author Johannes Reinhardt — see THIRD_PARTY_NOTICES.
// Only the columns this generator's geometry needs (`d1` shank/thread Ø,
// `d2` head Ø, `k` head height); BOLTS' fuller column set (`s`/`t_min`/`b`/
// `L`) lives in `onecad-library::tables`'s Rust-side mirror for
// authoring/metadata use, not here (spec §6: generators build geometry from
// the table they own; the library crate's copy is deliberately separate).
// Spec §6.2 seeds M2–M12; BOLTS' own table runs M1.4–M64 but M1.4/M1.8 carry
// `None` for `t_min`/`b` at that source (undersized for a practical
// hex-socket detail) — scoped to the spec's stated M2–M12 range rather than
// engineering around incomplete rows outside it.
struct Iso4762Size {
    double head_diameter_mm;   // d2 (dk)
    double head_height_mm;     // k
    double shank_diameter_mm;  // d1 (thread pitch diameter, cosmetic detail)
};

const std::map<std::string, Iso4762Size>& iso4762Table() {
    static const std::map<std::string, Iso4762Size> table = {
        {"M2", {3.8, 2.0, 2.0}},    {"M2.5", {4.5, 2.5, 2.5}}, {"M3", {5.5, 3.0, 3.0}},
        {"M4", {7.0, 4.0, 4.0}},    {"M5", {8.5, 5.0, 5.0}},   {"M6", {10.0, 6.0, 6.0}},
        {"M8", {13.0, 8.0, 8.0}},   {"M10", {16.0, 10.0, 10.0}}, {"M12", {18.0, 12.0, 12.0}},
    };
    return table;
}

// Builds an ISO 4762 SHCS solid for `thread`×`length_mm`: a head cylinder
// fused to a shank cylinder, coaxial on +Z. Origin sits at the head-underside
// seating plane — the natural mate seat for a hole (head above the surface,
// shank extending DOWN into the hole, matching the concentric-on-hole gesture
// spec §5.3 describes). `length_mm` is the free `length` param (spec §6.1:
// "length is free"), independent of the table's own max-threaded-length
// column. Never throws; failures surface as a recoverable `OP_FAILED`.
bool build_shcs(const std::string& op_label, const std::string& thread, double length_mm,
                TopoDS_Shape& solid_out, std::string& err) {
    const auto& table = iso4762Table();
    const auto it = table.find(thread);
    if (it == table.end()) {
        std::ostringstream keys;
        bool first = true;
        for (const auto& entry : table) {
            if (!first) keys << ", ";
            keys << entry.first;
            first = false;
        }
        err = op_label + ": unknown thread designation '" + thread +
              "' — known sizes: " + keys.str();
        return false;
    }
    const Iso4762Size& size = it->second;
    try {
        const gp_Ax2 head_axis(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 0.0, 1.0));
        const TopoDS_Shape head =
            BRepPrimAPI_MakeCylinder(head_axis, size.head_diameter_mm / 2.0, size.head_height_mm)
                .Shape();
        const gp_Ax2 shank_axis(gp_Pnt(0.0, 0.0, -length_mm), gp_Dir(0.0, 0.0, 1.0));
        const TopoDS_Shape shank =
            BRepPrimAPI_MakeCylinder(shank_axis, size.shank_diameter_mm / 2.0, length_mm).Shape();

        std::shared_ptr<BRepBuilderAPI_MakeShape> builder;
        const BooleanResult br = checked_boolean(head, shank, app::BooleanMode::Add,
                                                 /*parallel=*/false, json::object(),
                                                 /*cancel=*/nullptr, builder);
        if (!br.error_code.empty()) {
            err = op_label + ": " + thread + " SHCS head/shank fuse failed: " + br.error_message;
            return false;
        }
        solid_out = br.shape;
        return true;
    } catch (const Standard_Failure& f) {
        err = op_label + ": " + thread + " SHCS build raised: " +
              (f.GetMessageString() ? f.GetMessageString() : "OCCT");
        return false;
    }
}

// Reads `placement.{translate,rotate}` into a `gp_Trsf`, SAME normative order
// as TransformBody: `X' = T ∘ R(center, axis, angleDeg) · X` — rotate about
// the frozen pivot first, then translate. Absent `rotate` is the identity
// rotation (mirrors `FrozenPlacement`'s `#[serde(default)]`).
bool read_placement(const json& params, const std::string& op_label, gp_Trsf& trsf_out,
                    std::string& err) {
    if (!params.contains("placement") || !params["placement"].is_object()) {
        err = op_label + " requires a placement object";
        return false;
    }
    const json& placement = params["placement"];
    double tx = 0.0, ty = 0.0, tz = 0.0;
    if (!placement.contains("translate") || !placement["translate"].is_array() ||
        placement["translate"].size() != 3) {
        err = op_label + " placement.translate must be a 3-array";
        return false;
    }
    const json& t = placement["translate"];
    // Each entry is a bare number OR a `{value}` object (SCHEMA §4 Scalar wire form).
    auto scalar_at = [&](const json& v, const char* what, double& out) -> bool {
        if (v.is_number()) {
            out = v.get<double>();
        } else if (v.is_object() && v.contains("value") && v["value"].is_number()) {
            out = v["value"].get<double>();
        } else {
            err = op_label + " placement.translate." + what + " must be a scalar";
            return false;
        }
        if (!std::isfinite(out)) {
            err = op_label + " placement.translate." + what + " must be finite";
            return false;
        }
        return true;
    };
    if (!scalar_at(t[0], "x", tx) || !scalar_at(t[1], "y", ty) || !scalar_at(t[2], "z", tz)) {
        return false;
    }

    double cx = 0.0, cy = 0.0, cz = 0.0;
    double ax = 0.0, ay = 0.0, az = 1.0;
    double angle_deg = 0.0;
    if (placement.contains("rotate") && placement["rotate"].is_object()) {
        const json& rot = placement["rotate"];
        if (rot.contains("center")) {
            if (!rot["center"].is_array() || rot["center"].size() != 3 ||
                !scalar_at(rot["center"][0], "center.x", cx) ||
                !scalar_at(rot["center"][1], "center.y", cy) ||
                !scalar_at(rot["center"][2], "center.z", cz)) {
                if (err.empty()) err = op_label + " placement.rotate.center must be a 3-array";
                return false;
            }
        }
        if (rot.contains("axis")) {
            if (!rot["axis"].is_array() || rot["axis"].size() != 3 ||
                !scalar_at(rot["axis"][0], "axis.x", ax) ||
                !scalar_at(rot["axis"][1], "axis.y", ay) ||
                !scalar_at(rot["axis"][2], "axis.z", az)) {
                if (err.empty()) err = op_label + " placement.rotate.axis must be a 3-array";
                return false;
            }
        }
        if (rot.contains("angleDeg") && !scalar_at(rot["angleDeg"], "angleDeg", angle_deg)) {
            return false;
        }
    }
    const double axis_len2 = ax * ax + ay * ay + az * az;
    if (angle_deg != 0.0 && axis_len2 < kMinAxisLen2) {
        err = op_label + " placement.rotate.axis must be non-zero when angleDeg != 0";
        return false;
    }

    gp_Trsf rotation;
    if (angle_deg != 0.0) {
        rotation.SetRotation(gp_Ax1(gp_Pnt(cx, cy, cz), gp_Dir(ax, ay, az)), angle_deg * kPi / 180.0);
    }
    gp_Trsf translation;
    translation.SetTranslation(gp_Vec(tx, ty, tz));
    trsf_out = translation * rotation;
    return true;
}

// Shared pipeline behind BOTH `PlaceComponent` and `DetachComponent`
// (identical geometry construction — the two differ only in what the
// RECORD's params carry: PlaceComponent keeps a library identity + optional
// mate, DetachComponent carries neither, per spec §3.4's "no component_*
// fields remain"). `op_label` names the caller in error messages.
//
// `source.kind` MUST be `"generator"` (P0/WP-1.1 scope; `embedded` reaches
// the worker once WP-1.3 wires the wire-only blob-path injection, `document`
// in P3). The generator is TABLE-DRIVEN as of WP-2.1 (`iso4762Table()`) —
// every `generatorId` still builds an ISO 4762 SHCS (a distinct per-family
// generator dispatch is future scope once a second family is seeded), sized
// by `source.params.thread`/`.length` (defaulted for byte-identical P0/P1
// behavior when absent).
OpOutcome resolve_source_and_publish(OpContext& ctx, const json& params, const std::string& op_id,
                                     const std::string& op_label) {
    if (!params.contains("source") || !params["source"].is_object()) {
        return OpOutcome::fail("OP_FAILED", op_label + " requires a source object");
    }
    const json& source = params["source"];
    const std::string kind = read_str(source, "kind");
    if (kind != "generator") {
        return OpOutcome::fail("UNSUPPORTED",
                               op_label + ": source.kind '" + kind +
                                   "' not yet supported (this build implements 'generator' only)");
    }
    const std::string generator_id = read_str(source, "generatorId");
    if (generator_id.empty()) {
        return OpOutcome::fail("OP_FAILED", op_label + " source.generatorId must not be empty");
    }

    if (ctx.cancel != nullptr && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    // Free params live under `source.params` — the wire lowering has no
    // PlaceComponent special-case beyond `inputs[]`, so `ComponentSourceRef::
    // Generator.params` reaches here verbatim. Defaults reproduce P0/P1's
    // old hardcoded M6×20 exactly.
    std::string thread = kDefaultThread;
    double length_mm = kDefaultLengthMm;
    if (source.contains("params") && source["params"].is_object()) {
        const json& gp = source["params"];
        if (gp.contains("thread") && gp["thread"].is_string()) {
            thread = gp["thread"].get<std::string>();
        }
        if (gp.contains("length")) {
            const json& l = gp["length"];
            if (l.is_number()) {
                length_mm = l.get<double>();
            } else if (l.is_object() && l.contains("value") && l["value"].is_number()) {
                length_mm = l["value"].get<double>();
            }
        }
    }
    if (!std::isfinite(length_mm) || length_mm <= 0.0) {
        return OpOutcome::fail("OP_FAILED", op_label + ": source.params.length must be finite and positive");
    }

    TopoDS_Shape solid;
    std::string err;
    if (!build_shcs(op_label, thread, length_mm, solid, err)) {
        return OpOutcome::fail("OP_FAILED", err);
    }

    gp_Trsf trsf;
    if (!read_placement(params, op_label, trsf, err)) {
        return OpOutcome::fail("OP_FAILED", err);
    }
    try {
        BRepBuilderAPI_Transform xf(solid, trsf, /*Copy=*/Standard_True);
        if (!xf.IsDone() || xf.Shape().IsNull()) {
            return OpOutcome::fail("OP_FAILED", op_label + ": placement transform failed");
        }
        solid = xf.Shape();
    } catch (const Standard_Failure& f) {
        return OpOutcome::fail("OP_FAILED", op_label + ": placement transform raised: " +
                                                (f.GetMessageString() ? f.GetMessageString() : "OCCT"));
    }

    if (ctx.cancel != nullptr && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    // spec §9: a component resolves to exactly ONE solid in v1 — the same
    // publication policy every other NewBody-minting op satisfies.
    const kernel::validation::PublicationDecision decision = publication_decision(
        solid,
        kernel::validation::single_solid_policy(op_label, kernel::validation::PublicationTier::TierA));
    if (!decision.publishable()) {
        return OpOutcome::fail(decision.code, decision.message);
    }

    OpOutcome out;
    const std::string bid = "body_" + op_id;
    ctx.bodies.create(bid, op_id, solid);
    out.body_events.push_back({"created", bid, {}});
    out.body_ids.push_back(bid);
    return out;
}

}  // namespace

OpOutcome execute_place_component(OpContext& ctx, const json& op, const std::string& op_id) {
    const json params =
        (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();
    return resolve_source_and_publish(ctx, params, op_id, "PlaceComponent");
}

OpOutcome execute_detach_component(OpContext& ctx, const json& op, const std::string& op_id) {
    const json params =
        (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();
    return resolve_source_and_publish(ctx, params, op_id, "DetachComponent");
}

}  // namespace onecad::ops
