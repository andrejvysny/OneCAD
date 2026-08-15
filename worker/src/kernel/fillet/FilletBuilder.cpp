#include "kernel/fillet/FilletBuilder.h"

#include <algorithm>
#include <utility>

#include <Standard_Failure.hxx>

#include "kernel/fillet/FilletSemanticChecks.h"
#include "kernel/validation/GeometryPrecision.h"

namespace onecad::kernel::fillet {

namespace {

nlohmann::json audit_evidence(double radius,
                              const validation::ShapeAuditResult &input,
                              const validation::ShapeAuditResult &output) {
  return {{"metrics",
           {{"requestedRadius", radius},
            {"inputMaxTolerance", input.tolerances.maximum()},
            {"outputMaxTolerance", output.tolerances.maximum()},
            {"solidCount", output.solid_count},
            {"volume", output.volume},
            {"selfInterferenceCount", output.self_interference_count}}},
          {"inputAudit", input.to_json()},
          {"outputAudit", output.to_json()}};
}

nlohmann::json blend_evidence_json(const FilletSemanticResult &semantics) {
  const BlendEvidence &blend = semantics.evidence.blend;
  return {{"generatedFaceCount", semantics.evidence.generated_face_count},
          {"supportFaceCount", semantics.evidence.support_face_count},
          {"boundaryCount", blend.boundaries},
          {"sampleCount", blend.samples},
          {"maximumTangencyRadians", blend.maximum_tangency_radians},
          {"allowedTangencyRadians", semantics.allowed_tangency_radians},
          {"maximumSectionRadiusError", blend.maximum_profile_error},
          {"allowedSectionRadiusError", semantics.allowed_profile_error},
          {"minimumSectionRadius", blend.minimum_section_radius},
          {"maximumSectionRadius", blend.maximum_section_radius},
          {"coordinateMagnitude", blend.coordinate_magnitude}};
}

} // namespace

FilletBuilder::FilletBuilder(TopoDS_Shape body,
                             std::vector<ResolvedEdge> requested, double radius,
                             PartialResultQuery partial_result_query)
    : body_(std::move(body)), requested_(std::move(requested)), radius_(radius),
      partial_result_query_(partial_result_query) {}

BRepFilletAPI_MakeFillet &FilletBuilder::history() { return *builder_; }

FilletBuildResult FilletBuilder::fail_with_diagnostic(
    std::string code, std::string message, std::string diagnostic_code,
    const validation::ShapeAuditResult &output) {
  FilletBuildResult result;
  result.error_code = std::move(code);
  result.message = std::move(message);
  result.analysis = analysis_;
  result.input_audit = input_audit_;
  result.output_audit = output;
  diagnostics::OperationDiagnostic diagnostic;
  diagnostic.code = std::move(diagnostic_code);
  diagnostic.message = result.message;
  diagnostic.stage = "build";
  diagnostic.evidence = audit_evidence(radius_, input_audit_, output);
  result.diagnostics.push_back(std::move(diagnostic));
  return result;
}

bool FilletBuilder::configure(std::string &error) {
  try {
    builder_ = std::make_unique<BRepFilletAPI_MakeFillet>(body_);
    for (const ResolvedEdge &requested : requested_) {
      int contour = builder_->Contour(requested.edge);
      if (contour == 0) {
        builder_->Add(radius_, requested.edge);
        contour = builder_->Contour(requested.edge);
      }
      if (contour == 0) {
        error = "OCCT did not accept a requested fillet edge";
        return false;
      }
    }
    analysis_ = FilletAnalyzer::analyze(*builder_, body_, requested_);
    if (!analysis_.ok) {
      error = analysis_.message;
      return false;
    }
    const FilletSemanticResult assignment =
        validate_assignment(*builder_, analysis_, radius_);
    if (!assignment.ok) {
      error = assignment.message;
      return false;
    }
    return true;
  } catch (const Standard_Failure &failure) {
    const char *message = failure.GetMessageString();
    error = message ? message : "OCCT fillet setup failed";
  } catch (...) {
    error = "unknown OCCT fillet setup failure";
  }
  return false;
}

bool FilletBuilder::run_kernel_build(std::string &error) {
  try {
    builder_->Build();
  } catch (const Standard_Failure &failure) {
    const char *message = failure.GetMessageString();
    error = message ? message : "OCCT fillet build failed";
    return false;
  } catch (...) {
    error = "unknown OCCT fillet build failure";
    return false;
  }
  try {
    return builder_->IsDone();
  } catch (...) {
    error = "OCCT fillet completion query failed";
    return false;
  }
}

FilletBuildResult FilletBuilder::accept_result() {
  TopoDS_Shape shape;
  try {
    shape = builder_->Shape();
  } catch (...) {
    return fail_with_diagnostic("GEOMETRY_INVALID",
                                "Fillet result was unavailable",
                                "FILLET_INVALID_RESULT");
  }
  const validation::ShapeAuditResult output_audit =
      validation::audit_shape(shape);
  validation::PublicationPolicy output_policy =
      validation::single_solid_policy("Fillet", validation::PublicationTier::TierB);
  // Grows from the INPUT body's tolerance. `input_audit_.tolerances.maximum()` and
  // `precision_of(body_).input_tolerance` are the same three-way max over
  // face/edge/vertex; the audit's is reused so the input is measured once.
  output_policy.maximum_tolerance = validation::precision_of(body_).tolerance_ceiling(
      input_audit_.tolerances.maximum(), 2.0, 1.0e-6);
  const validation::PublicationDecision decision =
      validation::evaluate_publication_policy(output_audit, output_policy);
  if (!decision.publishable()) {
    return fail_with_diagnostic("GEOMETRY_INVALID",
                                "Fillet result failed shape audit",
                                "FILLET_INVALID_RESULT", output_audit);
  }
  const FilletSemanticResult semantics = validate_result(
      *builder_, analysis_, body_, shape, requested_, radius_);
  if (!semantics.ok) {
    FilletBuildResult failure = fail_with_diagnostic(
        "GEOMETRY_INVALID", semantics.message,
        "FILLET_SEMANTIC_CHECK_FAILED", output_audit);
    if (!failure.diagnostics.empty())
      failure.diagnostics.front().evidence["blendEvidence"] =
          blend_evidence_json(semantics);
    failure.fillet_evidence = semantics.evidence;
    return failure;
  }

  FilletBuildResult result;
  result.ok = true;
  result.shape = std::move(shape);
  result.analysis = analysis_;
  result.input_audit = input_audit_;
  result.output_audit = output_audit;
  result.fillet_evidence = semantics.evidence;
  return result;
}

FilletBuildResult FilletBuilder::build(const onecad::CancelToken *cancel) {
  if (!valid_constant_radius(radius_)) {
    return fail_with_diagnostic(
        "OP_FAILED", "Fillet radius must be finite and at least 0.001 mm",
        "FILLET_RADIUS_INVALID");
  }
  input_audit_ = validation::audit_shape(body_);
  const validation::PublicationDecision input_decision =
      validation::evaluate_publication_policy(
          input_audit_,
          validation::single_solid_policy("Fillet input", validation::PublicationTier::TierB));
  if (!input_decision.publishable()) {
    return fail_with_diagnostic("GEOMETRY_INVALID",
                                "Fillet input body failed shape audit",
                                "FILLET_INPUT_INVALID");
  }

  std::string error;
  if (!configure(error)) {
    return fail_with_diagnostic("OP_FAILED", error, "FILLET_CONTOUR_INVALID");
  }
  if (cancel && cancel->cancelled()) {
    FilletBuildResult result;
    result.cancelled = true;
    result.error_code = "CANCELLED";
    return result;
  }

  error = "Fillet operation failed";
  const bool done = run_kernel_build(error);
  const PartialResultProbe partial = partial_result_query_
                                         ? partial_result_query_(*builder_)
                                         : PartialResultProbe{};
  if (!partial.query_ok) {
    return fail_with_diagnostic("OP_FAILED", "Fillet result-state query failed",
                                "FILLET_STATE_QUERY_FAILED");
  }
  if (done && !partial.available)
    return accept_result();

  const validation::ShapeAuditResult bad_audit =
      validation::audit_shape(partial.bad_shape);
  FilletBuildResult result;
  result.error_code = "OP_FAILED";
  result.message =
      partial.available ? "Fillet produced only a partial result" : error;
  result.analysis = analysis_;
  result.input_audit = input_audit_;
  result.output_audit = bad_audit;
  result.diagnostics = FilletAnalyzer::failure_diagnostics(
      *builder_, requested_, analysis_, radius_, input_audit_, bad_audit,
      partial.available, result.message);
  return result;
}

} // namespace onecad::kernel::fillet
