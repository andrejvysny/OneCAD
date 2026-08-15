#include "kernel/validation/GeometryPrecision.h"

#include <algorithm>
#include <cmath>

#include <BRepBndLib.hxx>
#include <BRep_Tool.hxx>
#include <Bnd_Box.hxx>
#include <Precision.hxx>
#include <Standard_Failure.hxx>

namespace onecad::kernel::validation {

namespace {

// A bounding box that is void, or whose extent underflows, carries no usable
// scale. Matches the guard `ShapeAudit.cpp`'s micro-topology collection already
// uses, so the two agree about when a shape has no measurable scale.
constexpr double kDegenerateExtent = 1.0e-300;

double finite_or_zero(double value) {
  return std::isfinite(value) ? value : 0.0;
}

} // namespace

double GeometryPrecisionContext::authoring_resolution() const {
  return kAuthoringResolutionMm;
}

double GeometryPrecisionContext::linear_resolution() const {
  return std::max({Precision::Confusion(), input_tolerance,
                   coordinate_magnitude * kConditioning});
}

double GeometryPrecisionContext::semantic_length() const {
  return std::max(kSemanticLengthFloorMm, coordinate_magnitude * kConditioning);
}

double GeometryPrecisionContext::semantic_angular() const {
  // No conditioning term: adding a LENGTH-derived quantity to an angle is a
  // dimensional error. Conditioning enters an angular budget only through a
  // radius, which this context does not have — `BlendEvidence`'s tangency limit
  // divides by one for exactly that reason.
  return kSemanticAngularFloorRad;
}

double GeometryPrecisionContext::tolerance_ceiling(double base_tolerance, double growth,
                                                   double epsilon) const {
  return std::max(authoring_resolution(), base_tolerance * growth + epsilon);
}

double GeometryPrecisionContext::minimum_volume() const {
  const double side = authoring_resolution();
  return side * side * side;
}

double GeometryPrecisionContext::micro_edge_length() const {
  return kMicroFactor *
         std::max(Precision::Confusion(), coordinate_magnitude * kConditioning);
}

double GeometryPrecisionContext::sliver_face_width() const {
  return micro_edge_length();
}

nlohmann::json GeometryPrecisionContext::to_json() const {
  return {{"version", version},
          {"scaleDiagonal", scale_diagonal},
          {"coordinateMagnitude", coordinate_magnitude},
          {"inputTolerance", input_tolerance},
          {"featureMagnitude", feature_magnitude},
          {"authoringResolution", authoring_resolution()},
          {"linearResolution", linear_resolution()},
          {"semanticLength", semantic_length()},
          {"semanticAngular", semantic_angular()},
          {"minimumVolume", minimum_volume()},
          {"microEdgeLength", micro_edge_length()},
          {"sliverFaceWidth", sliver_face_width()}};
}

GeometryPrecisionContext precision_of(const TopoDS_Shape &shape) {
  return precision_of(shape, 0.0);
}

GeometryPrecisionContext precision_of(const TopoDS_Shape &shape, double feature_magnitude) {
  GeometryPrecisionContext out;
  out.feature_magnitude = finite_or_zero(std::abs(feature_magnitude));
  if (shape.IsNull())
    return out;

  try {
    Bnd_Box bounds;
    BRepBndLib::Add(shape, bounds);
    if (!bounds.IsVoid()) {
      double xmin, ymin, zmin, xmax, ymax, zmax;
      bounds.Get(xmin, ymin, zmin, xmax, ymax, zmax);
      const double diagonal =
          std::hypot(xmax - xmin, std::hypot(ymax - ymin, zmax - zmin));
      if (std::isfinite(diagonal) && diagonal > kDegenerateExtent)
        out.scale_diagonal = diagonal;
      // Largest |corner|, i.e. how far from the origin the arithmetic runs.
      // Both corners are checked because a box straddling the origin has its
      // worst conditioning on whichever side reaches further.
      out.coordinate_magnitude = finite_or_zero(
          std::max(std::hypot(xmin, std::hypot(ymin, zmin)),
                   std::hypot(xmax, std::hypot(ymax, zmax))));
    }

    // Same three-way maximum the operations were computing inline (Extrude's
    // `shape_max_tolerance`, Fillet's `ShapeTolerances::maximum()`), so a call
    // site moving here observes the identical number.
    out.input_tolerance = finite_or_zero(
        std::max({BRep_Tool::MaxTolerance(shape, TopAbs_FACE),
                  BRep_Tool::MaxTolerance(shape, TopAbs_EDGE),
                  BRep_Tool::MaxTolerance(shape, TopAbs_VERTEX)}));
  } catch (const Standard_Failure &) {
    // A shape we cannot measure yields floors, never a guess. Every derived
    // budget then falls back to its most conservative value.
    out.scale_diagonal = 0.0;
    out.coordinate_magnitude = 0.0;
    out.input_tolerance = 0.0;
  }
  return out;
}

} // namespace onecad::kernel::validation
