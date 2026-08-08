#pragma once

#include <string>

#include "nlohmann/json.hpp"

namespace onecad::kernel::diagnostics {

struct OperationDiagnostic {
  std::string severity = "error";
  std::string code;
  std::string message;
  std::string stage;
  nlohmann::json evidence = nlohmann::json::object();

  nlohmann::json to_json() const;
};

} // namespace onecad::kernel::diagnostics
