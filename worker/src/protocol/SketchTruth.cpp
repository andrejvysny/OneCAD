#include "SketchTruth.h"

#include <algorithm>

namespace onecad::protocol {
namespace {

using nlohmann::json;
namespace sk = core::sketch;
constexpr std::size_t kDiagnosticLimit = 64;
constexpr std::size_t kEvidenceLimit = 65536;

std::string constraint_id(const wire::WireIndex& index, const sk::ConstraintID& id) {
    const auto it = index.internal_constraint_to_wire.find(id);
    return it == index.internal_constraint_to_wire.end() ? std::string{} : it->second;
}

std::vector<std::string> constraint_ids(const wire::WireIndex& index,
                                      const std::vector<sk::ConstraintID>& ids) {
    std::vector<std::string> result;
    for (const auto& id : ids) {
        const auto mapped = constraint_id(index, id);
        if (!mapped.empty()) result.push_back(mapped);
    }
    std::sort(result.begin(), result.end());
    result.erase(std::unique(result.begin(), result.end()), result.end());
    return result;
}

json violation_json(const sk::ResidualViolation& violation, const wire::WireIndex& index) {
    return json{{"constraintId", constraint_id(index, violation.constraint)},
                {"dimension", violation.dimension == sk::ResidualDimension::Length ? "mm" : "rad"},
                {"residual", violation.residual}, {"tolerance", violation.tolerance},
                {"normalized", violation.normalized}, {"component", violation.component}};
}

json diagnostic(const std::string& reason, const std::string& message, json evidence) {
    if (evidence.dump().size() > kEvidenceLimit) {
        evidence["truncated"] = true;
        auto encoded_size = evidence.dump().size();
        for (auto& value : evidence) {
            if (!value.is_array()) continue;
            while (!value.empty() && encoded_size > kEvidenceLimit) {
                encoded_size -= value.back().dump().size() + (value.size() > 1 ? 1 : 0);
                value.erase(value.size() - 1);
            }
        }
        if (evidence.dump().size() > kEvidenceLimit) evidence = json{{"truncated", true}};
    }
    return json{{"code", "OP_FAILED"}, {"message", message}, {"severity", "error"},
                {"reasonCode", reason}, {"stage", "sketchSolve"}, {"evidence", std::move(evidence)}};
}

json entity_evidence(const sk::ExactIssue& issue, const wire::WireIndex& index) {
    json ids = json::array();
    for (const auto& id : issue.entities) {
        const auto point = index.point_to_handle.find(id);
        const auto edge = index.internal_edge_to_wire.find(id);
        if (point != index.point_to_handle.end()) ids.push_back(point->second);
        else if (edge != index.internal_edge_to_wire.end()) ids.push_back(edge->second);
    }
    return json{{"entityIds", std::move(ids)}};
}

}  // namespace

std::string exact_state_token(sk::ExactState state) {
    switch (state) {
        case sk::ExactState::UnderConstrained: return "UnderConstrained";
        case sk::ExactState::FullyConstrained: return "FullyConstrained";
        case sk::ExactState::OverConstrained: return "OverConstrained";
        case sk::ExactState::Conflicting: return "Conflicting";
    }
    return {};
}

json exact_truth_fields(const sk::ExactTruth& truth, const wire::WireIndex& index) {
    json reasons = json::object(), violations = json::array();
    for (const auto& [id, reason] : truth.reasons) {
        const auto mapped = constraint_id(index, id);
        if (!mapped.empty()) reasons[mapped] = reason == sk::ConflictReason::ResidualViolation
                                             ? "residualViolation" : "rankConflict";
    }
    for (const auto& violation : truth.residuals.violations) {
        violations.push_back(violation_json(violation, index));
    }
    return json{{"conflicting", constraint_ids(index, truth.conflicting)},
                {"reasons", std::move(reasons)},
                {"unassessed", constraint_ids(index, truth.residuals.unassessed)},
                {"violations", std::move(violations)},
                {"normalizedMaxResidual", truth.residuals.normalized_max}};
}

json exact_truth_diagnostics(const sk::ExactTruth& truth, const wire::WireIndex& index) {
    json diagnostics = json::array();
    // Incomplete truth has priority over violation detail under the carrier cap.
    for (const auto& issue : truth.issues) {
        if (diagnostics.size() == kDiagnosticLimit) break;
        auto evidence = entity_evidence(issue, index);
        if (issue.reason == "unassessedConstraint") {
            evidence["unassessed"] = constraint_ids(index, truth.residuals.unassessed);
        }
        diagnostics.push_back(diagnostic(issue.reason, issue.message, std::move(evidence)));
    }
    for (const auto& violation : truth.residuals.violations) {
        if (diagnostics.size() == kDiagnosticLimit) break;
        diagnostics.push_back(diagnostic("residualViolation", "Constraint residual exceeds acceptance budget",
                                         violation_json(violation, index)));
    }
    std::vector<sk::ConstraintID> rank;
    for (const auto& [id, reason] : truth.reasons) {
        if (reason == sk::ConflictReason::RankConflict) rank.push_back(id);
    }
    if (!rank.empty() && diagnostics.size() < kDiagnosticLimit) {
        diagnostics.push_back(diagnostic("rankConflict", "Committed constraints form an incompatible rank group",
                                         json{{"constraintIds", constraint_ids(index, rank)}}));
    }
    return diagnostics;
}

json exact_refusal(const sk::ExactTruth& truth, const wire::WireIndex& index) {
    return json{{"diagnostics", exact_truth_diagnostics(truth, index)}};
}

json translation_refusal(const wire::TranslateResult& result) {
    if (!result.invalid_geometry_entity) return json::object();
    return json{{"diagnostics", json::array({diagnostic("internalCouplingFailure", result.error,
        json{{"entityIds", json::array({*result.invalid_geometry_entity})}})})}};
}

}  // namespace onecad::protocol
