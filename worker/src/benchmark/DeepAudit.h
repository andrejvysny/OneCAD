#pragma once

#include <nlohmann/json_fwd.hpp>
#include <TopoDS_Shape.hxx>

namespace onecad::benchmark {

nlohmann::json deep_audit(const TopoDS_Shape &shape);

} // namespace onecad::benchmark
