#include "kernel/fillet/BlendEvidence.h"

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
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopTools_ShapeMapHasher.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>
#include <gp_Pnt2d.hxx>

namespace onecad::kernel::fillet {
namespace {

using EdgeFaces = NCollection_IndexedDataMap<
    TopoDS_Shape, NCollection_List<TopoDS_Shape>, TopTools_ShapeMapHasher>;

constexpr double kNormalTolerance = 1.0e-9;
constexpr double kTangencyRadians = 1.0e-9;
constexpr double kSectionRelative = 1.0e-9;
constexpr double kConditioning = 1.0e-14;

bool contains_face(const std::vector<TopoDS_Face> &faces,
                   const TopoDS_Shape &candidate) {
  return candidate.ShapeType() == TopAbs_FACE &&
         std::any_of(faces.begin(), faces.end(),
                     [&candidate](const TopoDS_Face &known) {
                       return known.IsSame(candidate);
                     });
}

void add_output_face(std::vector<TopoDS_Face> &faces,
                     const TopoDS_Shape &candidate,
                     const TopTools_IndexedMapOfShape &output_faces) {
  if (candidate.IsNull() || candidate.ShapeType() != TopAbs_FACE ||
      output_faces.FindIndex(candidate) == 0 || contains_face(faces, candidate))
    return;
  faces.push_back(TopoDS::Face(candidate));
}

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
                     const TopoDS_Face &support, int edge_samples,
                     BlendEvidence &out) {
  double first = 0.0;
  double last = 0.0;
  const Handle(Geom_Curve) curve = BRep_Tool::Curve(edge, first, last);
  if (curve.IsNull())
    return;
  int measured = 0;
  for (int i = 0; i < edge_samples; ++i) {
    const double t = first + (last - first) * (i + 0.5) / edge_samples;
    const gp_Pnt point = curve->Value(t);
    gp_Dir blend_normal;
    gp_Dir support_normal;
    if (!outward_normal(blend, point, blend_normal) ||
        !outward_normal(support, point, support_normal))
      continue;
    out.coordinate_magnitude =
        std::max(out.coordinate_magnitude, point.XYZ().Modulus());
    out.maximum_tangency_radians =
        std::max(out.maximum_tangency_radians,
                 static_cast<double>(blend_normal.Angle(support_normal)));
    ++measured;
  }
  if (measured > 0)
    ++out.boundaries;
}

void sample_face(const TopoDS_Face &face, double radius, int uv_grid,
                 BlendEvidence &out) {
  BRepAdaptor_Surface surface(face);
  BRepTopAdaptor_FClass2d classifier(face, Precision::PConfusion());
  const double u0 = surface.FirstUParameter();
  const double u1 = surface.LastUParameter();
  const double v0 = surface.FirstVParameter();
  const double v1 = surface.LastVParameter();
  if (!std::isfinite(u0) || !std::isfinite(u1) ||
      !std::isfinite(v0) || !std::isfinite(v1))
    return;
  for (int i = 0; i < uv_grid; ++i) {
    for (int j = 0; j < uv_grid; ++j) {
      const double u = u0 + (u1 - u0) * (i + 0.5) / uv_grid;
      const double v = v0 + (v1 - v0) * (j + 0.5) / uv_grid;
      if (classifier.Perform(gp_Pnt2d(u, v), Standard_False) != TopAbs_IN)
        continue;
      BRepLProp_SLProps properties(surface, u, v, 2, kNormalTolerance);
      if (!properties.IsCurvatureDefined())
        continue;
      const double curvature = std::max(std::abs(properties.MaxCurvature()),
                                        std::abs(properties.MinCurvature()));
      if (curvature <= kNormalTolerance)
        continue;
      const double section = 1.0 / curvature;
      out.coordinate_magnitude =
          std::max(out.coordinate_magnitude, properties.Value().XYZ().Modulus());
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

double fillet_section_radius_limit(double radius, double coordinate_magnitude) {
  return std::max(1.0e-9, std::abs(radius) * kSectionRelative) +
         coordinate_magnitude * kConditioning;
}

double fillet_tangency_limit(double radius, double coordinate_magnitude) {
  return kTangencyRadians +
         coordinate_magnitude * kConditioning /
             std::max(std::abs(radius), 1.0e-9);
}

BlendEvidence measure_blend_evidence(
    const TopoDS_Shape &output,
    const std::vector<TopoDS_Face> &blend_faces,
    const std::vector<TopoDS_Face> &support_faces, double radius,
    const BlendSamplingBudget &budget) {
  BlendEvidence out;
  if (output.IsNull() || blend_faces.empty())
    return out;
  EdgeFaces edge_faces;
  TopExp::MapShapesAndAncestors(output, TopAbs_EDGE, TopAbs_FACE, edge_faces);
  for (int i = 1; i <= edge_faces.Extent(); ++i) {
    const auto &faces = edge_faces.FindFromIndex(i);
    if (faces.Extent() != 2)
      continue;
    const bool first_is_blend = contains_face(blend_faces, faces.First());
    const bool last_is_blend = contains_face(blend_faces, faces.Last());
    if (first_is_blend == last_is_blend)
      continue;
    const TopoDS_Shape &support = first_is_blend ? faces.Last() : faces.First();
    if (!contains_face(support_faces, support))
      continue;
    const TopoDS_Edge edge = TopoDS::Edge(edge_faces.FindKey(i));
    if (BRep_Tool::Degenerated(edge))
      continue;
    sample_boundary(edge,
                    TopoDS::Face(first_is_blend ? faces.First() : faces.Last()),
                    TopoDS::Face(support), budget.edge_samples, out);
  }
  for (const TopoDS_Face &face : blend_faces)
    sample_face(face, radius, budget.uv_grid, out);
  return out;
}

FilletResultEvidence collect_fillet_result_evidence(
    BRepFilletAPI_MakeFillet &builder, const TopoDS_Shape &input,
    const TopoDS_Shape &output, const std::vector<ResolvedEdge> &requested,
    double radius) {
  FilletResultEvidence out;
  if (input.IsNull() || output.IsNull())
    return out;

  TopTools_IndexedMapOfShape output_faces;
  TopExp::MapShapes(output, TopAbs_FACE, output_faces);
  std::vector<TopoDS_Face> blend_faces;
  std::vector<TopoDS_Face> support_faces;

  EdgeFaces input_edge_faces;
  TopExp::MapShapesAndAncestors(input, TopAbs_EDGE, TopAbs_FACE,
                                input_edge_faces);
  for (const ResolvedEdge &selected : requested) {
    for (const TopoDS_Shape &generated : builder.Generated(selected.edge))
      add_output_face(blend_faces, generated, output_faces);

    if (!input_edge_faces.Contains(selected.edge))
      continue;
    for (const TopoDS_Shape &support :
         input_edge_faces.FindFromKey(selected.edge)) {
      const TopTools_ListOfShape &modified = builder.Modified(support);
      if (modified.IsEmpty()) {
        add_output_face(support_faces, support, output_faces);
        continue;
      }
      for (const TopoDS_Shape &successor : modified)
        add_output_face(support_faces, successor, output_faces);
    }
  }

  out.generated_face_count = static_cast<int>(blend_faces.size());
  out.support_face_count = static_cast<int>(support_faces.size());
  out.blend = measure_blend_evidence(output, blend_faces, support_faces, radius);
  return out;
}

} // namespace onecad::kernel::fillet
