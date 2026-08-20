#pragma once

#include <vector>

#include <BRepFilletAPI_MakeFillet.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>

#include "kernel/fillet/FilletAnalyzer.h"

namespace onecad::kernel::fillet {

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
};

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
