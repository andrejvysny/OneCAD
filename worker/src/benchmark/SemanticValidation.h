#pragma once

#include <nlohmann/json_fwd.hpp>

#include "benchmark/Types.h"

namespace onecad::benchmark {

struct ValidationSummary {
  nlohmann::json results;
  bool required_pass = false;
  bool publication_valid = false;
};

ValidationSummary validate_output(const Request &request,
                                  const GeneratedGeometry &geometry,
                                  const AdapterResult &adapter,
                                  const nlohmann::json &audit);

} // namespace onecad::benchmark
