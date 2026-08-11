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

namespace {

bool solid_like(TopAbs_ShapeEnum shape_type) {
  return shape_type == TopAbs_SOLID || shape_type == TopAbs_COMPSOLID ||
         shape_type == TopAbs_COMPOUND;
}

PublicationDecision refuse(std::string message) {
  return {PublicationDisposition::Refused, "GEOMETRY_INVALID", std::move(message)};
}

} // namespace

nlohmann::json ShapeTolerances::to_json() const {
  return {
      {"face", face}, {"edge", edge}, {"vertex", vertex}, {"max", maximum()}};
}

bool ShapeEvidence::publishable() const {
  return evaluate_publication_policy(
             *this, single_solid_policy("legacy-shape-audit", PublicationTier::TierB))
      .publishable();
}

nlohmann::json ShapeEvidence::to_json() const {
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

PublicationPolicy single_solid_policy(std::string name, PublicationTier tier) {
  PublicationPolicy policy;
  policy.name = std::move(name);
  policy.allowed_top_level_shapes = TopLevelShapePolicy::SolidLike;
  policy.tier = tier;
  return policy;
}

PublicationDecision evaluate_publication_policy(const ShapeEvidence &evidence,
                                                 const PublicationPolicy &policy) {
  if (evidence.null_shape)
    return refuse(policy.name + " produced null geometry");
  if (!evidence.error.empty())
    return refuse(policy.name + " shape audit failed: " + evidence.error);
  if (policy.require_brep_valid && !evidence.brep_valid)
    return refuse(policy.name + " produced invalid geometry");
  if (policy.allowed_top_level_shapes == TopLevelShapePolicy::SolidLike &&
      !solid_like(evidence.top_level_shape)) {
    return refuse(policy.name + " produced unsupported top-level shape");
  }
  if (evidence.solid_count == 0 && policy.allow_empty_lifecycle)
    return {PublicationDisposition::LifecycleOnly, "", ""};
  if (evidence.solid_count < policy.min_solid_count)
    return refuse(policy.name + " produced too few solids");
  if (policy.max_solid_count >= 0 && evidence.solid_count > policy.max_solid_count)
    return refuse(policy.name + " produced too many solids");
  if (policy.require_positive_volume &&
      (!std::isfinite(evidence.volume) || evidence.volume <= 0.0)) {
    return refuse(policy.name + " produced non-positive volume");
  }
  if (policy.tier == PublicationTier::TierB &&
      (!evidence.self_interference_checked || evidence.self_interference_count != 0)) {
    return refuse(policy.name + " failed self-interference validation");
  }
  return {PublicationDisposition::Publishable, "", ""};
}

ShapeEvidence collect_shape_evidence(const TopoDS_Shape &shape, PublicationTier tier) {
  ShapeEvidence out;
  out.null_shape = shape.IsNull();
  if (out.null_shape)
    return out;

  try {
    out.top_level_shape = shape.ShapeType();
    out.brep_valid = BRepCheck_Analyzer(shape).IsValid();
    TopTools_IndexedMapOfShape solids;
    TopExp::MapShapes(shape, TopAbs_SOLID, solids);
    out.solid_count = solids.Extent();
    out.volume = session::shape_volume(shape);
    out.tolerances.face = BRep_Tool::MaxTolerance(shape, TopAbs_FACE);
    out.tolerances.edge = BRep_Tool::MaxTolerance(shape, TopAbs_EDGE);
    out.tolerances.vertex = BRep_Tool::MaxTolerance(shape, TopAbs_VERTEX);

    if (tier == PublicationTier::TierB) {
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
    }
  } catch (const Standard_Failure &failure) {
    const char *message = failure.GetMessageString();
    out.error = message ? message : "OCCT shape audit failed";
  } catch (...) {
    out.error = "unknown shape audit failure";
  }
  return out;
}

} // namespace onecad::kernel::validation
