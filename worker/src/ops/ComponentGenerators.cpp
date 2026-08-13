// ComponentGenerators.cpp — see ComponentGenerators.h.
//
// The thread cutters and the ISO 4762 builder moved here VERBATIM from
// `ComponentOp.cpp` in WP-A1 (they were the whole of the generator lane back
// when there was exactly one family). Nine more families were added and then
// removed again with the seed catalog; ISO 4762's pinned exact-volume ctests
// are the proof that neither move changed the one family that stayed.
//
// The cutters keep their `thread_length_mm` argument — the threaded run
// measured from the shank TIP — even though the sole caller passes the full
// shank length. It is the seam a partially-threaded family (an ISO 4014 bolt's
// `b`) needs, and collapsing it would have to be undone to add one back.
#include "ops/ComponentGenerators.h"

#include <cmath>
#include <map>
#include <memory>
#include <string>

#include <BRep_Builder.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_MakeShape.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepLib.hxx>
#include <BRepOffsetAPI_MakePipeShell.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <Geom2d_Line.hxx>
#include <Geom_CylindricalSurface.hxx>
#include <Standard_Failure.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Wire.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Ax3.hxx>
#include <gp_Dir.hxx>
#include <gp_Dir2d.hxx>
#include <gp_Pnt.hxx>
#include <gp_Pnt2d.hxx>
#include <gp_Vec.hxx>

#include "modeling/BooleanMode.h"
#include "ops/FastenerTables.h"
#include "ops/OpCommon.h"

