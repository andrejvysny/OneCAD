// ComponentOp.cpp — see ComponentOp.h.
#include "ops/ComponentOp.h"

#include <cmath>
#include <map>
#include <memory>
#include <sstream>
#include <string>

#include <BRep_Builder.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepLib.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_MakeShape.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepOffsetAPI_MakePipeShell.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <Geom2d_Line.hxx>
#include <Geom_CylindricalSurface.hxx>
#include <Standard_Failure.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <TopoDS_Wire.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Ax3.hxx>
#include <gp_Dir.hxx>
#include <gp_Dir2d.hxx>
#include <gp_Pnt.hxx>
#include <gp_Pnt2d.hxx>
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
// WP-2.5: third free param, cosmetic stays default so every pre-existing
// caller (nothing sends `source.params.thread_detail` yet) stays
// byte-identical, same rule as thread/length above.
constexpr const char* kDefaultThreadDetail = "cosmetic";

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
//
// `pitch_mm` (WP-2.5) is a DIFFERENT provenance than the rest of this table:
// the ISO 261 coarse-pitch series, a public numeric standard, not BOLTS
// data — hand-transcribed, not subject to THIRD_PARTY_NOTICES.
struct Iso4762Size {
    double head_diameter_mm;   // d2 (dk)
    double head_height_mm;     // k
    double shank_diameter_mm;  // d1 (thread major/pitch Ø, cosmetic detail)
    double pitch_mm;           // ISO 261 coarse-pitch series
};

const std::map<std::string, Iso4762Size>& iso4762Table() {
    static const std::map<std::string, Iso4762Size> table = {
        {"M2", {3.8, 2.0, 2.0, 0.40}},    {"M2.5", {4.5, 2.5, 2.5, 0.45}},
        {"M3", {5.5, 3.0, 3.0, 0.50}},    {"M4", {7.0, 4.0, 4.0, 0.70}},
        {"M5", {8.5, 5.0, 5.0, 0.80}},    {"M6", {10.0, 6.0, 6.0, 1.00}},
        {"M8", {13.0, 8.0, 8.0, 1.25}},   {"M10", {16.0, 10.0, 10.0, 1.50}},
        {"M12", {18.0, 12.0, 12.0, 1.75}},
    };
    return table;
}

// WP-2.5 thread rendering level (spec §6.4). `Cosmetic` is the mainstream-
// CAD default (a plain cylinder — assembly-safe, fast); `Simplified` cuts
// discrete annular grooves; `Modeled` cuts a true helical V-thread, opt-in
// for 3D-print output.
enum class ThreadDetail { Cosmetic, Simplified, Modeled };

bool parse_thread_detail(const std::string& s, ThreadDetail& out) {
    if (s == "cosmetic") {
        out = ThreadDetail::Cosmetic;
        return true;
    }
    if (s == "simplified") {
        out = ThreadDetail::Simplified;
        return true;
    }
    if (s == "modeled") {
        out = ThreadDetail::Modeled;
        return true;
    }
    return false;
}

