#include "benchmark/SemanticValidation.h"

#include "benchmark/BlendEvidence.h"

#include <algorithm>
#include <cmath>

#include <BRepAdaptor_Surface.hxx>
#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <NCollection_IndexedDataMap.hxx>
#include <NCollection_List.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_ShapeMapHasher.hxx>
#include <TopoDS.hxx>
#include <nlohmann/json.hpp>

namespace onecad::benchmark {
namespace {

using EdgeFaces = NCollection_IndexedDataMap<
    TopoDS_Shape, NCollection_List<TopoDS_Shape>, TopTools_ShapeMapHasher>;
using json = nlohmann::json;

/// Defaults for the two recipe-agnostic blend validators, which carry no
/// per-case bounds of their own — a case that needs its own numbers declares
/// `tangencyTolerance` / `crossSectionTolerance` instead. Both are measured, not
/// assumed: over `fillet/matrix:m1`, both backends, the worst boundary angle is
/// 1.4e-14 rad and the worst section-radius deviation is 2.7e-14 mm (8e-15
/// relative). See `bench/robustness/README.md`.
constexpr double kTangencyRadians = 1.0e-9;
constexpr double kSectionRelative = 1.0e-9;

/// Conditioning term, in multiples of the coordinate magnitude. Double precision
/// is relative, so rebuilding the same model a million millimetres from the
/// origin costs about six orders of magnitude of absolute accuracy — and the
/// `farOriginTranslation` metamorph exists to provoke exactly that. Measured at
/// 1.7e6 mm: 2.7e-9 mm of section error and 7.5e-9 rad of boundary angle, both
/// several times inside this allowance. Without the term the probe would read
/// arithmetic as a defect.
constexpr double kConditioning = 1.0e-14;

/// Section-radius allowance: the feature's own scale, floored, plus whatever the
/// model's distance from the origin costs.
double section_limit(double radius, double magnitude) {
  return std::max(1.0e-9, radius * kSectionRelative) + magnitude * kConditioning;
}

/// An angle error is a position error over the feature size, so the same
/// conditioning term divides by the radius here.
double tangency_limit(double radius, double magnitude) {
  return kTangencyRadians +
         magnitude * kConditioning / std::max(radius, 1.0e-9);
}

double volume(const TopoDS_Shape &shape) {
  if (shape.IsNull())
    return 0.0;
  GProp_GProps props;
  BRepGProp::VolumeProperties(shape, props);
  return props.Mass();
}

double area(const TopoDS_Face &face) {
  GProp_GProps props;
  BRepGProp::SurfaceProperties(face, props);
  return props.Mass();
}

json metric(const std::string &name, double value,
            const std::string &unit = {}) {
  json out = {{"name", name}, {"value", value}};
  if (!unit.empty())
    out["unit"] = unit;
  return out;
}

json validator(const std::string &kind, bool required,
               const std::string &status, json metrics = json::array()) {
  return {{"kind", kind}, {"required", required}, {"status", status},
          {"metrics", std::move(metrics)}, {"diagnostics", json::array()}};
}

struct RadiusEvidence {
  int cylinders = 0;
  double maximum_error = 0.0;
};

RadiusEvidence cylinder_evidence(const TopoDS_Shape &shape, double radius) {
  RadiusEvidence out;
  for (TopExp_Explorer it(shape, TopAbs_FACE); it.More(); it.Next()) {
    const BRepAdaptor_Surface surface(TopoDS::Face(it.Current()));
    if (surface.GetType() != GeomAbs_Cylinder)
      continue;
    ++out.cylinders;
    out.maximum_error = std::max(
        out.maximum_error, std::abs(surface.Cylinder().Radius() - radius));
  }
  return out;
}

struct TangencyEvidence {
  int pairs = 0;
  double maximum_error = 0.0;
};

TangencyEvidence tangency_evidence(const TopoDS_Shape &shape, double radius) {
  TangencyEvidence out;
  EdgeFaces edge_faces;
  TopExp::MapShapesAndAncestors(shape, TopAbs_EDGE, TopAbs_FACE, edge_faces);
  for (int i = 1; i <= edge_faces.Extent(); ++i) {
    const auto &faces = edge_faces.FindFromIndex(i);
    if (faces.Extent() != 2)
      continue;
    BRepAdaptor_Surface a(TopoDS::Face(faces.First()));
    BRepAdaptor_Surface b(TopoDS::Face(faces.Last()));
    const bool ac = a.GetType() == GeomAbs_Cylinder;
    const bool bc = b.GetType() == GeomAbs_Cylinder;
    if (ac == bc || (ac ? b.GetType() : a.GetType()) != GeomAbs_Plane)
      continue;
    const auto cylinder = ac ? a.Cylinder() : b.Cylinder();
    const auto plane = ac ? b.Plane() : a.Plane();
    const double parallel = std::abs(cylinder.Axis().Direction().Dot(
        plane.Axis().Direction()));
    if (parallel > 1.0e-7)
      continue;
    const gp_Vec offset(plane.Location(), cylinder.Location());
    const double radial = std::abs(std::abs(offset.Dot(
        gp_Vec(plane.Axis().Direction()))) - radius);
    out.maximum_error = std::max(out.maximum_error, std::max(parallel, radial));
    ++out.pairs;
  }
  return out;
}

struct PlaneDescriptor {
  gp_Pnt location;
  gp_Dir normal;
  double face_area = 0.0;
};

std::vector<TopoDS_Vertex> selected_vertices(const GeneratedGeometry &geometry) {
  std::vector<TopoDS_Vertex> out;
  for (const TopoDS_Edge &edge : geometry.selected_edges) {
    for (TopExp_Explorer it(edge, TopAbs_VERTEX); it.More(); it.Next())
      out.push_back(TopoDS::Vertex(it.Current()));
  }
  return out;
}

bool touches_selected(const TopoDS_Face &face,
                      const std::vector<TopoDS_Vertex> &selected) {
  for (TopExp_Explorer it(face, TopAbs_VERTEX); it.More(); it.Next()) {
    if (std::any_of(selected.begin(), selected.end(), [&](const TopoDS_Vertex &vertex) {
          return vertex.IsSame(it.Current());
        }))
      return true;
  }
  return false;
}

std::vector<PlaneDescriptor> remote_planes(const GeneratedGeometry &geometry) {
  const auto vertices = selected_vertices(geometry);
  std::vector<PlaneDescriptor> out;
  for (TopExp_Explorer it(geometry.shape, TopAbs_FACE); it.More(); it.Next()) {
    const TopoDS_Face face = TopoDS::Face(it.Current());
    BRepAdaptor_Surface surface(face);
    if (!touches_selected(face, vertices) && surface.GetType() == GeomAbs_Plane)
      out.push_back({surface.Plane().Location(), surface.Plane().Axis().Direction(),
                     area(face)});
  }
  return out;
}

bool matching_plane(const PlaneDescriptor &input, const TopoDS_Shape &output) {
  for (TopExp_Explorer it(output, TopAbs_FACE); it.More(); it.Next()) {
    const TopoDS_Face face = TopoDS::Face(it.Current());
    BRepAdaptor_Surface surface(face);
    if (surface.GetType() != GeomAbs_Plane)
      continue;
    const auto plane = surface.Plane();
    const double angle = std::abs(input.normal.Dot(plane.Axis().Direction()));
    const double distance = std::abs(gp_Vec(input.location, plane.Location()).Dot(
        gp_Vec(input.normal)));
    if (angle >= 1.0 - 1.0e-9 && distance <= 1.0e-7 &&
        std::abs(input.face_area - area(face)) <=
            std::max(1.0e-7, input.face_area * 1.0e-9))
      return true;
  }
  return false;
}

std::pair<int, int> remote_supports(const GeneratedGeometry &geometry,
                                    const TopoDS_Shape &output) {
  const auto remote = remote_planes(geometry);
  int matched = 0;
  for (const PlaneDescriptor &plane : remote)
    matched += matching_plane(plane, output) ? 1 : 0;
  return {static_cast<int>(remote.size()), matched};
}

bool quality_passes(const CaseSpec &benchmark_case, const json &audit) {
  const json &quality = benchmark_case.limits["quality"];
  const json &tolerances = audit["tolerances"];
  const std::vector<std::pair<const char *, const char *>> mappings = {
      {"maxVertexTolerance", "vertex"}, {"maxEdgeTolerance", "edge"},
      {"maxFaceTolerance", "face"}};
  for (const auto &[limit, topology] : mappings) {
    if (quality.contains(limit) &&
        tolerances[topology]["maximum"].get<double>() > quality[limit].get<double>())
      return false;
  }
  const json &micro = audit["microTopology"];
  if (quality.contains("maxMicroEdges") &&
      micro["microEdgeCount"] > quality["maxMicroEdges"])
    return false;
  return !quality.contains("maxSliverFaces") ||
         micro["sliverFaceCount"] <= quality["maxSliverFaces"];
}

bool audit_passes(const CaseSpec &benchmark_case, const json &audit) {
  return audit.is_object() && audit.value("productionAudit", "fail") == "pass" &&
         audit.value("exactValid", false) &&
         audit.value("selfInterferenceFree", false) &&
         audit.value("closedManifold", false) && quality_passes(benchmark_case, audit);
}

struct Evidence {
  RadiusEvidence radius;
  TangencyEvidence tangency;
  BlendEvidence blend;
  std::pair<int, int> remote;
  double before = 0.0;
  double after = 0.0;
};

Evidence gather(const GeneratedGeometry &geometry, const AdapterResult &adapter,
                double effective_radius) {
  return {cylinder_evidence(adapter.output, effective_radius),
          tangency_evidence(adapter.output, effective_radius),
          blend_evidence(adapter.output, adapter.blend_faces, adapter.support_faces,
                         effective_radius),
          remote_supports(geometry, adapter.output), volume(geometry.shape),
          volume(adapter.output)};
}

/// Largest tolerance carried by any vertex, edge or face in an audit.
double maximum_tolerance(const json &audit) {
  double out = 0.0;
  const json &tolerances = audit.value("tolerances", json::object());
  for (const char *topology : {"vertex", "edge", "face"}) {
    if (tolerances.contains(topology))
      out = std::max(out, tolerances[topology].value("maximum", 0.0));
  }
  return out;
}

double tolerance_limit(const json &spec, double scale) {
  return spec.value("absolute", 0.0) +
         spec.value("relative", 0.0) * std::abs(scale);
}

json constant_radius(const json &spec, const AdapterResult &adapter,
                     double effective_radius) {
  const double limit = std::max(1.0e-9, effective_radius * 1.0e-9);
  const bool pass = adapter.contour_count > 0 &&
                    adapter.assigned_radius_count == adapter.contour_count &&
                    adapter.assigned_radius_max_error <= limit;
  return validator("constantRadius", spec.value("required", false),
                   pass ? "pass" : "fail",
                   json::array({metric("assignedContourCount", adapter.assigned_radius_count),
                                metric("maximumAssignedRadiusError",
                                       adapter.assigned_radius_max_error, "mm")}));
}

json threshold_validator(const std::string &kind, const json &spec,
                         const AdapterResult &adapter,
                         const Evidence &evidence, double effective_radius) {
  double measured = adapter.assigned_radius_max_error;
  double scale = effective_radius;
  std::string measured_name = "maximumAssignedRadiusError";
  if (kind == "tangencyTolerance") {
    // The generic boundary measurement, not the plane/cylinder-only one: a
    // tolerance the case declares must mean the same thing on every support.
    measured = evidence.blend.maximum_tangency_radians;
    measured_name = "maximumTangencyRadians";
  } else if (kind == "crossSectionTolerance") {
    measured = evidence.blend.maximum_profile_error;
    measured_name = "maximumSectionRadiusError";
  } else if (kind == "materialTolerance") {
    measured = std::abs(evidence.after - evidence.before);
    scale = evidence.before;
    measured_name = "absoluteVolumeChange";
  }
  const double limit = tolerance_limit(spec, scale);
  const bool minimum = kind == "materialTolerance";
  bool evidence_exists = minimum || adapter.assigned_radius_count > 0;
  if (kind == "tangencyTolerance")
    evidence_exists = evidence.blend.boundaries > 0;
  else if (kind == "crossSectionTolerance")
    evidence_exists = evidence.blend.samples > 0;
  const bool pass = evidence_exists && (minimum ? measured > limit : measured <= limit);
  return validator(kind, spec.value("required", false), pass ? "pass" : "fail",
                   json::array({metric(measured_name, measured),
                                metric(minimum ? "minimumChange" : "allowedError", limit)}));
}

json simple_validator(const std::string &kind, const json &spec,
                      const Request &request, const AdapterResult &adapter,
                      const Evidence &evidence, const json &audit,
                      double effective_radius) {
  const bool required = spec.value("required", false);
  if (kind == "generatedBlendFace")
    return validator(kind, required, adapter.generated_face_count > 0 ? "pass" : "fail",
                     json::array({metric("generatedFaceCount", adapter.generated_face_count)}));
  if (kind == "cylindricalRadius") {
    const double limit = std::max(1.0e-8, effective_radius * 1.0e-8);
    return validator(kind, required,
                     evidence.radius.cylinders > 0 &&
                             evidence.radius.maximum_error <= limit ? "pass" : "fail",
                     json::array({metric("cylinderCount", evidence.radius.cylinders),
                                  metric("maximumRadiusError", evidence.radius.maximum_error)}));
  }
  if (kind == "g1BoundaryTangency")
    return validator(kind, required,
                     evidence.tangency.pairs >= 2 &&
                             evidence.tangency.maximum_error <= 1.0e-7 ? "pass" : "fail");
  if (kind == "materialChange") {
    const bool decrease = spec.value("direction", "decrease") == "decrease";
    const bool pass = decrease ? evidence.after < evidence.before :
                                 evidence.after > evidence.before;
    return validator(kind, required, pass ? "pass" : "fail");
  }
  if (kind == "remoteSupportsUnchanged")
    return validator(kind, required,
                     evidence.remote.first == evidence.remote.second ? "pass" : "fail");
  if (kind == "deepAudit")
    return validator(kind, required,
                     audit_passes(request.benchmark_case, audit) ? "pass" : "fail");
  return validator(kind, required, kind == "metamorphicEquivalence" ? "notRun" :
                                                                  "notApplicable");
}

/// `supportTangency` and `crossSectionProfile` are the recipe-agnostic pair.
/// They read the builder's OWN generated faces rather than picking the blend out
/// of the output by surface type, so a cylindrical or conical support is not a
/// special case. Absent evidence reports `notApplicable`, which fails a required
/// check — silence must never read as a pass.
json blend_validator(const std::string &kind, const json &spec,
                     const Evidence &evidence, double effective_radius) {
  const bool required = spec.value("required", false);
  if (kind == "supportTangency") {
    if (evidence.blend.boundaries < 2)
      return validator(kind, required, "notApplicable",
                       json::array({metric("boundaryCount", evidence.blend.boundaries)}));
    const double allowed = tangency_limit(effective_radius,
                                          evidence.blend.coordinate_magnitude);
    const bool pass = evidence.blend.maximum_tangency_radians <= allowed;
    return validator(kind, required, pass ? "pass" : "fail",
                     json::array({metric("boundaryCount", evidence.blend.boundaries),
                                  metric("maximumTangencyRadians",
                                         evidence.blend.maximum_tangency_radians, "rad"),
                                  metric("allowedTangencyRadians", allowed, "rad"),
                                  metric("coordinateMagnitude",
                                         evidence.blend.coordinate_magnitude, "mm")}));
  }
  if (evidence.blend.samples == 0)
    return validator(kind, required, "notApplicable",
                     json::array({metric("sampleCount", 0)}));
  const double limit =
      section_limit(effective_radius, evidence.blend.coordinate_magnitude);
  const bool pass = evidence.blend.maximum_profile_error <= limit;
  return validator(kind, required, pass ? "pass" : "fail",
                   json::array({metric("sampleCount", evidence.blend.samples),
                                metric("maximumSectionRadiusError",
                                       evidence.blend.maximum_profile_error, "mm"),
                                metric("minimumSectionRadius",
                                       evidence.blend.minimum_section_radius, "mm"),
                                metric("maximumSectionRadius",
                                       evidence.blend.maximum_section_radius, "mm"),
                                metric("allowedError", limit, "mm")}));
}

/// The itemized halves of `deepAudit`. Keeping them separate is what lets a case
/// require manifoldness without also requiring the whole production audit, and
/// what gives a failing campaign a named cause instead of one aggregate bit.
json audit_validator(const std::string &kind, const json &spec,
                     const CaseSpec &benchmark_case, const json &audit,
                     const json &input_audit) {
  const bool required = spec.value("required", false);
  if (kind == "manifold")
    return validator(kind, required, audit.value("closedManifold", false) ? "pass" : "fail",
                     json::array({metric("faceCount", audit["counts"].value("faces", 0))}));
  if (kind == "noSelfIntersection")
    return validator(kind, required,
                     audit.value("selfInterferenceFree", false) ? "pass" : "fail");
  if (kind == "microTopology") {
    const json &micro = audit.value("microTopology", json::object());
    const json &quality = benchmark_case.limits.value("quality", json::object());
    const int micro_edges = micro.value("microEdgeCount", 0);
    const int slivers = micro.value("sliverFaceCount", 0);
    const bool pass = micro_edges <= quality.value("maxMicroEdges", 0) &&
                      slivers <= quality.value("maxSliverFaces", 0);
    return validator(kind, required, pass ? "pass" : "fail",
                     json::array({metric("microEdgeCount", micro_edges),
                                  metric("sliverFaceCount", slivers),
                                  metric("minimumEdgeRatio",
                                         micro.value("minimumEdgeRatio", 0.0)),
                                  metric("minimumFaceRatio",
                                         micro.value("minimumFaceRatio", 0.0))}));
  }
  // toleranceGrowth. Evidence first: input/output maxima and their ratio are
  // always reported. Gating uses the ceilings the CASE declares — what counts as
  // acceptable is a property of the case, not of the validator — so a case that
  // declares none reports `notApplicable`, which fails when it marked this
  // required. `deepAudit` gates the same ceilings as one aggregate bit; this
  // names which topology grew and by how much.
  const double before = maximum_tolerance(input_audit);
  const double after = maximum_tolerance(audit);
  const json &quality = benchmark_case.limits.value("quality", json::object());
  json metrics = json::array({metric("inputMaximumTolerance", before, "mm"),
                              metric("outputMaximumTolerance", after, "mm"),
                              metric("growthRatio", before > 0.0 ? after / before : 0.0)});
  const std::vector<std::pair<const char *, const char *>> ceilings = {
      {"maxVertexTolerance", "vertex"}, {"maxEdgeTolerance", "edge"},
      {"maxFaceTolerance", "face"}};
  bool declared = false;
  bool pass = true;
  for (const auto &[limit_key, topology] : ceilings) {
    if (!quality.contains(limit_key))
      continue;
    declared = true;
    const double limit = quality.value(limit_key, 0.0);
    const double measured = audit["tolerances"][topology].value("maximum", 0.0);
    metrics.push_back(metric(std::string(topology) + "Maximum", measured, "mm"));
    pass = pass && measured <= limit;
  }
  return validator(kind, required,
                   !declared ? "notApplicable" : (pass ? "pass" : "fail"), metrics);
}

json validate_one(const json &spec, const Request &request,
                  const AdapterResult &adapter, const Evidence &evidence,
                  const json &audit, const json &input_audit,
                  double effective_radius) {
  const std::string kind = spec.value("type", "unknown");
  const bool required = spec.value("required", false);
  if (!adapter.success)
    return validator(kind, required, "notRun");
  if (kind == "constantRadius")
    return constant_radius(spec, adapter, effective_radius);
  if (kind == "radiusTolerance" || kind == "tangencyTolerance" ||
      kind == "materialTolerance" || kind == "crossSectionTolerance")
    return threshold_validator(kind, spec, adapter, evidence, effective_radius);
  if (kind == "supportTangency" || kind == "crossSectionProfile")
    return blend_validator(kind, spec, evidence, effective_radius);
  if (kind == "manifold" || kind == "noSelfIntersection" ||
      kind == "microTopology" || kind == "toleranceGrowth")
    return audit_validator(kind, spec, request.benchmark_case, audit, input_audit);
  return simple_validator(kind, spec, request, adapter, evidence, audit,
                          effective_radius);
}

bool required_passes(const json &validators) {
  for (const json &item : validators) {
    if (item.value("required", false) && item.value("status", "fail") != "pass")
      return false;
  }
  return true;
}

} // namespace

ValidationSummary validate_output(const Request &request,
                                  const GeneratedGeometry &geometry,
                                  const AdapterResult &adapter,
                                  const json &audit, const json &input_audit,
                                  double effective_radius) {
  ValidationSummary summary;
  summary.results = json::array();
  const Evidence evidence =
      adapter.success ? gather(geometry, adapter, effective_radius) : Evidence{};
  for (const json &spec : request.benchmark_case.validators)
    summary.results.push_back(validate_one(spec, request, adapter, evidence, audit,
                                           input_audit, effective_radius));
  summary.required_pass = required_passes(summary.results);
  summary.publication_valid = adapter.success &&
                              audit_passes(request.benchmark_case, audit);
  return summary;
}

} // namespace onecad::benchmark
