#include "kernel/validation/ShapeAudit.h"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <limits>
#include <vector>

#include <BOPAlgo_CheckResult.hxx>
#include <BOPAlgo_CheckStatus.hxx>
#include <BRepAlgoAPI_Check.hxx>
#include <BRepBndLib.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepGProp.hxx>
#include <BRep_Tool.hxx>
#include <Bnd_Box.hxx>
#include <GProp_GProps.hxx>
#include <NCollection_IndexedMap.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ShapeMapHasher.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Iterator.hxx>

#include "kernel/validation/GeometryPrecision.h"
#include "session/ShapeMetrics.h"

namespace onecad::kernel::validation {

namespace {

bool solid_container(TopAbs_ShapeEnum shape_type) {
  return shape_type == TopAbs_SOLID || shape_type == TopAbs_COMPSOLID ||
         shape_type == TopAbs_COMPOUND;
}

void collect_structure_evidence(const TopoDS_Shape &shape, ShapeEvidence &out) {
  out.stray_topology_count = 0;
  const auto walk = [&](const auto &self, const TopoDS_Shape &node) -> void {
    const TopAbs_ShapeEnum type = node.ShapeType();
    if (type == TopAbs_SOLID)
      return;
    if (type != TopAbs_COMPOUND && type != TopAbs_COMPSOLID) {
      ++out.stray_topology_count;
      return;
    }
    for (TopoDS_Iterator it(node); it.More(); it.Next())
      self(self, it.Value());
  };
  walk(walk, shape);
  out.structure_checked = true;
}

const char *shape_type_name(TopAbs_ShapeEnum type) {
  static constexpr const char *names[] = {"compound", "compsolid", "solid", "shell",
                                           "face",     "wire",      "edge",  "vertex",
                                           "shape"};
  const int index = static_cast<int>(type);
  return index >= 0 && index <= static_cast<int>(TopAbs_SHAPE) ? names[index] : "unknown";
}

const char *disposition_name(PublicationDisposition disposition) {
  switch (disposition) {
  case PublicationDisposition::Publishable:
    return "publishable";
  case PublicationDisposition::LifecycleOnly:
    return "lifecycleOnly";
  case PublicationDisposition::Refused:
    return "refused";
  }
  return "refused";
}

double elapsed_ms(std::chrono::steady_clock::time_point start) {
  return std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - start)
      .count();
}

void collect_manifold_evidence(const TopoDS_Shape &shape, ShapeEvidence &out) {
  NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> edges;
  NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> faces;
  TopExp::MapShapes(shape, TopAbs_EDGE, edges);
  TopExp::MapShapes(shape, TopAbs_FACE, faces);
  std::vector<int> uses(static_cast<std::size_t>(edges.Extent() + 1), 0);
  for (int i = 1; i <= faces.Extent(); ++i) {
    for (TopExp_Explorer it(faces(i), TopAbs_EDGE); it.More(); it.Next()) {
      const int index = edges.FindIndex(it.Current());
      if (index > 0)
        ++uses[static_cast<std::size_t>(index)];
    }
  }
  for (int i = 1; i <= edges.Extent(); ++i) {
    if (BRep_Tool::Degenerated(TopoDS::Edge(edges(i))))
      continue;
    out.open_edge_count += uses[static_cast<std::size_t>(i)] < 2 ? 1 : 0;
    out.non_manifold_edge_count += uses[static_cast<std::size_t>(i)] > 2 ? 1 : 0;
  }
  out.manifold_checked = true;
}

double edge_length(const TopoDS_Shape &edge) {
  GProp_GProps properties;
  BRepGProp::LinearProperties(edge, properties);
  return properties.Mass();
}

double face_area(const TopoDS_Shape &face) {
  GProp_GProps properties;
  BRepGProp::SurfaceProperties(face, properties);
  return properties.Mass();
}

// Boundary length of one face, mm. Explored rather than mapped on purpose: a
// seam edge appears TWICE in a periodic face's wire and is traversed twice, so
// `TopExp::MapShapes` (which de-duplicates) would understate a cylinder's or a
// sphere's perimeter and overstate its width.
double face_perimeter(const TopoDS_Shape &face) {
  double total = 0.0;
  for (TopExp_Explorer it(face, TopAbs_EDGE); it.More(); it.Next()) {
    if (BRep_Tool::Degenerated(TopoDS::Edge(it.Current())))
      continue;
    total += edge_length(it.Current());
  }
  return total;
}

// What counts as a micro edge or a sliver face.
//
// Both thresholds are ABSOLUTE (mm) and LOCAL — derived from the shape's own
// precision context and, for an edge, from that edge's own B-Rep tolerance.
// They are deliberately NOT ratios to the global bounding-box diagonal, which
// is the yardstick this function used to use and which is wrong twice over: it
// judges a 0.01 mm feature by the size of the 10 m part it sits on, and inside
// the supported 0.01-10000 mm range it lands below OCCT's own `Confusion`, so
// no real defect could ever trip it.
//
// Degenerate edges are counted SEPARATELY and never as micro edges. A sphere
// pole and a cone apex are degenerate edges of zero length; counting them would
// make `max_micro_edge_count = 0` refuse every sphere. `collect_manifold_evidence`
// above has always skipped them — this now agrees with it.
//
// Edges and faces are MAPPED, not explored, so a shared edge is judged once.
void collect_micro_topology(const TopoDS_Shape &shape, ShapeEvidence &out) {
  Bnd_Box bounds;
  BRepBndLib::Add(shape, bounds);
  if (bounds.IsVoid())
    return;
  double xmin, ymin, zmin, xmax, ymax, zmax;
  bounds.Get(xmin, ymin, zmin, xmax, ymax, zmax);
  out.scale_diagonal = std::hypot(xmax - xmin, std::hypot(ymax - ymin, zmax - zmin));
  if (!std::isfinite(out.scale_diagonal) || out.scale_diagonal <= 1.0e-300)
    return;

  const GeometryPrecisionContext context = precision_of(shape);
  // `micro_edge_length()` is already `kMicroFactor * floor`; dividing it back
  // out recovers the floor the per-edge tolerance competes with, so an edge with
  // an ordinary tolerance is judged by exactly `micro_edge_length()`.
  const double micro_floor = context.micro_edge_length() / kMicroFactor;
  const double sliver_width = context.sliver_face_width();

  out.minimum_edge_ratio = std::numeric_limits<double>::infinity();
  out.minimum_face_ratio = std::numeric_limits<double>::infinity();
  out.minimum_edge_length = std::numeric_limits<double>::infinity();
  out.minimum_face_width = std::numeric_limits<double>::infinity();

  TopTools_IndexedMapOfShape edges;
  TopExp::MapShapes(shape, TopAbs_EDGE, edges);
  for (int i = 1; i <= edges.Extent(); ++i) {
    const TopoDS_Edge &edge = TopoDS::Edge(edges(i));
    if (BRep_Tool::Degenerated(edge)) {
      ++out.degenerate_edge_count;
      continue;
    }
    const double length = edge_length(edge);
    out.minimum_edge_length = std::min(out.minimum_edge_length, length);
    out.minimum_edge_ratio = std::min(out.minimum_edge_ratio, length / out.scale_diagonal);
    const double limit =
        kMicroFactor * std::max(static_cast<double>(BRep_Tool::Tolerance(edge)), micro_floor);
    out.micro_edge_count += length <= limit ? 1 : 0;
  }

  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(shape, TopAbs_FACE, faces);
  for (int i = 1; i <= faces.Extent(); ++i) {
    const double area = face_area(faces(i));
    out.minimum_face_ratio =
        std::min(out.minimum_face_ratio, area / (out.scale_diagonal * out.scale_diagonal));
    const double perimeter = face_perimeter(faces(i));
    // A face with no measurable boundary yields width 0, i.e. it classifies as a
    // sliver. Fail-CLOSED on purpose: a face we cannot measure is not a face we
    // may vouch for, and the alternative (width = +inf) would let the one shape
    // the metric cannot read be the one shape it always passes.
    const double width = perimeter > 0.0 ? 2.0 * area / perimeter : 0.0;
    out.minimum_face_width = std::min(out.minimum_face_width, width);
    out.sliver_face_count += width <= sliver_width ? 1 : 0;
  }

  if (!std::isfinite(out.minimum_edge_ratio))
    out.minimum_edge_ratio = 0.0;
  if (!std::isfinite(out.minimum_face_ratio))
    out.minimum_face_ratio = 0.0;
  if (!std::isfinite(out.minimum_edge_length))
    out.minimum_edge_length = 0.0;
  if (!std::isfinite(out.minimum_face_width))
    out.minimum_face_width = 0.0;
  out.micro_topology_checked = true;
}

PublicationDecision make_decision(PublicationDisposition disposition, std::string code,
                                  std::string reason_code, std::string message,
                                  const ShapeEvidence &evidence) {
  PublicationDecision out;
  out.disposition = disposition;
  out.code = std::move(code);
  out.reason_code = std::move(reason_code);
  out.message = std::move(message);
  out.evidence = evidence;
  out.timings.validator_ms = evidence.validator_duration_ms;
  return out;
}

PublicationDecision refuse(const ShapeEvidence &evidence, std::string reason_code,
                           std::string message) {
  return make_decision(PublicationDisposition::Refused, "GEOMETRY_INVALID",
                       std::move(reason_code), std::move(message), evidence);
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
                        {"topLevelShape", shape_type_name(top_level_shape)},
                        {"brepValid", brep_valid},
                        {"solidCount", solid_count},
                        {"strayTopologyCount", stray_topology_count},
                        {"structureChecked", structure_checked},
                        {"volume", volume},
                        {"volumeChecked", volume_checked},
                        {"selfInterferenceCount", self_interference_count},
                        {"selfInterferenceChecked", self_interference_checked},
                        {"openEdgeCount", open_edge_count},
                        {"nonManifoldEdgeCount", non_manifold_edge_count},
                        {"manifoldChecked", manifold_checked},
                        {"microEdgeCount", micro_edge_count},
                        {"sliverFaceCount", sliver_face_count},
                        {"degenerateEdgeCount", degenerate_edge_count},
                        {"minimumEdgeLength", minimum_edge_length},
                        {"minimumFaceWidth", minimum_face_width},
                        {"minimumEdgeRatio", minimum_edge_ratio},
                        {"minimumFaceRatio", minimum_face_ratio},
                        {"scaleDiagonal", scale_diagonal},
                        {"microTopologyChecked", micro_topology_checked},
                        {"tolerances", tolerances.to_json()},
                        {"tolerancesChecked", tolerances_checked}};
  if (!error.empty())
    out["auditError"] = error;
  return out;
}

