#pragma once

#include "SketchTypes.h"
#include <gp_Pnt2d.hxx>
#include <optional>

namespace onecad::core::sketch {
class Sketch;
namespace constraints {
class TangentAtPointConstraint;
}
struct TangentEndpoint {
    EntityID point;
    gp_Pnt2d at;
    double nx = 0, ny = 0, span = 0;
};
std::optional<TangentEndpoint> tangentEndpoint(const Sketch& sketch, const EntityID& owner,
                                               CurvePosition role,
                                               const gp_Pnt2d* common = nullptr);
std::optional<double> tangentBranch(double ax, double ay, double bx, double by);
std::string validateEndpointTangency(const Sketch& sketch,
                                     const constraints::TangentAtPointConstraint& constraint);
} // namespace onecad::core::sketch
