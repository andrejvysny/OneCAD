#pragma once

#include <string>

#include "nlohmann/json.hpp"
#include "ops/OpTypes.h"

namespace onecad::ops {

OpOutcome execute_loft(OpContext& ctx, const nlohmann::json& op, const std::string& op_id);

}  // namespace onecad::ops