nlohmann::json PublicationDecision::Timings::to_json() const {
  return {{"buildMs", build_ms}, {"validatorMs", validator_ms}};
}

nlohmann::json PublicationDecision::to_json() const {
  return {{"disposition", disposition_name(disposition)},
          {"code", code},
          {"reasonCode", reason_code},
          {"message", message},
          {"evidence", evidence.to_json()},
          {"timings", timings.to_json()}};
}

PublicationPolicy single_solid_policy(std::string name, PublicationTier tier) {
  PublicationPolicy policy;
  policy.name = std::move(name);
  policy.allowed_top_level_shapes = TopLevelShapePolicy::SingleBody;
  policy.require_closed_manifold = tier == PublicationTier::TierB;
  policy.tier = tier;
  return policy;
}

PublicationDecision evaluate_publication_policy(const ShapeEvidence &evidence,
                                                 const PublicationPolicy &policy) {
  if (evidence.null_shape)
    return refuse(evidence, "PUBLICATION_NULL_SHAPE",
                  policy.name + " produced null geometry");
  if (!evidence.error.empty())
    return refuse(evidence, "PUBLICATION_AUDIT_ERROR",
                  policy.name + " shape audit failed: " + evidence.error);
  if (policy.require_brep_valid && !evidence.brep_valid)
    return refuse(evidence, "PUBLICATION_BREP_INVALID",
                  policy.name + " produced invalid geometry");
  if (policy.allowed_top_level_shapes != TopLevelShapePolicy::Any) {
    if (!evidence.structure_checked)
      return refuse(evidence, "PUBLICATION_NO_STRUCTURE_EVIDENCE",
                    policy.name + " has no structural validation evidence");
    if (!solid_container(evidence.top_level_shape))
      return refuse(evidence, "PUBLICATION_UNSUPPORTED_TOP_LEVEL",
                    policy.name + " produced unsupported top-level shape");
    if (evidence.stray_topology_count != 0)
      return refuse(evidence, "PUBLICATION_STRAY_TOPOLOGY",
                    policy.name + " produced topology outside its solid payload");
  }
  if (evidence.solid_count == 0 && policy.allow_empty_lifecycle)
    return make_decision(PublicationDisposition::LifecycleOnly, "", "", "", evidence);
  if (evidence.solid_count < policy.min_solid_count)
    return refuse(evidence, "PUBLICATION_TOO_FEW_SOLIDS",
                  policy.name + " produced too few solids");
  if (policy.max_solid_count >= 0 && evidence.solid_count > policy.max_solid_count)
    return refuse(evidence, "PUBLICATION_TOO_MANY_SOLIDS",
                  policy.name + " produced too many solids");
  if (policy.allowed_top_level_shapes == TopLevelShapePolicy::SingleBody &&
      evidence.solid_count != 1)
    return refuse(evidence, "PUBLICATION_NOT_SINGLE_SOLID",
                  policy.name + " does not contain exactly one connected solid");
  if (policy.require_positive_volume &&
      (!evidence.volume_checked || !std::isfinite(evidence.volume) || evidence.volume <= 0.0)) {
    return refuse(evidence, "PUBLICATION_NON_POSITIVE_VOLUME",
                  policy.name + " produced non-positive or unmeasured volume");
  }
  if (policy.require_finite_tolerances) {
    const double maximum = evidence.tolerances.maximum();
    if (!evidence.tolerances_checked || !std::isfinite(maximum) || maximum < 0.0)
      return refuse(evidence, "PUBLICATION_TOLERANCE_UNKNOWN",
                    policy.name + " has unknown or invalid B-Rep tolerances");
    if (policy.maximum_tolerance >= 0.0 && maximum > policy.maximum_tolerance)
      return refuse(evidence, "PUBLICATION_TOLERANCE_BUDGET",
                    policy.name + " exceeded its B-Rep tolerance budget");
  }
  if (policy.require_closed_manifold &&
      (!evidence.manifold_checked || evidence.open_edge_count != 0 ||
       evidence.non_manifold_edge_count != 0)) {
    return refuse(evidence, "PUBLICATION_OPEN_MANIFOLD",
                  policy.name + " failed closed-manifold validation");
  }
  // Micro topology. Inert unless a policy enables a bound; when one IS enabled,
  // missing evidence is a REFUSAL, not a pass — an unmeasured shape has not been
  // shown to be free of the defect the bound exists to catch.
  const bool micro_bounded =
      policy.max_micro_edge_count >= 0 || policy.minimum_edge_length >= 0.0;
  const bool sliver_bounded =
      policy.max_sliver_face_count >= 0 || policy.minimum_face_width >= 0.0;
  if (micro_bounded || sliver_bounded) {
    if (!evidence.micro_topology_checked)
      return refuse(evidence, "PUBLICATION_MICRO_TOPOLOGY_UNKNOWN",
                    policy.name + " has no micro-topology evidence");
    if (policy.max_micro_edge_count >= 0 &&
        evidence.micro_edge_count > policy.max_micro_edge_count)
      return refuse(evidence, "PUBLICATION_MICRO_EDGE",
                    policy.name + " produced more micro edges than its budget allows");
    if (policy.minimum_edge_length >= 0.0 &&
        evidence.minimum_edge_length < policy.minimum_edge_length)
      return refuse(evidence, "PUBLICATION_MICRO_EDGE",
                    policy.name + " produced an edge below its minimum length");
    if (policy.max_sliver_face_count >= 0 &&
        evidence.sliver_face_count > policy.max_sliver_face_count)
      return refuse(evidence, "PUBLICATION_SLIVER_FACE",
                    policy.name + " produced more sliver faces than its budget allows");
    if (policy.minimum_face_width >= 0.0 &&
        evidence.minimum_face_width < policy.minimum_face_width)
      return refuse(evidence, "PUBLICATION_SLIVER_FACE",
                    policy.name + " produced a face below its minimum width");
  }
  if (policy.tier == PublicationTier::TierB &&
      (!evidence.self_interference_checked || evidence.self_interference_count != 0)) {
    return refuse(evidence, "PUBLICATION_SELF_INTERFERENCE",
                  policy.name + " failed self-interference validation");
  }
  return make_decision(PublicationDisposition::Publishable, "", "", "", evidence);
}

