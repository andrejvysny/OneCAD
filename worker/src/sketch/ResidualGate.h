#pragma once

#include "ExactTruth.h"

namespace onecad::core::sketch {

class Sketch;
class SketchConstraint;

ResidualCheck checkResiduals(const Sketch& sketch);
ResidualCheck checkResidualOperands(const Sketch& sketch);
ResidualCheck checkResiduals(const Sketch& sketch,
                            const std::vector<const SketchConstraint*>& constraints);
ExactTruth classifyExact(ResidualCheck residuals, bool numerical_success, int dof,
                         bool redundant, const std::vector<ConstraintID>& rank_conflicts);

}  // namespace onecad::core::sketch