// Builds the ISO 4762 SHCS BLANK for `thread`×`length_mm`: a head cylinder
// fused to a shank cylinder, coaxial on +Z. Origin sits at the head-underside
// seating plane — the natural mate seat for a hole (head above the surface,
// shank extending DOWN into the hole, matching the concentric-on-hole gesture
// spec §5.3 describes). `length_mm` is the free `length` param (spec §6.1:
// "length is free"), independent of the table's own max-threaded-length
// column. This IS the `cosmetic` result (unchanged since WP-2.1); `simplified`/
// `modeled` cut further into the shank produced here. Never throws; failures
// surface as a recoverable `OP_FAILED`.
bool build_shcs_blank(const std::string& op_label, const std::string& thread,
                      const Iso4762Size& size, double length_mm, TopoDS_Shape& solid_out,
                      std::string& err) {
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

// `simplified` thread detail: cuts N discrete annular grooves out of the
// shank (never the head), one per pitch, spanning [-length_mm, 0) on Z. Each
// groove is a shallow V revolved 360° around the shank axis into a ring
// solid; ALL N rings are accumulated into ONE `TopoDS_Compound` tool and cut
// in a SINGLE boolean — not N sequential booleans, which would accumulate
// tolerance debris and turn each ring into its own independent failure
// point (this is the same reasoning `checked_boolean` callers elsewhere in
// this worker already lean on for multi-feature cuts).
bool cut_simplified_thread(const std::string& op_label, const TopoDS_Shape& blank,
                           double shank_radius_mm, double length_mm, double pitch_mm,
                           TopoDS_Shape& solid_out, std::string& err) {
    const int n = static_cast<int>(std::floor(length_mm / pitch_mm));
    if (n <= 0) {
        // Shank too short for even one groove at this pitch — cosmetic blank stands.
        solid_out = blank;
        return true;
    }
    const double groove_depth = pitch_mm / 4.0;
    const double groove_half_width = pitch_mm / 4.0;
    const double r_outer = shank_radius_mm + groove_depth * 3.0;  // guarantees full penetration
    try {
        BRep_Builder cb;
        TopoDS_Compound rings;
        cb.MakeCompound(rings);
        const gp_Ax1 revol_axis(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 0.0, 1.0));
        for (int i = 0; i < n; ++i) {
            const double z = -length_mm + (i + 0.5) * pitch_mm;
            BRepBuilderAPI_MakePolygon poly;
            poly.Add(gp_Pnt(shank_radius_mm - groove_depth, 0.0, z));
            poly.Add(gp_Pnt(r_outer, 0.0, z - groove_half_width));
            poly.Add(gp_Pnt(r_outer, 0.0, z + groove_half_width));
            poly.Close();
            if (!poly.IsDone()) {
                err = op_label + ": simplified thread groove profile failed at ring " +
                      std::to_string(i);
                return false;
            }
            BRepBuilderAPI_MakeFace face_mk(poly.Wire());
            if (!face_mk.IsDone()) {
                err = op_label + ": simplified thread groove face failed at ring " +
                      std::to_string(i);
                return false;
            }
            BRepPrimAPI_MakeRevol revol(face_mk.Shape(), revol_axis, 2.0 * kPi, /*Copy=*/true);
            if (!revol.IsDone() || revol.Shape().IsNull()) {
                err = op_label + ": simplified thread groove revolve failed at ring " +
                      std::to_string(i);
                return false;
            }
            cb.Add(rings, revol.Shape());
        }
        std::shared_ptr<BRepBuilderAPI_MakeShape> builder;
        const BooleanResult br = checked_boolean(blank, rings, app::BooleanMode::Cut,
                                                 /*parallel=*/false, json::object(),
                                                 /*cancel=*/nullptr, builder);
        if (!br.error_code.empty()) {
            err = op_label + ": simplified thread cut failed: " + br.error_message;
            return false;
        }
        solid_out = br.shape;
        return true;
    } catch (const Standard_Failure& f) {
        err = op_label + ": simplified thread build raised: " +
              (f.GetMessageString() ? f.GetMessageString() : "OCCT");
        return false;
    }
}

// `modeled` thread detail: cuts a true helical single-start V-thread out of
// the shank via a swept solid, ISO 68-1-inspired (60° included angle) but
// FLAT-TRUNCATED (no root/crest fillet) — a deliberate fidelity cut, not a
// silent shortcut: a rounded root needs a fillet-on-profile step BEFORE the
// sweep, stacking a third independently-fragile OCCT operation (fillet +
// helical sweep + boolean cut) for a difference invisible outside extreme
// close-up/3D-print slicing. `MakePipeShell` on a helix is itself one of
// OCCT's more fragile sweep paths (self-intersection risk highest at the
// thread start/end) — WP-2.6's kernelbench extremes suite is where that gets
// stress-tested across the table; this function only proves it works.
bool cut_modeled_thread(const std::string& op_label, const TopoDS_Shape& blank,
                        double shank_radius_mm, double length_mm, double pitch_mm,
                        TopoDS_Shape& solid_out, std::string& err) {
    try {
        // Helix spine on a cylindrical surface whose origin sits at the shank
        // TIP (z = -length_mm), so the surface's own v-parameter equals
        // height-above-tip directly.
        const gp_Ax3 surface_axis(gp_Pnt(0.0, 0.0, -length_mm), gp_Dir(0.0, 0.0, 1.0));
        const Handle(Geom_CylindricalSurface) cyl =
            new Geom_CylindricalSurface(surface_axis, shank_radius_mm);

        // u (radians) advances 2π per pitch of v (mm) — the helix's rise.
        const double du_dv = 2.0 * kPi / pitch_mm;
        const Handle(Geom2d_Line) line2d =
            new Geom2d_Line(gp_Pnt2d(0.0, 0.0), gp_Dir2d(du_dv, 1.0));
        const double unit_v = 1.0 / std::sqrt(du_dv * du_dv + 1.0);
        const double t_end = length_mm / unit_v;

        BRepBuilderAPI_MakeEdge edge_mk(line2d, cyl, 0.0, t_end);
        if (!edge_mk.IsDone()) {
            err = op_label + ": modeled thread helix edge failed";
            return false;
        }
        // `BRepBuilderAPI_MakeEdge(Geom2d_Curve, Geom_Surface, ...)` builds a
        // p-curve-only edge — no 3D approximation curve — but
        // `BRepOffsetAPI_MakePipeShell::Build()` needs a real 3D curve on the
        // spine (a null one raises `Standard_NullObject` deep inside, with no
        // message). `BuildCurves3d` fills it in.
        TopoDS_Edge helix_edge = edge_mk.Edge();
        if (!BRepLib::BuildCurves3d(helix_edge)) {
            err = op_label + ": modeled thread helix 3D-curve approximation failed";
            return false;
        }
        BRepBuilderAPI_MakeWire wire_mk(helix_edge);
        if (!wire_mk.IsDone()) {
            err = op_label + ": modeled thread helix wire failed";
            return false;
        }
        const TopoDS_Wire spine = wire_mk.Wire();

        // Profile cross-section in the plane normal to the helix at its
        // start (u=0, v=0): a flat-truncated V wedge, apex cutting INTO the
        // shank, base clear of the surface on both flanks.
        gp_Pnt p0;
        gp_Vec d1u, d1v;
        cyl->D1(0.0, 0.0, p0, d1u, d1v);
        const gp_Vec tangent_vec = d1u.Multiplied(du_dv) + d1v;
        const gp_Dir tangent_dir(tangent_vec);
        const gp_Dir radial_dir = surface_axis.XDirection();
        const gp_Ax2 profile_axis(p0, tangent_dir, radial_dir);

        const double thread_depth = 0.613 * pitch_mm;   // ISO 68-1 coarse external depth
        const double half_width = pitch_mm / 2.0;        // flat-truncated crest/root
        const double margin = thread_depth * 3.0;         // clears the surface on the outside flank
        const gp_Vec x_dir(profile_axis.XDirection());
        const gp_Vec y_dir(profile_axis.YDirection());
        auto to3d = [&](double xl, double yl) {
            return p0.Translated(x_dir * xl + y_dir * yl);
        };

        BRepBuilderAPI_MakePolygon poly;
        poly.Add(to3d(-thread_depth, 0.0));
        poly.Add(to3d(margin, -half_width));
        poly.Add(to3d(margin, half_width));
        poly.Close();
        if (!poly.IsDone()) {
            err = op_label + ": modeled thread profile failed";
            return false;
        }

        BRepOffsetAPI_MakePipeShell pipe(spine);
        pipe.SetMode(true);  // Frenet trihedron — the natural fit for a helix.
        pipe.Add(poly.Wire());
        if (!pipe.IsReady()) {
            err = op_label + ": modeled thread pipe shell not ready";
            return false;
        }
        pipe.Build();
        if (!pipe.IsDone()) {
            err = op_label + ": modeled thread pipe shell build failed";
            return false;
        }
        pipe.MakeSolid();
        const TopoDS_Shape thread_tool = pipe.Shape();
        if (thread_tool.IsNull()) {
            err = op_label + ": modeled thread pipe shell produced no solid";
            return false;
        }

        std::shared_ptr<BRepBuilderAPI_MakeShape> builder;
        const BooleanResult br = checked_boolean(blank, thread_tool, app::BooleanMode::Cut,
                                                 /*parallel=*/false, json::object(),
                                                 /*cancel=*/nullptr, builder);
        if (!br.error_code.empty()) {
            err = op_label + ": modeled thread cut failed: " + br.error_message;
            return false;
        }
        solid_out = br.shape;
        return true;
    } catch (const Standard_Failure& f) {
        err = op_label + ": modeled thread build raised: " +
              (f.GetMessageString() ? f.GetMessageString() : "OCCT");
        return false;
    }
}

