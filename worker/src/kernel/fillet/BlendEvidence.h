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

BlendEvidence measure_blend_evidence(
    const TopoDS_Shape &output,
    const std::vector<TopoDS_Face> &blend_faces,
    const std::vector<TopoDS_Face> &support_faces, double radius);

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
