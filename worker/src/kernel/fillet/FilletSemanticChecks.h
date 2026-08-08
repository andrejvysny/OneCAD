#pragma once

#include <string>

#include <BRepFilletAPI_MakeFillet.hxx>

#include "kernel/fillet/FilletAnalyzer.h"

namespace onecad::kernel::fillet {

struct FilletSemanticResult {
  bool ok = false;
  std::string message;
  int generated_face_count = 0;
};

bool valid_constant_radius(double radius);

FilletSemanticResult validate_assignment(BRepFilletAPI_MakeFillet &builder,
                                         const FilletAnalysis &analysis,
                                         double requested_radius);

FilletSemanticResult validate_result(BRepFilletAPI_MakeFillet &builder,
                                     const FilletAnalysis &analysis);

} // namespace onecad::kernel::fillet
