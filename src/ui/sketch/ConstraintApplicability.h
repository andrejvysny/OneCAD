/**
 * @file ConstraintApplicability.h
 * @brief Selection-based applicability rules for implemented sketch constraints.
 */

#ifndef ONECAD_UI_SKETCH_CONSTRAINT_APPLICABILITY_H
#define ONECAD_UI_SKETCH_CONSTRAINT_APPLICABILITY_H

#include "../../app/selection/SelectionTypes.h"
#include "../../core/sketch/SketchTypes.h"

#include <unordered_set>
#include <vector>

namespace onecad::core::sketch {
class Sketch;
}

namespace onecad::ui {

struct ConstraintApplicabilityResult {
    std::unordered_set<core::sketch::ConstraintType> applicableConstraints;

    bool hasApplicableConstraints() const {
        return !applicableConstraints.empty();
    }

    bool isApplicable(core::sketch::ConstraintType type) const {
        return applicableConstraints.find(type) != applicableConstraints.end();
    }
};

/**
 * Compute applicability for currently implemented MainWindow constraint apply paths.
 */
ConstraintApplicabilityResult evaluateConstraintApplicability(
    const core::sketch::Sketch* sketch,
    const std::vector<app::selection::SelectionItem>& selection);

} // namespace onecad::ui

#endif // ONECAD_UI_SKETCH_CONSTRAINT_APPLICABILITY_H
