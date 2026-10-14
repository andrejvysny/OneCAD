#pragma once

#include "ExactTruth.h"

#include <array>
#include <map>
#include <set>

namespace onecad::core::sketch {

class Sketch;

// Complete parameter snapshot: exact refusal restores endpoints and curve
// scalars together, including the representational arc endpoint points.
struct SketchPose {
    std::map<EntityID, std::array<double, 2>> points;
    std::map<EntityID, std::vector<double>> curves;
    std::set<EntityID> locked_points;
    std::set<EntityID> locked_curves;

    explicit SketchPose(const Sketch& sketch);
    void restore(Sketch& sketch) const;
    static std::vector<ExactIssue> domainIntegrity(const Sketch& sketch);
    static std::vector<ExactIssue> projectFixedTargets(Sketch& sketch);
    std::vector<ExactIssue> assessCandidate(Sketch& sketch, bool numerical_success) const;
    std::vector<ExactIssue> integrity(const Sketch& sketch, bool check_coupling = true) const;
};

}  // namespace onecad::core::sketch
