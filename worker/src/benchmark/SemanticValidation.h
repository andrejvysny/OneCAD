#pragma once

#include <nlohmann/json_fwd.hpp>

#include "benchmark/Types.h"

namespace onecad::benchmark {

struct ValidationSummary {
  nlohmann::json results;
  bool required_pass = false;
  bool publication_valid = false;
};

/// `effective_radius` is the radius the operation actually ran with, which is
/// NOT the case's declared radius for the `scaled` and `parameterEpsilon`
/// metamorphs (see `Execution.cpp`). Validating a blend against the declared
/// value there measures the wrong shape and reports a false red.
ValidationSummary validate_output(const Request &request,
                                  const GeneratedGeometry &geometry,
                                  const AdapterResult &adapter,
                                  const nlohmann::json &audit,
                                  const nlohmann::json &input_audit,
                                  double effective_radius);

} // namespace onecad::benchmark
