#pragma once

#include <string>

#include <nlohmann/json_fwd.hpp>

#include "benchmark/Types.h"

namespace onecad::benchmark {

bool parse_request(const nlohmann::json &input, Request &out,
                   std::string &error);

} // namespace onecad::benchmark
