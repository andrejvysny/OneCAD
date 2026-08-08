#pragma once

#include <nlohmann/json_fwd.hpp>

#include "benchmark/Types.h"

namespace onecad::benchmark {

nlohmann::json execute_request(const Request &request);
nlohmann::json exception_result(const Request *request,
                                const std::string &failure_class,
                                const std::string &message);

} // namespace onecad::benchmark
