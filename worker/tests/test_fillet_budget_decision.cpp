// SCHEMA §7.3 "Two blend classes, two budgets" as a PURE decision.
//
// `judge_contour_evidence` is the whole acceptance rule for one contour, and
// every branch of it is a refusal a user can hit. Two of those branches cannot
// be reached from primitives on demand — OCCT cannot be asked for a blend it
// fitted coarser than 1e-3 mm, nor for one whose curvature is 1 % off the
// requested radius — so the geometry tests (`test_fillet_acceptance_envelope`,
// `test_fillet_range`) can only ever cover the branches this kernel happens to
// produce. This file injects the evidence directly and covers all of them,
// including the ORDER, which is normative: the fit bound is consulted before
// the section budget, and the section budget before tangency.
#include <cmath>
#include <cstdio>
#include <string>

#include "kernel/fillet/FilletSemanticChecks.h"

namespace kf = onecad::kernel::fillet;

namespace {

int failures = 0;

void check(bool condition, const std::string &message) {
  if (condition)
    return;
  ++failures;
  std::fprintf(stderr, "FAIL: %s\n", message.c_str());
}

// The authoring resolution, restated as the literal the rule is written against
// so a change to `GeometryPrecisionContext` cannot silently move this test's
// meaning along with the production value.
constexpr double kResolution = 1.0e-3;

kf::FilletContourEvidence contour(kf::BlendSurfaceClass surface_class,
                                  double tolerance, double section_error,
                                  double tangency) {
  kf::FilletContourEvidence out;
  out.index = 1;
  out.surface_class = surface_class;
  out.approximation_tolerance = tolerance;
  out.generated_face_count = 1;
  out.support_face_count = 2;
  out.approximated_face_count =
      surface_class == kf::BlendSurfaceClass::Approximated ? 1 : 0;
  out.blend.boundaries = 2;
  out.blend.samples = 25;
  out.blend.maximum_profile_error = section_error;
  out.blend.maximum_tangency_radians = tangency;
  return out;
}

kf::ContourVerdict judge(const kf::FilletContourEvidence &evidence,
                         double radius) {
  return kf::judge_contour_evidence(evidence, radius, kResolution);
}

void exact_first_publishes_whatever_the_class() {
  // An APPROXIMATED contour that is exact to machine precision — OCCT really
  // does produce these; the kernelbench cylinder-cylinder family measures
  // 6e-16 on B-spline faces. It must publish with NO warning and report the
  // EXACT budget, or every such row's evidence moves for nothing.
  const kf::ContourVerdict verdict =
      judge(contour(kf::BlendSurfaceClass::Approximated, 1.0e-4, 6.0e-16,
                    1.0e-15),
            2.0);
  check(verdict.refusal_code.empty(),
        "exact-first: an exact B-spline blend publishes, got " +
            verdict.refusal_code);
  check(!verdict.used_adaptive_budget,
        "exact-first: an exact B-spline blend does NOT use the adaptive budget "
        "and therefore does not warn");
  check(std::abs(verdict.allowed_profile_error - 2.0e-9) <= 1.0e-12,
        "exact-first: the REPORTED budget stays the exact one (max(1e-9, "
        "r*1e-9) = 2e-9), got " +
            std::to_string(verdict.allowed_profile_error));
}

void analytic_outside_exact_is_refused_as_before() {
  const kf::ContourVerdict verdict =
      judge(contour(kf::BlendSurfaceClass::Analytic, 0.0, 1.0e-6, 0.0), 2.0);
  check(verdict.refusal_code == "FILLET_SEMANTIC_CHECK_FAILED",
        "an ANALYTIC contour outside the exact budget keeps its own refusal, "
        "got \"" + verdict.refusal_code + "\"");
  check(!verdict.used_adaptive_budget,
        "an analytic contour never reaches the adaptive budget");
}

void approximated_inside_the_adaptive_budget_publishes() {
  // The shipped case-1 numbers: r = 2, face tolerance 5.06e-5, residual
  // 1.97e-3, G1 1.24e-5.
  const kf::ContourVerdict verdict =
      judge(contour(kf::BlendSurfaceClass::Approximated, 5.0611e-5, 1.9710e-3,
                    1.2367e-5),
            2.0);
  check(verdict.refusal_code.empty(),
        "the measured case-1 blend publishes, got " + verdict.refusal_code);
  check(verdict.used_adaptive_budget,
        "the measured case-1 blend needed the adaptive budget, so the step warns");
  check(std::abs(verdict.allowed_profile_error - 0.02) <= 1.0e-12,
        "profileBudget = min(r * kRelSection, kSectionCeiling) = 0.02, got " +
            std::to_string(verdict.allowed_profile_error));
  check(std::abs(verdict.allowed_tangency_radians - 4.0 * 5.0611e-5 / 2.0) <=
            1.0e-12,
        "tangencyBudget = kTangencyFactor * tol / r, got " +
            std::to_string(verdict.allowed_tangency_radians));
}

void fit_coarser_than_the_resolution_is_too_coarse() {
  // Rule 1, and it is FIRST: this evidence would also pass the section budget
  // (1.5e-3 < 0.02), so reaching TOO_COARSE proves the fit bound was consulted
  // before it.
  const kf::ContourVerdict verdict =
      judge(contour(kf::BlendSurfaceClass::Approximated, 1.1e-3, 1.5e-3, 0.0),
            2.0);
  check(verdict.refusal_code == "FILLET_BLEND_TOO_COARSE",
        "tol > res refuses FILLET_BLEND_TOO_COARSE, got \"" +
            verdict.refusal_code + "\"");
}

void section_beyond_one_percent_is_too_coarse() {
  // Rule 2: r = 2 gives a 0.02 mm budget; 0.03 is a 1.5 % curvature error.
  const kf::ContourVerdict verdict =
      judge(contour(kf::BlendSurfaceClass::Approximated, 1.0e-4, 0.03, 0.0),
            2.0);
  check(verdict.refusal_code == "FILLET_BLEND_TOO_COARSE",
        "a section residual above min(r/100, 0.05) refuses "
        "FILLET_BLEND_TOO_COARSE, got \"" + verdict.refusal_code + "\"");
  // The ceiling caps the budget for a large radius: r = 100 would give 1 mm by
  // the ratio alone, but kSectionCeiling holds it at 0.05.
  const kf::ContourVerdict capped =
      judge(contour(kf::BlendSurfaceClass::Approximated, 1.0e-4, 0.06, 0.0),
            100.0);
  check(capped.refusal_code == "FILLET_BLEND_TOO_COARSE",
        "kSectionCeiling caps the budget at 0.05 mm however large the radius, "
        "got \"" + capped.refusal_code + "\"");
  check(std::abs(capped.allowed_profile_error - 0.05) <= 1.0e-12,
        "the capped budget is reported as 0.05, got " +
            std::to_string(capped.allowed_profile_error));
}

void tangency_beyond_its_budget_is_a_semantic_failure() {
  // Rule 3, and it is LAST: the section residual here is inside 0.02, so only
  // the G1 error can be the cause — and it keeps the SEMANTIC code, not
  // TOO_COARSE, because a blend that is the right radius but not tangent is a
  // different defect from one that is the wrong radius.
  const kf::ContourVerdict verdict =
      judge(contour(kf::BlendSurfaceClass::Approximated, 1.0e-4, 1.0e-3, 1.0e-2),
            2.0);
  check(verdict.refusal_code == "FILLET_SEMANTIC_CHECK_FAILED",
        "a G1 error above the tangency budget refuses "
        "FILLET_SEMANTIC_CHECK_FAILED, got \"" + verdict.refusal_code + "\"");
}

} // namespace

int main() {
  exact_first_publishes_whatever_the_class();
  analytic_outside_exact_is_refused_as_before();
  approximated_inside_the_adaptive_budget_publishes();
  fit_coarser_than_the_resolution_is_too_coarse();
  section_beyond_one_percent_is_too_coarse();
  tangency_beyond_its_budget_is_a_semantic_failure();
  if (failures > 0)
    std::fprintf(stderr, "%d assertion(s) failed\n", failures);
  return failures == 0 ? 0 : 1;
}
