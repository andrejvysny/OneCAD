#include "EndpointTangency.h"
#include "Sketch.h"
#include "constraints/Constraints.h"
#include <algorithm>
#include <cmath>
#include <limits>
#include <numbers>

namespace onecad::core::sketch {
std::optional<TangentEndpoint> tangentEndpoint(const Sketch& sketch, const EntityID& owner,
                                               CurvePosition role, const gp_Pnt2d* common) {
    if (role != CurvePosition::Start && role != CurvePosition::End)
        return std::nullopt;
    TangentEndpoint endpoint;
    if (const auto* line = sketch.getEntityAs<SketchLine>(owner)) {
        const auto* start = sketch.getEntityAs<SketchPoint>(line->startPointId());
        const auto* end = sketch.getEntityAs<SketchPoint>(line->endPointId());
        if (!start || !end)
            return std::nullopt;
        const auto* selected = role == CurvePosition::Start ? start : end;
        endpoint.point = selected->id();
        endpoint.at = selected->position();
        const double dx = end->x() - start->x(), dy = end->y() - start->y();
        endpoint.nx = -dy;
        endpoint.ny = dx;
        endpoint.span = std::hypot(dx, dy);
    } else if (const auto* arc = sketch.getEntityAs<SketchArc>(owner)) {
        if (!arc->hasEndpointPoints())
            return std::nullopt;
        const auto* center = sketch.getEntityAs<SketchPoint>(arc->centerPointId());
        const auto* point = sketch.getEntityAs<SketchPoint>(
            role == CurvePosition::Start ? arc->startPointId() : arc->endPointId());
        if (!center || !point)
            return std::nullopt;
        endpoint.point = point->id();
        endpoint.at = point->position();
        const auto& at = common ? *common : endpoint.at;
        endpoint.nx = center->x() - at.X();
        endpoint.ny = center->y() - at.Y();
        endpoint.span = arc->sweepAngle() > 0.0 && arc->sweepAngle() < 2 * std::numbers::pi &&
                                std::isfinite(arc->sweepAngle())
                            ? arc->radius()
                            : 0.0;
    } else
        return std::nullopt;
    return endpoint;
}

std::optional<double> tangentBranch(double ax, double ay, double bx, double by) {
    const double a = std::hypot(ax, ay), b = std::hypot(bx, by);
    if (!std::isfinite(a) || !std::isfinite(b) || a <= 0.0 || b <= 0.0)
        return std::nullopt;
    ax /= a;
    ay /= a;
    bx /= b;
    by /= b;
    const double x = ax * bx, y = ay * by, dot = x + y;
    // Refuse a root choice whose dot sign is inside floating-point arithmetic uncertainty.
    const double bound = 8 * std::numeric_limits<double>::epsilon() * (std::abs(x) + std::abs(y));
    if (std::abs(dot) <= bound)
        return std::nullopt;
    return dot > 0.0 ? 0.0 : std::numbers::pi;
}

std::string validateEndpointTangency(const Sketch& sketch,
                                     const constraints::TangentAtPointConstraint& c) {
    const auto a = tangentEndpoint(sketch, c.entity1(), c.position1());
    const auto b = tangentEndpoint(sketch, c.entity2(), c.position2(), a ? &a->at : nullptr);
    if (!a || !b || c.entity1() == c.entity2())
        return "TANGENT_ENDPOINT_UNRESOLVED";
    bool welded = a->point == b->point;
    for (const auto& constraint : sketch.getAllConstraints()) {
        const auto* weld = dynamic_cast<const constraints::CoincidentConstraint*>(constraint.get());
        if (welded || !weld)
            continue;
        welded = (weld->point1() == a->point && weld->point2() == b->point) ||
                 (weld->point2() == a->point && weld->point1() == b->point);
    }
    if (!welded)
        return "TANGENT_ENDPOINT_NOT_WELDED";
    for (const auto* endpoint : {&*a, &*b}) {
        if (!std::isfinite(endpoint->span) || endpoint->span <= 0.0 ||
            !std::isfinite(endpoint->at.X()) || !std::isfinite(endpoint->at.Y()) ||
            !std::isfinite(std::hypot(endpoint->nx, endpoint->ny)) ||
            std::hypot(endpoint->nx, endpoint->ny) <= 0.0)
            return "TANGENT_DEGENERATE_OPERAND";
    }
    if (!c.branch() && !tangentBranch(a->nx, a->ny, b->nx, b->ny))
        return "TANGENT_BRANCH_AMBIGUOUS";
    return {};
}
} // namespace onecad::core::sketch
