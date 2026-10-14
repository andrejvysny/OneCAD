#include "SketchPose.h"

#include "ResidualGeometry.h"
#include "Sketch.h"
#include "constraints/Constraints.h"

#include <algorithm>
#include <cmath>

namespace onecad::core::sketch {
namespace {

std::vector<double> curve_values(const SketchEntity& entity) {
    if (const auto* a = dynamic_cast<const SketchArc*>(&entity)) {
        return {a->radius(), a->startAngle(), a->endAngle()};
    }
    if (const auto* c = dynamic_cast<const SketchCircle*>(&entity)) return {c->radius()};
    if (const auto* e = dynamic_cast<const SketchEllipse*>(&entity)) {
        return {e->majorRadius(), e->minorRadius(), e->rotation()};
    }
    return {};
}

bool arc_coupled(const Sketch& sketch, const SketchArc& arc) {
    if (!arc.hasEndpointPoints()) return true;
    const auto* center = sketch.getEntityAs<SketchPoint>(arc.centerPointId());
    const auto* start = sketch.getEntityAs<SketchPoint>(arc.startPointId());
    const auto* end = sketch.getEntityAs<SketchPoint>(arc.endPointId());
    if (!center || !start || !end) return false;
    const auto expected_start = arc.startPoint(center->position());
    const auto expected_end = arc.endPoint(center->position());
    const double start_error = expected_start.Distance(start->position());
    const double end_error = expected_end.Distance(end->position());
    return std::isfinite(start_error) && std::isfinite(end_error) &&
           start_error <= kExactLengthToleranceMm && end_error <= kExactLengthToleranceMm;
}

void check_fixed_targets(const Sketch& sketch, std::vector<ExactIssue>& issues) {
    for (const auto& constraint : sketch.getAllConstraints()) {
        const auto* fixed = dynamic_cast<const constraints::FixedConstraint*>(constraint.get());
        if (!fixed) continue;
        const auto* point = sketch.getEntityAs<SketchPoint>(fixed->pointId());
        if (!point || point->x() != fixed->fixedX() || point->y() != fixed->fixedY()) {
            issues.push_back({"internalCouplingFailure", "Explicit Fixed target was not retained exactly",
                              {fixed->pointId()}});
        }
    }
}

}  // namespace

SketchPose::SketchPose(const Sketch& sketch) {
    for (const auto& entity : sketch.getAllEntities()) {
        if (const auto* p = dynamic_cast<const SketchPoint*>(entity.get())) {
            points[p->id()] = {p->x(), p->y()};
        }
        auto values = curve_values(*entity);
        if (!values.empty()) curves[entity->id()] = std::move(values);
        if (!entity->isReferenceLocked()) continue;
        if (!curve_values(*entity).empty()) locked_curves.insert(entity->id());
        if (const auto* a = dynamic_cast<const SketchArc*>(entity.get())) {
            locked_points.insert(a->centerPointId());
        } else {
            for (const auto& id : sketch.entityPointIds(entity->id())) locked_points.insert(id);
        }
    }
}

void SketchPose::restore(Sketch& sketch) const {
    for (const auto& [id, at] : points) {
        if (auto* p = sketch.getEntityAs<SketchPoint>(id)) p->setPosition(at[0], at[1]);
    }
    for (const auto& [id, values] : curves) {
        if (auto* a = sketch.getEntityAs<SketchArc>(id)) {
            a->radius() = values[0];
            a->startAngle() = values[1];
            a->endAngle() = values[2];
        } else if (auto* c = sketch.getEntityAs<SketchCircle>(id)) {
            c->radius() = values[0];
        } else if (auto* e = sketch.getEntityAs<SketchEllipse>(id)) {
            e->setMajorRadius(values[0]);
            e->setMinorRadius(values[1]);
            e->setRotation(values[2]);
        }
    }
}

std::vector<ExactIssue> SketchPose::domainIntegrity(const Sketch& sketch) {
    std::vector<ExactIssue> issues;
    for (const auto& entity : sketch.getAllEntities()) {
        const auto id = entity->id();
        if (!finiteEntity(sketch, id)) {
            issues.push_back({"internalCouplingFailure", "Geometry is missing or nonfinite", {id}});
            continue;
        }
        const auto values = curve_values(*entity);
        const auto radii = entity->type() == EntityType::Ellipse ? 2U : 1U;
        for (std::size_t i = 0; i < std::min(values.size(), static_cast<std::size_t>(radii)); ++i) {
            if (values[i] < 0.0) {
                issues.push_back({"internalCouplingFailure", "Curve radius is negative", {id}});
                break;
            }
        }
    }
    return issues;
}

std::vector<ExactIssue> SketchPose::projectFixedTargets(Sketch& sketch) {
    std::vector<ExactIssue> issues;
    std::map<PointID, std::array<double, 2>> targets;
    for (const auto& constraint : sketch.getAllConstraints()) {
        const auto* fixed = dynamic_cast<const constraints::FixedConstraint*>(constraint.get());
        if (!fixed) continue;
        if (!std::isfinite(fixed->fixedX()) || !std::isfinite(fixed->fixedY()) ||
            !sketch.getEntityAs<SketchPoint>(fixed->pointId())) {
            issues.push_back({"internalCouplingFailure", "Fixed target is missing or nonfinite", {fixed->pointId()}});
        } else {
            targets[fixed->pointId()] = {fixed->fixedX(), fixed->fixedY()};
        }
    }
    if (!issues.empty()) return issues;
    for (const auto& [id, at] : targets) sketch.getEntityAs<SketchPoint>(id)->setPosition(at[0], at[1]);
    return issues;
}

std::vector<ExactIssue> SketchPose::assessCandidate(Sketch& sketch, bool numerical_success) const {
    if (!numerical_success) return integrity(sketch);
    auto issues = domainIntegrity(sketch);
    if (!issues.empty()) return issues;
    issues = projectFixedTargets(sketch);
    if (!issues.empty()) return issues;
    // Numerical pins can round immutable scalars. Restore their captured values
    // before assessment; exact Fixed/coupling checks still reject contradictions.
    for (const auto& id : locked_points) {
        const auto& at = points.at(id);
        sketch.getEntityAs<SketchPoint>(id)->setPosition(at[0], at[1]);
    }
    for (const auto& id : locked_curves) {
        const auto& values = curves.at(id);
        if (auto* arc = sketch.getEntityAs<SketchArc>(id)) {
            arc->radius() = values[0];
            arc->startAngle() = values[1];
            arc->endAngle() = values[2];
        } else if (auto* circle = sketch.getEntityAs<SketchCircle>(id)) {
            circle->radius() = values[0];
        } else if (auto* ellipse = sketch.getEntityAs<SketchEllipse>(id)) {
            ellipse->setMajorRadius(values[0]);
            ellipse->setMinorRadius(values[1]);
            ellipse->setRotation(values[2]);
        }
    }
    return integrity(sketch);
}

std::vector<ExactIssue> SketchPose::integrity(const Sketch& sketch, bool check_coupling) const {
    auto issues = domainIntegrity(sketch);
    for (const auto& entity : sketch.getAllEntities()) {
        const auto id = entity->id();
        if (!finiteEntity(sketch, id)) continue;
        if (check_coupling) {
            const auto* arc = dynamic_cast<const SketchArc*>(entity.get());
            if (arc && !arc_coupled(sketch, *arc)) {
                issues.push_back({"internalCouplingFailure", "Arc endpoint coupling exceeds 0.0001 mm", {id}});
            }
        }
        if (locked_points.count(id)) {
            const auto* p = dynamic_cast<const SketchPoint*>(entity.get());
            const auto prior = points.find(id);
            if (!p || prior == points.end() || p->x() != prior->second[0] || p->y() != prior->second[1]) {
                issues.push_back({"internalCouplingFailure", "Reference-locked point moved", {id}});
            }
        }
        if (locked_curves.count(id) && curve_values(*entity) != curves.at(id)) {
            issues.push_back({"internalCouplingFailure", "Reference-locked curve parameters moved", {id}});
        }
    }
    if (check_coupling) check_fixed_targets(sketch, issues);
    return issues;
}

}  // namespace onecad::core::sketch
