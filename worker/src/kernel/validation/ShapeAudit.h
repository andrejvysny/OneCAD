#pragma once

#include <algorithm>
#include <string>

#include <TopoDS_Shape.hxx>

#include "nlohmann/json.hpp"

namespace onecad::kernel::validation {

struct ShapeTolerances {
  double face = 0.0;
  double edge = 0.0;
  double vertex = 0.0;

  double maximum() const { return std::max({face, edge, vertex}); }
  nlohmann::json to_json() const;
};

struct ShapeAuditResult {
  bool null_shape = true;
  bool brep_valid = false;
  int solid_count = 0;
  double volume = 0.0;
  int self_interference_count = 0;
  bool self_interference_checked = false;
  ShapeTolerances tolerances;
  std::string error;

  bool publishable() const;
  nlohmann::json to_json() const;
};

ShapeAuditResult audit_shape(const TopoDS_Shape &shape);

} // namespace onecad::kernel::validation
