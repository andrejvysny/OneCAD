#include "benchmark/SemanticValidation.h"

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
  std::pair<int, int> remote;
  double before = 0.0;
  double after = 0.0;
};

Evidence gather(const Request &request, const GeneratedGeometry &geometry,
                const AdapterResult &adapter) {
  return {cylinder_evidence(adapter.output, request.benchmark_case.radius),
          tangency_evidence(adapter.output, request.benchmark_case.radius),
          remote_supports(geometry, adapter.output), volume(geometry.shape),
          volume(adapter.output)};
}

double tolerance_limit(const json &spec, double scale) {
  return spec.value("absolute", 0.0) +
         spec.value("relative", 0.0) * std::abs(scale);
}

json constant_radius(const json &spec, const Request &request,
                     const AdapterResult &adapter) {
  const double limit = std::max(1.0e-9, request.benchmark_case.radius * 1.0e-9);
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
                         const Request &request, const AdapterResult &adapter,
                         const Evidence &evidence) {
  double measured = adapter.assigned_radius_max_error;
  double scale = request.benchmark_case.radius;
  std::string measured_name = "maximumAssignedRadiusError";
  if (kind == "tangencyTolerance") {
    measured = evidence.tangency.maximum_error;
    measured_name = "maximumTangencyError";
  } else if (kind == "materialTolerance") {
    measured = std::abs(evidence.after - evidence.before);
    scale = evidence.before;
    measured_name = "absoluteVolumeChange";
  }
  const double limit = tolerance_limit(spec, scale);
  const bool minimum = kind == "materialTolerance";
  const bool evidence_exists = minimum || adapter.assigned_radius_count > 0 ||
                               evidence.tangency.pairs > 0;
  const bool pass = evidence_exists && (minimum ? measured > limit : measured <= limit);
  return validator(kind, spec.value("required", false), pass ? "pass" : "fail",
                   json::array({metric(measured_name, measured),
                                metric(minimum ? "minimumChange" : "allowedError", limit)}));
}

json simple_validator(const std::string &kind, const json &spec,
                      const Request &request, const AdapterResult &adapter,
                      const Evidence &evidence, const json &audit) {
  const bool required = spec.value("required", false);
  if (kind == "generatedBlendFace")
    return validator(kind, required, adapter.generated_face_count > 0 ? "pass" : "fail",
                     json::array({metric("generatedFaceCount", adapter.generated_face_count)}));
  if (kind == "cylindricalRadius") {
    const double limit = std::max(1.0e-8, request.benchmark_case.radius * 1.0e-8);
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

json validate_one(const json &spec, const Request &request,
                  const AdapterResult &adapter, const Evidence &evidence,
                  const json &audit) {
  const std::string kind = spec.value("type", "unknown");
  const bool required = spec.value("required", false);
  if (!adapter.success)
    return validator(kind, required, "notRun");
  if (kind == "constantRadius")
    return constant_radius(spec, request, adapter);
  if (kind == "radiusTolerance" || kind == "tangencyTolerance" ||
      kind == "materialTolerance")
    return threshold_validator(kind, spec, request, adapter, evidence);
  return simple_validator(kind, spec, request, adapter, evidence, audit);
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
                                  const json &audit) {
  ValidationSummary summary;
  summary.results = json::array();
  const Evidence evidence = adapter.success ? gather(request, geometry, adapter) : Evidence{};
  for (const json &spec : request.benchmark_case.validators)
    summary.results.push_back(validate_one(spec, request, adapter, evidence, audit));
  summary.required_pass = required_passes(summary.results);
  summary.publication_valid = adapter.success &&
                              audit_passes(request.benchmark_case, audit);
  return summary;
}

} // namespace onecad::benchmark
