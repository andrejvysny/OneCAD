#include "ResidualGeometry.h"

#include "Sketch.h"
#include "constraints/Constraints.h"

#include <algorithm>
#include <cmath>
#include <limits>

namespace onecad::core::sketch {

bool finiteEntity(const Sketch& sketch, const EntityID& id) {
    const auto* entity = sketch.getEntity(id);
    if (!entity) return false;
    if (const auto* p = dynamic_cast<const SketchPoint*>(entity)) {
        return std::isfinite(p->x()) && std::isfinite(p->y());
    }
    std::vector<EntityID> points;
    if (const auto* line = dynamic_cast<const SketchLine*>(entity)) {
        points = {line->startPointId(), line->endPointId()};
    } else if (const auto* arc = dynamic_cast<const SketchArc*>(entity)) {
        points = {arc->centerPointId()};
        if (arc->hasEndpointPoints()) {
            points.push_back(arc->startPointId());
            points.push_back(arc->endPointId());
        }
    } else if (const auto* circle = dynamic_cast<const SketchCircle*>(entity)) {
        points = {circle->centerPointId()};
    } else if (const auto* ellipse = dynamic_cast<const SketchEllipse*>(entity)) {
        points = {ellipse->centerPointId()};
    }
    for (const auto& point_id : points) {
        const auto* p = sketch.getEntityAs<SketchPoint>(point_id);
        if (!p || !std::isfinite(p->x()) || !std::isfinite(p->y())) return false;
    }
    if (const auto* arc = dynamic_cast<const SketchArc*>(entity)) {
        return std::isfinite(arc->radius()) && std::isfinite(arc->startAngle()) &&
               std::isfinite(arc->endAngle());
    }
    if (const auto* circle = dynamic_cast<const SketchCircle*>(entity)) {
        return std::isfinite(circle->radius());
    }
    if (const auto* ellipse = dynamic_cast<const SketchEllipse*>(entity)) {
        return std::isfinite(ellipse->majorRadius()) && std::isfinite(ellipse->minorRadius()) &&
               std::isfinite(ellipse->rotation());
    }
    return entity->type() == EntityType::Line;
}

bool finiteOperands(const Sketch& sketch, const SketchConstraint& constraint) {
    if (!sketch.validateConstraintSupport(constraint).supported) return false;
    bool direction_required = constraint.type() == ConstraintType::Tangent ||
                              constraint.type() == ConstraintType::Distance ||
                              constraint.type() == ConstraintType::Symmetric;
    if (const auto* on = dynamic_cast<const constraints::PointOnCurveConstraint*>(&constraint)) {
        direction_required = on->position() == CurvePosition::Arbitrary;
    }
    for (const auto& id : constraint.referencedEntities()) {
        if (!finiteEntity(sketch, id)) return false;
        if (!direction_required) continue;
        const auto* line = sketch.getEntityAs<SketchLine>(id);
        if (!line) continue;
        const auto* a = sketch.getEntityAs<SketchPoint>(line->startPointId());
        const auto* b = sketch.getEntityAs<SketchPoint>(line->endPointId());
        const double length = std::hypot(b->x() - a->x(), b->y() - a->y());
        if (!std::isfinite(length) || length <= 0.0) return false;
    }
    if (const auto* dimension = dynamic_cast<const DimensionalConstraint*>(&constraint)) {
        if (!std::isfinite(dimension->value())) return false;
    }
    if (const auto* fixed = dynamic_cast<const constraints::FixedConstraint*>(&constraint)) {
        if (!std::isfinite(fixed->fixedX()) || !std::isfinite(fixed->fixedY())) return false;
    }
    return true;
}

std::optional<double> angularSpan(const Sketch& sketch, const SketchConstraint& constraint) {
    double span = 0.0;
    for (const auto& id : constraint.referencedEntities()) {
        const auto* line = sketch.getEntityAs<SketchLine>(id);
        if (!line) return std::nullopt;
        const auto* a = sketch.getEntityAs<SketchPoint>(line->startPointId());
        const auto* b = sketch.getEntityAs<SketchPoint>(line->endPointId());
        if (!a || !b) return std::nullopt;
        const double length = std::hypot(b->x() - a->x(), b->y() - a->y());
        if (!std::isfinite(length) || length <= 0.0) return std::nullopt;
        span = std::max(span, length);
    }
    return span > 0.0 ? std::optional<double>(span) : std::nullopt;
}

std::optional<double> supportLineResidual(const Sketch& sketch, const SketchConstraint& constraint) {
    const auto* on = dynamic_cast<const constraints::PointOnCurveConstraint*>(&constraint);
    if (!on || on->position() != CurvePosition::Arbitrary) return std::nullopt;
    const auto* line = sketch.getEntityAs<SketchLine>(on->curveId());
    if (!line) return std::nullopt;
    const auto* p = sketch.getEntityAs<SketchPoint>(on->pointId());
    const auto* a = sketch.getEntityAs<SketchPoint>(line->startPointId());
    const auto* b = sketch.getEntityAs<SketchPoint>(line->endPointId());
    if (!p || !a || !b) return std::numeric_limits<double>::infinity();
    const double dx = b->x() - a->x(), dy = b->y() - a->y();
    const double length = std::hypot(dx, dy);
    if (!std::isfinite(length) || length <= 0.0) return std::numeric_limits<double>::infinity();
    return std::abs((p->x() - a->x()) * (dy / length) -
                    (p->y() - a->y()) * (dx / length));
}

}  // namespace onecad::core::sketch
