#pragma once

#include <vector>

#include <BRepFilletAPI_MakeFillet.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>

#include "kernel/fillet/FilletAnalyzer.h"

namespace onecad::kernel::fillet {

// ── Fillet acceptance constants (SCHEMA §7.3 "Two blend classes, two budgets")
//
// The EXACT budgets, for an ANALYTIC (KPart) blend. OCCT builds those from a
// closed form, so the only error is arithmetic and the allowance is arithmetic
// too: `max(1e-9, |r|·kSectionRelative) + coordinateMagnitude·kConditioning`
// for the section radius, `kTangencyRadians` (+ conditioning) for G1. These
// three moved here from this header's .cpp so the exact and the adaptive
// constants sit in one place; their VALUES are unchanged.
inline constexpr double kSectionRelative = 1.0e-9;
inline constexpr double kTangencyRadians = 1.0e-9;
inline constexpr double kConditioning = 1.0e-14;

// The ADAPTIVE budgets, for an APPROXIMATED (walked, B-spline) blend, and
// NORMATIVE constants of SCHEMA §7.3 — changing one is a §14 entry with a new
// measurement.
//
// MEASURED 2026-09-06 on OCCT 8.0.1 (probe `test_fillet_acceptance_envelope`):
// the two approximated blends carry `BRep_Tool` face tolerances 5.06e-5 and
// 1.16e-5 mm (1.0e-4 and 6.34e-5 with their edges/vertices) while their section
// residuals are 1.97e-3 and 2.87e-3 mm — 39x and 247x the tolerance. That ratio
// is not a defect and not boundable by a constant: the residual is
// CURVATURE-derived (`|1/kmax - r|`, a second derivative of the fitted surface)
// and scales as `tol / h²`, so no positional multiple of `tol` describes it.
// The two quantities therefore get the two budgets that fit what they ARE — a
// positional bound on the fit, and a radius-relative bound on the section:
//   1. `tol > authoring_resolution()` ⇒ FILLET_BLEND_TOO_COARSE (the fit itself
//      is coarser than the smallest change OneCAD claims to perform).
//   2. section residual > `min(|r| · kRelSection, kSectionCeiling)` ⇒
//      FILLET_BLEND_TOO_COARSE (measured margins 10x and 3.5x; a 0.1-0.3 %
//      curvature wobble is invisible to printing and machining, 1 % is not).
//   3. G1 error > `min(max(1e-9, kTangencyFactor · tol / |r|), 1e-2 rad)` ⇒
//      FILLET_SEMANTIC_CHECK_FAILED (measured G1 errors are 0.49 and 0.42 of
//      `tol/|r|`, so 4 carries an 8x margin).
inline constexpr double kRelSection = 0.01;
inline constexpr double kSectionCeiling = 0.05;
inline constexpr double kTangencyFactor = 4.0;

// The largest G1 error the adaptive tangency budget may ever allow, radians.
inline constexpr double kTangencyBudgetCeiling = 1.0e-2;

// How OCCT built the blend. `Approximated` is not a defect — it is the walked
// path OCCT takes whenever no KPart closed form applies (a cylinder/cylinder
// tee, an oblique elliptical rim) — but it IS a different accuracy claim, which
// is why it gets different budgets and a `FILLET_BLEND_APPROXIMATED` warning.
enum class BlendSurfaceClass { Analytic, Approximated };

const char *blend_surface_class_name(BlendSurfaceClass value);

// Recipe-agnostic measurements of produced blend geometry. Zero samples or zero
// boundaries mean UNKNOWN, never success.
struct BlendEvidence {
  int boundaries = 0;
  double maximum_tangency_radians = 0.0;
  int samples = 0;
  double maximum_profile_error = 0.0;
  double minimum_section_radius = 0.0;
  double maximum_section_radius = 0.0;
  double coordinate_magnitude = 0.0;
};

struct FilletResultEvidence {
  BlendEvidence blend;
  int generated_face_count = 0;
  int support_face_count = 0;
  // SCHEMA §7.3 "Two blend classes": the result is APPROXIMATED iff any of its
  // blend faces is. `approximation_tolerance` is the largest `BRep_Tool`
  // tolerance over the approximated blend FACES — and is 0 for an analytic
  // result. This aggregate is EVIDENCE only; the acceptance decision is taken
  // per contour (`FilletContourEvidence`), because an analytic contour must
  // never be judged by an approximated neighbour's budget.
  BlendSurfaceClass surface_class = BlendSurfaceClass::Analytic;
  double approximation_tolerance = 0.0;
  int approximated_face_count = 0;
};

// One contour's own measurements. SCHEMA §7.3: "Classification, measurement and
// budget are all PER CONTOUR."
struct FilletContourEvidence {
  int index = 0;
  BlendEvidence blend;
  int generated_face_count = 0;
  int support_face_count = 0;
  BlendSurfaceClass surface_class = BlendSurfaceClass::Analytic;
  double approximation_tolerance = 0.0;
  int approximated_face_count = 0;
};

// Classify one builder-generated blend face: plane / cylinder / cone / sphere /
// torus is analytic, everything else approximated. A trimmed surface is
// unwrapped to its basis first — a trimmed cylinder is still a cylinder.
//
// Surfaces of revolution / extrusion / offset land in "approximated". That is
// deliberate and harmless: the exact budgets are applied FIRST, so a
// representation is only ever consulted for a blend that already failed them.
BlendSurfaceClass classify_blend_face(const TopoDS_Face &face);

// Largest `BRep_Tool` tolerance carried by `face` ITSELF. SCHEMA §7.3 defines
// `approximationTolerance` over the blend faces and NEVER their edges or
// vertices: those are shared with the support faces, so a coarse input would
// inflate the number (measured 2x) and silently widen the budget of a blend
// OCCT actually fitted tightly.
double blend_face_tolerance(const TopoDS_Face &face);

// Per-contour evidence for every contour in `contour_indices`, measured on that
// contour's OWN generated blend faces and its own edges' support faces.
std::vector<FilletContourEvidence> collect_fillet_contour_evidence(
    BRepFilletAPI_MakeFillet &builder, const std::vector<int> &contour_indices,
    const TopoDS_Shape &input, const TopoDS_Shape &output, double radius);

// How densely `measure_blend_evidence` samples. `edge_samples` points per shared
// boundary edge (tangency), `uv_grid` per parametric axis over the blend face
// (section radius), so a blend face costs `uv_grid`^2 curvature evaluations.
//
// THE DEFAULTS ARE FROZEN AT TODAY'S NUMBERS. They are the density every Fillet
// result, every recognition and both kernelbench digest baselines were measured
// at, so changing them is a behaviour change, not a tuning knob. A caller that
// needs more confidence — a direct-modelling path that will DESTROY the face it
// is measuring — passes a bigger budget explicitly and owns that decision.
//
// A non-positive count samples nothing, which yields zero evidence; every
// consumer reads that as Ambiguous / refused, never as a passing measurement.
struct BlendSamplingBudget {
  int edge_samples = 9;
  int uv_grid = 5;
};

BlendEvidence measure_blend_evidence(
    const TopoDS_Shape &output,
    const std::vector<TopoDS_Face> &blend_faces,
    const std::vector<TopoDS_Face> &support_faces, double radius,
    const BlendSamplingBudget &budget = {});

// Collects generated blend faces and the selected edges' own surviving support
// faces from builder history, filtering every face to the actual output before
// measuring radius and G1 boundary tangency.
FilletResultEvidence collect_fillet_result_evidence(
    BRepFilletAPI_MakeFillet &builder, const TopoDS_Shape &input,
    const TopoDS_Shape &output, const std::vector<ResolvedEdge> &requested,
    double radius);

double fillet_section_radius_limit(double radius, double coordinate_magnitude);
double fillet_tangency_limit(double radius, double coordinate_magnitude);

} // namespace onecad::kernel::fillet
