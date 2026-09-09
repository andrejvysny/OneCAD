#pragma once

#include <string>
#include <vector>

#include <BRepFilletAPI_MakeFillet.hxx>
#include <TopoDS_Shape.hxx>

#include "kernel/fillet/BlendEvidence.h"
#include "kernel/fillet/FilletAnalyzer.h"

namespace onecad::kernel::fillet {

struct FilletSemanticResult {
  bool ok = false;
  std::string message;
  int generated_face_count = 0;
  FilletResultEvidence evidence;
  double allowed_profile_error = 0.0;
  double allowed_tangency_radians = 0.0;
  // The op-specific reason code for a refusal (SCHEMA §7.3). A section budget
  // breach on an APPROXIMATED blend is `FILLET_BLEND_TOO_COARSE`; every other
  // refusal keeps the default. Meaningless when `ok`.
  std::string refusal_code = "FILLET_SEMANTIC_CHECK_FAILED";
  // Contours that needed the ADAPTIVE budget: approximated AND outside the exact
  // budgets. An approximated contour that met the exact budgets is not counted
  // and does not make the step warn (SCHEMA §7.3 "exact first").
  int approximated_contour_count = 0;
  // At least one contour needed the adaptive budget, so the step carries the
  // `FILLET_BLEND_APPROXIMATED` warning. Meaningless unless `ok`.
  bool adaptive_budget_used = false;
  // EVIDENCE, never a refusal: the largest edge count at any contour END vertex.
  // A valence-5 rib corner rounds at residual 0 on this kernel.
  int max_contour_vertex_valence = 0;
};

bool valid_constant_radius(double radius);

FilletSemanticResult validate_assignment(BRepFilletAPI_MakeFillet &builder,
                                         const FilletAnalysis &analysis,
                                         double requested_radius);

FilletSemanticResult validate_result(
    BRepFilletAPI_MakeFillet &builder, const FilletAnalysis &analysis,
    const TopoDS_Shape &input, const TopoDS_Shape &output,
    const std::vector<ResolvedEdge> &requested, double requested_radius);

// The PER-CONTOUR half of `validate_result`, reached only when the whole result
// failed the exact budgets. Exposed so a unit test can drive the budget decision
// without building geometry that lands on each branch.
FilletSemanticResult judge_contours(BRepFilletAPI_MakeFillet &builder,
                                    const FilletAnalysis &analysis,
                                    const TopoDS_Shape &input,
                                    const TopoDS_Shape &output,
                                    double requested_radius,
                                    FilletSemanticResult out);

// The adaptive budget decision as a PURE function of the measured evidence
// (SCHEMA §7.3 rules 1-3, exact-first applied by the caller). Returns the
// refusal code, or an empty string when the contour publishes.
struct ContourVerdict {
  std::string refusal_code;  // "" ⇒ publishes
  std::string message;
  double allowed_profile_error = 0.0;
  double allowed_tangency_radians = 0.0;
  bool used_adaptive_budget = false;
};

ContourVerdict judge_contour_evidence(const FilletContourEvidence &contour,
                                      double requested_radius,
                                      double authoring_resolution);

} // namespace onecad::kernel::fillet
