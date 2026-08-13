#pragma once

#include <vector>

#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>

namespace onecad::benchmark {

/// Recipe-agnostic measurements of the produced blend.
///
/// Neither quantity asks what SHAPE the supports are, which is what separates
/// this from `cylindricalRadius` / `g1BoundaryTangency`: those read the blend
/// off the output by surface type and only hold when both supports are planes.
struct BlendEvidence {
  /// Blend/support boundary edges sampled.
  int boundaries = 0;
  /// Worst angle, in radians, between the blend's outward normal and its
  /// support's along a shared boundary. G1 means this is zero.
  double maximum_tangency_radians = 0.0;
  /// Interior points sampled on the blend faces.
  int samples = 0;
  /// Worst absolute deviation of the blend's section radius from the radius the
  /// operation requested.
  double maximum_profile_error = 0.0;
  double minimum_section_radius = 0.0;
  double maximum_section_radius = 0.0;
  /// Largest coordinate magnitude among the sampled points. Double precision is
  /// RELATIVE, so a model rebuilt a million millimetres from the origin carries
  /// a proportionally larger absolute error; a tolerance that ignores this reads
  /// conditioning as a defect.
  double coordinate_magnitude = 0.0;
};

/// Measures `blend_faces` (the fillet builder's OWN generated faces) inside
/// `output`, with tangency taken only on boundaries shared with `support_faces`
/// (the selected edge's own supports, likewise from the builder's history). An
/// empty blend set yields zero boundaries and zero samples, which every consumer
/// must treat as "no evidence", never as a pass.
BlendEvidence blend_evidence(const TopoDS_Shape &output,
                             const std::vector<TopoDS_Face> &blend_faces,
                             const std::vector<TopoDS_Face> &support_faces,
                             double radius);

} // namespace onecad::benchmark
