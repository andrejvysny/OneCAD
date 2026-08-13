// ComponentGenerators.cpp — see ComponentGenerators.h.
//
// The thread cutters and the ISO 4762 builder moved here VERBATIM from
// `ComponentOp.cpp` in WP-A1 (they were the whole of the generator lane back
// when there was exactly one family). The only change to either cutter is a
// new `thread_length_mm` argument — the threaded run measured from the shank
// TIP — which the fully-threaded families pass as the full shank length,
// reproducing the previous behavior exactly. ISO 4762's exact-volume ctests
// are the proof that the move changed nothing.
#include "ops/ComponentGenerators.h"

#include <algorithm>
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
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <BRepPrimAPI_MakeSphere.hxx>
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
#include "ops/MachineElementTables.h"
#include "ops/OpCommon.h"

namespace onecad::ops {

using nlohmann::json;
namespace ft = onecad::ops::fasteners;
namespace me = onecad::ops::machine_elements;

namespace {

constexpr double kPi = 3.14159265358979323846;

// The bearing code an instance that names none gets. Same rule as the op
// layer's `M6` default for `thread`: an ABSENT param takes the family's
// documented default (the seed package declares the identical one), while a
// PRESENT but unknown one is refused. Never a substitution.
constexpr const char* kDefaultBearingCode = "608";

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

// A regular hexagonal prism of `width_across_flats` (the standards' `s`),
// `height` tall, based at z = 0 and rising +Z. Vertices sit on the
// circumradius `s / √3`, which puts the FLATS at `s / 2` from the axis —
// across-flats is what every fastener standard tabulates.
bool build_hex_prism(const std::string& op_label, double width_across_flats, double height,
                     TopoDS_Shape& out, std::string& err) {
    const double circumradius = width_across_flats / std::sqrt(3.0);
    BRepBuilderAPI_MakePolygon poly;
    for (int i = 0; i < 6; ++i) {
        const double a = (kPi / 3.0) * static_cast<double>(i);
        poly.Add(gp_Pnt(circumradius * std::cos(a), circumradius * std::sin(a), 0.0));
    }
    poly.Close();
    if (!poly.IsDone()) {
        err = op_label + ": hex head profile failed";
        return false;
    }
    BRepBuilderAPI_MakeFace face(poly.Wire());
    if (!face.IsDone()) {
        err = op_label + ": hex head face failed";
        return false;
    }
    BRepPrimAPI_MakePrism prism(face.Shape(), gp_Vec(0.0, 0.0, height));
    if (!prism.IsDone() || prism.Shape().IsNull()) {
        err = op_label + ": hex head prism failed";
        return false;
    }
    out = prism.Shape();
    return true;
}

// A spherical cap of base Ø `head_diameter` and height `head_height`, based at
// z = 0 — the button head of ISO 7380-1, rendered as a pure dome.
//
// DELIBERATE FIDELITY CUT, stated rather than hidden: a real button head has a
// short cylindrical land under the dome and a hex socket in its crown. Neither
// changes where the part seats, what it clears, or how it mates; the same
// reasoning `thread_detail: cosmetic` rests on. The socket is invisible at
// assembly scale and is not modeled for ANY family here.
bool build_dome_head(const std::string& op_label, double head_diameter, double head_height,
                     TopoDS_Shape& out, std::string& err) {
    const double a = head_diameter / 2.0;
    const double k = head_height;
    if (!(a > 0.0) || !(k > 0.0)) {
        err = op_label + ": button head needs a positive head diameter and height";
        return false;
    }
    // Sphere through (a, 0) and (0, k): R = (a² + k²) / 2k, centered on the
    // axis at z = k − R, so the sphere meets z = 0 exactly at radius a.
    const double r_sphere = (a * a + k * k) / (2.0 * k);
    const TopoDS_Shape sphere =
        BRepPrimAPI_MakeSphere(gp_Pnt(0.0, 0.0, k - r_sphere), r_sphere).Shape();
    const TopoDS_Shape band =
        BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 0.0, 1.0)), a, k)
            .Shape();
    return generator_boolean(op_label + ": button head cap", sphere, band,
                             app::BooleanMode::Intersect, out, err);
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

