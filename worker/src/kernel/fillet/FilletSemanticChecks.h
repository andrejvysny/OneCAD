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
};

bool valid_constant_radius(double radius);

FilletSemanticResult validate_assignment(BRepFilletAPI_MakeFillet &builder,
                                         const FilletAnalysis &analysis,
                                         double requested_radius);

FilletSemanticResult validate_result(
    BRepFilletAPI_MakeFillet &builder, const FilletAnalysis &analysis,
    const TopoDS_Shape &input, const TopoDS_Shape &output,
    const std::vector<ResolvedEdge> &requested, double requested_radius);

} // namespace onecad::kernel::fillet
