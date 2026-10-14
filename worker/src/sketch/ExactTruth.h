#pragma once

#include "SketchTypes.h"

#include <map>
#include <optional>
#include <string>
#include <vector>

namespace onecad::core::sketch {

inline constexpr double kExactLengthToleranceMm = 1e-4;

enum class ExactState { UnderConstrained, FullyConstrained, OverConstrained, Conflicting };
enum class ConflictReason { RankConflict, ResidualViolation };
enum class ResidualDimension { Length, Angle };

struct ResidualViolation {
    ConstraintID constraint;
    ResidualDimension dimension = ResidualDimension::Length;
    double residual = 0.0;
    double tolerance = kExactLengthToleranceMm;
    double normalized = 0.0;
    std::string component;
};

struct ExactIssue {
    std::string reason;
    std::string message;
    std::vector<EntityID> entities;
};

struct ResidualCheck {
    std::vector<ResidualViolation> violations;
    std::vector<ConstraintID> unassessed;
    double normalized_max = 0.0;
};

struct ExactTruth {
    std::optional<ExactState> state;
    int dof = 0;
    bool redundant = false;
    std::vector<ConstraintID> conflicting;
    std::map<ConstraintID, ConflictReason> reasons;
    ResidualCheck residuals;
    std::vector<ExactIssue> issues;

    bool publishable() const { return state.has_value() && issues.empty(); }
    bool admissible() const {
        return publishable() && state != ExactState::Conflicting;
    }
};

}  // namespace onecad::core::sketch
