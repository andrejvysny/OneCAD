#include "benchmark/Execution.h"

#include <chrono>
#include <cmath>

#include <BRepAdaptor_Surface.hxx>
#include <BRepClass3d_SolidClassifier.hxx>
#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <nlohmann/json.hpp>

#include "benchmark/Artifacts.h"
#include "benchmark/DeepAudit.h"
#include "benchmark/FilletRun.h"
#include "benchmark/Geometry.h"
#include "benchmark/SemanticValidation.h"
#include "io/BrepCodec.h"
#include "util/Hashing.h"

namespace onecad::benchmark {
namespace {

using Clock = std::chrono::steady_clock;
using json = nlohmann::json;

std::string verdict(const CaseSpec &benchmark_case, const AdapterResult &adapter,
                    bool required_pass, bool publication_valid) {
  const bool valid_success = adapter.success && publication_valid && required_pass;
  if (benchmark_case.expected_domain == "supported")
    return valid_success ? "pass" : "fail";
  if (benchmark_case.expected_domain == "expectedLimit")
    return adapter.success ? (valid_success ? "pass" : "fail") :
                             (adapter.operation_state == "refused" ? "pass" : "fail");
  if (adapter.operation_state == "exception" || adapter.partial ||
      (adapter.success && !valid_success))
    return "fail";
  if (benchmark_case.expected_domain == "exploratory" && valid_success)
    return "pass";
  return "characterization";
}

std::string input_digest(const Request &request,
                         const GeneratedGeometry &geometry) {
  std::vector<std::uint8_t> bytes;
  const std::string error = onecad::io::write_brep_compound({geometry.shape}, bytes);
  std::string payload = request.benchmark_case.canonical.dump();
  payload += request.canonical["variant"].dump();
  if (error.empty())
    payload.append(reinterpret_cast<const char *>(bytes.data()), bytes.size());
  return onecad::hashing::sha256_hex(payload);
}

std::string exception_input_digest(const Request *request) {
  if (!request)
    return onecad::hashing::sha256_hex(std::string());
  std::string payload = request->benchmark_case.canonical.dump();
  payload += request->canonical["variant"].dump();
  return onecad::hashing::sha256_hex(payload);
}

void quantize(json &value) {
  if (value.is_number_float()) {
    value = std::round(value.get<double>() * 1.0e9) / 1.0e9;
  } else if (value.is_array()) {
    for (json &item : value)
      quantize(item);
  } else if (value.is_object()) {
    for (auto it = value.begin(); it != value.end(); ++it)
      quantize(it.value());
  }
}

std::string normalized_digest(const json &result) {
  json normalized = {{"executionState", result["executionState"]},
                     {"operationState", result["operationState"]},
                     {"failureClass", result["failureClass"]},
                     {"inputAudit", result["inputAudit"]},
                     {"outputAudit", result["outputAudit"]},
                     {"validators", result["validators"]},
                     {"selectionEvidence", result["selectionEvidence"]},
                     {"shapeSignature", result["shapeSignature"]}};
  quantize(normalized);
  return onecad::hashing::sha256_hex(normalized.dump());
}

json identity(const std::string &name, const std::string &version,
              const std::string &build_id) {
  return {{"name", name}, {"version", version}, {"buildId", build_id}};
}

// The RESULT schema is frozen at v1, so the generator block is emitted as its
// v1 subset rather than echoed from the case. A case-v2 generator additionally
// names an operation `family`, and passing that through would make every v2
// result fail the supervisor's strict result validation.
json generator_identity(const GeneratorSpec &generator) {
  return {{"name", generator.name},
          {"version", generator.version},
          {"seed", generator.seed}};
}

json base_result(const Request &request) {
  return {{"schemaVersion", 1}, {"caseId", request.benchmark_case.case_id},
          {"generator", generator_identity(request.benchmark_case.generator)},
          {"backend", request.backend},
          {"kernel", identity("OCCT", ONECAD_OCCT_VERSION, ONECAD_OCCT_BUILD_ID)},
          {"modeler", identity(request.backend == "onecad" ? "OneCAD" : "raw-occt",
                               request.backend == "onecad" ? "FilletBuilder-v1" : "direct-v1",
                               ONECAD_OCCT_BUILD_ID)},
          {"expectedDomain", request.benchmark_case.expected_domain},
          {"metamorph", nullptr}, {"search", nullptr}, {"shapeSignature", nullptr},
          {"resource", {{"peakRssBytes", nullptr}, {"exitCode", nullptr},
                         {"signal", nullptr}, {"stderrTruncated", false}}}};
}

std::string operation_state(const AdapterResult &adapter,
                            bool publication_valid) {
  if (adapter.success && publication_valid)
    return "success";
  if (adapter.success)
    return "badShape";
  if (adapter.partial)
    return "partial";
  if (adapter.operation_state == "invalid-result")
    return "badShape";
  return "rejected";
}

std::string failure_class(const AdapterResult &adapter, bool required_pass,
                          bool publication_valid) {
  if (adapter.success && !publication_valid)
    return "publicationInvalid";
  if (adapter.success)
    return required_pass ? "none" : "semanticFailed";
  if (adapter.partial)
    return "partialResult";
  if (adapter.operation_state == "exception")
    return "exception";
  if (adapter.operation_state == "invalid-result")
    return "badShape";
  return "kernelRejected";
}

std::string surface_kind_name(GeomAbs_SurfaceType type) {
  switch (type) {
  case GeomAbs_Plane:
    return "plane";
  case GeomAbs_Cylinder:
    return "cylinder";
  case GeomAbs_Cone:
    return "cone";
  default:
    return "other";
  }
}

json face_samples(const TopoDS_Shape &shape) {
  json out = json::array();
  for (TopExp_Explorer it(shape, TopAbs_FACE); it.More() && out.size() < 256; it.Next()) {
    const TopoDS_Face face = TopoDS::Face(it.Current());
    GProp_GProps props;
    BRepGProp::SurfaceProperties(face, props);
    const gp_Pnt centroid = props.CentreOfMass();
    const BRepAdaptor_Surface surface(face);
    out.push_back({{"point", {centroid.X(), centroid.Y(), centroid.Z()}},
                   {"surfaceKind", surface_kind_name(surface.GetType())}});
  }
  return out;
}

std::string classification_name(TopAbs_State state) {
  switch (state) {
  case TopAbs_IN:
    return "in";
  case TopAbs_OUT:
    return "out";
  case TopAbs_ON:
    return "on";
  default:
    return "unknown";
  }
}

json point_classifications(const TopoDS_Shape &shape,
                           const std::vector<gp_Pnt> &points) {
  json out = json::array();
  for (const gp_Pnt &point : points) {
    BRepClass3d_SolidClassifier classifier(shape, point, 1.0e-6);
    out.push_back({{"point", {point.X(), point.Y(), point.Z()}},
                   {"state", classification_name(classifier.State())}});
  }
  return out;
}

json shape_signature(const GeneratedGeometry &geometry,
                     const AdapterResult &adapter) {
  if (!adapter.success || adapter.output.IsNull())
    return nullptr;
  return {{"rotationCenter", geometry.rotation_center},
          {"faceSamples", face_samples(adapter.output)},
          {"pointClassifications",
           point_classifications(adapter.output, geometry.probe_points)}};
}

json diagnostics(const AdapterResult &adapter) {
  json out = json::array();
  if (!adapter.message.empty())
    out.push_back({{"code", "operation-detail"}, {"phase", "operation"},
                   {"message", adapter.message.substr(0, 4096)}});
  if (!adapter.diagnostics.empty()) {
    const std::string evidence = adapter.diagnostics.dump();
    out.push_back({{"code", "kernel-evidence"}, {"phase", "operation"},
                   {"message", evidence.substr(0, 4096)}});
  }
  return out;
}

} // namespace

json execute_request(const Request &request) {
  const auto start = Clock::now();
  json result = base_result(request);
  GeneratedGeometry geometry;
  std::string error;
  if (!generate_geometry(request.benchmark_case, request.variant, geometry, error))
    return exception_result(&request, "internal", error);
  double effective_radius = request.benchmark_case.radius;
  if (request.variant.name == "parameterEpsilon") {
    effective_radius *= (1.0 + request.variant.epsilon_relative_delta);
  } else if (request.variant.name == "scaled") {
    effective_radius *= request.variant.scale_factor;
  }
  const auto audit_start = Clock::now();
  const json input_audit = deep_audit(geometry.shape);
  const auto operation_start = Clock::now();
  const AdapterResult adapter = run_fillet(request.backend, geometry, effective_radius);
  const auto operation_end = Clock::now();
  const json output_audit = adapter.success ? deep_audit(adapter.output) : json(nullptr);
  const auto audit_end = Clock::now();
  const ValidationSummary validation = validate_output(
      request, geometry, adapter, output_audit.is_null() ? json::object() : output_audit);
  result.update({{"verdict", verdict(request.benchmark_case, adapter,
                                      validation.required_pass, validation.publication_valid)},
                 {"executionState", "completed"},
                 {"operationState", operation_state(adapter, validation.publication_valid)},
                 {"failureClass", failure_class(adapter, validation.required_pass,
                                                  validation.publication_valid)},
                 {"diagnostics", diagnostics(adapter)},
                 {"inputAudit", input_audit}, {"outputAudit", output_audit},
                 {"validators", validation.results},
                 {"selectionEvidence", geometry.selection_evidence},
                 {"shapeSignature", shape_signature(geometry, adapter)},
                 {"timing", {{"wallMs", std::chrono::duration<double, std::milli>(Clock::now() - start).count()},
                              {"operationMs", std::chrono::duration<double, std::milli>(operation_end - operation_start).count()},
                              {"auditMs", std::chrono::duration<double, std::milli>((operation_start - audit_start) + (audit_end - operation_end)).count()}}},
                 {"artifacts", write_artifacts(request, geometry, adapter)},
                 {"inputDigest", input_digest(request, geometry)}});
  result["normalizedDigest"] = normalized_digest(result);
  return result;
}

json exception_result(const Request *request, const std::string &failure_class,
                      const std::string &message) {
  json result = request ? base_result(*request) : json{{"schemaVersion", 1}};
  result.update({{"verdict", "fail"}, {"executionState", "exception"},
                 {"operationState", "notRun"}, {"failureClass", failure_class},
                 {"diagnostics", json::array({{{"code", "runner-exception"},
                                                {"phase", "operation"},
                                                {"message", message.substr(0, 4096)}}})},
                 {"inputAudit", nullptr}, {"outputAudit", nullptr},
                 {"validators", json::array()}, {"selectionEvidence", json::array()},
                 {"timing", {{"wallMs", 0.0}, {"operationMs", 0.0}, {"auditMs", 0.0}}},
                 {"resource", {{"peakRssBytes", nullptr}, {"exitCode", nullptr},
                                {"signal", nullptr}, {"stderrTruncated", false}}},
                 {"metamorph", nullptr}, {"search", nullptr}, {"shapeSignature", nullptr},
                 {"artifacts", {{"canonicalCase", nullptr}, {"request", nullptr},
                                 {"stderr", nullptr}, {"inputShape", nullptr},
                                 {"outputShape", nullptr}}},
                 {"inputDigest", exception_input_digest(request)}});
  result["normalizedDigest"] = normalized_digest(result);
  return result;
}

} // namespace onecad::benchmark
