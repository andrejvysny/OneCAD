#pragma once

#include <string>

#include <nlohmann/json_fwd.hpp>

#include "benchmark/Types.h"

namespace onecad::benchmark {

bool parse_request(const nlohmann::json &input, Request &out,
                   std::string &error);

/// case-v2 (`bench/robustness/schemas/case-v2.schema.json`). v1 is frozen and
/// parsed by its own path; the two formats are never readable as each other.
bool parse_case_v2(const nlohmann::json &value, CaseSpec &out,
                   std::string &error);

} // namespace onecad::benchmark
