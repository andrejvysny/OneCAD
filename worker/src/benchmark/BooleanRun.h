#pragma once

#include "benchmark/Types.h"

namespace onecad::benchmark {

AdapterResult run_boolean(const std::string &backend,
                          const GeneratedGeometry &geometry,
                          const std::string &mode);

} // namespace onecad::benchmark
