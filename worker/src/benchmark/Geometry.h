#pragma once

#include <string>

#include "benchmark/Types.h"

namespace onecad::benchmark {

bool generate_geometry(const CaseSpec &benchmark_case,
                       const VariantSpec &variant, GeneratedGeometry &out,
                       std::string &error);

} // namespace onecad::benchmark
