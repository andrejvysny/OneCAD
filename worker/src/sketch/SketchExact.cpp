#include "Sketch.h"

#include "ResidualGate.h"
#include "SketchPose.h"
#include "solver/ConstraintSolver.h"

#include <algorithm>

namespace onecad::core::sketch {

SolveResult Sketch::solveExact() {
    SolveResult result;
    const SketchPose before(*this);
    auto input_issues = before.integrity(*this, false);
    const auto input_residuals = checkResidualOperands(*this);
    if (!input_issues.empty() || !input_residuals.unassessed.empty()) {
        result.truth = classifyExact(input_residuals, false, 0, false, {});
        result.truth.issues.insert(result.truth.issues.end(), input_issues.begin(), input_issues.end());
        result.errorMessage = "Exact sketch input is unreadable or nonfinite";
        lastConflictingConstraints_.clear();
        return result;
    }
    if (!solver_ || solverDirty_) rebuildSolver();
    if (constraints_.empty() && !hasInternalCouplings()) {
        result.success = true;
    } else {
        if (solver_) {
            const auto numerical = solver_->solve(this);
            result.success = numerical.success;
            result.iterations = numerical.iterations;
            result.errorMessage = numerical.errorMessage;
        } else {
            result.errorMessage = "Solver not available";
        }
    }
    finalizeExact(result, before);
    return result;
}

void Sketch::finalizeExact(SolveResult& result, const SketchPose& before) {
    auto issues = before.assessCandidate(*this, result.success);
    const auto candidate_residuals = checkResiduals(*this);
    if (!candidate_residuals.unassessed.empty()) {
        issues.push_back({"unassessedConstraint", "Retained constraint residual is unreadable", {}});
    }
    if (!issues.empty()) {
        before.restore(*this);
        result.success = false;
        result.errorMessage = "Exact sketch candidate failed internal integrity";
    }
    int dof = std::max(naiveDegreesOfFreedom(), 0);
    bool redundant = false;
    std::vector<ConstraintID> rank_conflicts;
    if (solver_) {
        const int diagnosed = solver_->diagnoseRetainedPose();
        if (!hasSolverUnsupportedEntities()) dof = diagnosed;
        redundant = !hasSolverUnsupportedEntities() && solver_->hasRedundant();
        rank_conflicts = solver_->findConflictingConstraints();
    }
    result.truth = classifyExact(checkResiduals(*this), result.success, dof, redundant, rank_conflicts);
    if (!candidate_residuals.unassessed.empty()) result.truth.residuals.unassessed = candidate_residuals.unassessed;
    result.truth.issues.insert(result.truth.issues.end(), issues.begin(), issues.end());
    result.residual = maxConstraintResidual(*this, constraints_);
    result.conflictingConstraints = result.truth.conflicting;
    lastConflictingConstraints_ = result.truth.conflicting;
    cachedDOF_ = dof;
    dofDirty_ = false;
}

}  // namespace onecad::core::sketch