// A string param, or `fallback` when the caller sent none.
std::string text_param(const GeneratorRequest& req, const std::string& key,
                       const std::string& fallback) {
    const auto it = req.text_params.find(key);
    return it == req.text_params.end() ? fallback : it->second;
}

// `lookup`'s twin for the families keyed by something other than a thread
// designation. `what` names the key in the error ("bearing code").
template <typename Row>
const Row* lookup_key(const std::map<std::string, Row>& table, const std::string& op_label,
                      const std::string& what, const std::string& key, std::string& err) {
    const auto it = table.find(key);
    if (it == table.end()) {
        err = op_label + ": unknown " + what + " '" + key +
              "' — known values: " + me::known_keys(table);
        return nullptr;
    }
    return &it->second;
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

// ISO 7380-1 button head screw: a domed head fused to the shank.
bool build_button_head(const GeneratorRequest& req, TopoDS_Shape& out, std::string& err) {
    const ft::ScrewSize* size = lookup(ft::iso7380_table(), req.op_label, req.thread, err);
    if (size == nullptr) return false;
    try {
        TopoDS_Shape head;
        if (!build_dome_head(req.op_label, size->head_diameter_mm, size->head_height_mm, head,
                             err)) {
            return false;
        }
        TopoDS_Shape blank;
        if (!fuse_head_and_shank(req.op_label, req.thread, head, size->shank_diameter_mm,
                                 req.length_mm, blank, err)) {
            return false;
        }
        return apply_thread_detail(req.op_label, blank, size->shank_diameter_mm, req.length_mm,
                                   req.length_mm, size->pitch_mm, req.detail, out, err);
    } catch (const Standard_Failure& f) {
        err = req.op_label + ": " + req.thread + " button head build raised: " +
              (f.GetMessageString() ? f.GetMessageString() : "OCCT");
        return false;
    }
}

// ISO 4014 / ISO 4017 hexagon head screw. The two standards share every
// geometric column and differ ONLY in the threaded run: 4017 threads the whole
// shank (`thread_length_mm == 0` in the table, read as "all of it"), 4014
// threads the standard's `b` from the tip — capped at the shank length, so a
// bolt shorter than `b` is threaded end to end rather than beyond its own tip.
bool build_hex_screw(const std::map<std::string, ft::HexScrewSize>& table,
                     const GeneratorRequest& req, TopoDS_Shape& out, std::string& err) {
    const ft::HexScrewSize* size = lookup(table, req.op_label, req.thread, err);
    if (size == nullptr) return false;
    try {
        TopoDS_Shape head;
        if (!build_hex_prism(req.op_label, size->width_across_flats_mm, size->head_height_mm, head,
                             err)) {
            return false;
        }
        TopoDS_Shape blank;
        if (!fuse_head_and_shank(req.op_label, req.thread, head, size->shank_diameter_mm,
                                 req.length_mm, blank, err)) {
            return false;
        }
        const double threaded = (size->thread_length_mm <= 0.0)
                                    ? req.length_mm
                                    : std::min(size->thread_length_mm, req.length_mm);
        return apply_thread_detail(req.op_label, blank, size->shank_diameter_mm, req.length_mm,
                                   threaded, size->pitch_mm, req.detail, out, err);
    } catch (const Standard_Failure& f) {
        err = req.op_label + ": " + req.thread + " hex screw build raised: " +
              (f.GetMessageString() ? f.GetMessageString() : "OCCT");
        return false;
    }
}

// ISO 4032 hexagon nut: a hex prism seated at z = 0, bored through. The
// internal thread is cosmetic in v1 (a plain bore at the nominal Ø) — a nut's
// thread is invisible in assembly and modeling it would be the most fragile
// geometry in the catalog for no placement benefit. `thread_detail` is
// accepted and ignored, and the seed package does not offer it.
bool build_hex_nut(const GeneratorRequest& req, TopoDS_Shape& out, std::string& err) {
    const ft::NutSize* size = lookup(ft::iso4032_table(), req.op_label, req.thread, err);
    if (size == nullptr) return false;
    try {
        TopoDS_Shape body;
        if (!build_hex_prism(req.op_label, size->width_across_flats_mm, size->thickness_mm, body,
                             err)) {
            return false;
        }
        // Bore overshoots both faces by 1 mm so the cut is a clean through
        // hole, never a coplanar-face boolean.
        const gp_Ax2 bore_axis(gp_Pnt(0.0, 0.0, -1.0), gp_Dir(0.0, 0.0, 1.0));
        const TopoDS_Shape bore =
            BRepPrimAPI_MakeCylinder(bore_axis, size->bore_diameter_mm / 2.0,
                                     size->thickness_mm + 2.0)
                .Shape();
        return generator_boolean(req.op_label + ": " + req.thread + " nut bore", body, bore,
                                 app::BooleanMode::Cut, out, err);
    } catch (const Standard_Failure& f) {
        err = req.op_label + ": " + req.thread + " nut build raised: " +
              (f.GetMessageString() ? f.GetMessageString() : "OCCT");
        return false;
    }
}

// ISO 7089 / ISO 7093-1 plain washer: an annulus seated at z = 0. The two
// standards differ only in their table (normal vs large series).
bool build_washer(const std::map<std::string, ft::WasherSize>& table, const GeneratorRequest& req,
                  TopoDS_Shape& out, std::string& err) {
    const ft::WasherSize* size = lookup(table, req.op_label, req.thread, err);
    if (size == nullptr) return false;
    try {
        const gp_Ax2 disc_axis(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 0.0, 1.0));
        const TopoDS_Shape disc =
            BRepPrimAPI_MakeCylinder(disc_axis, size->outer_diameter_mm / 2.0, size->thickness_mm)
                .Shape();
        const gp_Ax2 bore_axis(gp_Pnt(0.0, 0.0, -1.0), gp_Dir(0.0, 0.0, 1.0));
        const TopoDS_Shape bore =
            BRepPrimAPI_MakeCylinder(bore_axis, size->inner_diameter_mm / 2.0,
                                     size->thickness_mm + 2.0)
                .Shape();
        return generator_boolean(req.op_label + ": " + req.thread + " washer bore", disc, bore,
                                 app::BooleanMode::Cut, out, err);
    } catch (const Standard_Failure& f) {
        err = req.op_label + ": " + req.thread + " washer build raised: " +
              (f.GetMessageString() ? f.GetMessageString() : "OCCT");
        return false;
    }
}

