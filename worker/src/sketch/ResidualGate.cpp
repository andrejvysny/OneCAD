#include "ResidualGate.h"

#include "ResidualGeometry.h"
#include "Sketch.h"

#include <algorithm>
#include <cmath>

namespace onecad::core::sketch {
namespace {

std::optional<ResidualDimension> dimension(ConstraintType type) {
    switch (type) {
        case ConstraintType::Parallel:
        case ConstraintType::Perpendicular:
        case ConstraintType::Angle:
        case ConstraintType::TangentAtPoint:
            return ResidualDimension::Angle;
        case ConstraintType::Coincident:
        case ConstraintType::Horizontal:
        case ConstraintType::Vertical:
        case ConstraintType::Fixed:
        case ConstraintType::Midpoint:
        case ConstraintType::OnCurve:
        case ConstraintType::Tangent:
        case ConstraintType::Concentric:
        case ConstraintType::Equal:
        case ConstraintType::Distance:
        case ConstraintType::HorizontalDistance:
        case ConstraintType::VerticalDistance:
        case ConstraintType::Radius:
        case ConstraintType::Diameter:
        case ConstraintType::Symmetric:
        case ConstraintType::HorizontalPoints:
        case ConstraintType::VerticalPoints:
            return ResidualDimension::Length;
    }
    return std::nullopt;
}

bool measure(ResidualCheck& result, const ConstraintID& id, ResidualDimension unit,
             double residual, double tolerance, const std::string& component) {
    residual = std::abs(residual);
    const double normalized = residual / tolerance;
    if (!std::isfinite(residual) || !std::isfinite(tolerance) || tolerance <= 0.0 ||
        !std::isfinite(normalized)) return false;
    result.normalized_max = std::max(result.normalized_max, normalized);
    if (residual > tolerance) {
        result.violations.push_back({id, unit, residual, tolerance, normalized, component});
    }
    return true;
}

bool assess(ResidualCheck& result, const Sketch& sketch, const SketchConstraint& constraint) {
    const auto unit = dimension(constraint.type());
    if (!unit || !finiteOperands(sketch, constraint)) return false;
    double tolerance = kExactLengthToleranceMm;
    if (*unit == ResidualDimension::Angle) {
        const auto span = angularSpan(sketch, constraint);
        if (!span) return false;
        tolerance /= *span;
    }
    const auto support = supportLineResidual(sketch, constraint);
    const double residual = support ? *support : constraint.getError(sketch);
    if (!measure(result, constraint.id(), *unit, residual, tolerance, "constraint")) return false;
    if (constraint.type() != ConstraintType::Tangent) return true;
    for (const auto& angle : weldedTangentAngles(sketch, constraint)) {
        if (!std::isfinite(angle.span) || angle.span <= 0.0 ||
            !measure(result, constraint.id(), ResidualDimension::Angle, angle.residual,
                     kExactLengthToleranceMm / angle.span, "endpointTangency")) return false;
    }
    return true;
}

template <class T> void sorted_unique(std::vector<T>& values) {
    std::sort(values.begin(), values.end());
    values.erase(std::unique(values.begin(), values.end()), values.end());
}

}  // namespace

ResidualCheck checkResiduals(const Sketch& sketch,
                            const std::vector<const SketchConstraint*>& constraints) {
    ResidualCheck result;
    for (const auto* constraint : constraints) {
        if (!constraint) continue;
        try {
            if (assess(result, sketch, *constraint)) continue;
        } catch (...) {
            // Evaluator failure is incomplete truth, never a zero residual.
        }
        result.unassessed.push_back(constraint->id());
    }
    sorted_unique(result.unassessed);
    return result;
}

ResidualCheck checkResiduals(const Sketch& sketch) {
    std::vector<const SketchConstraint*> constraints;
    for (const auto& constraint : sketch.getAllConstraints()) constraints.push_back(constraint.get());
    return checkResiduals(sketch, constraints);
}

ResidualCheck checkResidualOperands(const Sketch& sketch) {
    ResidualCheck result;
    for (const auto& constraint : sketch.getAllConstraints()) {
        const auto unit = dimension(constraint->type());
        if (!unit || !finiteOperands(sketch, *constraint) ||
            (*unit == ResidualDimension::Angle && !angularSpan(sketch, *constraint))) {
            result.unassessed.push_back(constraint->id());
        }
    }
    sorted_unique(result.unassessed);
    return result;
}

ExactTruth classifyExact(ResidualCheck residuals, bool numerical_success, int dof,
                         bool redundant, const std::vector<ConstraintID>& rank_conflicts) {
    ExactTruth truth;
    truth.dof = std::max(dof, 0);
    truth.redundant = redundant;
    truth.residuals = std::move(residuals);
    for (const auto& id : rank_conflicts) truth.reasons[id] = ConflictReason::RankConflict;
    for (const auto& violation : truth.residuals.violations) {
        truth.reasons[violation.constraint] = ConflictReason::ResidualViolation;
    }
    for (const auto& [id, _] : truth.reasons) truth.conflicting.push_back(id);
    if (!truth.residuals.unassessed.empty()) {
        truth.issues.push_back({"unassessedConstraint", "Exact residual evaluation is incomplete", {}});
    } else if (!truth.conflicting.empty()) {
        truth.state = ExactState::Conflicting;
    } else if (dof < 0) {
        truth.issues.push_back({"numericallyUnresolved", "Released rank diagnosis is unavailable", {}});
    } else if (!numerical_success) {
        truth.issues.push_back({"numericallyUnresolved", "Exact solve did not converge", {}});
    } else {
        truth.state = redundant ? ExactState::OverConstrained :
                      dof == 0 ? ExactState::FullyConstrained : ExactState::UnderConstrained;
    }
    return truth;
}

}  // namespace onecad::core::sketch
