#pragma once

#include "benchmark/Types.h"

namespace onecad::benchmark {

AdapterResult run_fillet(const std::string &backend,
                         const GeneratedGeometry &geometry, double radius);

} // namespace onecad::benchmark
