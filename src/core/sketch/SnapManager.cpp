/**
 * @file SnapManager.cpp
 * @brief Implementation of snap system for precision drawing
 */

#include "SnapManager.h"
#include "Sketch.h"
#include "SketchPoint.h"
#include "SketchLine.h"
#include "SketchArc.h"
#include "SketchCircle.h"
#include "SketchEllipse.h"
#include <algorithm>
#include <cmath>
#include <limits>

namespace onecad::core::sketch {

namespace {
// Helper to convert gp_Pnt2d to Vec2d
inline Vec2d toVec2d(const gp_Pnt2d& p) {
    return {p.X(), p.Y()};
}

// Helper to convert Vec2d to gp_Pnt2d
inline gp_Pnt2d toGpPnt2d(const Vec2d& v) {
    return gp_Pnt2d(v.x, v.y);
}

constexpr double PI = 3.14159265358979323846;
constexpr double TWO_PI = 2.0 * PI;
} // anonymous namespace

SnapManager::SnapManager() {
    // Initialize all snap types to enabled by default
    for (int i = static_cast<int>(SnapType::Vertex);
         i <= static_cast<int>(SnapType::ActiveLayer3D); ++i) {
        snapTypeEnabled_[static_cast<SnapType>(i)] = true;
    }
}

void SnapManager::setSnapEnabled(SnapType type, bool enabled) {
    snapTypeEnabled_[type] = enabled;
}

bool SnapManager::isSnapEnabled(SnapType type) const {
    auto it = snapTypeEnabled_.find(type);
    return it != snapTypeEnabled_.end() ? it->second : true;
}

void SnapManager::setAllSnapsEnabled(bool enabled) {
    for (auto& [type, state] : snapTypeEnabled_) {
        state = enabled;
    }
    gridSnapEnabled_ = enabled;
}

void SnapManager::setExternalGeometry(const std::vector<Vec2d>& points,
                                      const std::vector<std::pair<Vec2d, Vec2d>>& lines)
{
    extPoints_ = points;
    extLines_ = lines;
}

SnapResult SnapManager::findBestSnap(
    const Vec2d& cursorPos,
    const Sketch& sketch,
    const std::unordered_set<EntityID>& excludeEntities) const
{
    if (!enabled_) {
        return SnapResult{};
    }

    if (spatialHashEnabled_) {
        rebuildSpatialHash(sketch);
    }

    auto snaps = findAllSnaps(cursorPos, sketch, excludeEntities);
    if (snaps.empty()) {
        return SnapResult{};
    }

    // Sort by priority (type first, then distance)
    std::sort(snaps.begin(), snaps.end());
    return snaps.front();
}

std::vector<SnapResult> SnapManager::findAllSnaps(
    const Vec2d& cursorPos,
    const Sketch& sketch,
    const std::unordered_set<EntityID>& excludeEntities) const
{
    if (!enabled_) {
        return {};
    }

    std::unordered_set<EntityID> candidateSet;
    const std::unordered_set<EntityID>* candidateFilter = nullptr;
    if (spatialHashEnabled_) {
        rebuildSpatialHash(sketch);
        const std::vector<EntityID> candidateIds = spatialHash_.query(cursorPos, snapRadius_);
        candidateSet.insert(candidateIds.begin(), candidateIds.end());
        candidateFilter = &candidateSet;
    }

    std::vector<SnapResult> results;
    const double radiusSq = snapRadius_ * snapRadius_;

    // Find all snap types in priority order
    if (isSnapEnabled(SnapType::Vertex)) {
        findVertexSnaps(cursorPos, sketch, excludeEntities, candidateFilter, radiusSq, results);
    }
    if (isSnapEnabled(SnapType::Endpoint)) {
        findEndpointSnaps(cursorPos, sketch, excludeEntities, candidateFilter, radiusSq, results);
    }
    if (isSnapEnabled(SnapType::Midpoint)) {
        findMidpointSnaps(cursorPos, sketch, excludeEntities, candidateFilter, radiusSq, results);
    }
    if (isSnapEnabled(SnapType::Center)) {
        findCenterSnaps(cursorPos, sketch, excludeEntities, candidateFilter, radiusSq, results);
    }
    if (isSnapEnabled(SnapType::Quadrant)) {
        findQuadrantSnaps(cursorPos, sketch, excludeEntities, candidateFilter, radiusSq, results);
    }
    if (isSnapEnabled(SnapType::Intersection)) {
        findIntersectionSnaps(cursorPos, sketch, excludeEntities, candidateFilter, radiusSq, results);
    }
    if (isSnapEnabled(SnapType::OnCurve)) {
        findOnCurveSnaps(cursorPos, sketch, excludeEntities, candidateFilter, radiusSq, results);
    }
    if (gridSnapEnabled_ && isSnapEnabled(SnapType::Grid)) {
        findGridSnaps(cursorPos, radiusSq, results);
    }
    if (isSnapEnabled(SnapType::Perpendicular)) {
        findPerpendicularSnaps(cursorPos, sketch, excludeEntities, radiusSq, results);
    }
    if (isSnapEnabled(SnapType::Tangent)) {
        findTangentSnaps(cursorPos, sketch, excludeEntities, radiusSq, results);
    }
    if (isSnapEnabled(SnapType::ActiveLayer3D)) {
        findExternalSnaps(cursorPos, radiusSq, results);
    }

    return results;
}

void SnapManager::rebuildSpatialHash(const Sketch& sketch) const {
    const size_t entityCount = sketch.getEntityCount();
    if (entityCount == lastEntityCount_) {
        return;
    }

    spatialHash_.rebuild(sketch);
    lastEntityCount_ = entityCount;
}

bool SnapManager::shouldConsiderEntity(const EntityID& entityId,
                                       const std::unordered_set<EntityID>* candidateSet) const
{
    if (!candidateSet) {
        return true;
    }
    return candidateSet->count(entityId) > 0;
}

// ========== Individual Snap Type Finders ==========

void SnapManager::findVertexSnaps(
    const Vec2d& cursorPos,
    const Sketch& sketch,
    const std::unordered_set<EntityID>& excludeEntities,
    const std::unordered_set<EntityID>* candidateSet,
    double radiusSq,
    std::vector<SnapResult>& results) const
{
    for (const auto& entity : sketch.getAllEntities()) {
        if (!shouldConsiderEntity(entity->id(), candidateSet)) continue;
        if (excludeEntities.count(entity->id())) continue;
        if (entity->type() != EntityType::Point) continue;

        const auto* point = static_cast<const SketchPoint*>(entity.get());
        Vec2d pos = toVec2d(point->position());
        double distSq = distanceSquared(cursorPos, pos);

        if (distSq <= radiusSq) {
            results.push_back({
                .snapped = true,
                .type = SnapType::Vertex,
                .position = pos,
                .entityId = entity->id(),
                .pointId = entity->id(),
                .distance = std::sqrt(distSq)
            });
        }
    }
}

void SnapManager::findEndpointSnaps(
    const Vec2d& cursorPos,
    const Sketch& sketch,
    const std::unordered_set<EntityID>& excludeEntities,
    const std::unordered_set<EntityID>* candidateSet,
    double radiusSq,
    std::vector<SnapResult>& results) const
{
    for (const auto& entity : sketch.getAllEntities()) {
        if (!shouldConsiderEntity(entity->id(), candidateSet)) continue;
        if (excludeEntities.count(entity->id())) continue;

        if (entity->type() == EntityType::Line) {
            const auto* line = static_cast<const SketchLine*>(entity.get());

            // Get start point
            const auto* startPt = sketch.getEntityAs<SketchPoint>(line->startPointId());
            if (startPt && !excludeEntities.count(line->startPointId())) {
                Vec2d pos = toVec2d(startPt->position());
                double distSq = distanceSquared(cursorPos, pos);
                if (distSq <= radiusSq) {
                    results.push_back({
                        .snapped = true,
                        .type = SnapType::Endpoint,
                        .position = pos,
                        .entityId = entity->id(),
                        .pointId = line->startPointId(),
                        .distance = std::sqrt(distSq)
                    });
                }
            }

            // Get end point
            const auto* endPt = sketch.getEntityAs<SketchPoint>(line->endPointId());
            if (endPt && !excludeEntities.count(line->endPointId())) {
                Vec2d pos = toVec2d(endPt->position());
                double distSq = distanceSquared(cursorPos, pos);
                if (distSq <= radiusSq) {
                    results.push_back({
                        .snapped = true,
                        .type = SnapType::Endpoint,
                        .position = pos,
                        .entityId = entity->id(),
                        .pointId = line->endPointId(),
                        .distance = std::sqrt(distSq)
                    });
                }
            }
        }
        else if (entity->type() == EntityType::Arc) {
            const auto* arc = static_cast<const SketchArc*>(entity.get());
            const auto* centerPt = sketch.getEntityAs<SketchPoint>(arc->centerPointId());
            if (!centerPt) continue;

            gp_Pnt2d center = centerPt->position();

            // Arc start point
            Vec2d startPos = toVec2d(arc->startPoint(center));
            double distSq = distanceSquared(cursorPos, startPos);
            if (distSq <= radiusSq) {
                results.push_back({
                    .snapped = true,
                    .type = SnapType::Endpoint,
                    .position = startPos,
                    .entityId = entity->id(),
                    .distance = std::sqrt(distSq)
                });
            }

            // Arc end point
            Vec2d endPos = toVec2d(arc->endPoint(center));
            distSq = distanceSquared(cursorPos, endPos);
            if (distSq <= radiusSq) {
                results.push_back({
                    .snapped = true,
                    .type = SnapType::Endpoint,
                    .position = endPos,
                    .entityId = entity->id(),
                    .distance = std::sqrt(distSq)
                });
            }
        }
    }
}

void SnapManager::findMidpointSnaps(
    const Vec2d& cursorPos,
    const Sketch& sketch,
    const std::unordered_set<EntityID>& excludeEntities,
    const std::unordered_set<EntityID>* candidateSet,
    double radiusSq,
    std::vector<SnapResult>& results) const
{
    for (const auto& entity : sketch.getAllEntities()) {
        if (!shouldConsiderEntity(entity->id(), candidateSet)) continue;
        if (excludeEntities.count(entity->id())) continue;

        if (entity->type() == EntityType::Line) {
            const auto* line = static_cast<const SketchLine*>(entity.get());
            const auto* startPt = sketch.getEntityAs<SketchPoint>(line->startPointId());
            const auto* endPt = sketch.getEntityAs<SketchPoint>(line->endPointId());
            if (!startPt || !endPt) continue;

            gp_Pnt2d mid = SketchLine::midpoint(startPt->position(), endPt->position());
            Vec2d midPos = toVec2d(mid);
            double distSq = distanceSquared(cursorPos, midPos);

            if (distSq <= radiusSq) {
                results.push_back({
                    .snapped = true,
                    .type = SnapType::Midpoint,
                    .position = midPos,
                    .entityId = entity->id(),
                    .distance = std::sqrt(distSq)
                });
            }
        }
        else if (entity->type() == EntityType::Arc) {
            const auto* arc = static_cast<const SketchArc*>(entity.get());
            const auto* centerPt = sketch.getEntityAs<SketchPoint>(arc->centerPointId());
            if (!centerPt) continue;

            Vec2d midPos = toVec2d(arc->midpoint(centerPt->position()));
            double distSq = distanceSquared(cursorPos, midPos);

            if (distSq <= radiusSq) {
                results.push_back({
                    .snapped = true,
                    .type = SnapType::Midpoint,
                    .position = midPos,
                    .entityId = entity->id(),
                    .distance = std::sqrt(distSq)
                });
            }
        }
    }
}

void SnapManager::findCenterSnaps(
    const Vec2d& cursorPos,
    const Sketch& sketch,
    const std::unordered_set<EntityID>& excludeEntities,
    const std::unordered_set<EntityID>* candidateSet,
    double radiusSq,
    std::vector<SnapResult>& results) const
{
    for (const auto& entity : sketch.getAllEntities()) {
        if (!shouldConsiderEntity(entity->id(), candidateSet)) continue;
        if (excludeEntities.count(entity->id())) continue;

        const SketchPoint* centerPt = nullptr;

        if (entity->type() == EntityType::Arc) {
            const auto* arc = static_cast<const SketchArc*>(entity.get());
            centerPt = sketch.getEntityAs<SketchPoint>(arc->centerPointId());
        }
        else if (entity->type() == EntityType::Circle) {
            const auto* circle = static_cast<const SketchCircle*>(entity.get());
            centerPt = sketch.getEntityAs<SketchPoint>(circle->centerPointId());
        }
        else if (entity->type() == EntityType::Ellipse) {
            const auto* ellipse = static_cast<const SketchEllipse*>(entity.get());
            centerPt = sketch.getEntityAs<SketchPoint>(ellipse->centerPointId());
        }

        if (!centerPt) continue;

        Vec2d pos = toVec2d(centerPt->position());
        double distSq = distanceSquared(cursorPos, pos);

        if (distSq <= radiusSq) {
            results.push_back({
                .snapped = true,
                .type = SnapType::Center,
                .position = pos,
                .entityId = entity->id(),
                .pointId = centerPt->id(),
                .distance = std::sqrt(distSq)
            });
        }
    }
}

void SnapManager::findQuadrantSnaps(
    const Vec2d& cursorPos,
    const Sketch& sketch,
    const std::unordered_set<EntityID>& excludeEntities,
    const std::unordered_set<EntityID>* candidateSet,
    double radiusSq,
    std::vector<SnapResult>& results) const
{
    // Quadrant angles: 0, 90, 180, 270 degrees
    constexpr double quadrantAngles[4] = {0.0, PI / 2.0, PI, 3.0 * PI / 2.0};

    for (const auto& entity : sketch.getAllEntities()) {
        if (!shouldConsiderEntity(entity->id(), candidateSet)) continue;
        if (excludeEntities.count(entity->id())) continue;

        if (entity->type() == EntityType::Circle) {
            const auto* circle = static_cast<const SketchCircle*>(entity.get());
            const auto* centerPt = sketch.getEntityAs<SketchPoint>(circle->centerPointId());
            if (!centerPt) continue;

            Vec2d center = toVec2d(centerPt->position());
            double r = circle->radius();

            for (double angle : quadrantAngles) {
                Vec2d quadPt = {
                    center.x + r * std::cos(angle),
                    center.y + r * std::sin(angle)
                };
                double distSq = distanceSquared(cursorPos, quadPt);

                if (distSq <= radiusSq) {
                    results.push_back({
                        .snapped = true,
                        .type = SnapType::Quadrant,
                        .position = quadPt,
                        .entityId = entity->id(),
                        .distance = std::sqrt(distSq)
                    });
                }
            }
        }
        else if (entity->type() == EntityType::Arc) {
            const auto* arc = static_cast<const SketchArc*>(entity.get());
            const auto* centerPt = sketch.getEntityAs<SketchPoint>(arc->centerPointId());
            if (!centerPt) continue;

            Vec2d center = toVec2d(centerPt->position());
            double r = arc->radius();

            for (double angle : quadrantAngles) {
                // Only include quadrant if it's within arc extent
                if (!arc->containsAngle(angle)) continue;

                Vec2d quadPt = {
                    center.x + r * std::cos(angle),
                    center.y + r * std::sin(angle)
                };
                double distSq = distanceSquared(cursorPos, quadPt);

                if (distSq <= radiusSq) {
                    results.push_back({
                        .snapped = true,
                        .type = SnapType::Quadrant,
                        .position = quadPt,
                        .entityId = entity->id(),
                        .distance = std::sqrt(distSq)
                    });
                }
            }
        }
        else if (entity->type() == EntityType::Ellipse) {
            const auto* ellipse = static_cast<const SketchEllipse*>(entity.get());
            const auto* centerPt = sketch.getEntityAs<SketchPoint>(ellipse->centerPointId());
            if (!centerPt) continue;

            const gp_Pnt2d centerPos = centerPt->position();
            for (double angle : quadrantAngles) {
                Vec2d quadPt = toVec2d(ellipse->pointAtParameter(centerPos, angle));
                const double distSq = distanceSquared(cursorPos, quadPt);
                if (distSq <= radiusSq) {
                    results.push_back({
                        .snapped = true,
                        .type = SnapType::Quadrant,
                        .position = quadPt,
                        .entityId = entity->id(),
                        .distance = std::sqrt(distSq)
                    });
                }
            }
        }
    }
}

void SnapManager::findIntersectionSnaps(
    const Vec2d& cursorPos,
    const Sketch& sketch,
    const std::unordered_set<EntityID>& excludeEntities,
    const std::unordered_set<EntityID>* candidateSet,
    double radiusSq,
    std::vector<SnapResult>& results) const
{
    // Collect all non-excluded entities for intersection testing
    std::vector<const SketchEntity*> entities;
    for (const auto& entity : sketch.getAllEntities()) {
        if (!shouldConsiderEntity(entity->id(), candidateSet)) continue;
        if (excludeEntities.count(entity->id())) continue;
        if (entity->type() == EntityType::Line ||
            entity->type() == EntityType::Arc ||
            entity->type() == EntityType::Circle ||
            entity->type() == EntityType::Ellipse) {
            entities.push_back(entity.get());
        }
    }

    // Test all pairs
    for (size_t i = 0; i < entities.size(); ++i) {
        for (size_t j = i + 1; j < entities.size(); ++j) {
            auto intersections = findEntityIntersections(
                entities[i], entities[j], sketch);

            for (const auto& pt : intersections) {
                double distSq = distanceSquared(cursorPos, pt);
                if (distSq <= radiusSq) {
                    results.push_back({
                        .snapped = true,
                        .type = SnapType::Intersection,
                        .position = pt,
                        .entityId = entities[i]->id(),
                        .secondEntityId = entities[j]->id(),
                        .distance = std::sqrt(distSq)
                    });
                }
            }
        }
    }
}

void SnapManager::findOnCurveSnaps(
    const Vec2d& cursorPos,
    const Sketch& sketch,
    const std::unordered_set<EntityID>& excludeEntities,
    const std::unordered_set<EntityID>* candidateSet,
    double radiusSq,
    std::vector<SnapResult>& results) const
{
    for (const auto& entity : sketch.getAllEntities()) {
        if (!shouldConsiderEntity(entity->id(), candidateSet)) continue;
        if (excludeEntities.count(entity->id())) continue;

        Vec2d nearestPt;
        bool found = false;

        if (entity->type() == EntityType::Line) {
            const auto* line = static_cast<const SketchLine*>(entity.get());
            const auto* startPt = sketch.getEntityAs<SketchPoint>(line->startPointId());
            const auto* endPt = sketch.getEntityAs<SketchPoint>(line->endPointId());
            if (!startPt || !endPt) continue;

            nearestPt = nearestPointOnLine(
                cursorPos,
                toVec2d(startPt->position()),
                toVec2d(endPt->position())
            );
            found = true;
        }
        else if (entity->type() == EntityType::Circle) {
            const auto* circle = static_cast<const SketchCircle*>(entity.get());
            const auto* centerPt = sketch.getEntityAs<SketchPoint>(circle->centerPointId());
            if (!centerPt) continue;

            nearestPt = nearestPointOnCircle(
                cursorPos,
                toVec2d(centerPt->position()),
                circle->radius()
            );
            found = true;
        }
        else if (entity->type() == EntityType::Arc) {
            const auto* arc = static_cast<const SketchArc*>(entity.get());
            const auto* centerPt = sketch.getEntityAs<SketchPoint>(arc->centerPointId());
            if (!centerPt) continue;

            // Find nearest point on full circle, then check if within arc
            Vec2d center = toVec2d(centerPt->position());
            Vec2d onCircle = nearestPointOnCircle(cursorPos, center, arc->radius());

            // Check if point is within arc extent
            double angle = std::atan2(onCircle.y - center.y, onCircle.x - center.x);
            if (arc->containsAngle(angle)) {
                nearestPt = onCircle;
                found = true;
            } else {
                // Snap to nearest arc endpoint
                Vec2d start = toVec2d(arc->startPoint(centerPt->position()));
                Vec2d end = toVec2d(arc->endPoint(centerPt->position()));
                double dStart = distanceSquared(cursorPos, start);
                double dEnd = distanceSquared(cursorPos, end);
                nearestPt = (dStart < dEnd) ? start : end;
                found = true;
            }
        }
        else if (entity->type() == EntityType::Ellipse) {
            const auto* ellipse = static_cast<const SketchEllipse*>(entity.get());
            const auto* centerPt = sketch.getEntityAs<SketchPoint>(ellipse->centerPointId());
            if (!centerPt) continue;

            const gp_Pnt2d center = centerPt->position();
            constexpr int sampleCount = 32;
            const double step = TWO_PI / static_cast<double>(sampleCount);

            auto pointAt = [&](double t) {
                return toVec2d(ellipse->pointAtParameter(center, t));
            };
            auto distAt = [&](double t) {
                return distanceSquared(cursorPos, pointAt(t));
            };

            int bestIndex = 0;
            double bestDistSq = std::numeric_limits<double>::max();
            for (int i = 0; i < sampleCount; ++i) {
                const double t = static_cast<double>(i) * step;
                const double d = distAt(t);
                if (d < bestDistSq) {
                    bestDistSq = d;
                    bestIndex = i;
                }
            }

            double left = (static_cast<double>(bestIndex) - 1.0) * step;
            double right = (static_cast<double>(bestIndex) + 1.0) * step;

            // Ternary search around best sampled segment.
            for (int iter = 0; iter < 18; ++iter) {
                const double m1 = left + (right - left) / 3.0;
                const double m2 = right - (right - left) / 3.0;
                if (distAt(m1) < distAt(m2)) {
                    right = m2;
                } else {
                    left = m1;
                }
            }

            const double bestT = 0.5 * (left + right);
            nearestPt = pointAt(bestT);
            found = true;
        }

        if (found) {
            double distSq = distanceSquared(cursorPos, nearestPt);
            if (distSq <= radiusSq) {
                results.push_back({
                    .snapped = true,
                    .type = SnapType::OnCurve,
                    .position = nearestPt,
                    .entityId = entity->id(),
                    .distance = std::sqrt(distSq)
                });
            }
        }
    }
}

void SnapManager::findGridSnaps(
    const Vec2d& cursorPos,
    double radiusSq,
    std::vector<SnapResult>& results) const
{
    if (gridSize_ <= 0) return;

    // Find nearest grid point
    double gx = std::round(cursorPos.x / gridSize_) * gridSize_;
    double gy = std::round(cursorPos.y / gridSize_) * gridSize_;
    Vec2d gridPt = {gx, gy};

    double distSq = distanceSquared(cursorPos, gridPt);
    if (distSq <= radiusSq) {
        results.push_back({
            .snapped = true,
            .type = SnapType::Grid,
            .position = gridPt,
            .distance = std::sqrt(distSq)
        });
    }
}

void SnapManager::findPerpendicularSnaps(
    const Vec2d& cursorPos,
    const Sketch& sketch,
    const std::unordered_set<EntityID>& excludeEntities,
    double radiusSq,
    std::vector<SnapResult>& results) const
{
    constexpr double kGeomEps = 1e-6;

    for (const auto& entity : sketch.getAllEntities()) {
        if (excludeEntities.count(entity->id())) continue;

        Vec2d foot;
        bool found = false;

        if (entity->type() == EntityType::Line) {
            const auto* line = static_cast<const SketchLine*>(entity.get());
            const auto* startPt = sketch.getEntityAs<SketchPoint>(line->startPointId());
            const auto* endPt = sketch.getEntityAs<SketchPoint>(line->endPointId());
            if (!startPt || !endPt) continue;

            const Vec2d startPos = toVec2d(startPt->position());
            const Vec2d endPos = toVec2d(endPt->position());
            const Vec2d lineDir{endPos.x - startPos.x, endPos.y - startPos.y};
            const double len2 = lineDir.x * lineDir.x + lineDir.y * lineDir.y;
            if (len2 < 1e-12) continue;

            const double t = ((cursorPos.x - startPos.x) * lineDir.x +
                              (cursorPos.y - startPos.y) * lineDir.y) / len2;
            if (t < 0.0 || t > 1.0) continue;

            foot = {
                startPos.x + t * lineDir.x,
                startPos.y + t * lineDir.y
            };
            found = true;
        }
        else if (entity->type() == EntityType::Circle) {
            const auto* circle = static_cast<const SketchCircle*>(entity.get());
            const auto* centerPt = sketch.getEntityAs<SketchPoint>(circle->centerPointId());
            if (!centerPt) continue;

            const double radius = circle->radius();
            if (radius < kGeomEps) continue;

            const Vec2d center = toVec2d(centerPt->position());
            const double dx = cursorPos.x - center.x;
            const double dy = cursorPos.y - center.y;
            const double len2 = dx * dx + dy * dy;
            if (len2 < 1e-12) continue;

            const double len = std::sqrt(len2);
            foot = {
                center.x + radius * dx / len,
                center.y + radius * dy / len
            };
            found = true;
        }
        else if (entity->type() == EntityType::Arc) {
            const auto* arc = static_cast<const SketchArc*>(entity.get());
            const auto* centerPt = sketch.getEntityAs<SketchPoint>(arc->centerPointId());
            if (!centerPt) continue;

            const double radius = arc->radius();
            if (radius < kGeomEps) continue;

            const Vec2d center = toVec2d(centerPt->position());
            const double dx = cursorPos.x - center.x;
            const double dy = cursorPos.y - center.y;
            const double len2 = dx * dx + dy * dy;
            if (len2 < 1e-12) continue;

            const double len = std::sqrt(len2);
            const Vec2d onCircle{
                center.x + radius * dx / len,
                center.y + radius * dy / len
            };

            const double angle = std::atan2(onCircle.y - center.y, onCircle.x - center.x);
            if (!arc->containsAngle(angle)) continue;

            foot = onCircle;
            found = true;
        }

        if (!found) continue;

        const double distSq = distanceSquared(cursorPos, foot);
        if (distSq <= radiusSq) {
            results.push_back({
                .snapped = true,
                .type = SnapType::Perpendicular,
                .position = foot,
                .entityId = entity->id(),
                .distance = std::sqrt(distSq)
            });
        }
    }
}

void SnapManager::findTangentSnaps(
    const Vec2d& cursorPos,
    const Sketch& sketch,
    const std::unordered_set<EntityID>& excludeEntities,
    double radiusSq,
    std::vector<SnapResult>& results) const
{
    constexpr double kGeomEps = 1e-6;

    for (const auto& entity : sketch.getAllEntities()) {
        if (excludeEntities.count(entity->id())) continue;

        Vec2d center;
        double radius = 0.0;
        bool isArc = false;
        const SketchArc* arc = nullptr;

        if (entity->type() == EntityType::Circle) {
            const auto* circle = static_cast<const SketchCircle*>(entity.get());
            const auto* centerPt = sketch.getEntityAs<SketchPoint>(circle->centerPointId());
            if (!centerPt) continue;
            center = toVec2d(centerPt->position());
            radius = circle->radius();
        }
        else if (entity->type() == EntityType::Arc) {
            arc = static_cast<const SketchArc*>(entity.get());
            const auto* centerPt = sketch.getEntityAs<SketchPoint>(arc->centerPointId());
            if (!centerPt) continue;
            center = toVec2d(centerPt->position());
            radius = arc->radius();
            isArc = true;
        }
        else if (entity->type() == EntityType::Ellipse) {
            // TODO: Add analytic tangent snap solving for ellipses.
            continue;
        }
        else {
            continue;
        }

        if (radius < kGeomEps) continue;

        const Vec2d vp{cursorPos.x - center.x, cursorPos.y - center.y};
        const double c2 = vp.x * vp.x + vp.y * vp.y;
        const double r2 = radius * radius;
        if (c2 <= r2 + 1e-12) continue;

        const double discriminant = c2 - r2;
        const double sqrtDisc = std::sqrt(discriminant);
        const Vec2d offset{-vp.y, vp.x};
        const double scale = radius * sqrtDisc / c2;
        const Vec2d base{
            center.x + vp.x * (r2 / c2),
            center.y + vp.y * (r2 / c2)
        };

        const Vec2d tp1{base.x + offset.x * scale, base.y + offset.y * scale};
        const Vec2d tp2{base.x - offset.x * scale, base.y - offset.y * scale};

        auto isValidTangentPoint = [&](const Vec2d& point) {
            if (!isArc) return true;
            const double angle = std::atan2(point.y - center.y, point.x - center.x);
            return arc->containsAngle(angle);
        };

        const bool tp1Valid = isValidTangentPoint(tp1);
        const bool tp2Valid = isValidTangentPoint(tp2);
        if (!tp1Valid && !tp2Valid) continue;

        double bestDistSq = std::numeric_limits<double>::max();
        Vec2d bestPoint;
        if (tp1Valid) {
            bestDistSq = distanceSquared(cursorPos, tp1);
            bestPoint = tp1;
        }
        if (tp2Valid) {
            const double distSq = distanceSquared(cursorPos, tp2);
            if (distSq < bestDistSq) {
                bestDistSq = distSq;
                bestPoint = tp2;
            }
        }

        if (bestDistSq <= radiusSq) {
            results.push_back({
                .snapped = true,
                .type = SnapType::Tangent,
                .position = bestPoint,
                .entityId = entity->id(),
                .distance = std::sqrt(bestDistSq)
            });
        }
    }
}

// ========== Geometry Helpers ==========

double SnapManager::distanceSquared(const Vec2d& a, const Vec2d& b) {
    double dx = b.x - a.x;
    double dy = b.y - a.y;
    return dx * dx + dy * dy;
}

Vec2d SnapManager::nearestPointOnLine(
    const Vec2d& point,
    const Vec2d& lineStart,
    const Vec2d& lineEnd)
{
    double dx = lineEnd.x - lineStart.x;
    double dy = lineEnd.y - lineStart.y;
    double lengthSq = dx * dx + dy * dy;

    if (lengthSq < 1e-12) {
        // Degenerate line, return start
        return lineStart;
    }

    // Project point onto line, clamped to [0, 1]
    double t = ((point.x - lineStart.x) * dx + (point.y - lineStart.y) * dy) / lengthSq;
    t = std::max(0.0, std::min(1.0, t));

    return {
        lineStart.x + t * dx,
        lineStart.y + t * dy
    };
}

Vec2d SnapManager::nearestPointOnCircle(
    const Vec2d& point,
    const Vec2d& center,
    double radius)
{
    double dx = point.x - center.x;
    double dy = point.y - center.y;
    double dist = std::sqrt(dx * dx + dy * dy);

    if (dist < 1e-12) {
        // Point at center, return arbitrary point on circle
        return {center.x + radius, center.y};
    }

    return {
        center.x + radius * dx / dist,
        center.y + radius * dy / dist
    };
}

std::optional<Vec2d> SnapManager::lineLineIntersection(
    const Vec2d& p1, const Vec2d& p2,
    const Vec2d& p3, const Vec2d& p4)
{
    double d1x = p2.x - p1.x;
    double d1y = p2.y - p1.y;
    double d2x = p4.x - p3.x;
    double d2y = p4.y - p3.y;

    double cross = d1x * d2y - d1y * d2x;
    if (std::abs(cross) < 1e-12) {
        // Lines are parallel
        return std::nullopt;
    }

    double dx = p3.x - p1.x;
    double dy = p3.y - p1.y;

    double t = (dx * d2y - dy * d2x) / cross;
    double u = (dx * d1y - dy * d1x) / cross;

    // Check if intersection is within both line segments
    if (t >= 0.0 && t <= 1.0 && u >= 0.0 && u <= 1.0) {
        return Vec2d{
            p1.x + t * d1x,
            p1.y + t * d1y
        };
    }

    return std::nullopt;
}

std::vector<Vec2d> SnapManager::lineCircleIntersection(
    const Vec2d& lineStart,
    const Vec2d& lineEnd,
    const Vec2d& center,
    double radius)
{
    std::vector<Vec2d> result;

    double dx = lineEnd.x - lineStart.x;
    double dy = lineEnd.y - lineStart.y;
    double fx = lineStart.x - center.x;
    double fy = lineStart.y - center.y;

    double a = dx * dx + dy * dy;
    double b = 2.0 * (fx * dx + fy * dy);
    double c = fx * fx + fy * fy - radius * radius;

    double discriminant = b * b - 4.0 * a * c;
    if (discriminant < 0 || a < 1e-12) {
        return result;
    }

    discriminant = std::sqrt(discriminant);

    double t1 = (-b - discriminant) / (2.0 * a);
    double t2 = (-b + discriminant) / (2.0 * a);

    if (t1 >= 0.0 && t1 <= 1.0) {
        result.push_back({
            lineStart.x + t1 * dx,
            lineStart.y + t1 * dy
        });
    }

    if (t2 >= 0.0 && t2 <= 1.0 && std::abs(t2 - t1) > 1e-12) {
        result.push_back({
            lineStart.x + t2 * dx,
            lineStart.y + t2 * dy
        });
    }

    return result;
}

std::vector<Vec2d> SnapManager::circleCircleIntersection(
    const Vec2d& center1,
    double radius1,
    const Vec2d& center2,
    double radius2)
{
    std::vector<Vec2d> result;

    double dx = center2.x - center1.x;
    double dy = center2.y - center1.y;
    double d = std::sqrt(dx * dx + dy * dy);

    // Check for no intersection
    if (d > radius1 + radius2 + 1e-12 ||
        d < std::abs(radius1 - radius2) - 1e-12 ||
        d < 1e-12) {
        return result;
    }

    double a = (radius1 * radius1 - radius2 * radius2 + d * d) / (2.0 * d);
    double h_sq = radius1 * radius1 - a * a;
    if (h_sq < 0) h_sq = 0;
    double h = std::sqrt(h_sq);

    // Midpoint along line between centers
    double px = center1.x + a * dx / d;
    double py = center1.y + a * dy / d;

    // Perpendicular direction
    double rx = -dy / d;
    double ry = dx / d;

    result.push_back({px + h * rx, py + h * ry});
    if (h > 1e-12) {
        result.push_back({px - h * rx, py - h * ry});
    }

    return result;
}

// ========== Helper for intersection finding ==========

void SnapManager::findExternalSnaps(
    const Vec2d& cursorPos,
    double radiusSq,
    std::vector<SnapResult>& results) const
{
    // Snap to 3D Points
    for (const auto& pt : extPoints_) {
        double distSq = distanceSquared(cursorPos, pt);
        if (distSq <= radiusSq) {
            results.push_back({
                .snapped = true,
                .type = SnapType::ActiveLayer3D,
                .position = pt,
                .distance = std::sqrt(distSq)
            });
        }
    }

    // Snap to 3D Lines/Edges
    for (const auto& line : extLines_) {
        Vec2d nearest = nearestPointOnLine(cursorPos, line.first, line.second);
        double distSq = distanceSquared(cursorPos, nearest);
        if (distSq <= radiusSq) {
            results.push_back({
                .snapped = true,
                .type = SnapType::ActiveLayer3D,
                .position = nearest,
                .distance = std::sqrt(distSq)
            });
        }
    }
}

std::vector<Vec2d> SnapManager::findEntityIntersections(
    const SketchEntity* e1,
    const SketchEntity* e2,
    const Sketch& sketch) const
{
    std::vector<Vec2d> result;

    auto getLinePoints = [&sketch](const SketchLine* line, Vec2d& start, Vec2d& end) -> bool {
        const auto* startPt = sketch.getEntityAs<SketchPoint>(line->startPointId());
        const auto* endPt = sketch.getEntityAs<SketchPoint>(line->endPointId());
        if (!startPt || !endPt) {
            return false;
        }
        start = toVec2d(startPt->position());
        end = toVec2d(endPt->position());
        return true;
    };

    auto getCircleData = [&sketch](const SketchCircle* circle, Vec2d& center, double& radius) -> bool {
        const auto* centerPt = sketch.getEntityAs<SketchPoint>(circle->centerPointId());
        if (!centerPt) {
            return false;
        }
        center = toVec2d(centerPt->position());
        radius = circle->radius();
        return true;
    };

    auto getArcData = [&sketch](const SketchArc* arc, Vec2d& center, double& radius) -> bool {
        const auto* centerPt = sketch.getEntityAs<SketchPoint>(arc->centerPointId());
        if (!centerPt) {
            return false;
        }
        center = toVec2d(centerPt->position());
        radius = arc->radius();
        return true;
    };

    auto getEllipseData = [&sketch](const SketchEllipse* ellipse,
                                    Vec2d& center,
                                    double& majorRadius,
                                    double& minorRadius,
                                    double& rotation) -> bool {
        const auto* centerPt = sketch.getEntityAs<SketchPoint>(ellipse->centerPointId());
        if (!centerPt) {
            return false;
        }
        center = toVec2d(centerPt->position());
        majorRadius = ellipse->majorRadius();
        minorRadius = ellipse->minorRadius();
        rotation = ellipse->rotation();
        return majorRadius > 1e-12 && minorRadius > 1e-12;
    };

    auto lineEllipseIntersection = [](const Vec2d& lineStart,
                                      const Vec2d& lineEnd,
                                      const Vec2d& center,
                                      double majorRadius,
                                      double minorRadius,
                                      double rotation) -> std::vector<Vec2d> {
        std::vector<Vec2d> intersections;

        const double cosR = std::cos(rotation);
        const double sinR = std::sin(rotation);

        auto toEllipseLocal = [&](const Vec2d& worldPt) {
            const double dx = worldPt.x - center.x;
            const double dy = worldPt.y - center.y;
            return Vec2d{
                dx * cosR + dy * sinR,
                -dx * sinR + dy * cosR
            };
        };

        auto fromEllipseLocal = [&](const Vec2d& localPt) {
            return Vec2d{
                center.x + localPt.x * cosR - localPt.y * sinR,
                center.y + localPt.x * sinR + localPt.y * cosR
            };
        };

        const Vec2d p0Local = toEllipseLocal(lineStart);
        const Vec2d p1Local = toEllipseLocal(lineEnd);

        const Vec2d p0{p0Local.x / majorRadius, p0Local.y / minorRadius};
        const Vec2d p1{p1Local.x / majorRadius, p1Local.y / minorRadius};
        const Vec2d d{p1.x - p0.x, p1.y - p0.y};

        const double a = d.x * d.x + d.y * d.y;
        const double b = 2.0 * (p0.x * d.x + p0.y * d.y);
        const double c = p0.x * p0.x + p0.y * p0.y - 1.0;

        const double discriminant = b * b - 4.0 * a * c;
        if (a < 1e-12 || discriminant < 0.0) {
            return intersections;
        }

        const double sqrtDiscriminant = std::sqrt(discriminant);
        const double t1 = (-b - sqrtDiscriminant) / (2.0 * a);
        const double t2 = (-b + sqrtDiscriminant) / (2.0 * a);

        auto appendIfOnSegment = [&](double t) {
            if (t < 0.0 || t > 1.0) {
                return;
            }
            const Vec2d hitLocal{
                p0Local.x + t * (p1Local.x - p0Local.x),
                p0Local.y + t * (p1Local.y - p0Local.y)
            };
            intersections.push_back(fromEllipseLocal(hitLocal));
        };

        appendIfOnSegment(t1);
        if (std::abs(t2 - t1) > 1e-12) {
            appendIfOnSegment(t2);
        }

        return intersections;
    };

    // Line-Line
    if (e1->type() == EntityType::Line && e2->type() == EntityType::Line) {
        Vec2d p1;
        Vec2d p2;
        Vec2d p3;
        Vec2d p4;
        if (!getLinePoints(static_cast<const SketchLine*>(e1), p1, p2) ||
            !getLinePoints(static_cast<const SketchLine*>(e2), p3, p4)) {
            return result;
        }
        if (auto pt = lineLineIntersection(p1, p2, p3, p4)) {
            result.push_back(*pt);
        }
    }
    // Line-Circle
    else if (e1->type() == EntityType::Line && e2->type() == EntityType::Circle) {
        Vec2d p1;
        Vec2d p2;
        Vec2d center;
        double radius = 0.0;
        if (!getLinePoints(static_cast<const SketchLine*>(e1), p1, p2) ||
            !getCircleData(static_cast<const SketchCircle*>(e2), center, radius)) {
            return result;
        }
        auto pts = lineCircleIntersection(p1, p2, center, radius);
        result.insert(result.end(), pts.begin(), pts.end());
    }
    else if (e1->type() == EntityType::Circle && e2->type() == EntityType::Line) {
        Vec2d center;
        Vec2d p1;
        Vec2d p2;
        double radius = 0.0;
        if (!getCircleData(static_cast<const SketchCircle*>(e1), center, radius) ||
            !getLinePoints(static_cast<const SketchLine*>(e2), p1, p2)) {
            return result;
        }
        auto pts = lineCircleIntersection(p1, p2, center, radius);
        result.insert(result.end(), pts.begin(), pts.end());
    }
    // Circle-Circle
    else if (e1->type() == EntityType::Circle && e2->type() == EntityType::Circle) {
        Vec2d c1;
        Vec2d c2;
        double r1 = 0.0;
        double r2 = 0.0;
        if (!getCircleData(static_cast<const SketchCircle*>(e1), c1, r1) ||
            !getCircleData(static_cast<const SketchCircle*>(e2), c2, r2)) {
            return result;
        }
        auto pts = circleCircleIntersection(c1, r1, c2, r2);
        result.insert(result.end(), pts.begin(), pts.end());
    }
    // Line-Arc
    else if (e1->type() == EntityType::Line && e2->type() == EntityType::Arc) {
        Vec2d p1;
        Vec2d p2;
        Vec2d center;
        double radius = 0.0;
        if (!getLinePoints(static_cast<const SketchLine*>(e1), p1, p2) ||
            !getArcData(static_cast<const SketchArc*>(e2), center, radius)) {
            return result;
        }
        auto pts = lineCircleIntersection(p1, p2, center, radius);
        for (const auto& pt : pts) {
            double angle = std::atan2(pt.y - center.y, pt.x - center.x);
            if (static_cast<const SketchArc*>(e2)->containsAngle(angle)) {
                result.push_back(pt);
            }
        }
    }
    else if (e1->type() == EntityType::Arc && e2->type() == EntityType::Line) {
        Vec2d center;
        Vec2d p1;
        Vec2d p2;
        double radius = 0.0;
        if (!getArcData(static_cast<const SketchArc*>(e1), center, radius) ||
            !getLinePoints(static_cast<const SketchLine*>(e2), p1, p2)) {
            return result;
        }
        auto pts = lineCircleIntersection(p1, p2, center, radius);
        for (const auto& pt : pts) {
            double angle = std::atan2(pt.y - center.y, pt.x - center.x);
            if (static_cast<const SketchArc*>(e1)->containsAngle(angle)) {
                result.push_back(pt);
            }
        }
    }
    // Arc-Circle
    else if (e1->type() == EntityType::Arc && e2->type() == EntityType::Circle) {
        Vec2d c1;
        Vec2d c2;
        double r1 = 0.0;
        double r2 = 0.0;
        if (!getArcData(static_cast<const SketchArc*>(e1), c1, r1) ||
            !getCircleData(static_cast<const SketchCircle*>(e2), c2, r2)) {
            return result;
        }
        auto pts = circleCircleIntersection(c1, r1, c2, r2);
        for (const auto& pt : pts) {
            double angle = std::atan2(pt.y - c1.y, pt.x - c1.x);
            if (static_cast<const SketchArc*>(e1)->containsAngle(angle)) {
                result.push_back(pt);
            }
        }
    }
    else if (e1->type() == EntityType::Circle && e2->type() == EntityType::Arc) {
        Vec2d c1;
        Vec2d c2;
        double r1 = 0.0;
        double r2 = 0.0;
        if (!getCircleData(static_cast<const SketchCircle*>(e1), c1, r1) ||
            !getArcData(static_cast<const SketchArc*>(e2), c2, r2)) {
            return result;
        }
        auto pts = circleCircleIntersection(c1, r1, c2, r2);
        for (const auto& pt : pts) {
            double angle = std::atan2(pt.y - c2.y, pt.x - c2.x);
            if (static_cast<const SketchArc*>(e2)->containsAngle(angle)) {
                result.push_back(pt);
            }
        }
    }
    // Arc-Arc
    else if (e1->type() == EntityType::Arc && e2->type() == EntityType::Arc) {
        Vec2d c1;
        Vec2d c2;
        double r1 = 0.0;
        double r2 = 0.0;
        if (!getArcData(static_cast<const SketchArc*>(e1), c1, r1) ||
            !getArcData(static_cast<const SketchArc*>(e2), c2, r2)) {
            return result;
        }
        auto pts = circleCircleIntersection(c1, r1, c2, r2);
        for (const auto& pt : pts) {
            double angle1 = std::atan2(pt.y - c1.y, pt.x - c1.x);
            double angle2 = std::atan2(pt.y - c2.y, pt.x - c2.x);
            if (static_cast<const SketchArc*>(e1)->containsAngle(angle1) &&
                static_cast<const SketchArc*>(e2)->containsAngle(angle2)) {
                result.push_back(pt);
            }
        }
    }
    // Line-Ellipse
    else if (e1->type() == EntityType::Line && e2->type() == EntityType::Ellipse) {
        Vec2d p1;
        Vec2d p2;
        Vec2d center;
        double majorRadius = 0.0;
        double minorRadius = 0.0;
        double rotation = 0.0;
        if (!getLinePoints(static_cast<const SketchLine*>(e1), p1, p2) ||
            !getEllipseData(static_cast<const SketchEllipse*>(e2), center, majorRadius, minorRadius, rotation)) {
            return result;
        }
        auto pts = lineEllipseIntersection(p1, p2, center, majorRadius, minorRadius, rotation);
        result.insert(result.end(), pts.begin(), pts.end());
    }
    else if (e1->type() == EntityType::Ellipse && e2->type() == EntityType::Line) {
        Vec2d center;
        Vec2d p1;
        Vec2d p2;
        double majorRadius = 0.0;
        double minorRadius = 0.0;
        double rotation = 0.0;
        if (!getEllipseData(static_cast<const SketchEllipse*>(e1), center, majorRadius, minorRadius, rotation) ||
            !getLinePoints(static_cast<const SketchLine*>(e2), p1, p2)) {
            return result;
        }
        auto pts = lineEllipseIntersection(p1, p2, center, majorRadius, minorRadius, rotation);
        result.insert(result.end(), pts.begin(), pts.end());
    }

    return result;
}

} // namespace onecad::core::sketch
