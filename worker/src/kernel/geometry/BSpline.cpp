// BSpline.cpp — see BSpline.h.

#include "kernel/geometry/BSpline.h"

#include <BRepBuilderAPI_MakeEdge.hxx>
#include <GeomAPI_Interpolate.hxx>
#include <Standard_Failure.hxx>
#include <NCollection_Array1.hxx>
#include <TColgp_HArray1OfPnt.hxx>

namespace onecad::kernel::geometry {

namespace {

std::string occt_reason(const Standard_Failure& f) {
  const char* what = f.what();
  return (what && *what) ? what : "OCCT";
}

}  // namespace

BSplineCurveResult interpolate_through_points(const std::vector<gp_Pnt>& points, bool periodic) {
  BSplineCurveResult out;

  // Collapse coincident consecutive samples. GeomAPI_Interpolate raises on
  // them, and a repeated point carries no information about the curve.
  std::vector<gp_Pnt> clean;
  clean.reserve(points.size());
  for (const auto& p : points) {
    if (!clean.empty() && clean.back().Distance(p) <= kInterpolateDedupTolerance) {
      ++out.droppedDuplicates;
      continue;
    }
    clean.push_back(p);
  }

  if (clean.size() < 2) {
    out.error = "cannot interpolate a curve through " + std::to_string(clean.size()) +
                " distinct point(s) (need at least 2; " + std::to_string(points.size()) +
                " were supplied, " + std::to_string(out.droppedDuplicates) + " coincident)";
    return out;
  }
  if (periodic && clean.size() < 3) {
    out.error = "a periodic interpolation needs at least 3 distinct points";
    return out;
  }

  try {
    Handle(NCollection_HArray1<gp_Pnt>) array =
        new NCollection_HArray1<gp_Pnt>(1, static_cast<int>(clean.size()));
    for (std::size_t i = 0; i < clean.size(); ++i) {
      array->SetValue(static_cast<int>(i + 1), clean[i]);
    }

    GeomAPI_Interpolate interp(array, periodic,
                               kInterpolateDedupTolerance);
    interp.Perform();
    if (!interp.IsDone()) {
      out.error = "B-spline interpolation did not converge over " +
                  std::to_string(clean.size()) + " points";
      return out;
    }
    Handle(Geom_BSplineCurve) curve = interp.Curve();
    if (curve.IsNull()) {
      // OCCT can report success and still hand back nothing. Treated as a
      // refusal, never propagated — see BSpline.h's failure policy.
      out.error = "B-spline interpolation reported success but produced a null curve";
      return out;
    }
    out.ok = true;
    out.curve = curve;
    return out;
  } catch (const Standard_Failure& f) {
    out.error = "B-spline interpolation failed: " + occt_reason(f);
    return out;
  } catch (...) {
    out.error = "B-spline interpolation failed";
    return out;
  }
}

BSplineEdgeResult interpolated_edge(const std::vector<gp_Pnt>& points, bool periodic) {
  BSplineEdgeResult out;
  const BSplineCurveResult curve = interpolate_through_points(points, periodic);
  out.droppedDuplicates = curve.droppedDuplicates;
  if (!curve.ok) {
    out.error = curve.error;
    return out;
  }
  try {
    BRepBuilderAPI_MakeEdge mk(curve.curve);
    if (!mk.IsDone()) {
      out.error = "interpolated curve could not be made into an edge";
      return out;
    }
    out.edge = mk.Edge();
    if (out.edge.IsNull()) {
      out.error = "interpolated edge is null";
      return out;
    }
    out.ok = true;
    return out;
  } catch (const Standard_Failure& f) {
    out.error = "interpolated edge could not be built: " + occt_reason(f);
    return out;
  } catch (...) {
    out.error = "interpolated edge could not be built";
    return out;
  }
}

}  // namespace onecad::kernel::geometry
