#include "ResidualGeometry.h"

#include "Sketch.h"
#include "constraints/Constraints.h"

#include <algorithm>
#include <cmath>
#include <unordered_set>

namespace onecad::core::sketch {
namespace {

bool welded(const Sketch& sketch, const EntityID& a, const EntityID& b) {
    std::unordered_set<EntityID> connected{a};
    bool changed = true;
    while (changed) {
        changed = false;
        for (const auto& constraint : sketch.getAllConstraints()) {
            const auto* c = dynamic_cast<const constraints::CoincidentConstraint*>(constraint.get());
            EntityID first, second;
            if (c) {
                first = c->point1();
                second = c->point2();
            } else if (const auto* on = dynamic_cast<const constraints::PointOnCurveConstraint*>(constraint.get())) {
                if (on->position() == CurvePosition::Arbitrary) continue;
                const auto* line = sketch.getEntityAs<SketchLine>(on->curveId());
                if (!line) continue;
                first = on->pointId();
                second = on->position() == CurvePosition::Start ? line->startPointId() : line->endPointId();
            } else continue;
            if (connected.count(first)) changed |= connected.insert(second).second;
            if (connected.count(second)) changed |= connected.insert(first).second;
        }
    }
    return connected.count(b) > 0;
}

struct EndpointTangent {
    EntityID point;
    double x = 0.0, y = 0.0, span = 0.0;
};

std::vector<EndpointTangent> endpoints(const Sketch& sketch, const EntityID& id) {
    if (const auto* line = sketch.getEntityAs<SketchLine>(id)) {
        const auto* a = sketch.getEntityAs<SketchPoint>(line->startPointId());
        const auto* b = sketch.getEntityAs<SketchPoint>(line->endPointId());
        if (!a || !b) return {};
        const double x = b->x() - a->x(), y = b->y() - a->y();
        const double span = std::hypot(x, y);
        return {{a->id(), x, y, span}, {b->id(), x, y, span}};
    }
    if (const auto* arc = sketch.getEntityAs<SketchArc>(id)) {
        if (!arc->hasEndpointPoints()) return {};
        return {{arc->startPointId(), -std::sin(arc->startAngle()),
                 std::cos(arc->startAngle()), arc->radius()},
                {arc->endPointId(), -std::sin(arc->endAngle()),
                 std::cos(arc->endAngle()), arc->radius()}};
    }
    return {};
}

}  // namespace

std::vector<TangentAngle> weldedTangentAngles(const Sketch& sketch,
                                           const SketchConstraint& constraint) {
    const auto* tangent = dynamic_cast<const constraints::TangentConstraint*>(&constraint);
    if (!tangent) return {};
    std::vector<TangentAngle> out;
    for (const auto& a : endpoints(sketch, tangent->entity1())) {
        for (const auto& b : endpoints(sketch, tangent->entity2())) {
            if (!welded(sketch, a.point, b.point)) continue;
            if (a.span <= 0.0 || b.span <= 0.0) {
                out.push_back({0.0, 0.0});
                continue;
            }
            // Undirected tangent alignment adds no orientation intent. For two
            // arcs, Ri*|sin(delta)| <= max(Ri)*|delta| bounds either endpoint.
            const double angle = std::atan2(std::abs(a.x * b.y - a.y * b.x),
                                            std::abs(a.x * b.x + a.y * b.y));
            out.push_back({angle, std::max(a.span, b.span)});
        }
    }
    return out;
}

}  // namespace onecad::core::sketch
