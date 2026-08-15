#pragma once

#include <string>
#include <vector>

#include <TopoDS_Shape.hxx>

#include "kernel/fillet/BlendEvidence.h"

namespace onecad::kernel::fillet {

enum class BlendRecognitionStatus { Recognized, NotBlend, Ambiguous };

struct BlendRecognition {
  int face_ordinal = 0;
  BlendRecognitionStatus status = BlendRecognitionStatus::NotBlend;
  double radius = 0.0;
  std::vector<int> support_face_ordinals;
  BlendEvidence evidence;
  std::string reason;
};

struct BlendRecognitionReport {
  std::vector<BlendRecognition> faces;

  bool has_ambiguous() const;
  std::vector<BlendRecognition> recognized() const;
};

// Geometric constant-radius blend recognition for shapes with no feature
// provenance (notably imported B-Reps). Recognition is evidence-based and
// fail-closed: exactly two G1 support faces, measurable trimmed-domain curvature,
// constant section radius, and measured G1 tangency are mandatory.
BlendRecognitionReport recognize_constant_radius_blends(
    const TopoDS_Shape &body);

} // namespace onecad::kernel::fillet