ShapeEvidence collect_shape_evidence(const TopoDS_Shape &shape, PublicationTier tier) {
  const auto started = std::chrono::steady_clock::now();
  ShapeEvidence out;
  out.null_shape = shape.IsNull();
  if (out.null_shape) {
    out.validator_duration_ms = elapsed_ms(started);
    return out;
  }

  try {
    out.top_level_shape = shape.ShapeType();
    out.brep_valid = BRepCheck_Analyzer(shape).IsValid();
    TopTools_IndexedMapOfShape solids;
    TopExp::MapShapes(shape, TopAbs_SOLID, solids);
    out.solid_count = solids.Extent();
    collect_structure_evidence(shape, out);
    out.volume = session::shape_volume(shape);
    out.volume_checked = true;
    out.tolerances.face = BRep_Tool::MaxTolerance(shape, TopAbs_FACE);
    out.tolerances.edge = BRep_Tool::MaxTolerance(shape, TopAbs_EDGE);
    out.tolerances.vertex = BRep_Tool::MaxTolerance(shape, TopAbs_VERTEX);
    out.tolerances_checked = true;

    // EVERY tier, deliberately. The micro-topology bounds live on the POLICY,
    // not on the tier, so a Tier A policy may carry one — and if collection were
    // tier-gated, that policy would refuse everything it saw as
    // `PUBLICATION_MICRO_TOPOLOGY_UNKNOWN`. The Tier A callers are not marginal:
    // `validate_modeling_input` runs Tier A on every operation's INPUT, and
    // `result_validation_tier` downgrades every interactive preview to Tier A.
    // Affordable because this is GProp and tolerance queries only — no BOP work,
    // which is the one thing Tier A exists to skip.
    collect_micro_topology(shape, out);

    if (tier == PublicationTier::TierB) {
      collect_manifold_evidence(shape, out);
      BRepAlgoAPI_Check checker;
      checker.SetData(shape, /*bTestSE=*/false, /*bTestSI=*/true);
      checker.SetRunParallel(false);
      checker.Perform();
      if (checker.HasErrors()) {
        out.error = "OCCT self-interference check failed";
        out.validator_duration_ms = elapsed_ms(started);
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
  out.validator_duration_ms = elapsed_ms(started);
  return out;
}

} // namespace onecad::kernel::validation
