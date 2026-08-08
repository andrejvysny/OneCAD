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

} // namespace onecad::benchmark
