#pragma once

#include <nlohmann/json_fwd.hpp>

#include "benchmark/Types.h"

namespace onecad::benchmark {

nlohmann::json write_artifacts(const Request &request,
                               const GeneratedGeometry &geometry,
                               const AdapterResult &adapter);

} // namespace onecad::benchmark
