#pragma once

#include <cstddef>
#include <string>

#include <nlohmann/json_fwd.hpp>

#include "benchmark/Types.h"

namespace onecad::benchmark {

bool parse_selector(const nlohmann::json &value, SelectorSpec &out,
                    std::string &error);
bool selector_matches_case(const SelectorSpec &selector,
                           const std::string &generator,
                           const std::string &recipe_type,
                           std::size_t feature_index, std::string &error);

/// case-v2 selector: top level rather than nested under the operation, with the
/// wider surface/curve/mode vocabulary the expanded matrix needs.
bool parse_selector_v2(const nlohmann::json &value, SelectorSpec &out,
                       std::string &error);
/// `has_feature_index` distinguishes "the recipe carries no feature index" from
/// index 0 — a v2 `supportPair` case has no feature index at all, and silently
/// treating that as 0 would let a mismatched provenance through.
bool selector_matches_case_v2(const SelectorSpec &selector,
                              const std::string &generator,
                              const std::string &recipe,
                              bool has_feature_index,
                              std::size_t feature_index, std::string &error);

} // namespace onecad::benchmark
