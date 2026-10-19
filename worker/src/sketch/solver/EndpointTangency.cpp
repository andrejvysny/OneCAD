#include "ConstraintSolver.h"
#include "../EndpointTangency.h"
#include "../SketchArc.h"
#include "../SketchLine.h"
#include "../SketchPoint.h"
#include "../constraints/Constraints.h"
#include <GCS.h>

namespace onecad::core::sketch {
namespace {
GCS::Point point(SketchPoint* p) {
    auto& at = p->position().ChangeCoord();
    return GCS::Point(&at.ChangeCoord(1), &at.ChangeCoord(2));
}
} // namespace
bool ConstraintSolver::translateEndpointTangency(constraints::TangentAtPointConstraint* c,
                                                 int tag) {
    GCS::Point selected;
    auto curve = [&](const EntityID& id, CurvePosition role) -> std::unique_ptr<GCS::Curve> {
        if (const auto found = linesById_.find(id); found != linesById_.end()) {
            const auto* line = found->second;
            const auto start = pointsById_.find(line->startPointId()),
                       end = pointsById_.find(line->endPointId());
            if (start == pointsById_.end() || end == pointsById_.end())
                return {};
            auto result = std::make_unique<GCS::Line>();
            result->p1 = point(start->second);
            result->p2 = point(end->second);
            selected = role == CurvePosition::Start ? result->p1 : result->p2;
            return result;
        }
        const auto found = arcsById_.find(id);
        if (found == arcsById_.end())
            return {};
        auto* arc = found->second;
        const auto center = pointsById_.find(arc->centerPointId());
        const auto start = pointsById_.find(arc->startPointId()),
                   end = pointsById_.find(arc->endPointId());
        if (center == pointsById_.end() || start == pointsById_.end() || end == pointsById_.end())
            return {};
        auto result = std::make_unique<GCS::Arc>();
        result->center = point(center->second);
        result->rad = &arc->radius();
        result->startAngle = &arc->startAngle();
        result->endAngle = &arc->endAngle();
        result->start = point(start->second);
        result->end = point(end->second);
        selected = role == CurvePosition::Start ? result->start : result->end;
        return result;
    };
    auto first = curve(c->entity1(), c->position1());
    auto weld = selected;
    auto second = curve(c->entity2(), c->position2());
    if (!first || !second)
        return false;
    const auto a = first->CalculateNormal(weld), b = second->CalculateNormal(weld);
    const auto branch = c->branch() ? c->branch() : tangentBranch(a.x, a.y, b.x, b.y);
    if (!branch)
        return false;
    c->freezeBranch(*branch);
    // PlaneGCS retains this pointer; constants stay outside the unknown parameter vector.
    tangentAngles_.push_back(*branch);
    gcsSystem_->addConstraintAngleViaPoint(*first, *second, weld, &tangentAngles_.back(), tag,
                                           true);
    return true;
}
} // namespace onecad::core::sketch
