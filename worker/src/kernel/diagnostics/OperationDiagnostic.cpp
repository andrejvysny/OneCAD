#include "kernel/diagnostics/OperationDiagnostic.h"

namespace onecad::kernel::diagnostics {

nlohmann::json OperationDiagnostic::to_json() const {
  nlohmann::json out = {
      {"severity", severity}, {"code", code}, {"message", message}};
  if (!stage.empty())
    out["stage"] = stage;
  if (evidence.is_object() && !evidence.empty() &&
      evidence.dump().size() <= 65'536) {
    out["evidence"] = evidence;
  }
  return out;
}

} // namespace onecad::kernel::diagnostics
