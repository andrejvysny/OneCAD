#include "Constraints.h"
#include "../EndpointTangency.h"
#include "../Sketch.h"
#include <cmath>
#include <limits>

namespace onecad::core::sketch::constraints {
TangentAtPointConstraint::TangentAtPointConstraint(const EntityID& first, const EntityID& second,
                                                   CurvePosition first_role,
                                                   CurvePosition second_role)
    : m_entity1(first), m_entity2(second), m_position1(first_role), m_position2(second_role) {}
std::vector<EntityID> TangentAtPointConstraint::referencedEntities() const {
    return {m_entity1, m_entity2};
}
bool TangentAtPointConstraint::isSatisfied(const Sketch& sketch, double tolerance) const {
    return getError(sketch) <= tolerance;
}
double TangentAtPointConstraint::getError(const Sketch& sketch) const {
    if (!validateEndpointTangency(sketch, *this).empty())
        return std::numeric_limits<double>::infinity();
    const auto a = tangentEndpoint(sketch, m_entity1, m_position1);
    const auto b = tangentEndpoint(sketch, m_entity2, m_position2, &a->at);
    const auto branch = m_branch ? m_branch : tangentBranch(a->nx, a->ny, b->nx, b->ny);
    if (!branch)
        return std::numeric_limits<double>::infinity();
    const double angle = std::atan2(b->ny, b->nx) - std::atan2(a->ny, a->nx) - *branch;
    return std::abs(std::atan2(std::sin(angle), std::cos(angle)));
}
gp_Pnt2d TangentAtPointConstraint::getIconPosition(const Sketch& sketch) const {
    const auto endpoint = tangentEndpoint(sketch, m_entity1, m_position1);
    return endpoint ? endpoint->at : gp_Pnt2d{};
}
} // namespace onecad::core::sketch::constraints
