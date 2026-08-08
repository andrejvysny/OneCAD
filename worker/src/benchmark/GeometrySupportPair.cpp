#include "benchmark/GeometrySupportPair.h"

#include <cmath>

#include <BRepAlgoAPI_Common.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCone.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Trsf.hxx>

namespace onecad::benchmark {
namespace {

constexpr double kPi = 3.141592653589793238462643383279502884;

double radians(double degrees) { return degrees * kPi / 180.0; }

/// A support carried by the prism family, described in the shared 2D profile:
/// the support leaves the origin along `tangent`, with material on the
/// `inward` side. For support A that is the +X axis with material at +Y; for
/// support B it is the direction at the requested dihedral.
struct ProfileSupport {
  double tangent_degrees = 0.0;
  double inward_x = 0.0;
  double inward_y = 1.0;
};

TopoDS_Shape half_space_prism(const ProfileSupport &support, double extent,
                              double height) {
  // Template occupies y >= 0 with its support face on the XZ plane, then spins
  // about Z so that face carries the support's own tangent direction.
  const TopoDS_Shape box =
      BRepPrimAPI_MakeBox(gp_Pnt(-extent * 0.5, 0.0, 0.0), extent, extent, height)
          .Shape();
  gp_Trsf rotation;
  rotation.SetRotation(gp_Ax1(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 0.0, 1.0)),
                       radians(support.tangent_degrees));
  return BRepBuilderAPI_Transform(box, rotation, true).Shape();
}

/// A cylinder tangent to the support's line AT THE ORIGIN, with material
/// inside it — so near the origin it is locally the same half-plane a planar
/// support would give, and the dihedral is exactly the profile angle.
TopoDS_Shape tangent_cylinder(const ProfileSupport &support, double radius,
                              double height) {
  const gp_Ax2 axis(
      gp_Pnt(radius * support.inward_x, radius * support.inward_y, 0.0),
      gp_Dir(0.0, 0.0, 1.0));
  return BRepPrimAPI_MakeCylinder(axis, radius, height).Shape();
}

bool support_solid(const SupportSpec &support, const ProfileSupport &profile,
                   double extent, double height, TopoDS_Shape &out,
                   std::string &error) {
  if (support.kind == "plane") {
    out = half_space_prism(profile, extent, height);
    return true;
  }
  if (support.kind == "cylinder") {
    out = tangent_cylinder(profile, *support.radius, height);
    return true;
  }
  error = "support kind '" + support.kind + "' is not generated yet";
  return false;
}

bool single_solid(const TopoDS_Shape &shape, TopoDS_Shape &out) {
  int count = 0;
  for (TopExp_Explorer it(shape, TopAbs_SOLID); it.More(); it.Next()) {
    out = it.Current();
    ++count;
  }
  return count == 1;
}

double lateral_extent(const RecipeParameters &parameters,
                      const SupportSpec &a, const SupportSpec &b) {
  if (parameters.neighbor_size && *parameters.neighbor_size > 0.0)
    return *parameters.neighbor_size;
  double radius = 0.0;
  if (a.kind == "cylinder")
    radius = std::max(radius, *a.radius);
  if (b.kind == "cylinder")
    radius = std::max(radius, *b.radius);
  // A planar support has no size of its own; it is sized to comfortably
  // outrun whichever curved support bounds the region.
  return radius > 0.0 ? 4.0 * radius : 50.0;
}

bool make_prism_pair(const RecipeParameters &parameters, const SupportSpec &a,
                     const SupportSpec &b, double height, TopoDS_Shape &out,
                     std::string &error) {
  const double dihedral = *parameters.dihedral_degrees;
  if (dihedral <= 1.0e-6 || dihedral >= 180.0 - 1.0e-6) {
    error = "prismatic support pairs need a dihedral strictly inside (0, 180)";
    return false;
  }
  const double extent = lateral_extent(parameters, a, b);
  const ProfileSupport profile_a{0.0, 0.0, 1.0};
  // Support B leaves the origin at the dihedral angle; its inward normal is B's
  // tangent turned a quarter turn back toward A, so the material is the wedge
  // between the two tangents.
  const ProfileSupport profile_b{dihedral + 180.0, std::sin(radians(dihedral)),
                                 -std::cos(radians(dihedral))};
  TopoDS_Shape solid_a;
  TopoDS_Shape solid_b;
  if (!support_solid(a, profile_a, extent, height, solid_a, error) ||
      !support_solid(b, profile_b, extent, height, solid_b, error))
    return false;
  BRepAlgoAPI_Common common(solid_a, solid_b);
  if (!common.IsDone() || !single_solid(common.Shape(), out)) {
    error = "support pair intersection did not produce a single solid";
    return false;
  }
  return true;
}

bool make_cone_pair(const RecipeParameters &parameters, const SupportSpec &cone,
                    double height, TopoDS_Shape &out, std::string &error) {
  const double dihedral = *parameters.dihedral_degrees;
  const double half_angle = *cone.half_angle_degrees;
  // The frustum's base circle meets its base plane at exactly 90 - halfAngle.
  // The case does not get to claim otherwise.
  if (std::abs((90.0 - half_angle) - dihedral) > 1.0e-6) {
    error = "plane-cone dihedral must equal 90 - halfAngleDegrees";
    return false;
  }
  const double base = *cone.radius;
  const double top = base - height * std::tan(radians(half_angle));
  if (top <= 1.0e-9) {
    error = "plane-cone frustum collapses to an apex at this edge length";
    return false;
  }
  const gp_Ax2 axis(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 0.0, 1.0));
  const TopoDS_Shape frustum =
      BRepPrimAPI_MakeCone(axis, base, top, height).Shape();
  return single_solid(frustum, out)
             ? true
             : (error = "cone recipe did not produce a single solid", false);
}

} // namespace

bool make_support_pair(const RecipeParameters &parameters, TopoDS_Shape &out,
                       std::string &error) {
  const SupportSpec &a = *parameters.support_a;
  const SupportSpec &b = *parameters.support_b;
  if (!parameters.convexity.empty() && parameters.convexity != "convex") {
    error = "only convex support pairs are generated yet";
    return false;
  }
  const double height =
      parameters.edge_length && *parameters.edge_length > 0.0
          ? *parameters.edge_length
          : 50.0;
  const bool cone_pair = (a.kind == "plane" && b.kind == "cone") ||
                         (a.kind == "cone" && b.kind == "plane");
  if (cone_pair)
    return make_cone_pair(parameters, a.kind == "cone" ? a : b, height, out,
                          error);
  return make_prism_pair(parameters, a, b, height, out, error);
}

} // namespace onecad::benchmark