// Race radial wall and face relief, as fractions of the bearing's own
// boundary dimensions. RENDERING choices, deliberately NOT table data: ISO 15
// tabulates the boundary dimensions and nothing inside them, so a tabulated
// "race width" would be an invented dimension (spec §6.2: "stepped-ring
// geometry suffices").
constexpr double kBearingRaceFraction = 0.25;    // of the radial wall (D − d)/2
constexpr double kBearingReliefFraction = 0.15;  // of the width B

// ISO 15 deep-groove ball bearing: ONE revolved solid, never a two-ring
// assembly — a component resolves to exactly one solid (spec §9), and the
// rings of a real bearing are connected only through its balls.
//
// The r–z cross-section is the boundary rectangle (bore → OD, 0 → width) with
// a shallow annular relief cut into BOTH faces between the two races, so the
// part reads as a bearing rather than a plain spacer while staying one
// connected ring. Balls, cage, seals and chamfers are not modeled: none of
// them changes where the bearing seats or what it clears, the same reasoning
// `thread_detail: cosmetic` rests on.
bool build_bearing(const GeneratorRequest& req, TopoDS_Shape& out, std::string& err) {
    const std::string code = text_param(req, "code", kDefaultBearingCode);
    const me::BearingSize* size =
        lookup_key(me::iso15_table(), req.op_label, "bearing code", code, err);
    if (size == nullptr) return false;
    try {
        const double ri = size->bore_diameter_mm / 2.0;
        const double ro = size->outer_diameter_mm / 2.0;
        const double w = size->width_mm;
        const double t = (ro - ri) * kBearingRaceFraction;
        const double g = w * kBearingReliefFraction;

        // Closed r–z profile, walked once around: up the bore, across the
        // lower relief, up the outside wall, back across the upper relief.
        BRepBuilderAPI_MakePolygon poly;
        poly.Add(gp_Pnt(ri, 0.0, 0.0));
        poly.Add(gp_Pnt(ri + t, 0.0, 0.0));
        poly.Add(gp_Pnt(ri + t, 0.0, g));
        poly.Add(gp_Pnt(ro - t, 0.0, g));
        poly.Add(gp_Pnt(ro - t, 0.0, 0.0));
        poly.Add(gp_Pnt(ro, 0.0, 0.0));
        poly.Add(gp_Pnt(ro, 0.0, w));
        poly.Add(gp_Pnt(ro - t, 0.0, w));
        poly.Add(gp_Pnt(ro - t, 0.0, w - g));
        poly.Add(gp_Pnt(ri + t, 0.0, w - g));
        poly.Add(gp_Pnt(ri + t, 0.0, w));
        poly.Add(gp_Pnt(ri, 0.0, w));
        poly.Close();
        if (!poly.IsDone()) {
            err = req.op_label + ": bearing " + code + " section profile failed";
            return false;
        }
        BRepBuilderAPI_MakeFace face(poly.Wire());
        if (!face.IsDone()) {
            err = req.op_label + ": bearing " + code + " section face failed";
            return false;
        }
        BRepPrimAPI_MakeRevol revol(face.Shape(), gp_Ax1(gp_Pnt(0.0, 0.0, 0.0),
                                                        gp_Dir(0.0, 0.0, 1.0)),
                                    2.0 * kPi, /*Copy=*/true);
        if (!revol.IsDone() || revol.Shape().IsNull()) {
            err = req.op_label + ": bearing " + code + " revolve failed";
            return false;
        }
        out = revol.Shape();
        return true;
    } catch (const Standard_Failure& f) {
        err = req.op_label + ": bearing " + code + " build raised: " +
              (f.GetMessageString() ? f.GetMessageString() : "OCCT");
        return false;
    }
}

