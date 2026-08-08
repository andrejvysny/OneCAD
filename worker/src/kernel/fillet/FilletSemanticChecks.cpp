#include "kernel/fillet/FilletSemanticChecks.h"

#include <algorithm>
#include <cmath>

#include <Standard_Failure.hxx>

namespace onecad::kernel::fillet {

bool valid_constant_radius(double radius) {
  return std::isfinite(radius) && radius >= 1.0e-3;
}

FilletSemanticResult validate_assignment(BRepFilletAPI_MakeFillet &builder,
                                         const FilletAnalysis &analysis,
                                         double requested_radius) {
  FilletSemanticResult out;
  if (!analysis.ok || analysis.contours.empty()) {
    out.message =
        analysis.message.empty() ? "fillet has no contour" : analysis.message;
    return out;
  }
  const double tolerance =
      std::max(1.0e-12, std::abs(requested_radius) * 1.0e-12);
  try {
    for (const FilletContour &contour : analysis.contours) {
      if (!builder.IsConstant(contour.index)) {
        out.message = "assigned fillet law is not constant";
        return out;
      }
      if (std::abs(builder.Radius(contour.index) - requested_radius) >
          tolerance) {
        out.message = "assigned fillet radius differs from requested radius";
        return out;
      }
    }
    out.ok = true;
  } catch (const Standard_Failure &failure) {
    const char *message = failure.GetMessageString();
    out.message = message ? message : "fillet law query failed";
  } catch (...) {
    out.message = "unknown fillet law query failure";
  }
  return out;
}

FilletSemanticResult validate_result(BRepFilletAPI_MakeFillet &builder,
                                     const FilletAnalysis &analysis) {
  FilletSemanticResult out;
  try {
    for (const FilletContour &contour : analysis.contours) {
      int contour_faces = 0;
      for (int i = 1; i <= builder.NbEdges(contour.index); ++i) {
        for (const TopoDS_Shape &generated :
             builder.Generated(builder.Edge(contour.index, i))) {
          if (generated.ShapeType() == TopAbs_FACE)
            ++contour_faces;
        }
      }
      if (contour_faces == 0) {
        out.message = "successful contour generated no blend faces";
        return out;
      }
      out.generated_face_count += contour_faces;
    }
    if (builder.NbSurfaces() <= 0) {
      out.message = "fillet generated no blend surfaces";
      return out;
    }
    out.ok = true;
  } catch (const Standard_Failure &failure) {
    const char *message = failure.GetMessageString();
    out.message = message ? message : "fillet result query failed";
  } catch (...) {
    out.message = "unknown fillet result query failure";
  }
  return out;
}

} // namespace onecad::kernel::fillet
