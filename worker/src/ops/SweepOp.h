#pragma once

#include <string>

#include "ops/OpTypes.h"

namespace onecad::ops {

// Bounded NewBody sweep: circular region along one closed analytic sketch curve.
OpOutcome execute_sweep(OpContext& ctx, const nlohmann::json& op, const std::string& op_id);

}  // namespace onecad::ops