// Builds an ISO 4762 SHCS solid for `thread`×`length_mm`×`detail`, dispatching
// to the cosmetic blank plus (for simplified/modeled) a further shank-only
// cut. Never throws; failures surface as a recoverable `OP_FAILED`.
bool build_shcs(const std::string& op_label, const std::string& thread, double length_mm,
                ThreadDetail detail, TopoDS_Shape& solid_out, std::string& err) {
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
    TopoDS_Shape blank;
    if (!build_shcs_blank(op_label, thread, size, length_mm, blank, err)) {
        return false;
    }
    switch (detail) {
        case ThreadDetail::Cosmetic:
            solid_out = blank;
            return true;
        case ThreadDetail::Simplified:
            return cut_simplified_thread(op_label, blank, size.shank_diameter_mm / 2.0, length_mm,
                                         size.pitch_mm, solid_out, err);
        case ThreadDetail::Modeled:
            return cut_modeled_thread(op_label, blank, size.shank_diameter_mm / 2.0, length_mm,
                                      size.pitch_mm, solid_out, err);
    }
    err = op_label + ": unreachable thread_detail dispatch";
    return false;
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
    // old hardcoded M6×20 exactly; `thread_detail` defaults to `cosmetic` for
    // the same byte-identical reason (WP-2.5).
    std::string thread = kDefaultThread;
    double length_mm = kDefaultLengthMm;
    std::string thread_detail_str = kDefaultThreadDetail;
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
        if (gp.contains("thread_detail") && gp["thread_detail"].is_string()) {
            thread_detail_str = gp["thread_detail"].get<std::string>();
        }
    }
    if (!std::isfinite(length_mm) || length_mm <= 0.0) {
        return OpOutcome::fail("OP_FAILED", op_label + ": source.params.length must be finite and positive");
    }
    ThreadDetail thread_detail;
    if (!parse_thread_detail(thread_detail_str, thread_detail)) {
        return OpOutcome::fail("OP_FAILED", op_label + ": unknown thread_detail '" + thread_detail_str +
                                                 "' — known values: cosmetic, simplified, modeled");
    }

    TopoDS_Shape solid;
    std::string err;
    if (!build_shcs(op_label, thread, length_mm, thread_detail, solid, err)) {
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
