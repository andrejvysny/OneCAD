// BSpline.h — interpolate a curve THROUGH a sampled point run.
//
// The first member of the new kernel geometry capability layer the Gear
// Generator plan calls for (phase G1). Every analytic gear flank that is not
// a circular arc reaches OCCT through here: the samplers under
// `kernel/gear/` emit `Point2d` runs, and this turns them into real
// geometry.
//
// It wraps `GeomAPI_Interpolate` — the through-points (C2 cubic)
// interpolator, NOT `GeomAPI_PointsToBSpline`, which APPROXIMATES within a
// tolerance. That distinction is load-bearing: the reference implementation
// uses FreeCAD's `BSplineCurve.interpolate`, which is the same through-points
// semantics, so an approximating fit here would silently move every flank
// off the analytic curve the samplers were oracle-tested against.
//
// FAILURE POLICY. `GeomAPI_Interpolate` signals trouble in three different
// ways — a `Standard_ConstructionError` throw on coincident consecutive
// points, an `IsDone() == false`, or (worst) a successful build carrying a
// null handle. All three are funnelled into one graded refusal with a named
// reason. Nothing here ever returns an empty curve as if it were a result,
// because a null curve becomes a null edge becomes a wire that silently
// omits a flank, which is exactly the class of defect this project treats as
// systemic.

#pragma once

#include <string>
#include <vector>

#include <Geom_BSplineCurve.hxx>
#include <TopoDS_Edge.hxx>
#include <gp_Pnt.hxx>

namespace onecad::kernel::geometry {

/// Points closer together than this are treated as the same sample. Chosen to
/// sit an order of magnitude above OCCT's `Precision::Confusion()` (1e-7) so a
/// run that merely grazes confusion is cleaned rather than throwing inside
/// OCCT, while anything a caller could plausibly mean as two distinct samples
/// survives.
inline constexpr double kInterpolateDedupTolerance = 1e-6;

struct BSplineCurveResult {
  bool ok = false;
  std::string error;
  Handle(Geom_BSplineCurve) curve;
  /// How many coincident samples were dropped before interpolating. Non-zero
  /// is legal (a sampler may emit a repeated endpoint at a segment join) but
  /// worth surfacing: a large count means the caller's sample count is finer
  /// than its own curve's arc length can support.
  std::size_t droppedDuplicates = 0;
};

/// Interpolate a C2 cubic B-spline through `points`, in order.
///
/// Consecutive samples within `kInterpolateDedupTolerance` are collapsed —
/// deduplicating a repeated point does not change the curve, whereas handing
/// it to OCCT raises. At least two DISTINCT points must remain, else this
/// refuses rather than returning a degenerate curve.
///
/// `periodic` closes the curve; callers building an open flank want false.
BSplineCurveResult interpolate_through_points(const std::vector<gp_Pnt>& points,
                                              bool periodic = false);

struct BSplineEdgeResult {
  bool ok = false;
  std::string error;
  TopoDS_Edge edge;
  std::size_t droppedDuplicates = 0;
};

/// `interpolate_through_points` + `BRepBuilderAPI_MakeEdge`, with the same
/// fail-closed contract: a refusal never yields a usable-looking null edge.
BSplineEdgeResult interpolated_edge(const std::vector<gp_Pnt>& points, bool periodic = false);

}  // namespace onecad::kernel::geometry
