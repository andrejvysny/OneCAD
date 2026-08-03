// HoleTool.h — the CUTTING SOLID of a machined hole (WP-C T3, SCHEMA §7.3).
//
// Split out of HoleOp.cpp so the geometry (which is all arithmetic) is separable
// from the ref-resolution / publish plumbing (which is all session state).
//
// The tool is one fused solid, always drilled along `axis` (the host face's INWARD
// normal) starting from `origin` (the re-projected hole centre ON the face plane):
//
//   simple       drill cylinder, radius = diameter/2, length = `depth`
//   counterbore  + a coaxial cylinder, radius = cbDiameter/2, length = cbDepth,
//                  seated AT the face
//   countersink  + a coaxial CONE, csDiameter at the face converging to the drill
//                  diameter. Cone depth is derived, never authored:
//
//                      half     = csAngleDeg / 2            (half of the INCLUDED angle)
//                      csDepth  = (csDiameter − diameter) / 2 / tan(half)
//
//                  i.e. the axial run needed for the radius to fall from csR to
//                  drillR at the cone's half-angle. The drill continues below it.
//
// FACE OVERSHOOT. Every piece that is "seated at the face" actually starts
// [`kFaceOvershoot`] OUTSIDE the face and is lengthened by the same amount. The
// overshoot lies in the void on the outward side of the face (an outward normal
// points out of material by definition), so it removes NOTHING extra — but it keeps
// the tool's cap off the host's face plane, which is the coincident-face
// configuration OCCT booleans are worst at. Depths therefore stay EXACT: a blind
// `depth` hole removes exactly `π·(diameter/2)²·depth`.
//
// For the countersink the overshoot is applied as a FRUSTUM extension (the top
// radius grows by `overshoot·tan(half)`), so the cone's cross-section at the face
// plane is still exactly `csDiameter` — extending a cone along its own axis cannot
// move its profile.
//
// THROUGH-ALL (`depth` absent/null) uses a bounded ray extent, not a magic number:
// the farthest projection of the host's bbox corners along the axis, plus a margin,
// mirroring `ExtrudeOp`'s `through_all_distance`. Bounded — never `1e6` — so the
// boolean stays numerically sane.
#pragma once

#include <string>

#include <TopoDS_Shape.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>

namespace onecad::ops {

// How far outside the face every face-seated piece starts (mm). See the FACE
// OVERSHOOT note above — it is deliberately a fixed constant so two runs of the
// same document build a bit-identical tool.
inline constexpr double kFaceOvershoot = 1e-2;

// The fully-resolved dimensions of one hole, in mm / degrees. `depth <= 0` means
// THROUGH-ALL (the caller substitutes the ray extent before building).
struct HoleDims {
    double diameter = 0.0;
    double depth = 0.0;  // > 0 always by the time it reaches `build_hole_tool`
    // Counterbore (both zero ⇒ absent).
    double cb_diameter = 0.0;
    double cb_depth = 0.0;
    // Countersink (both zero ⇒ absent).
    double cs_diameter = 0.0;
    double cs_angle_deg = 0.0;
};

// The axial run a countersink cone needs for its radius to fall from
// `cs_diameter/2` to `diameter/2` at half of the included angle. Zero when the
// countersink is absent or degenerate.
double countersink_depth(const HoleDims& d);

// The bounded through-all length along `axis` from `origin` for a host `target`:
// the farthest bbox-corner projection + 1% of the bbox diagonal + 1 mm (the
// `ExtrudeOp::through_all_distance` margin). Falls back to a finite constant when
// the bbox is void. Always > 0.
double through_all_length(const gp_Pnt& origin, const gp_Dir& axis, const TopoDS_Shape& target);

// Build the fused cutting solid. Returns a null shape and fills `err` with a named,
// human-facing reason on failure (OCCT throw, degenerate radius, fuse failure).
TopoDS_Shape build_hole_tool(const gp_Pnt& origin, const gp_Dir& axis, const HoleDims& dims,
                             std::string& err);

}  // namespace onecad::ops
