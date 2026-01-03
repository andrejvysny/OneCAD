#include "Sketch.h"
#include "constraints/Constraints.h"
#include "solver/ConstraintSolver.h"
#include "solver/SolverAdapter.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonParseError>
#include <QString>

#include <algorithm>
#include <limits>
#include <numbers>
#include <utility>

namespace onecad::core::sketch {

Sketch::Sketch(const SketchPlane& plane)
    : plane_(plane) {
}

Sketch::~Sketch() = default;

Sketch::Sketch(Sketch&& other) noexcept = default;

Sketch& Sketch::operator=(Sketch&& other) noexcept = default;

EntityID Sketch::addPoint(double x, double y, bool construction) {
    auto point = std::make_unique<SketchPoint>(x, y);
    point->setConstruction(construction);

    EntityID id = point->id();
    entityIndex_[id] = entities_.size();
    entities_.push_back(std::move(point));

    invalidateSolver();
    return id;
}

EntityID Sketch::addLine(EntityID startId, EntityID endId, bool construction) {
    auto* startPoint = getEntityAs<SketchPoint>(startId);
    auto* endPoint = getEntityAs<SketchPoint>(endId);
    if (!startPoint || !endPoint) {
        return {};
    }

    auto line = std::make_unique<SketchLine>(startId, endId);
    line->setConstruction(construction);

    EntityID id = line->id();
    entityIndex_[id] = entities_.size();
    entities_.push_back(std::move(line));

    startPoint->addConnectedEntity(id);
    endPoint->addConnectedEntity(id);

    invalidateSolver();
    return id;
}

EntityID Sketch::addLine(double x1, double y1, double x2, double y2, bool construction) {
    EntityID startId = addPoint(x1, y1, construction);
    EntityID endId = addPoint(x2, y2, construction);
    if (startId.empty() || endId.empty()) {
        return {};
    }
    return addLine(startId, endId, construction);
}

EntityID Sketch::addArc(EntityID centerId, double radius, double startAngle,
                        double endAngle, bool construction) {
    auto* centerPoint = getEntityAs<SketchPoint>(centerId);
    if (!centerPoint) {
        return {};
    }

    auto arc = std::make_unique<SketchArc>(centerId, radius, startAngle, endAngle);
    arc->setConstruction(construction);

    EntityID id = arc->id();
    entityIndex_[id] = entities_.size();
    entities_.push_back(std::move(arc));

    centerPoint->addConnectedEntity(id);

    invalidateSolver();
    return id;
}

EntityID Sketch::addCircle(EntityID centerId, double radius, bool construction) {
    auto* centerPoint = getEntityAs<SketchPoint>(centerId);
    if (!centerPoint) {
        return {};
    }

    auto circle = std::make_unique<SketchCircle>(centerId, radius);
    circle->setConstruction(construction);

    EntityID id = circle->id();
    entityIndex_[id] = entities_.size();
    entities_.push_back(std::move(circle));

    centerPoint->addConnectedEntity(id);

    invalidateSolver();
    return id;
}

bool Sketch::removeEntity(EntityID id) {
    auto it = entityIndex_.find(id);
    if (it == entityIndex_.end()) {
        return false;
    }

    if (it->second >= entities_.size()) {
        rebuildEntityIndex();
        it = entityIndex_.find(id);
        if (it == entityIndex_.end() || it->second >= entities_.size()) {
            return false;
        }
    }

    SketchEntity* entity = entities_[it->second].get();
    if (!entity) {
        return false;
    }

    if (auto* point = dynamic_cast<SketchPoint*>(entity)) {
        std::vector<EntityID> dependents;
        for (const auto& candidate : entities_) {
            if (!candidate) {
                continue;
            }
            if (auto* line = dynamic_cast<SketchLine*>(candidate.get())) {
                if (line->startPointId() == id || line->endPointId() == id) {
                    dependents.push_back(line->id());
                }
            } else if (auto* arc = dynamic_cast<SketchArc*>(candidate.get())) {
                if (arc->centerPointId() == id) {
                    dependents.push_back(arc->id());
                }
            } else if (auto* circle = dynamic_cast<SketchCircle*>(candidate.get())) {
                if (circle->centerPointId() == id) {
                    dependents.push_back(circle->id());
                }
            }
        }

        for (const auto& depId : dependents) {
            if (depId != id) {
                removeEntity(depId);
            }
        }

        if (!point->connectedEntities().empty()) {
            for (const auto& entityId : point->connectedEntities()) {
                auto* connected = getEntity(entityId);
                if (!connected) {
                    continue;
                }
                if (auto* line = dynamic_cast<SketchLine*>(connected)) {
                    removeEntity(line->id());
                } else if (auto* arc = dynamic_cast<SketchArc*>(connected)) {
                    removeEntity(arc->id());
                } else if (auto* circle = dynamic_cast<SketchCircle*>(connected)) {
                    removeEntity(circle->id());
                }
            }
        }
    }

    if (auto* line = dynamic_cast<SketchLine*>(entity)) {
        if (auto* start = getEntityAs<SketchPoint>(line->startPointId())) {
            start->removeConnectedEntity(line->id());
        }
        if (auto* end = getEntityAs<SketchPoint>(line->endPointId())) {
            end->removeConnectedEntity(line->id());
        }
    } else if (auto* arc = dynamic_cast<SketchArc*>(entity)) {
        if (auto* center = getEntityAs<SketchPoint>(arc->centerPointId())) {
            center->removeConnectedEntity(arc->id());
        }
    } else if (auto* circle = dynamic_cast<SketchCircle*>(entity)) {
        if (auto* center = getEntityAs<SketchPoint>(circle->centerPointId())) {
            center->removeConnectedEntity(circle->id());
        }
    }

    bool removedConstraints = false;
    for (size_t i = 0; i < constraints_.size();) {
        if (constraints_[i] && constraints_[i]->references(id)) {
            constraints_.erase(constraints_.begin() + static_cast<long>(i));
            removedConstraints = true;
        } else {
            ++i;
        }
    }

    if (removedConstraints) {
        rebuildConstraintIndex();
    }

    entities_.erase(entities_.begin() + static_cast<long>(it->second));
    rebuildEntityIndex();
    invalidateSolver();
    return true;
}

SketchEntity* Sketch::getEntity(EntityID id) {
    auto it = entityIndex_.find(id);
    if (it == entityIndex_.end()) {
        return nullptr;
    }
    if (it->second >= entities_.size()) {
        rebuildEntityIndex();
        it = entityIndex_.find(id);
        if (it == entityIndex_.end() || it->second >= entities_.size()) {
            return nullptr;
        }
    }
    return entities_[it->second].get();
}

const SketchEntity* Sketch::getEntity(EntityID id) const {
    auto it = entityIndex_.find(id);
    if (it == entityIndex_.end()) {
        return nullptr;
    }
    if (it->second >= entities_.size()) {
        return nullptr;
    }
    return entities_[it->second].get();
}

std::vector<SketchEntity*> Sketch::getEntitiesByType(EntityType type) {
    std::vector<SketchEntity*> results;
    for (auto& entity : entities_) {
        if (entity->type() == type) {
            results.push_back(entity.get());
        }
    }
    return results;
}

ConstraintID Sketch::addConstraint(std::unique_ptr<SketchConstraint> constraint) {
    if (!constraint) {
        return {};
    }

    for (const auto& entityId : constraint->referencedEntities()) {
        if (entityId.empty() || !getEntity(entityId)) {
            return {};
        }
    }

    ConstraintID id = constraint->id();
    constraintIndex_[id] = constraints_.size();
    constraints_.push_back(std::move(constraint));

    invalidateSolver();
    return id;
}

ConstraintID Sketch::addCoincident(EntityID point1, EntityID point2) {
    return addConstraint(std::make_unique<constraints::CoincidentConstraint>(point1, point2));
}

ConstraintID Sketch::addHorizontal(EntityID lineOrPoint1, EntityID point2) {
    EntityID lineId = lineOrPoint1;
    if (!point2.empty()) {
        for (const auto& entity : entities_) {
            auto* line = dynamic_cast<SketchLine*>(entity.get());
            if (!line) {
                continue;
            }
            bool matches = (line->startPointId() == lineOrPoint1 && line->endPointId() == point2) ||
                           (line->startPointId() == point2 && line->endPointId() == lineOrPoint1);
            if (matches) {
                lineId = line->id();
                break;
            }
        }
    }

    if (!getEntityAs<SketchLine>(lineId)) {
        return {};
    }

    return addConstraint(std::make_unique<constraints::HorizontalConstraint>(lineId));
}

ConstraintID Sketch::addVertical(EntityID lineOrPoint1, EntityID point2) {
    EntityID lineId = lineOrPoint1;
    if (!point2.empty()) {
        for (const auto& entity : entities_) {
            auto* line = dynamic_cast<SketchLine*>(entity.get());
            if (!line) {
                continue;
            }
            bool matches = (line->startPointId() == lineOrPoint1 && line->endPointId() == point2) ||
                           (line->startPointId() == point2 && line->endPointId() == lineOrPoint1);
            if (matches) {
                lineId = line->id();
                break;
            }
        }
    }

    if (!getEntityAs<SketchLine>(lineId)) {
        return {};
    }

    return addConstraint(std::make_unique<constraints::VerticalConstraint>(lineId));
}

ConstraintID Sketch::addParallel(EntityID line1, EntityID line2) {
    if (!getEntityAs<SketchLine>(line1) || !getEntityAs<SketchLine>(line2)) {
        return {};
    }
    return addConstraint(std::make_unique<constraints::ParallelConstraint>(line1, line2));
}

ConstraintID Sketch::addPerpendicular(EntityID line1, EntityID line2) {
    if (!getEntityAs<SketchLine>(line1) || !getEntityAs<SketchLine>(line2)) {
        return {};
    }
    return addConstraint(std::make_unique<constraints::PerpendicularConstraint>(line1, line2));
}

ConstraintID Sketch::addDistance(EntityID entity1, EntityID entity2, double distance) {
    if (!getEntity(entity1) || !getEntity(entity2)) {
        return {};
    }
    return addConstraint(std::make_unique<constraints::DistanceConstraint>(entity1, entity2, distance));
}

ConstraintID Sketch::addRadius(EntityID arcOrCircle, double radius) {
    if (!getEntityAs<SketchArc>(arcOrCircle) && !getEntityAs<SketchCircle>(arcOrCircle)) {
        return {};
    }
    return addConstraint(std::make_unique<constraints::RadiusConstraint>(arcOrCircle, radius));
}

ConstraintID Sketch::addAngle(EntityID line1, EntityID line2, double angleDegrees) {
    if (!getEntityAs<SketchLine>(line1) || !getEntityAs<SketchLine>(line2)) {
        return {};
    }
    double radians = angleDegrees * std::numbers::pi_v<double> / 180.0;
    return addConstraint(std::make_unique<constraints::AngleConstraint>(line1, line2, radians));
}

ConstraintID Sketch::addFixed(EntityID) {
    return {};
}

bool Sketch::removeConstraint(ConstraintID id) {
    auto it = constraintIndex_.find(id);
    if (it == constraintIndex_.end()) {
        return false;
    }

    if (it->second >= constraints_.size()) {
        rebuildConstraintIndex();
        it = constraintIndex_.find(id);
        if (it == constraintIndex_.end() || it->second >= constraints_.size()) {
            return false;
        }
    }

    constraints_.erase(constraints_.begin() + static_cast<long>(it->second));
    rebuildConstraintIndex();
    invalidateSolver();
    return true;
}

SketchConstraint* Sketch::getConstraint(ConstraintID id) {
    auto it = constraintIndex_.find(id);
    if (it == constraintIndex_.end()) {
        return nullptr;
    }
    if (it->second >= constraints_.size()) {
        rebuildConstraintIndex();
        it = constraintIndex_.find(id);
        if (it == constraintIndex_.end() || it->second >= constraints_.size()) {
            return nullptr;
        }
    }
    return constraints_[it->second].get();
}

const SketchConstraint* Sketch::getConstraint(ConstraintID id) const {
    auto it = constraintIndex_.find(id);
    if (it == constraintIndex_.end() || it->second >= constraints_.size()) {
        return nullptr;
    }
    return constraints_[it->second].get();
}

std::vector<SketchConstraint*> Sketch::getConstraintsForEntity(EntityID entityId) {
    std::vector<SketchConstraint*> results;
    for (const auto& constraint : constraints_) {
        if (constraint && constraint->references(entityId)) {
            results.push_back(constraint.get());
        }
    }
    return results;
}

SolveResult Sketch::solve() {
    SolveResult result;

    if (constraints_.empty()) {
        result.success = true;
        return result;
    }

    if (!solver_ || solverDirty_) {
        rebuildSolver();
    }

    if (!solver_) {
        result.success = false;
        result.errorMessage = "Solver not available";
        return result;
    }

    SolverResult solverResult = solver_->solve();
    result.success = solverResult.success;
    result.iterations = solverResult.iterations;
    result.residual = solverResult.residual;
    result.conflictingConstraints = solverResult.conflictingConstraints;
    result.errorMessage = solverResult.errorMessage;
    return result;
}

SolveResult Sketch::solveWithDrag(EntityID draggedPoint, const Vec2d& targetPos) {
    SolveResult result;

    auto* point = getEntityAs<SketchPoint>(draggedPoint);
    if (!point) {
        result.success = false;
        result.errorMessage = "Dragged point not found";
        return result;
    }

    if (constraints_.empty()) {
        point->setPosition(targetPos.x, targetPos.y);
        result.success = true;
        return result;
    }

    if (!solver_ || solverDirty_) {
        rebuildSolver();
    }

    if (!solver_) {
        result.success = false;
        result.errorMessage = "Solver not available";
        return result;
    }

    SolverResult solverResult = solver_->solveWithDrag(draggedPoint, targetPos);
    result.success = solverResult.success;
    result.iterations = solverResult.iterations;
    result.residual = solverResult.residual;
    result.conflictingConstraints = solverResult.conflictingConstraints;
    result.errorMessage = solverResult.errorMessage;
    return result;
}

int Sketch::getDegreesOfFreedom() const {
    if (!dofDirty_ && cachedDOF_ >= 0) {
        return cachedDOF_;
    }

    int total = 0;
    for (const auto& entity : entities_) {
        if (entity) {
            total += entity->degreesOfFreedom();
        }
    }

    for (const auto& constraint : constraints_) {
        if (constraint) {
            total -= constraint->degreesRemoved();
        }
    }

    if (total < 0) {
        total = 0;
    }

    cachedDOF_ = total;
    dofDirty_ = false;
    return cachedDOF_;
}

bool Sketch::isOverConstrained() const {
    int total = 0;
    for (const auto& entity : entities_) {
        if (entity) {
            total += entity->degreesOfFreedom();
        }
    }
    for (const auto& constraint : constraints_) {
        if (constraint) {
            total -= constraint->degreesRemoved();
        }
    }
    return total < 0;
}

std::vector<ConstraintID> Sketch::getConflictingConstraints() const {
    return {};
}

ValidationResult Sketch::validate() const {
    ValidationResult result;

    for (const auto& entity : entities_) {
        if (!entity) {
            continue;
        }

        if (auto* point = dynamic_cast<SketchPoint*>(entity.get())) {
            if (point->connectedEntities().empty()) {
                result.warnings.push_back("Orphaned point: " + point->id());
                result.invalidEntities.push_back(point->id());
            }
            continue;
        }

        if (auto* line = dynamic_cast<SketchLine*>(entity.get())) {
            auto* start = getEntityAs<SketchPoint>(line->startPointId());
            auto* end = getEntityAs<SketchPoint>(line->endPointId());
            if (!start || !end) {
                result.valid = false;
                result.errors.push_back("Line has missing endpoint: " + line->id());
                result.invalidEntities.push_back(line->id());
                continue;
            }
            double length = SketchLine::length(start->position(), end->position());
            if (length < constants::MIN_GEOMETRY_SIZE) {
                result.valid = false;
                result.errors.push_back("Line length too small: " + line->id());
                result.invalidEntities.push_back(line->id());
            }
            continue;
        }

        if (auto* arc = dynamic_cast<SketchArc*>(entity.get())) {
            if (arc->radius() < constants::MIN_GEOMETRY_SIZE) {
                result.valid = false;
                result.errors.push_back("Arc radius too small: " + arc->id());
                result.invalidEntities.push_back(arc->id());
            }
            continue;
        }

        if (auto* circle = dynamic_cast<SketchCircle*>(entity.get())) {
            if (circle->radius() < constants::MIN_GEOMETRY_SIZE) {
                result.valid = false;
                result.errors.push_back("Circle radius too small: " + circle->id());
                result.invalidEntities.push_back(circle->id());
            }
        }
    }

    return result;
}

std::string Sketch::toJson() const {
    QJsonObject root;
    root["version"] = 1;

    QJsonObject plane;
    plane["origin"] = QJsonArray{plane_.origin.x, plane_.origin.y, plane_.origin.z};
    plane["xAxis"] = QJsonArray{plane_.xAxis.x, plane_.xAxis.y, plane_.xAxis.z};
    plane["yAxis"] = QJsonArray{plane_.yAxis.x, plane_.yAxis.y, plane_.yAxis.z};
    plane["normal"] = QJsonArray{plane_.normal.x, plane_.normal.y, plane_.normal.z};
    root["plane"] = plane;

    QJsonArray entityArray;
    for (const auto& entity : entities_) {
        if (!entity) {
            continue;
        }
        QJsonObject obj;
        entity->serialize(obj);
        entityArray.append(obj);
    }
    root["entities"] = entityArray;

    QJsonArray constraintArray;
    for (const auto& constraint : constraints_) {
        if (!constraint) {
            continue;
        }
        QJsonObject obj;
        constraint->serialize(obj);
        constraintArray.append(obj);
    }
    root["constraints"] = constraintArray;

    QJsonDocument doc(root);
    return doc.toJson(QJsonDocument::Compact).toStdString();
}

std::unique_ptr<Sketch> Sketch::fromJson(const std::string& json) {
    QJsonParseError parseError;
    QJsonDocument doc = QJsonDocument::fromJson(QString::fromStdString(json).toUtf8(), &parseError);
    if (parseError.error != QJsonParseError::NoError || !doc.isObject()) {
        return nullptr;
    }

    QJsonObject root = doc.object();
    SketchPlane plane = SketchPlane::XY();
    if (root.contains("plane") && root["plane"].isObject()) {
        QJsonObject planeObj = root["plane"].toObject();
        auto readVec3 = [](const QJsonObject& obj, const char* key, Vec3d& out) {
            if (!obj.contains(key) || !obj[key].isArray()) {
                return false;
            }
            QJsonArray arr = obj[key].toArray();
            if (arr.size() != 3) {
                return false;
            }
            if (!arr[0].isDouble() || !arr[1].isDouble() || !arr[2].isDouble()) {
                return false;
            }
            out.x = arr[0].toDouble();
            out.y = arr[1].toDouble();
            out.z = arr[2].toDouble();
            return true;
        };

        readVec3(planeObj, "origin", plane.origin);
        readVec3(planeObj, "xAxis", plane.xAxis);
        readVec3(planeObj, "yAxis", plane.yAxis);
        readVec3(planeObj, "normal", plane.normal);
    }

    auto sketch = std::make_unique<Sketch>(plane);

    if (root.contains("entities") && root["entities"].isArray()) {
        QJsonArray entities = root["entities"].toArray();
        for (const auto& value : entities) {
            if (!value.isObject()) {
                return nullptr;
            }
            QJsonObject obj = value.toObject();
            if (!obj.contains("type") || !obj["type"].isString()) {
                return nullptr;
            }

            std::string type = obj["type"].toString().toStdString();
            std::unique_ptr<SketchEntity> entity;
            if (type == "Point") {
                entity = std::make_unique<SketchPoint>();
            } else if (type == "Line") {
                entity = std::make_unique<SketchLine>();
            } else if (type == "Arc") {
                entity = std::make_unique<SketchArc>();
            } else if (type == "Circle") {
                entity = std::make_unique<SketchCircle>();
            } else {
                return nullptr;
            }

            if (!entity->deserialize(obj)) {
                return nullptr;
            }

            EntityID id = entity->id();
            sketch->entityIndex_[id] = sketch->entities_.size();
            sketch->entities_.push_back(std::move(entity));
        }
    }

    for (const auto& entity : sketch->entities_) {
        if (!entity) {
            continue;
        }
        if (auto* line = dynamic_cast<SketchLine*>(entity.get())) {
            if (auto* start = sketch->getEntityAs<SketchPoint>(line->startPointId())) {
                start->addConnectedEntity(line->id());
            }
            if (auto* end = sketch->getEntityAs<SketchPoint>(line->endPointId())) {
                end->addConnectedEntity(line->id());
            }
        } else if (auto* arc = dynamic_cast<SketchArc*>(entity.get())) {
            if (auto* center = sketch->getEntityAs<SketchPoint>(arc->centerPointId())) {
                center->addConnectedEntity(arc->id());
            }
        } else if (auto* circle = dynamic_cast<SketchCircle*>(entity.get())) {
            if (auto* center = sketch->getEntityAs<SketchPoint>(circle->centerPointId())) {
                center->addConnectedEntity(circle->id());
            }
        }
    }

    if (root.contains("constraints") && root["constraints"].isArray()) {
        QJsonArray constraints = root["constraints"].toArray();
        for (const auto& value : constraints) {
            if (!value.isObject()) {
                return nullptr;
            }
            auto constraint = ConstraintFactory::fromJson(value.toObject());
            if (!constraint) {
                return nullptr;
            }
            ConstraintID id = constraint->id();
            sketch->constraintIndex_[id] = sketch->constraints_.size();
            sketch->constraints_.push_back(std::move(constraint));
        }
    }

    sketch->invalidateSolver();
    return sketch;
}

EntityID Sketch::findNearest(const Vec2d& pos, double tolerance,
                             std::optional<EntityType> filter) const {
    EntityID bestId;
    double bestDistance = tolerance;
    gp_Pnt2d query(pos.x, pos.y);

    for (const auto& entity : entities_) {
        if (!entity) {
            continue;
        }
        if (filter && entity->type() != *filter) {
            continue;
        }

        double distance = std::numeric_limits<double>::infinity();
        switch (entity->type()) {
            case EntityType::Point: {
                auto* point = dynamic_cast<SketchPoint*>(entity.get());
                distance = point ? point->distanceTo(query) : distance;
                break;
            }
            case EntityType::Line: {
                auto* line = dynamic_cast<SketchLine*>(entity.get());
                if (!line) {
                    break;
                }
                auto* start = getEntityAs<SketchPoint>(line->startPointId());
                auto* end = getEntityAs<SketchPoint>(line->endPointId());
                if (!start || !end) {
                    break;
                }
                distance = SketchLine::distanceToPoint(query, start->position(), end->position());
                break;
            }
            case EntityType::Arc: {
                auto* arc = dynamic_cast<SketchArc*>(entity.get());
                if (!arc) {
                    break;
                }
                auto* center = getEntityAs<SketchPoint>(arc->centerPointId());
                if (!center) {
                    break;
                }
                double radial = std::abs(center->position().Distance(query) - arc->radius());
                if (arc->isNearWithCenter(query, center->position(), tolerance)) {
                    distance = radial;
                }
                break;
            }
            case EntityType::Circle: {
                auto* circle = dynamic_cast<SketchCircle*>(entity.get());
                if (!circle) {
                    break;
                }
                auto* center = getEntityAs<SketchPoint>(circle->centerPointId());
                if (!center) {
                    break;
                }
                distance = std::abs(center->position().Distance(query) - circle->radius());
                break;
            }
            default:
                break;
        }

        if (distance <= bestDistance) {
            bestDistance = distance;
            bestId = entity->id();
        }
    }

    return bestId;
}

std::vector<EntityID> Sketch::findInRect(const Vec2d& min, const Vec2d& max) const {
    std::vector<EntityID> results;
    BoundingBox2d rect;
    rect.minX = std::min(min.x, max.x);
    rect.minY = std::min(min.y, max.y);
    rect.maxX = std::max(min.x, max.x);
    rect.maxY = std::max(min.y, max.y);

    for (const auto& entity : entities_) {
        if (!entity) {
            continue;
        }

        BoundingBox2d bounds;
        switch (entity->type()) {
            case EntityType::Point: {
                auto* point = dynamic_cast<SketchPoint*>(entity.get());
                if (point) {
                    bounds = point->bounds();
                }
                break;
            }
            case EntityType::Line: {
                auto* line = dynamic_cast<SketchLine*>(entity.get());
                if (!line) {
                    break;
                }
                auto* start = getEntityAs<SketchPoint>(line->startPointId());
                auto* end = getEntityAs<SketchPoint>(line->endPointId());
                if (!start || !end) {
                    break;
                }
                bounds = SketchLine::boundsWithPoints(start->position(), end->position());
                break;
            }
            case EntityType::Arc: {
                auto* arc = dynamic_cast<SketchArc*>(entity.get());
                if (!arc) {
                    break;
                }
                auto* center = getEntityAs<SketchPoint>(arc->centerPointId());
                if (!center) {
                    break;
                }
                bounds = arc->boundsWithCenter(center->position());
                break;
            }
            case EntityType::Circle: {
                auto* circle = dynamic_cast<SketchCircle*>(entity.get());
                if (!circle) {
                    break;
                }
                auto* center = getEntityAs<SketchPoint>(circle->centerPointId());
                if (!center) {
                    break;
                }
                bounds = circle->boundsWithCenter(center->position());
                break;
            }
            default:
                break;
        }

        if (!bounds.isEmpty() && bounds.intersects(rect)) {
            results.push_back(entity->id());
        }
    }

    return results;
}

void Sketch::invalidateSolver() {
    solverDirty_ = true;
    dofDirty_ = true;
}

void Sketch::rebuildSolver() {
    solver_ = std::make_unique<ConstraintSolver>();
    SolverAdapter::populateSolver(*this, *solver_);

    solverDirty_ = false;
}

void Sketch::rebuildEntityIndex() {
    entityIndex_.clear();
    for (size_t i = 0; i < entities_.size(); ++i) {
        entityIndex_[entities_[i]->id()] = i;
    }
}

void Sketch::rebuildConstraintIndex() {
    constraintIndex_.clear();
    for (size_t i = 0; i < constraints_.size(); ++i) {
        constraintIndex_[constraints_[i]->id()] = i;
    }
}

} // namespace onecad::core::sketch
