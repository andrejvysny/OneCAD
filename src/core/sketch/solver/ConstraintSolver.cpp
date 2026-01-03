#include "ConstraintSolver.h"
#include "../SketchPoint.h"
#include "../SketchLine.h"
#include "../SketchArc.h"
#include "../SketchCircle.h"
#include "../SketchConstraint.h"

#include <algorithm>
#include <chrono>

namespace onecad::core::sketch {

namespace {

enum class PlaceholderConstraintType : int {
    Coincident = 1,
    Horizontal,
    Vertical,
    Parallel,
    Perpendicular,
    Distance,
    Radius,
    Angle,
    Fixed,
    Tangent,
    Concentric,
    Equal,
    Midpoint,
    Symmetric,
    OnCurve,
    HorizontalDistance,
    VerticalDistance,
    Diameter
};

} // namespace

ConstraintSolver::ConstraintSolver()
    : config_() {
}

ConstraintSolver::ConstraintSolver(const SolverConfig& config)
    : config_(config) {
}

void ConstraintSolver::setConfig(const SolverConfig& config) {
    config_ = config;
}

void ConstraintSolver::clear() {
    entityToGcsId_.clear();
    constraintToGcsTag_.clear();
    pointsById_.clear();
    linesById_.clear();
    arcsById_.clear();
    circlesById_.clear();
    constraints_.clear();
    parameterBackup_.clear();
    nextEntityTag_ = 1;
    nextConstraintTag_ = 1;
}

void ConstraintSolver::addPoint(SketchPoint* point) {
    if (!point) {
        return;
    }
    pointsById_[point->id()] = point;
    entityToGcsId_[point->id()] = nextEntityTag_++;
}

void ConstraintSolver::addLine(SketchLine* line) {
    if (!line) {
        return;
    }
    linesById_[line->id()] = line;
    entityToGcsId_[line->id()] = nextEntityTag_++;
}

void ConstraintSolver::addArc(SketchArc* arc) {
    if (!arc) {
        return;
    }
    arcsById_[arc->id()] = arc;
    entityToGcsId_[arc->id()] = nextEntityTag_++;
}

void ConstraintSolver::addCircle(SketchCircle* circle) {
    if (!circle) {
        return;
    }
    circlesById_[circle->id()] = circle;
    entityToGcsId_[circle->id()] = nextEntityTag_++;
}

bool ConstraintSolver::addConstraint(SketchConstraint* constraint) {
    if (!constraint) {
        return false;
    }
    if (translateConstraint(constraint) < 0) {
        return false;
    }
    constraints_.push_back(constraint);
    constraintToGcsTag_[constraint->id()] = nextConstraintTag_++;
    return true;
}

void ConstraintSolver::removeEntity(EntityID id) {
    entityToGcsId_.erase(id);
    pointsById_.erase(id);
    linesById_.erase(id);
    arcsById_.erase(id);
    circlesById_.erase(id);
}

void ConstraintSolver::removeConstraint(ConstraintID id) {
    constraintToGcsTag_.erase(id);
    constraints_.erase(std::remove_if(constraints_.begin(), constraints_.end(),
                                      [&](const SketchConstraint* c) {
                                          return c && c->id() == id;
                                      }),
                       constraints_.end());
}

SolverResult ConstraintSolver::solve() {
    SolverResult result;
    auto start = std::chrono::steady_clock::now();

    backupParameters();

    result.success = true;
    result.status = SolverResult::Status::Success;
    result.iterations = 0;
    result.residual = 0.0;

    auto end = std::chrono::steady_clock::now();
    result.solveTime = std::chrono::duration_cast<std::chrono::microseconds>(end - start);
    totalSolves_++;
    successfulSolves_++;
    totalSolveTime_ += result.solveTime;

    return result;
}

SolverResult ConstraintSolver::solveWithDrag(EntityID pointId, const Vec2d& targetPos) {
    auto it = pointsById_.find(pointId);
    if (it == pointsById_.end()) {
        SolverResult result;
        result.success = false;
        result.status = SolverResult::Status::InvalidInput;
        result.errorMessage = "Dragged point not found";
        return result;
    }

    SketchPoint* point = it->second;
    point->setPosition(targetPos.x, targetPos.y);
    return solve();
}

void ConstraintSolver::applySolution() {
    // Placeholder: solution is applied directly via parameter binding.
}

void ConstraintSolver::revertSolution() {
    restoreParameters();
}

DOFResult ConstraintSolver::calculateDOF() const {
    DOFResult result;

    for (const auto& [id, point] : pointsById_) {
        result.total += 2;
        result.entityContributions.emplace_back(id, 2);
    }

    for (const auto& [id, arc] : arcsById_) {
        (void)arc;
        result.total += 3;
        result.entityContributions.emplace_back(id, 3);
    }

    for (const auto& [id, circle] : circlesById_) {
        (void)circle;
        result.total += 1;
        result.entityContributions.emplace_back(id, 1);
    }

    for (const auto& constraint : constraints_) {
        if (!constraint) {
            continue;
        }
        int reduction = getConstraintDOFReduction(constraint->type());
        result.total -= reduction;
        result.constraintReductions.emplace_back(constraint->id(), reduction);
    }

    if (result.total < 0) {
        result.total = 0;
    }

    return result;
}

std::vector<ConstraintID> ConstraintSolver::findRedundantConstraints() const {
    return {};
}

bool ConstraintSolver::isSolvable() const {
    return true;
}

void ConstraintSolver::solveAsync(std::function<void(SolverResult)> callback) {
    if (solving_) {
        return;
    }
    solving_ = true;
    SolverResult result = solve();
    solving_ = false;
    if (callback) {
        callback(result);
    }
}

void ConstraintSolver::cancelSolve() {
    cancelRequested_ = true;
}

void ConstraintSolver::backupParameters() {
    parameterBackup_.clear();

    for (const auto& [id, point] : pointsById_) {
        if (!point) {
            continue;
        }
        ParameterBackup backup;
        backup.entityId = id;
        backup.type = EntityType::Point;
        backup.values = {point->x(), point->y()};
        parameterBackup_.push_back(std::move(backup));
    }

    for (const auto& [id, arc] : arcsById_) {
        if (!arc) {
            continue;
        }
        ParameterBackup backup;
        backup.entityId = id;
        backup.type = EntityType::Arc;
        backup.values = {arc->radius(), arc->startAngle(), arc->endAngle()};
        parameterBackup_.push_back(std::move(backup));
    }

    for (const auto& [id, circle] : circlesById_) {
        if (!circle) {
            continue;
        }
        ParameterBackup backup;
        backup.entityId = id;
        backup.type = EntityType::Circle;
        backup.values = {circle->radius()};
        parameterBackup_.push_back(std::move(backup));
    }
}

void ConstraintSolver::restoreParameters() {
    for (const auto& backup : parameterBackup_) {
        switch (backup.type) {
            case EntityType::Point: {
                auto it = pointsById_.find(backup.entityId);
                if (it != pointsById_.end() && it->second && backup.values.size() >= 2) {
                    it->second->setPosition(backup.values[0], backup.values[1]);
                }
                break;
            }
            case EntityType::Arc: {
                auto it = arcsById_.find(backup.entityId);
                if (it != arcsById_.end() && it->second && backup.values.size() >= 3) {
                    it->second->setRadius(backup.values[0]);
                    it->second->setStartAngle(backup.values[1]);
                    it->second->setEndAngle(backup.values[2]);
                }
                break;
            }
            case EntityType::Circle: {
                auto it = circlesById_.find(backup.entityId);
                if (it != circlesById_.end() && it->second && !backup.values.empty()) {
                    it->second->setRadius(backup.values[0]);
                }
                break;
            }
            default:
                break;
        }
    }
}

int ConstraintSolver::translateConstraint(SketchConstraint* constraint) {
    if (!constraint) {
        return -1;
    }

    switch (constraint->type()) {
        case ConstraintType::Coincident:
            return static_cast<int>(PlaceholderConstraintType::Coincident);
        case ConstraintType::Horizontal:
            return static_cast<int>(PlaceholderConstraintType::Horizontal);
        case ConstraintType::Vertical:
            return static_cast<int>(PlaceholderConstraintType::Vertical);
        case ConstraintType::Parallel:
            return static_cast<int>(PlaceholderConstraintType::Parallel);
        case ConstraintType::Perpendicular:
            return static_cast<int>(PlaceholderConstraintType::Perpendicular);
        case ConstraintType::Distance:
            return static_cast<int>(PlaceholderConstraintType::Distance);
        case ConstraintType::Radius:
            return static_cast<int>(PlaceholderConstraintType::Radius);
        case ConstraintType::Angle:
            return static_cast<int>(PlaceholderConstraintType::Angle);
        case ConstraintType::Fixed:
            return static_cast<int>(PlaceholderConstraintType::Fixed);
        case ConstraintType::Tangent:
            return static_cast<int>(PlaceholderConstraintType::Tangent);
        case ConstraintType::Concentric:
            return static_cast<int>(PlaceholderConstraintType::Concentric);
        case ConstraintType::Equal:
            return static_cast<int>(PlaceholderConstraintType::Equal);
        case ConstraintType::Midpoint:
            return static_cast<int>(PlaceholderConstraintType::Midpoint);
        case ConstraintType::Symmetric:
            return static_cast<int>(PlaceholderConstraintType::Symmetric);
        case ConstraintType::OnCurve:
            return static_cast<int>(PlaceholderConstraintType::OnCurve);
        case ConstraintType::HorizontalDistance:
            return static_cast<int>(PlaceholderConstraintType::HorizontalDistance);
        case ConstraintType::VerticalDistance:
            return static_cast<int>(PlaceholderConstraintType::VerticalDistance);
        case ConstraintType::Diameter:
            return static_cast<int>(PlaceholderConstraintType::Diameter);
    }
    return -1;
}

} // namespace onecad::core::sketch
