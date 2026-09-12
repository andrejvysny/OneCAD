#pragma once

#include <optional>
#include <cstddef>
#include <string>

#include "nlohmann/json_fwd.hpp"
#include "ops/OpTypes.h"

namespace onecad::session {

enum class FeaturePatternExecutionClass {
    HostModifier,
    IndependentBody,
    SharedHost,
};

std::optional<ops::OpOutcome> validate_feature_pattern_sources(
    const nlohmann::json& op, nlohmann::json& params, int& count,
    FeaturePatternExecutionClass& execution_class, std::size_t& creator_index,
    std::string& host_body_id);

}