namespace onecad::ops {

using nlohmann::json;
namespace ft = onecad::ops::fasteners;

namespace {

constexpr double kPi = 3.14159265358979323846;

// One boolean, with the determinism/cancel arguments every generator uses
// identically (single-threaded, no occtOptions, no cancel token — a generator
// is a handful of primitives, not a user-scale regen step).
bool generator_boolean(const std::string& what, const TopoDS_Shape& target,
                       const TopoDS_Shape& tool, app::BooleanMode mode, TopoDS_Shape& out,
                       std::string& err) {
    std::shared_ptr<BRepBuilderAPI_MakeShape> builder;
    const BooleanResult br = checked_boolean(target, tool, mode, /*parallel=*/false, json::object(),
                                             /*cancel=*/nullptr, builder);
    if (!br.error_code.empty()) {
        err = what + " failed: " + br.error_message;
        return false;
    }
    out = br.shape;
    return true;
}

// Fuses an already-built head (occupying z ∈ [0, k]) to a shank of
// `shank_diameter` running z ∈ [−length_mm, 0]. Head-first/tool-second boolean
// order is load-bearing for ISO 4762's pinned exact-volume ctests.
bool fuse_head_and_shank(const std::string& op_label, const std::string& thread,
                         const TopoDS_Shape& head, double shank_diameter, double length_mm,
                         TopoDS_Shape& out, std::string& err) {
    const gp_Ax2 shank_axis(gp_Pnt(0.0, 0.0, -length_mm), gp_Dir(0.0, 0.0, 1.0));
    const TopoDS_Shape shank =
        BRepPrimAPI_MakeCylinder(shank_axis, shank_diameter / 2.0, length_mm).Shape();
    return generator_boolean(op_label + ": " + thread + " head/shank fuse", head, shank,
                             app::BooleanMode::Add, out, err);
}

// `simplified` thread detail: cuts N discrete annular grooves out of the
// shank (never the head), one per pitch, spanning `thread_length_mm` measured
// from the tip at z = −length_mm. Each groove is a shallow V revolved 360°
// around the shank axis into a ring solid; ALL N rings are accumulated into
// ONE `TopoDS_Compound` tool and cut in a SINGLE boolean — not N sequential
// booleans, which would accumulate tolerance debris and turn each ring into
// its own independent failure point.
bool cut_simplified_thread(const std::string& op_label, const TopoDS_Shape& blank,
                           double shank_radius_mm, double length_mm, double thread_length_mm,
                           double pitch_mm, TopoDS_Shape& solid_out, std::string& err) {
    const int n = static_cast<int>(std::floor(thread_length_mm / pitch_mm));
    if (n <= 0) {
        // Threaded run too short for even one groove at this pitch — cosmetic blank stands.
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
            const double z = -length_mm + (static_cast<double>(i) + 0.5) * pitch_mm;
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
        return generator_boolean(op_label + ": simplified thread cut", blank, rings,
                                 app::BooleanMode::Cut, solid_out, err);
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
// thread start/end) — kernelbench's extremes suite is where that gets
// stress-tested across the table; this function only proves it works.
bool cut_modeled_thread(const std::string& op_label, const TopoDS_Shape& blank,
                        double shank_radius_mm, double length_mm, double thread_length_mm,
                        double pitch_mm, TopoDS_Shape& solid_out, std::string& err) {
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
        const double t_end = thread_length_mm / unit_v;

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

        return generator_boolean(op_label + ": modeled thread cut", blank, thread_tool,
                                 app::BooleanMode::Cut, solid_out, err);
    } catch (const Standard_Failure& f) {
        err = op_label + ": modeled thread build raised: " +
              (f.GetMessageString() ? f.GetMessageString() : "OCCT");
        return false;
    }
}

// Applies `detail` to an already-fused screw blank. `thread_length_mm` is the
// threaded run measured from the tip — the full shank for a fully-threaded
// family, the standard's `b` for a partially-threaded one.
bool apply_thread_detail(const std::string& op_label, const TopoDS_Shape& blank,
                         double shank_diameter_mm, double length_mm, double thread_length_mm,
                         double pitch_mm, ThreadDetail detail, TopoDS_Shape& solid_out,
                         std::string& err) {
    switch (detail) {
        case ThreadDetail::Cosmetic:
            solid_out = blank;
            return true;
        case ThreadDetail::Simplified:
            return cut_simplified_thread(op_label, blank, shank_diameter_mm / 2.0, length_mm,
                                         thread_length_mm, pitch_mm, solid_out, err);
        case ThreadDetail::Modeled:
            return cut_modeled_thread(op_label, blank, shank_diameter_mm / 2.0, length_mm,
                                      thread_length_mm, pitch_mm, solid_out, err);
    }
    err = op_label + ": unreachable thread_detail dispatch";
    return false;
}

// Looks `thread` up in `table`, failing loudly with the known sizes — never a
// silent nearest-size substitution (spec §0 invariant 4).
template <typename Row>
const Row* lookup(const std::map<std::string, Row>& table, const std::string& op_label,
                  const std::string& thread, std::string& err) {
    const auto it = table.find(thread);
    if (it == table.end()) {
        err = op_label + ": unknown thread designation '" + thread +
              "' — known sizes: " + ft::known_threads(table);
        return nullptr;
    }
    return &it->second;
}

// --- the families ------------------------------------------------------

// ISO 4762 socket cap screw: a plain cylindrical head fused to the shank.
bool build_socket_cap(const GeneratorRequest& req, TopoDS_Shape& out, std::string& err) {
    const ft::ScrewSize* size = lookup(ft::iso4762_table(), req.op_label, req.thread, err);
    if (size == nullptr) return false;
    try {
        const gp_Ax2 head_axis(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 0.0, 1.0));
        const TopoDS_Shape head =
            BRepPrimAPI_MakeCylinder(head_axis, size->head_diameter_mm / 2.0, size->head_height_mm)
                .Shape();
        TopoDS_Shape blank;
        if (!fuse_head_and_shank(req.op_label, req.thread, head, size->shank_diameter_mm,
                                 req.length_mm, blank, err)) {
            return false;
        }
        return apply_thread_detail(req.op_label, blank, size->shank_diameter_mm, req.length_mm,
                                   req.length_mm, size->pitch_mm, req.detail, out, err);
    } catch (const Standard_Failure& f) {
        err = req.op_label + ": " + req.thread + " socket cap build raised: " +
              (f.GetMessageString() ? f.GetMessageString() : "OCCT");
        return false;
    }
}

}  // namespace

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

std::string known_generator_ids() {
    return "iso4762";
}

bool build_component(const std::string& generator_id, const GeneratorRequest& req,
                     TopoDS_Shape& solid_out, std::string& err) {
    if (generator_id == "iso4762") return build_socket_cap(req, solid_out, err);
    err = req.op_label + ": unknown source.generatorId '" + generator_id +
          "' — known generators: " + known_generator_ids();
    return false;
}

}  // namespace onecad::ops
