#include "ConstraintSolver.h"
#include "../constraints/Constraints.h"
#include "../SketchPoint.h"
#include "../SketchLine.h"
#include "../SketchArc.h"
#include "../SketchCircle.h"
#include "../SketchConstraint.h"

#include <GCS.h>

#include <algorithm>
#include <chrono>
#include <cmath>
#include <limits>

namespace onecad::core::sketch {

namespace {

double* coordPtr(SketchPoint* point, int coordIndex) {
    gp_XY& coords = point->position().ChangeCoord();
    return &coords.ChangeCoord(coordIndex);
}

GCS::Point makePoint(SketchPoint* point) {
    return GCS::Point(coordPtr(point, 1), coordPtr(point, 2));
}

bool lineEndpoints(const std::unordered_map<EntityID, SketchPoint*>& pointsById,
                   const SketchLine* line,
                   SketchPoint*& start,
                   SketchPoint*& end) {
    if (!line) {
        return false;
    }

    auto startIt = pointsById.find(line->startPointId());
    auto endIt = pointsById.find(line->endPointId());
    if (startIt == pointsById.end() || endIt == pointsById.end()) {
        return false;
    }

    start = startIt->second;
    end = endIt->second;
    return start && end;
}

bool circleCenter(const std::unordered_map<EntityID, SketchPoint*>& pointsById,
                  const SketchCircle* circle,
                  SketchPoint*& center) {
    if (!circle) {
        return false;
    }

    auto centerIt = pointsById.find(circle->centerPointId());
    if (centerIt == pointsById.end()) {
        return false;
    }

    center = centerIt->second;
    return center != nullptr;
}

bool arcCenter(const std::unordered_map<EntityID, SketchPoint*>& pointsById,
               const SketchArc* arc,
               SketchPoint*& center) {
    if (!arc) {
        return false;
    }

    auto centerIt = pointsById.find(arc->centerPointId());
    if (centerIt == pointsById.end()) {
        return false;
    }

    center = centerIt->second;
    return center != nullptr;
}

GCS::Line makeLine(SketchPoint* start, SketchPoint* end) {
    GCS::Line line;
    line.p1 = makePoint(start);
    line.p2 = makePoint(end);
    return line;
}

GCS::Circle makeCircle(SketchPoint* center, SketchCircle* circle) {
    GCS::Circle gcsCircle;
    gcsCircle.center = makePoint(center);
    gcsCircle.rad = &circle->radius();
    return gcsCircle;
}

GCS::Arc makeArc(SketchPoint* center, SketchArc* arc) {
    GCS::Arc gcsArc;
    gcsArc.center = makePoint(center);
    gcsArc.rad = &arc->radius();
    gcsArc.startAngle = &arc->startAngle();
    gcsArc.endAngle = &arc->endAngle();
    return gcsArc;
}

GCS::Algorithm toGcsAlgorithm(SolverConfig::Algorithm algorithm) {
    switch (algorithm) {
        case SolverConfig::Algorithm::LevenbergMarquardt:
            return GCS::LevenbergMarquardt;
        case SolverConfig::Algorithm::DogLeg:
            return GCS::DogLeg;
        case SolverConfig::Algorithm::BFGS:
            return GCS::BFGS;
    }
    return GCS::DogLeg;
}

SolverResult::Status toSolverStatus(int gcsStatus) {
    switch (gcsStatus) {
        case GCS::Success:
            return SolverResult::Status::Success;
        case GCS::Converged:
            return SolverResult::Status::PartialSuccess;
        case GCS::Failed:
            return SolverResult::Status::Diverged;
        case GCS::SuccessfulSolutionInvalid:
            return SolverResult::Status::InvalidInput;
        default:
            return SolverResult::Status::InternalError;
    }
}

} // namespace

ConstraintSolver::ConstraintSolver()
    : config_(),
      gcsSystem_(std::make_unique<GCS::System>()) {
    configureSystem();
}

ConstraintSolver::ConstraintSolver(const SolverConfig& config)
    : config_(config),
      gcsSystem_(std::make_unique<GCS::System>()) {
    configureSystem();
}

ConstraintSolver::~ConstraintSolver() = default;

void ConstraintSolver::setConfig(const SolverConfig& config) {
    config_ = config;
    configureSystem();
}

void ConstraintSolver::clear() {
    entityToGcsId_.clear();
    constraintToGcsTag_.clear();
    gcsTagToConstraint_.clear();
    pointsById_.clear();
    linesById_.clear();
    arcsById_.clear();
    circlesById_.clear();
    constraints_.clear();
    parameterBackup_.clear();
    parameters_.clear();
    drivenParameters_.clear();
    nextEntityTag_ = 1;
    nextConstraintTag_ = 1;

    if (!gcsSystem_) {
        gcsSystem_ = std::make_unique<GCS::System>();
    }
    gcsSystem_->clear();
    configureSystem();
}

void ConstraintSolver::addPoint(SketchPoint* point) {
    if (!point) {
        return;
    }
    if (pointsById_.find(point->id()) != pointsById_.end()) {
        return;
    }

    pointsById_[point->id()] = point;
    entityToGcsId_[point->id()] = nextEntityTag_++;
    parameters_.push_back(coordPtr(point, 1));
    parameters_.push_back(coordPtr(point, 2));
}

void ConstraintSolver::addLine(SketchLine* line) {
    if (!line) {
        return;
    }
    if (linesById_.find(line->id()) != linesById_.end()) {
        return;
    }
    linesById_[line->id()] = line;
    entityToGcsId_[line->id()] = nextEntityTag_++;
}

void ConstraintSolver::addArc(SketchArc* arc) {
    if (!arc) {
        return;
    }
    if (arcsById_.find(arc->id()) != arcsById_.end()) {
        return;
    }
    arcsById_[arc->id()] = arc;
    entityToGcsId_[arc->id()] = nextEntityTag_++;
    parameters_.push_back(&arc->radius());
    parameters_.push_back(&arc->startAngle());
    parameters_.push_back(&arc->endAngle());
}

void ConstraintSolver::addCircle(SketchCircle* circle) {
    if (!circle) {
        return;
    }
    if (circlesById_.find(circle->id()) != circlesById_.end()) {
        return;
    }
    circlesById_[circle->id()] = circle;
    entityToGcsId_[circle->id()] = nextEntityTag_++;
    parameters_.push_back(&circle->radius());
}

bool ConstraintSolver::addConstraint(SketchConstraint* constraint) {
    if (!constraint || !gcsSystem_) {
        return false;
    }

    int tagId = nextConstraintTag_;
    if (!translateConstraint(constraint, tagId)) {
        return false;
    }

    constraints_.push_back(constraint);
    constraintToGcsTag_[constraint->id()] = tagId;
    gcsTagToConstraint_[tagId] = constraint->id();
    nextConstraintTag_++;
    gcsSystem_->invalidatedDiagnosis();
    return true;
}

void ConstraintSolver::removeEntity(EntityID id) {
    entityToGcsId_.erase(id);
    pointsById_.erase(id);
    linesById_.erase(id);
    arcsById_.erase(id);
    circlesById_.erase(id);

    parameters_.clear();
    for (const auto& [pointId, point] : pointsById_) {
        parameters_.push_back(coordPtr(point, 1));
        parameters_.push_back(coordPtr(point, 2));
    }
    for (const auto& [arcId, arc] : arcsById_) {
        parameters_.push_back(&arc->radius());
        parameters_.push_back(&arc->startAngle());
        parameters_.push_back(&arc->endAngle());
    }
    for (const auto& [circleId, circle] : circlesById_) {
        parameters_.push_back(&circle->radius());
    }
}

void ConstraintSolver::removeConstraint(ConstraintID id) {
    auto tagIt = constraintToGcsTag_.find(id);
    if (tagIt != constraintToGcsTag_.end()) {
        if (gcsSystem_) {
            gcsSystem_->clearByTag(tagIt->second);
            gcsSystem_->invalidatedDiagnosis();
        }
        gcsTagToConstraint_.erase(tagIt->second);
        constraintToGcsTag_.erase(tagIt);
    }

    constraints_.erase(std::remove_if(constraints_.begin(), constraints_.end(),
                                      [&](const SketchConstraint* c) {
                                          return c && c->id() == id;
                                      }),
                       constraints_.end());
}

SolverResult ConstraintSolver::solve() {
    SolverResult result;
    auto start = std::chrono::steady_clock::now();

    if (!gcsSystem_) {
        result.success = false;
        result.status = SolverResult::Status::InternalError;
        result.errorMessage = "PlaneGCS system not available";
        return result;
    }

    backupParameters();

    gcsSystem_->declareUnknowns(parameters_);
    gcsSystem_->declareDrivenParams(drivenParameters_);

    GCS::Algorithm alg = toGcsAlgorithm(config_.algorithm);
    gcsSystem_->initSolution(alg);

    int status = gcsSystem_->solve(true, alg, false);
    if (status == GCS::Failed && config_.algorithm == SolverConfig::Algorithm::DogLeg) {
        status = gcsSystem_->solve(true, GCS::LevenbergMarquardt, false);
    }

    result.status = toSolverStatus(status);
    result.success = (status == GCS::Success || status == GCS::Converged);

    if (result.success) {
        gcsSystem_->applySolution();
    } else {
        gcsSystem_->undoSolution();
        restoreParameters();
    }

    std::vector<int> conflictingTags;
    gcsSystem_->getConflicting(conflictingTags);
    for (int tag : conflictingTags) {
        auto it = gcsTagToConstraint_.find(tag);
        if (it != gcsTagToConstraint_.end()) {
            result.conflictingConstraints.push_back(it->second);
        }
    }

    if (config_.detectRedundant) {
        std::vector<int> redundantTags;
        gcsSystem_->getRedundant(redundantTags);
        for (int tag : redundantTags) {
            auto it = gcsTagToConstraint_.find(tag);
            if (it != gcsTagToConstraint_.end()) {
                result.redundantConstraints.push_back(it->second);
            }
        }
        if (!result.redundantConstraints.empty() && result.success) {
            result.status = SolverResult::Status::Redundant;
        }
    }

    auto end = std::chrono::steady_clock::now();
    result.solveTime = std::chrono::duration_cast<std::chrono::microseconds>(end - start);
    totalSolves_++;
    if (result.success) {
        successfulSolves_++;
    }
    totalSolveTime_ += result.solveTime;

    if (config_.timeoutMs > 0) {
        auto elapsedMs = std::chrono::duration_cast<std::chrono::milliseconds>(result.solveTime).count();
        if (elapsedMs > config_.timeoutMs) {
            result.status = SolverResult::Status::Timeout;
            result.success = false;
            gcsSystem_->undoSolution();
            restoreParameters();
        }
    }

    return result;
}

SolverResult ConstraintSolver::solveWithDrag(EntityID pointId, const Vec2d& targetPos,
                                             const std::unordered_set<EntityID>& pointIdsToFix) {
    auto it = pointsById_.find(pointId);
    if (it == pointsById_.end() || !it->second) {
        SolverResult result;
        result.success = false;
        result.status = SolverResult::Status::InvalidInput;
        result.errorMessage = "Dragged point not found";
        return result;
    }

    if (!gcsSystem_) {
        SolverResult result;
        result.success = false;
        result.status = SolverResult::Status::InternalError;
        result.errorMessage = "PlaneGCS system not available";
        return result;
    }

    constexpr int dragTag = -1;
    gcsSystem_->clearByTag(dragTag);

    // Fix either:
    // - all non-dragged points (legacy/default behavior when pointIdsToFix is empty), or
    // - only the explicitly requested set.
    struct FixedCoord {
        double x;
        double y;
    };
    std::unordered_map<EntityID, FixedCoord> fixedPositions;
    const bool fixAllOtherPoints = pointIdsToFix.empty();
    for (const auto& [id, point] : pointsById_) {
        if (id == pointId || !point) {
            continue;
        }
        if (!fixAllOtherPoints && pointIdsToFix.find(id) == pointIdsToFix.end()) {
            continue;
        }
        fixedPositions[id] = {point->position().X(), point->position().Y()};
    }
    for (auto& [id, coord] : fixedPositions) {
        auto pointIt = pointsById_.find(id);
        if (pointIt == pointsById_.end() || !pointIt->second) {
            continue;
        }
        GCS::Point gcsPoint = makePoint(pointIt->second);
        gcsSystem_->addConstraintCoordinateX(gcsPoint, &coord.x, dragTag, true);
        gcsSystem_->addConstraintCoordinateY(gcsPoint, &coord.y, dragTag, true);
    }

    double targetX = targetPos.x;
    double targetY = targetPos.y;
    GCS::Point dragPoint = makePoint(it->second);
    gcsSystem_->addConstraintCoordinateX(dragPoint, &targetX, dragTag, true);
    gcsSystem_->addConstraintCoordinateY(dragPoint, &targetY, dragTag, true);

    SolverResult result = solve();

    gcsSystem_->clearByTag(dragTag);
    gcsSystem_->invalidatedDiagnosis();

    return result;
}

void ConstraintSolver::applySolution() {
    if (gcsSystem_) {
        gcsSystem_->applySolution();
    }
}

void ConstraintSolver::revertSolution() {
    if (gcsSystem_) {
        gcsSystem_->undoSolution();
    }
    restoreParameters();
}

DOFResult ConstraintSolver::calculateDOF() const {
    DOFResult result;

    for (const auto& [id, point] : pointsById_) {
        (void)point;
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
    std::vector<ConstraintID> result;
    if (!gcsSystem_) {
        return result;
    }

    std::vector<int> redundantTags;
    gcsSystem_->getRedundant(redundantTags);
    for (int tag : redundantTags) {
        auto it = gcsTagToConstraint_.find(tag);
        if (it != gcsTagToConstraint_.end()) {
            result.push_back(it->second);
        }
    }
    return result;
}

bool ConstraintSolver::isSolvable() const {
    if (!gcsSystem_) {
        return false;
    }
    return !gcsSystem_->hasConflicting();
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

bool ConstraintSolver::translateConstraint(SketchConstraint* constraint, int tagId) {
    if (!constraint || !gcsSystem_) {
        return false;
    }

    using namespace onecad::core::sketch::constraints;

    auto getPoint = [&](const EntityID& id) -> SketchPoint* {
        auto it = pointsById_.find(id);
        return it != pointsById_.end() ? it->second : nullptr;
    };
    auto getLine = [&](const EntityID& id) -> SketchLine* {
        auto it = linesById_.find(id);
        return it != linesById_.end() ? it->second : nullptr;
    };
    auto getCircle = [&](const EntityID& id) -> SketchCircle* {
        auto it = circlesById_.find(id);
        return it != circlesById_.end() ? it->second : nullptr;
    };
    auto getArc = [&](const EntityID& id) -> SketchArc* {
        auto it = arcsById_.find(id);
        return it != arcsById_.end() ? it->second : nullptr;
    };

    if (auto* coincident = dynamic_cast<CoincidentConstraint*>(constraint)) {
        auto* p1 = getPoint(coincident->point1());
        auto* p2 = getPoint(coincident->point2());
        if (!p1 || !p2) {
            return false;
        }
        auto gp1 = makePoint(p1);
        auto gp2 = makePoint(p2);
        gcsSystem_->addConstraintP2PCoincident(gp1, gp2, tagId, true);
        return true;
    }

    if (auto* horizontal = dynamic_cast<HorizontalConstraint*>(constraint)) {
        auto* line = getLine(horizontal->lineId());
        if (!line) {
            return false;
        }
        SketchPoint* start = nullptr;
        SketchPoint* end = nullptr;
        if (!lineEndpoints(pointsById_, line, start, end)) {
            return false;
        }
        auto gp1 = makePoint(start);
        auto gp2 = makePoint(end);
        gcsSystem_->addConstraintHorizontal(gp1, gp2, tagId, true);
        return true;
    }

    if (auto* vertical = dynamic_cast<VerticalConstraint*>(constraint)) {
        auto* line = getLine(vertical->lineId());
        if (!line) {
            return false;
        }
        SketchPoint* start = nullptr;
        SketchPoint* end = nullptr;
        if (!lineEndpoints(pointsById_, line, start, end)) {
            return false;
        }
        auto gp1 = makePoint(start);
        auto gp2 = makePoint(end);
        gcsSystem_->addConstraintVertical(gp1, gp2, tagId, true);
        return true;
    }

    if (auto* parallel = dynamic_cast<ParallelConstraint*>(constraint)) {
        auto* line1 = getLine(parallel->line1());
        auto* line2 = getLine(parallel->line2());
        if (!line1 || !line2) {
            return false;
        }
        SketchPoint* l1s = nullptr;
        SketchPoint* l1e = nullptr;
        SketchPoint* l2s = nullptr;
        SketchPoint* l2e = nullptr;
        if (!lineEndpoints(pointsById_, line1, l1s, l1e) ||
            !lineEndpoints(pointsById_, line2, l2s, l2e)) {
            return false;
        }
        GCS::Line l1 = makeLine(l1s, l1e);
        GCS::Line l2 = makeLine(l2s, l2e);
        gcsSystem_->addConstraintParallel(l1, l2, tagId, true);
        return true;
    }

    if (auto* perpendicular = dynamic_cast<PerpendicularConstraint*>(constraint)) {
        auto* line1 = getLine(perpendicular->line1());
        auto* line2 = getLine(perpendicular->line2());
        if (!line1 || !line2) {
            return false;
        }
        SketchPoint* l1s = nullptr;
        SketchPoint* l1e = nullptr;
        SketchPoint* l2s = nullptr;
        SketchPoint* l2e = nullptr;
        if (!lineEndpoints(pointsById_, line1, l1s, l1e) ||
            !lineEndpoints(pointsById_, line2, l2s, l2e)) {
            return false;
        }
        GCS::Line l1 = makeLine(l1s, l1e);
        GCS::Line l2 = makeLine(l2s, l2e);
        gcsSystem_->addConstraintPerpendicular(l1, l2, tagId, true);
        return true;
    }

    if (auto* distance = dynamic_cast<DistanceConstraint*>(constraint)) {
        auto* p1 = getPoint(distance->entity1());
        auto* p2 = getPoint(distance->entity2());
        auto* line1 = getLine(distance->entity1());
        auto* line2 = getLine(distance->entity2());

        if (p1 && p2) {
            auto gp1 = makePoint(p1);
            auto gp2 = makePoint(p2);
            gcsSystem_->addConstraintP2PDistance(gp1, gp2, distance->valuePtr(), tagId, true);
            return true;
        }

        if (p1 && line2) {
            SketchPoint* l2s = nullptr;
            SketchPoint* l2e = nullptr;
            if (!lineEndpoints(pointsById_, line2, l2s, l2e)) {
                return false;
            }
            GCS::Line line = makeLine(l2s, l2e);
            auto gp1 = makePoint(p1);
            gcsSystem_->addConstraintP2LDistance(gp1, line, distance->valuePtr(), tagId, true);
            return true;
        }

        if (p2 && line1) {
            SketchPoint* l1s = nullptr;
            SketchPoint* l1e = nullptr;
            if (!lineEndpoints(pointsById_, line1, l1s, l1e)) {
                return false;
            }
            GCS::Line line = makeLine(l1s, l1e);
            auto gp2 = makePoint(p2);
            gcsSystem_->addConstraintP2LDistance(gp2, line, distance->valuePtr(), tagId, true);
            return true;
        }

        if (line1 && line2) {
            SketchPoint* l1s = nullptr;
            SketchPoint* l1e = nullptr;
            SketchPoint* l2s = nullptr;
            SketchPoint* l2e = nullptr;
            if (!lineEndpoints(pointsById_, line1, l1s, l1e) ||
                !lineEndpoints(pointsById_, line2, l2s, l2e)) {
                return false;
            }
            GCS::Line line = makeLine(l2s, l2e);
            auto gp1 = makePoint(l1s);
            gcsSystem_->addConstraintP2LDistance(gp1, line, distance->valuePtr(), tagId, true);
            return true;
        }

        return false;
    }

    if (auto* angle = dynamic_cast<AngleConstraint*>(constraint)) {
        auto* line1 = getLine(angle->line1());
        auto* line2 = getLine(angle->line2());
        if (!line1 || !line2) {
            return false;
        }
        SketchPoint* l1s = nullptr;
        SketchPoint* l1e = nullptr;
        SketchPoint* l2s = nullptr;
        SketchPoint* l2e = nullptr;
        if (!lineEndpoints(pointsById_, line1, l1s, l1e) ||
            !lineEndpoints(pointsById_, line2, l2s, l2e)) {
            return false;
        }
        GCS::Line l1 = makeLine(l1s, l1e);
        GCS::Line l2 = makeLine(l2s, l2e);
        gcsSystem_->addConstraintL2LAngle(l1, l2, angle->valuePtr(), tagId, true);
        return true;
    }

    if (auto* radius = dynamic_cast<RadiusConstraint*>(constraint)) {
        auto* circle = getCircle(radius->entityId());
        if (circle) {
            SketchPoint* center = nullptr;
            if (!circleCenter(pointsById_, circle, center)) {
                return false;
            }
            GCS::Circle circleObj = makeCircle(center, circle);
            gcsSystem_->addConstraintCircleRadius(circleObj, radius->valuePtr(), tagId, true);
            return true;
        }

        auto* arc = getArc(radius->entityId());
        if (arc) {
            SketchPoint* center = nullptr;
            if (!arcCenter(pointsById_, arc, center)) {
                return false;
            }
            GCS::Arc arcObj = makeArc(center, arc);
            gcsSystem_->addConstraintArcRadius(arcObj, radius->valuePtr(), tagId, true);
            return true;
        }

        return false;
    }

    if (auto* tangent = dynamic_cast<TangentConstraint*>(constraint)) {
        auto* line1 = getLine(tangent->entity1());
        auto* line2 = getLine(tangent->entity2());
        auto* circle1 = getCircle(tangent->entity1());
        auto* circle2 = getCircle(tangent->entity2());
        auto* arc1 = getArc(tangent->entity1());
        auto* arc2 = getArc(tangent->entity2());

        if (line1 && circle2) {
            SketchPoint* l1s = nullptr;
            SketchPoint* l1e = nullptr;
            if (!lineEndpoints(pointsById_, line1, l1s, l1e)) {
                return false;
            }
            SketchPoint* center = nullptr;
            if (!circleCenter(pointsById_, circle2, center)) {
                return false;
            }
            GCS::Line line = makeLine(l1s, l1e);
            GCS::Circle circle = makeCircle(center, circle2);
            gcsSystem_->addConstraintTangent(line, circle, tagId, true);
            return true;
        }

        if (line2 && circle1) {
            SketchPoint* l2s = nullptr;
            SketchPoint* l2e = nullptr;
            if (!lineEndpoints(pointsById_, line2, l2s, l2e)) {
                return false;
            }
            SketchPoint* center = nullptr;
            if (!circleCenter(pointsById_, circle1, center)) {
                return false;
            }
            GCS::Line line = makeLine(l2s, l2e);
            GCS::Circle circle = makeCircle(center, circle1);
            gcsSystem_->addConstraintTangent(line, circle, tagId, true);
            return true;
        }

        if (line1 && arc2) {
            SketchPoint* l1s = nullptr;
            SketchPoint* l1e = nullptr;
            if (!lineEndpoints(pointsById_, line1, l1s, l1e)) {
                return false;
            }
            SketchPoint* center = nullptr;
            if (!arcCenter(pointsById_, arc2, center)) {
                return false;
            }
            GCS::Line line = makeLine(l1s, l1e);
            GCS::Arc arc = makeArc(center, arc2);
            gcsSystem_->addConstraintTangent(line, arc, tagId, true);
            return true;
        }

        if (line2 && arc1) {
            SketchPoint* l2s = nullptr;
            SketchPoint* l2e = nullptr;
            if (!lineEndpoints(pointsById_, line2, l2s, l2e)) {
                return false;
            }
            SketchPoint* center = nullptr;
            if (!arcCenter(pointsById_, arc1, center)) {
                return false;
            }
            GCS::Line line = makeLine(l2s, l2e);
            GCS::Arc arc = makeArc(center, arc1);
            gcsSystem_->addConstraintTangent(line, arc, tagId, true);
            return true;
        }

        if (circle1 && circle2) {
            SketchPoint* c1 = nullptr;
            SketchPoint* c2 = nullptr;
            if (!circleCenter(pointsById_, circle1, c1) ||
                !circleCenter(pointsById_, circle2, c2)) {
                return false;
            }
            GCS::Circle circleObj1 = makeCircle(c1, circle1);
            GCS::Circle circleObj2 = makeCircle(c2, circle2);
            gcsSystem_->addConstraintTangent(circleObj1, circleObj2, tagId, true);
            return true;
        }

        if (arc1 && arc2) {
            SketchPoint* c1 = nullptr;
            SketchPoint* c2 = nullptr;
            if (!arcCenter(pointsById_, arc1, c1) ||
                !arcCenter(pointsById_, arc2, c2)) {
                return false;
            }
            GCS::Arc arcObj1 = makeArc(c1, arc1);
            GCS::Arc arcObj2 = makeArc(c2, arc2);
            gcsSystem_->addConstraintTangent(arcObj1, arcObj2, tagId, true);
            return true;
        }

        if (circle1 && arc2) {
            SketchPoint* c1 = nullptr;
            SketchPoint* c2 = nullptr;
            if (!circleCenter(pointsById_, circle1, c1) ||
                !arcCenter(pointsById_, arc2, c2)) {
                return false;
            }
            GCS::Circle circle = makeCircle(c1, circle1);
            GCS::Arc arc = makeArc(c2, arc2);
            gcsSystem_->addConstraintTangent(circle, arc, tagId, true);
            return true;
        }

        if (arc1 && circle2) {
            SketchPoint* c1 = nullptr;
            SketchPoint* c2 = nullptr;
            if (!arcCenter(pointsById_, arc1, c1) ||
                !circleCenter(pointsById_, circle2, c2)) {
                return false;
            }
            GCS::Arc arc = makeArc(c1, arc1);
            GCS::Circle circle = makeCircle(c2, circle2);
            gcsSystem_->addConstraintTangent(circle, arc, tagId, true);
            return true;
        }

        return false;
    }

    if (auto* fixed = dynamic_cast<FixedConstraint*>(constraint)) {
        auto* p = getPoint(fixed->pointId());
        if (!p) {
            return false;
        }
        auto gp = makePoint(p);
        // Use const_cast to get mutable pointer to constraint's stored values
        double* xPtr = const_cast<double*>(&fixed->fixedXRef());
        double* yPtr = const_cast<double*>(&fixed->fixedYRef());
        gcsSystem_->addConstraintCoordinateX(gp, xPtr, tagId, true);
        gcsSystem_->addConstraintCoordinateY(gp, yPtr, tagId, true);
        return true;
    }

    if (auto* midpoint = dynamic_cast<MidpointConstraint*>(constraint)) {
        auto* p = getPoint(midpoint->pointId());
        auto* line = getLine(midpoint->lineId());
        if (!p || !line) {
            return false;
        }
        SketchPoint* start = nullptr;
        SketchPoint* end = nullptr;
        if (!lineEndpoints(pointsById_, line, start, end)) {
            return false;
        }
        auto gp = makePoint(p);
        GCS::Line gcsLine = makeLine(start, end);
        // Midpoint = point on line AND on perpendicular bisector
        gcsSystem_->addConstraintPointOnLine(gp, gcsLine, tagId, true);
        gcsSystem_->addConstraintPointOnPerpBisector(gp, gcsLine, tagId, true);
        return true;
    }

    if (auto* equal = dynamic_cast<EqualConstraint*>(constraint)) {
        auto* line1 = getLine(equal->entity1());
        auto* line2 = getLine(equal->entity2());
        if (line1 && line2) {
            SketchPoint* l1s = nullptr;
            SketchPoint* l1e = nullptr;
            SketchPoint* l2s = nullptr;
            SketchPoint* l2e = nullptr;
            if (!lineEndpoints(pointsById_, line1, l1s, l1e) ||
                !lineEndpoints(pointsById_, line2, l2s, l2e)) {
                return false;
            }
            GCS::Line l1 = makeLine(l1s, l1e);
            GCS::Line l2 = makeLine(l2s, l2e);
            gcsSystem_->addConstraintEqualLength(l1, l2, tagId, true);
            return true;
        }

        auto* circle1 = getCircle(equal->entity1());
        auto* circle2 = getCircle(equal->entity2());
        auto* arc1 = getArc(equal->entity1());
        auto* arc2 = getArc(equal->entity2());

        if (circle1 && circle2) {
            SketchPoint* c1 = nullptr;
            SketchPoint* c2 = nullptr;
            if (!circleCenter(pointsById_, circle1, c1) ||
                !circleCenter(pointsById_, circle2, c2)) {
                return false;
            }
            GCS::Circle circleObj1 = makeCircle(c1, circle1);
            GCS::Circle circleObj2 = makeCircle(c2, circle2);
            gcsSystem_->addConstraintEqualRadius(circleObj1, circleObj2, tagId, true);
            return true;
        }

        if (circle1 && arc2) {
            SketchPoint* c1 = nullptr;
            SketchPoint* c2 = nullptr;
            if (!circleCenter(pointsById_, circle1, c1) ||
                !arcCenter(pointsById_, arc2, c2)) {
                return false;
            }
            GCS::Circle circle = makeCircle(c1, circle1);
            GCS::Arc arc = makeArc(c2, arc2);
            gcsSystem_->addConstraintEqualRadius(circle, arc, tagId, true);
            return true;
        }

        if (arc1 && arc2) {
            SketchPoint* c1 = nullptr;
            SketchPoint* c2 = nullptr;
            if (!arcCenter(pointsById_, arc1, c1) ||
                !arcCenter(pointsById_, arc2, c2)) {
                return false;
            }
            GCS::Arc arcObj1 = makeArc(c1, arc1);
            GCS::Arc arcObj2 = makeArc(c2, arc2);
            gcsSystem_->addConstraintEqualRadius(arcObj1, arcObj2, tagId, true);
            return true;
        }

        if (arc1 && circle2) {
            SketchPoint* c1 = nullptr;
            SketchPoint* c2 = nullptr;
            if (!arcCenter(pointsById_, arc1, c1) ||
                !circleCenter(pointsById_, circle2, c2)) {
                return false;
            }
            GCS::Arc arc = makeArc(c1, arc1);
            GCS::Circle circle = makeCircle(c2, circle2);
            gcsSystem_->addConstraintEqualRadius(circle, arc, tagId, true);
            return true;
        }

        return false;
    }

    return false;
}

void ConstraintSolver::configureSystem() {
    if (!gcsSystem_) {
        return;
    }
    gcsSystem_->setConvergence(config_.tolerance);
    gcsSystem_->setMaxIterations(config_.maxIterations);
    gcsSystem_->setConvergenceRedundant(config_.tolerance);
    gcsSystem_->setMaxIterationsRedundant(config_.maxIterations);
}

} // namespace onecad::core::sketch
