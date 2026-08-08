#include "kernel/validation/ShapeAudit.h"

#include <cmath>

#include <BOPAlgo_CheckResult.hxx>
#include <BOPAlgo_CheckStatus.hxx>
#include <BRepAlgoAPI_Check.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRep_Tool.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>

#include "session/ShapeMetrics.h"

namespace onecad::kernel::validation {

nlohmann::json ShapeTolerances::to_json() const {
  return {
      {"face", face}, {"edge", edge}, {"vertex", vertex}, {"max", maximum()}};
}

bool ShapeAuditResult::publishable() const {
  return error.empty() && !null_shape && brep_valid && solid_count == 1 &&
         std::isfinite(volume) && volume > 0.0 && self_interference_checked &&
         self_interference_count == 0;
}

nlohmann::json ShapeAuditResult::to_json() const {
  nlohmann::json out = {{"nullShape", null_shape},
                        {"brepValid", brep_valid},
                        {"solidCount", solid_count},
                        {"volume", volume},
                        {"selfInterferenceCount", self_interference_count},
                        {"selfInterferenceChecked", self_interference_checked},
                        {"tolerances", tolerances.to_json()}};
  if (!error.empty())
    out["auditError"] = error;
  return out;
}

ShapeAuditResult audit_shape(const TopoDS_Shape &shape) {
  ShapeAuditResult out;
  out.null_shape = shape.IsNull();
  if (out.null_shape)
    return out;

  try {
    out.brep_valid = BRepCheck_Analyzer(shape).IsValid();
    TopTools_IndexedMapOfShape solids;
    TopExp::MapShapes(shape, TopAbs_SOLID, solids);
    out.solid_count = solids.Extent();
    out.volume = session::shape_volume(shape);
    out.tolerances.face = BRep_Tool::MaxTolerance(shape, TopAbs_FACE);
    out.tolerances.edge = BRep_Tool::MaxTolerance(shape, TopAbs_EDGE);
    out.tolerances.vertex = BRep_Tool::MaxTolerance(shape, TopAbs_VERTEX);

    BRepAlgoAPI_Check checker;
    checker.SetData(shape, /*bTestSE=*/false, /*bTestSI=*/true);
    checker.SetRunParallel(false);
    checker.Perform();
    if (checker.HasErrors()) {
      out.error = "OCCT self-interference check failed";
      return out;
    }
    for (const BOPAlgo_CheckResult &result : checker.Result()) {
      if (result.GetCheckStatus() == BOPAlgo_SelfIntersect) {
        ++out.self_interference_count;
      }
    }
    out.self_interference_checked = true;
  } catch (const Standard_Failure &failure) {
    const char *message = failure.GetMessageString();
    out.error = message ? message : "OCCT shape audit failed";
  } catch (...) {
    out.error = "unknown shape audit failure";
  }
  return out;
}

} // namespace onecad::kernel::validation
