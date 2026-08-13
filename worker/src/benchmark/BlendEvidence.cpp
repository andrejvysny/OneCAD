#include "benchmark/BlendEvidence.h"

#include <algorithm>
#include <cmath>

#include <BRepAdaptor_Surface.hxx>
#include <BRepLProp_SLProps.hxx>
#include <BRepTopAdaptor_FClass2d.hxx>
#include <BRep_Tool.hxx>
#include <GeomAPI_ProjectPointOnSurf.hxx>
#include <Geom_Curve.hxx>
#include <Geom_Surface.hxx>
#include <NCollection_IndexedDataMap.hxx>
#include <NCollection_List.hxx>
#include <Precision.hxx>
#include <TopAbs_Orientation.hxx>
#include <TopExp.hxx>
#include <TopTools_ShapeMapHasher.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>
#include <gp_Pnt2d.hxx>

namespace onecad::benchmark {
namespace {

using EdgeFaces = NCollection_IndexedDataMap<
    TopoDS_Shape, NCollection_List<TopoDS_Shape>, TopTools_ShapeMapHasher>;

constexpr int kEdgeSamples = 9;
constexpr int kFaceSamplesPerAxis = 5;
constexpr double kNormalTolerance = 1.0e-9;

bool is_blend(const TopoDS_Shape &face,
              const std::vector<TopoDS_Face> &blend_faces) {
  return std::any_of(blend_faces.begin(), blend_faces.end(),
                     [&face](const TopoDS_Face &known) {
                       return known.IsSame(face);
                     });
}

/// Outward normal of `face` at the surface point nearest `point`. Returns false
/// when the surface has no defined normal there, which must not be read as a
/// tangency failure.
bool outward_normal(const TopoDS_Face &face, const gp_Pnt &point, gp_Dir &out) {
  const Handle(Geom_Surface) surface = BRep_Tool::Surface(face);
  if (surface.IsNull())
    return false;
  GeomAPI_ProjectPointOnSurf projector(point, surface);
  if (!projector.IsDone() || projector.NbPoints() < 1)
    return false;
  double u = 0.0;
  double v = 0.0;
  projector.LowerDistanceParameters(u, v);
  BRepLProp_SLProps properties(BRepAdaptor_Surface(face), u, v, 1,
                               kNormalTolerance);
  if (!properties.IsNormalDefined())
    return false;
  out = properties.Normal();
  if (face.Orientation() == TopAbs_REVERSED)
    out.Reverse();
  return true;
}

void sample_boundary(const TopoDS_Edge &edge, const TopoDS_Face &blend,
                     const TopoDS_Face &support, BlendEvidence &out) {
  double first = 0.0;
  double last = 0.0;
  const Handle(Geom_Curve) curve = BRep_Tool::Curve(edge, first, last);
  if (curve.IsNull())
    return;
  ++out.boundaries;
  for (int i = 0; i < kEdgeSamples; ++i) {
    const double t = first + (last - first) * (i + 0.5) / kEdgeSamples;
    const gp_Pnt point = curve->Value(t);
    out.coordinate_magnitude =
        std::max(out.coordinate_magnitude, point.XYZ().Modulus());
    gp_Dir blend_normal;
    gp_Dir support_normal;
    if (!outward_normal(blend, point, blend_normal) ||
        !outward_normal(support, point, support_normal))
      continue;
    out.maximum_tangency_radians =
        std::max(out.maximum_tangency_radians,
                 static_cast<double>(blend_normal.Angle(support_normal)));
  }
}

/// The section radius of a constant-radius blend is `1 / |k|` for the larger
/// principal curvature: a constant-radius blend is a canal surface, and its
/// circular sections ARE lines of curvature, so that curvature is exactly the
/// reciprocal of the requested radius on plane, cylinder and cone supports
/// alike. The along-spine curvature is the smaller one for every pair in the
/// matrix; a case where it is not would surface here as a measured failure
/// rather than a silently wrong pass.
void sample_face(const TopoDS_Face &face, double radius, BlendEvidence &out) {
  BRepAdaptor_Surface surface(face);
  BRepTopAdaptor_FClass2d classifier(face, Precision::Confusion());
  const double u0 = surface.FirstUParameter();
  const double u1 = surface.LastUParameter();
  const double v0 = surface.FirstVParameter();
  const double v1 = surface.LastVParameter();
  for (int i = 0; i < kFaceSamplesPerAxis; ++i) {
    for (int j = 0; j < kFaceSamplesPerAxis; ++j) {
      const double u = u0 + (u1 - u0) * (i + 0.5) / kFaceSamplesPerAxis;
      const double v = v0 + (v1 - v0) * (j + 0.5) / kFaceSamplesPerAxis;
      if (classifier.Perform(gp_Pnt2d(u, v)) == TopAbs_OUT)
        continue;
      BRepLProp_SLProps properties(surface, u, v, 2, kNormalTolerance);
      if (!properties.IsCurvatureDefined())
        continue;
      const double curvature = std::max(std::abs(properties.MaxCurvature()),
                                        std::abs(properties.MinCurvature()));
      if (curvature <= kNormalTolerance)
        continue;
      out.coordinate_magnitude =
          std::max(out.coordinate_magnitude, properties.Value().XYZ().Modulus());
      const double section = 1.0 / curvature;
      out.minimum_section_radius = out.samples == 0
                                       ? section
                                       : std::min(out.minimum_section_radius, section);
      out.maximum_section_radius = std::max(out.maximum_section_radius, section);
      out.maximum_profile_error =
          std::max(out.maximum_profile_error, std::abs(section - radius));
      ++out.samples;
    }
  }
}

} // namespace

BlendEvidence blend_evidence(const TopoDS_Shape &output,
                             const std::vector<TopoDS_Face> &blend_faces,
                             const std::vector<TopoDS_Face> &support_faces,
                             double radius) {
  BlendEvidence out;
  if (output.IsNull() || blend_faces.empty())
    return out;
  EdgeFaces edge_faces;
  TopExp::MapShapesAndAncestors(output, TopAbs_EDGE, TopAbs_FACE, edge_faces);
  for (int i = 1; i <= edge_faces.Extent(); ++i) {
    const auto &faces = edge_faces.FindFromIndex(i);
    if (faces.Extent() != 2)
      continue;
    const bool first_is_blend = is_blend(faces.First(), blend_faces);
    const bool last_is_blend = is_blend(faces.Last(), blend_faces);
    if (first_is_blend == last_is_blend)
      continue;
    // Only the selected edge's OWN supports owe tangency. The blend also meets
    // the end caps, at a right angle, and sampling those would fail every case.
    if (!is_blend(first_is_blend ? faces.Last() : faces.First(), support_faces))
      continue;
    const TopoDS_Edge edge = TopoDS::Edge(edge_faces.FindKey(i));
    if (BRep_Tool::Degenerated(edge))
      continue;
    sample_boundary(edge, TopoDS::Face(first_is_blend ? faces.First() : faces.Last()),
                    TopoDS::Face(first_is_blend ? faces.Last() : faces.First()), out);
  }
  for (const TopoDS_Face &face : blend_faces)
    sample_face(face, radius, out);
  return out;
}

} // namespace onecad::benchmark
