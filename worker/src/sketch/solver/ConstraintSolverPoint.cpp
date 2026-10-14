#include "ConstraintSolver.h"

#include "PointBranch.h"
#include "../ExactTruth.h"
#include "../Sketch.h"
#include "../SketchPose.h"
#include "../SketchArc.h"
#include "../SketchCircle.h"
#include "../SketchLine.h"
#include "../SketchPoint.h"
#include "../constraints/Constraints.h"

#include <GCS.h>
#include <algorithm>
#include <cmath>

namespace onecad::core::sketch {
namespace {
double* coordinate(SketchPoint* point, int axis) {
    return &point->position().ChangeCoord().ChangeCoord(axis);
}
SolverResult refused(const std::string& reason) {
    SolverResult result;
    result.status = SolverResult::Status::InvalidInput;
    result.errorMessage = reason;
    return result;
}
}  // namespace

struct ConstraintSolver::PointScope {
    ConstraintSolver& solver;
    std::optional<SketchPose> pose;
    PointScope(ConstraintSolver& owner, SketchPoint* point, Sketch* context) : solver(owner) {
        if (context) pose.emplace(*context);
        solver.pointContext_ = context;
        solver.pointPose_ = pose ? &*pose : nullptr;
        solver.gcsSystem_->clearByTag(-1);
        solver.pointParameters_ = solver.gcsSystem_->connectedUnknowns(
            {coordinate(point, 1), coordinate(point, 2)}, solver.parameters_);
        solver.gcsSystem_->setScopedUnknowns(true);
    }
    ~PointScope() {
        solver.gcsSystem_->clearByTag(-1);
        solver.gcsSystem_->setScopedUnknowns(false);
        solver.pointParameters_.clear();
        solver.pointContext_ = nullptr;
        solver.pointPose_ = nullptr;
        solver.diagnoseRetainedPose();
    }
};

SolverResult ConstraintSolver::solveWithDrag(EntityID pointId, const Vec2d& targetPos, Sketch* context) {
    const auto point = pointsById_.find(pointId);
    if (point == pointsById_.end() || !point->second) return refused("Dragged point not found");
    if (!gcsSystem_) return refused("PlaneGCS system not available");
    if (!std::isfinite(targetPos.x) || !std::isfinite(targetPos.y)) return refused("Nonfinite point target");
    PointScope scope(*this, point->second, context);
    const auto before = captureDragSolveSnapshot();
    const auto start = before.pointPositions.at(pointId);
    const double distance = std::hypot(targetPos.x-start.x, targetPos.y-start.y);
    if (!std::isfinite(distance)) return refused("Nonfinite point displacement");
    const int steps = distance <= 100 ? 1 : static_cast<int>(std::min(10.0, std::ceil(distance/100)));
    SolverResult result;
    for (int step = 1; step <= steps; ++step) {
        const double t = static_cast<double>(step)/steps;
        result = advancePointDrag(pointId, {start.x+(targetPos.x-start.x)*t,
                                            start.y+(targetPos.y-start.y)*t}, 8);
        if (!result.success) {
            restorePointDragSnapshot(before);
            return result;
        }
    }
    return result;
}

SolverResult ConstraintSolver::solveWithDragSingleStep(EntityID pointId, const Vec2d& targetPos) {
    gcsSystem_->clearByTag(-1);
    double x = targetPos.x, y = targetPos.y;
    auto* point = pointsById_.at(pointId);
    GCS::Point driven(coordinate(point, 1), coordinate(point, 2));
    gcsSystem_->addConstraintCoordinateX(driven, &x, -1, true);
    gcsSystem_->addConstraintCoordinateY(driven, &y, -1, true);
    auto result = solve();
    gcsSystem_->clearByTag(-1);
    gcsSystem_->invalidatedDiagnosis();
    return result;
}

bool ConstraintSolver::pointCandidateValid() {
    for (const double* parameter : parameters_) if (!std::isfinite(*parameter)) return false;
    for (const auto& [id, circle] : circlesById_) if (circle->radius() < 0) return false;
    if (pointContext_ && pointPose_) {
        if (!pointPose_->assessCandidate(*pointContext_, true).empty()) return false;
    }
    for (const auto& [id, arc] : arcsById_) {
        if (arc->radius() < 0) return false;
        const bool affected = std::any_of(pointParameters_.begin(), pointParameters_.end(),
            [arc](const double* value) { return value == &arc->radius() ||
                value == &arc->startAngle() || value == &arc->endAngle(); });
        if (affected) {
            const double sweep = arc->sweepAngle();
            if (sweep < 1e-3 || sweep > 2*std::acos(-1.0)-1e-3) return false;
        }
        if (!arc->hasEndpointPoints()) continue;
        const auto* center = pointsById_.at(arc->centerPointId());
        const auto* start = pointsById_.at(arc->startPointId());
        const auto* end = pointsById_.at(arc->endPointId());
        const double start_error = arc->startPoint(center->position()).Distance(start->position());
        const double end_error = arc->endPoint(center->position()).Distance(end->position());
        if (!std::isfinite(start_error) || !std::isfinite(end_error) ||
            start_error > kExactLengthToleranceMm || end_error > kExactLengthToleranceMm) return false;
    }
    return true;
}

void ConstraintSolver::restorePointDragSnapshot(const DragSolveSnapshot& before) {
    for (const auto& [id, at] : before.pointPositions) pointsById_.at(id)->setPosition(at.x, at.y);
    for (const auto& [id, at] : before.arcStates) {
        auto* arc = arcsById_.at(id);
        arc->radius() = at.radius;
        arc->startAngle() = at.startAngle;
        arc->endAngle() = at.endAngle;
    }
    for (const auto& [id, radius] : before.circleRadii) circlesById_.at(id)->radius() = radius;
    gcsSystem_->clearByTag(-1);
    gcsSystem_->invalidatedDiagnosis();
}

bool ConstraintSolver::pointBranchContinues(const DragSolveSnapshot& before) const {
    for (const auto* constraint : constraints_) {
        const auto* angle = dynamic_cast<const constraints::AngleConstraint*>(constraint);
        if (!angle) continue;
        const auto* first = linesById_.at(angle->line1());
        const auto* second = linesById_.at(angle->line2());
        const EntityID ids[4] = {first->startPointId(), first->endPointId(),
                                  second->startPointId(), second->endPointId()};
        bool affected = false;
        for (const auto& id : ids) {
            auto* point = pointsById_.at(id);
            for (int axis : {1,2}) {
                affected = affected || std::find(pointParameters_.begin(), pointParameters_.end(),
                                                  coordinate(point, axis)) != pointParameters_.end();
            }
        }
        if (!affected) continue;
        Vec2d prior[2], after[2];
        for (int i = 0; i < 2; ++i) {
            const auto a = before.pointPositions.at(ids[2*i]), b = before.pointPositions.at(ids[2*i+1]);
            prior[i] = {b.x-a.x, b.y-a.y};
            const auto* p = pointsById_.at(ids[2*i]);
            const auto* q = pointsById_.at(ids[2*i+1]);
            after[i] = {q->x()-p->x(), q->y()-p->y()};
        }
        if (!::onecad::core::sketch::pointBranchContinues(prior[0], prior[1], after[0], after[1])) return false;
    }
    return true;
}

SolverResult ConstraintSolver::advancePointDrag(EntityID pointId, const Vec2d& targetPos, int depth) {
    const auto before = captureDragSolveSnapshot();
    if (!pointBranchContinues(before)) return refused("branchBoundary");
    auto result = solveWithDragSingleStep(pointId, targetPos);
    if (!result.success) return result;
    if (!pointCandidateValid()) {
        restorePointDragSnapshot(before);
        return refused("Point drag candidate failed geometry integrity");
    }
    if (pointBranchContinues(before)) return result;
    restorePointDragSnapshot(before);
    if (depth == 0) return refused("branchBoundary");
    const auto start = before.pointPositions.at(pointId);
    const Vec2d midpoint{start.x+(targetPos.x-start.x)/2, start.y+(targetPos.y-start.y)/2};
    result = advancePointDrag(pointId, midpoint, depth-1);
    if (result.success) result = advancePointDrag(pointId, targetPos, depth-1);
    if (!result.success) {
        restorePointDragSnapshot(before);
        return refused("branchBoundary");
    }
    return result;
}

}  // namespace onecad::core::sketch
