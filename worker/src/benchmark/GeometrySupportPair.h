#pragma once

#include <string>

#include <TopoDS_Shape.hxx>

#include "benchmark/Types.h"

namespace onecad::benchmark {

/// Builds the `supportPair` recipe: one solid whose two named support surfaces
/// meet along a single shared edge at the requested dihedral angle.
///
/// Two constructions, chosen by the support kinds:
///
/// * **Prism family** (plane|cylinder × plane|cylinder). Every support is
///   prismatic along Z, so the whole configuration is a 2D profile extruded
///   along Z and the shared edge is the straight vertical line through the
///   ORIGIN. The dihedral is free over `(0, 180)`. A rolling-ball blend of this
///   edge is a cylinder, which is what lets `cylindricalRadius` and
///   `constantRadius` gate the result.
/// * **Cone family** (plane × cone). A frustum's base circle against its own
///   base plane. Here the dihedral is not free — it is `90 - halfAngle` — so
///   the declared angle is CHECKED against the geometry and a disagreement is
///   refused rather than silently generating a different case than the file
///   describes. The shared edge is the base circle and its blend is a torus.
///
/// Unimplemented kinds (sphere, torus, bspline) and concave supports are
/// refused by name; an option is not a capability.
bool make_support_pair(const RecipeParameters &parameters, TopoDS_Shape &out,
                       std::string &error);

} // namespace onecad::benchmark
