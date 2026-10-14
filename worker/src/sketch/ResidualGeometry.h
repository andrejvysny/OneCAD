#pragma once

#include "SketchTypes.h"

#include <optional>
#include <vector>

namespace onecad::core::sketch {

class Sketch;
class SketchConstraint;

bool finiteEntity(const Sketch& sketch, const EntityID& id);
bool finiteOperands(const Sketch& sketch, const SketchConstraint& constraint);
std::optional<double> angularSpan(const Sketch& sketch, const SketchConstraint& constraint);
std::optional<double> supportLineResidual(const Sketch& sketch, const SketchConstraint& constraint);
struct TangentAngle {
    double residual = 0.0;
    double span = 0.0;
};
std::vector<TangentAngle> weldedTangentAngles(const Sketch& sketch,
                                           const SketchConstraint& constraint);

}  // namespace onecad::core::sketch