// NEMA stepper motor (frame `"17"` / `"23"`): body block + pilot boss + shaft,
// fused, with the four mounting holes drilled blind into the faceplate.
//
// Body length is the free param (spec §6.2), and it is the ONLY free
// dimension — everything else is fixed by the frame. A length the frame does
// not admit is refused rather than clamped: a 5 mm-long NEMA 17 is not a
// smaller motor, it is not a motor.
//
// DELIBERATE FIDELITY CUTS, stated rather than hidden: the body is a plain
// square prism (a real one has chamfered corners and a stepped end-bell), the
// shaft has no D-flat or keyway, and there are no wire leads or connector.
// None of the three changes the footprint, the pilot fit, or the shaft
// centre — the only things a mate resolves against.
bool build_stepper_motor(const std::string& frame, const GeneratorRequest& req, TopoDS_Shape& out,
                         std::string& err) {
    const me::MotorSize* size =
        lookup_key(me::nema_table(), req.op_label, "NEMA frame", frame, err);
    if (size == nullptr) return false;
    const std::string what = "NEMA " + frame;
    const double body_length =
        req.length_given ? req.length_mm : size->default_body_length_mm;
    if (body_length < size->min_body_length_mm) {
        err = req.op_label + ": " + what + " body length " + std::to_string(body_length) +
              " mm is below the frame's minimum of " + std::to_string(size->min_body_length_mm) +
              " mm";
        return false;
    }
    try {
        const double half = size->frame_square_mm / 2.0;
        const TopoDS_Shape body =
            BRepPrimAPI_MakeBox(gp_Pnt(-half, -half, 0.0), size->frame_square_mm,
                                size->frame_square_mm, body_length)
                .Shape();
        const TopoDS_Shape pilot =
            BRepPrimAPI_MakeCylinder(
                gp_Ax2(gp_Pnt(0.0, 0.0, -size->pilot_height_mm), gp_Dir(0.0, 0.0, 1.0)),
                size->pilot_diameter_mm / 2.0, size->pilot_height_mm)
                .Shape();
        const TopoDS_Shape shaft =
            BRepPrimAPI_MakeCylinder(
                gp_Ax2(gp_Pnt(0.0, 0.0, -size->shaft_length_mm), gp_Dir(0.0, 0.0, 1.0)),
                size->shaft_diameter_mm / 2.0, size->shaft_length_mm)
                .Shape();

        // Pilot and shaft overlap (the shaft runs through the boss), so they
        // are fused one at a time rather than offered as one compound tool.
        TopoDS_Shape fused;
        if (!generator_boolean(req.op_label + ": " + what + " pilot boss fuse", body, pilot,
                               app::BooleanMode::Add, fused, err)) {
            return false;
        }
        if (!generator_boolean(req.op_label + ": " + what + " shaft fuse", fused, shaft,
                               app::BooleanMode::Add, fused, err)) {
            return false;
        }

        // Four blind mounting holes on the square pattern, drilled from the
        // faceplate into the body. One compound tool, one boolean — the same
        // rule the simplified thread's rings follow. The cutters overshoot
        // below z = 0 so the cut is never a coplanar-face boolean; nothing of
        // the part lives out there (the pattern clears the pilot boss).
        BRep_Builder cb;
        TopoDS_Compound holes;
        cb.MakeCompound(holes);
        const double p = size->mounting_hole_pitch_mm / 2.0;
        for (const double sx : {-1.0, 1.0}) {
            for (const double sy : {-1.0, 1.0}) {
                cb.Add(holes,
                       BRepPrimAPI_MakeCylinder(
                           gp_Ax2(gp_Pnt(sx * p, sy * p, -1.0), gp_Dir(0.0, 0.0, 1.0)),
                           size->mounting_hole_diameter_mm / 2.0,
                           size->mounting_hole_depth_mm + 1.0)
                           .Shape());
            }
        }
        return generator_boolean(req.op_label + ": " + what + " mounting holes", fused, holes,
                                 app::BooleanMode::Cut, out, err);
    } catch (const Standard_Failure& f) {
        err = req.op_label + ": " + what + " build raised: " +
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
    return "iso15, iso4014, iso4017, iso4032, iso4762, iso7089, iso7093, iso7380, nema17, nema23";
}

bool build_component(const std::string& generator_id, const GeneratorRequest& req,
                     TopoDS_Shape& solid_out, std::string& err) {
    if (generator_id == "iso4762") return build_socket_cap(req, solid_out, err);
    if (generator_id == "iso7380") return build_button_head(req, solid_out, err);
    if (generator_id == "iso4014") {
        return build_hex_screw(ft::iso4014_table(), req, solid_out, err);
    }
    if (generator_id == "iso4017") {
        return build_hex_screw(ft::iso4017_table(), req, solid_out, err);
    }
    if (generator_id == "iso4032") return build_hex_nut(req, solid_out, err);
    if (generator_id == "iso7089") return build_washer(ft::iso7089_table(), req, solid_out, err);
    if (generator_id == "iso7093") return build_washer(ft::iso7093_table(), req, solid_out, err);
    if (generator_id == "iso15") return build_bearing(req, solid_out, err);
    if (generator_id == "nema17") return build_stepper_motor("17", req, solid_out, err);
    if (generator_id == "nema23") return build_stepper_motor("23", req, solid_out, err);
    err = req.op_label + ": unknown source.generatorId '" + generator_id +
          "' — known generators: " + known_generator_ids();
    return false;
}

}  // namespace onecad::ops
