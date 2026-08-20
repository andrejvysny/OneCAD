#pragma once

#include <string>
#include <vector>

#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>

#include "kernel/fillet/AnalyticBlend.h"
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

// One face's answer: the sampled recognition AND the analytic tangency
// certificate, reported side by side because they answer different questions.
// `recognition` describes what was measured (radius, supports, evidence,
// refusal reason); `proof` says whether the face is EXACTLY tangent to both
// supports — not whether it is a blend (see `AnalyticBlend.h`). A consumer that
// destroys geometry needs both of these AND the rest of WP3's proof layers;
// neither field alone authorizes anything.
struct TargetedBlendRecognition {
  BlendRecognition recognition;
  bool recognized = false;
  AnalyticBlendProof proof = AnalyticBlendProof::Unknown;
  BlendConvexity convexity = BlendConvexity::Unknown;
  std::vector<TopoDS_Face> support_faces;
};

// Geometric constant-radius blend recognition for shapes with no feature
// provenance (notably imported B-Reps). Recognition is evidence-based and
// fail-closed: exactly two G1 support faces, measurable trimmed-domain curvature,
// constant section radius, and measured G1 tangency are mandatory.
BlendRecognitionReport recognize_constant_radius_blends(
    const TopoDS_Shape &body);

// The same classification for ONE face of `body`, plus its analytic proof and
// convexity. Exists so a caller holding a picked face pays for one face instead
// of every face in the body. `face` is located in `body` by identity; a face
// that is not part of `body` yields a refusal with a reason, never a guess.
TargetedBlendRecognition recognize_blend_at(const TopoDS_Face &face,
                                            const TopoDS_Shape &body);

} // namespace onecad::kernel::fillet
