#pragma once

#include <memory>
#include <string>
#include <vector>

#include <BRepFilletAPI_MakeFillet.hxx>
#include <TopoDS_Shape.hxx>

#include "kernel/diagnostics/OperationDiagnostic.h"
#include "kernel/fillet/BlendEvidence.h"
#include "kernel/fillet/FilletAnalyzer.h"
#include "kernel/validation/ShapeAudit.h"
#include "util/Cancel.h"

namespace onecad::kernel::fillet {

struct FilletBuildResult {
  bool ok = false;
  bool cancelled = false;
  std::string error_code = "OP_FAILED";
  std::string message;
  TopoDS_Shape shape;
  FilletAnalysis analysis;
  validation::ShapeAuditResult input_audit;
  validation::ShapeAuditResult output_audit;
  FilletResultEvidence fillet_evidence;
  std::vector<diagnostics::OperationDiagnostic> diagnostics;
};

class FilletBuilder {
public:
  using PartialResultQuery = PartialResultProbe (*)(BRepFilletAPI_MakeFillet &);

  FilletBuilder(TopoDS_Shape body, std::vector<ResolvedEdge> requested,
                double radius,
                PartialResultQuery partial_result_query =
                    &FilletAnalyzer::partial_result);

  FilletBuildResult build(const onecad::CancelToken *cancel = nullptr);
  BRepFilletAPI_MakeFillet &history();

private:
  FilletBuildResult
  fail_with_diagnostic(std::string code, std::string message,
                       std::string diagnostic_code,
                       const validation::ShapeAuditResult &output = {});
  bool configure(std::string &error);
  bool run_kernel_build(std::string &error);
  FilletBuildResult accept_result();

  TopoDS_Shape body_;
  std::vector<ResolvedEdge> requested_;
  double radius_ = 0.0;
  PartialResultQuery partial_result_query_ = nullptr;
  std::unique_ptr<BRepFilletAPI_MakeFillet> builder_;
  FilletAnalysis analysis_;
  validation::ShapeAuditResult input_audit_;
};

} // namespace onecad::kernel::fillet
