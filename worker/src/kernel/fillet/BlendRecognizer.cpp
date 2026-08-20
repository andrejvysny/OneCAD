#include "kernel/fillet/BlendRecognizer.h"

#include <algorithm>
#include <cmath>

#include <BRepLib.hxx>
#include <GeomAbs_Shape.hxx>
#include <NCollection_IndexedDataMap.hxx>
#include <NCollection_List.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ShapeMapHasher.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>

namespace onecad::kernel::fillet {
namespace {

using EdgeFaces = NCollection_IndexedDataMap<
    TopoDS_Shape, NCollection_List<TopoDS_Shape>, TopTools_ShapeMapHasher>;

constexpr double kContinuityAngularTolerance = 1.0e-7;
constexpr double kConstantRadiusRelativeTolerance = 1.0e-6;
constexpr double kConstantRadiusAbsoluteTolerance = 1.0e-6;

void add_unique(std::vector<TopoDS_Face> &faces, const TopoDS_Shape &candidate) {
  if (candidate.ShapeType() != TopAbs_FACE)
    return;
  const TopoDS_Face face = TopoDS::Face(candidate);
  if (std::none_of(faces.begin(), faces.end(), [&face](const TopoDS_Face &known) {
        return known.IsSame(face);
      }))
    faces.push_back(face);
}

struct Classification {
  BlendRecognition result;
  std::vector<TopoDS_Face> supports;
};

// The whole-body sweep and the targeted entry point MUST agree face for face,
// so there is exactly one copy of the classification and both call it.
Classification classify_face(const TopoDS_Shape &body,
                             const TopoDS_Face &candidate, int ordinal,
                             const TopTools_IndexedMapOfShape &faces,
                             const EdgeFaces &edge_faces) {
  Classification out;
  BlendRecognition &result = out.result;
  result.face_ordinal = ordinal;

  for (int edge_index = 1; edge_index <= edge_faces.Extent(); ++edge_index) {
    const auto &adjacent = edge_faces.FindFromIndex(edge_index);
    if (adjacent.Extent() != 2)
      continue;
    const bool first = adjacent.First().IsSame(candidate);
    const bool last = adjacent.Last().IsSame(candidate);
    if (first == last)
      continue;
    const TopoDS_Face support =
        TopoDS::Face(first ? adjacent.Last() : adjacent.First());
    try {
      if (BRepLib::ContinuityOfFaces(
              TopoDS::Edge(edge_faces.FindKey(edge_index)), candidate, support,
              kContinuityAngularTolerance) >= GeomAbs_G1) {
        add_unique(out.supports, support);
      }
    } catch (...) {
      result.status = BlendRecognitionStatus::Ambiguous;
      result.reason = "surface continuity could not be measured";
    }
  }

  if (result.status == BlendRecognitionStatus::Ambiguous)
    return out;
  if (out.supports.size() != 2) {
    result.reason = out.supports.size() > 2
                        ? "more than two G1 support faces"
                        : "not bounded by exactly two G1 support faces";
    result.status = out.supports.size() > 2 ? BlendRecognitionStatus::Ambiguous
                                            : BlendRecognitionStatus::NotBlend;
    return out;
  }

  result.evidence = measure_blend_evidence(body, {candidate}, out.supports, 0.0);
  if (result.evidence.samples == 0 || result.evidence.boundaries < 2) {
    result.status = BlendRecognitionStatus::Ambiguous;
    result.reason = "insufficient trimmed-domain radius/tangency evidence";
    return out;
  }
  result.radius = 0.5 * (result.evidence.minimum_section_radius +
                         result.evidence.maximum_section_radius);
  const double radius_spread = result.evidence.maximum_section_radius -
                               result.evidence.minimum_section_radius;
  const double radius_limit =
      std::max(kConstantRadiusAbsoluteTolerance,
               std::abs(result.radius) * kConstantRadiusRelativeTolerance);
  if (!(result.radius > 0.0) || radius_spread > radius_limit) {
    result.status = BlendRecognitionStatus::NotBlend;
    result.reason = "section radius is not constant";
    return out;
  }
  if (result.evidence.maximum_tangency_radians >
      fillet_tangency_limit(result.radius,
                            result.evidence.coordinate_magnitude)) {
    result.status = BlendRecognitionStatus::NotBlend;
    result.reason = "support boundaries are not G1 tangent";
    return out;
  }

  for (const TopoDS_Face &support : out.supports)
    result.support_face_ordinals.push_back(faces.FindIndex(support));
  std::sort(result.support_face_ordinals.begin(),
            result.support_face_ordinals.end());
  result.status = BlendRecognitionStatus::Recognized;
  result.reason = "constant-radius two-support blend";
  return out;
}

} // namespace

bool BlendRecognitionReport::has_ambiguous() const {
  return std::any_of(faces.begin(), faces.end(), [](const BlendRecognition &face) {
    return face.status == BlendRecognitionStatus::Ambiguous;
  });
}

std::vector<BlendRecognition> BlendRecognitionReport::recognized() const {
  std::vector<BlendRecognition> out;
  for (const BlendRecognition &face : faces) {
    if (face.status == BlendRecognitionStatus::Recognized)
      out.push_back(face);
  }
  return out;
}

BlendRecognitionReport recognize_constant_radius_blends(const TopoDS_Shape &body) {
  BlendRecognitionReport report;
  if (body.IsNull())
    return report;

  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(body, TopAbs_FACE, faces);
  EdgeFaces edge_faces;
  TopExp::MapShapesAndAncestors(body, TopAbs_EDGE, TopAbs_FACE, edge_faces);

  for (int ordinal = 1; ordinal <= faces.Extent(); ++ordinal) {
    report.faces.push_back(
        classify_face(body, TopoDS::Face(faces(ordinal)), ordinal, faces,
                      edge_faces)
            .result);
  }
  return report;
}

TargetedBlendRecognition recognize_blend_at(const TopoDS_Face &face,
                                            const TopoDS_Shape &body) {
  TargetedBlendRecognition out;
  if (face.IsNull() || body.IsNull()) {
    out.recognition.reason = "null face or body";
    return out;
  }

  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(body, TopAbs_FACE, faces);
  const int ordinal = faces.FindIndex(face);
  if (ordinal == 0) {
    out.recognition.reason = "face does not belong to the body";
    return out;
  }
  // The body's own instance, so orientation-sensitive queries (outward normal,
  // convexity) read the orientation the body assigns rather than the caller's.
  const TopoDS_Face candidate = TopoDS::Face(faces(ordinal));

  EdgeFaces edge_faces;
  TopExp::MapShapesAndAncestors(body, TopAbs_EDGE, TopAbs_FACE, edge_faces);
  Classification classified =
      classify_face(body, candidate, ordinal, faces, edge_faces);

  out.recognized =
      classified.result.status == BlendRecognitionStatus::Recognized;
  out.convexity = blend_convexity(candidate);
  if (classified.supports.size() == 2) {
    out.proof = analytic_cylinder_blend(candidate, classified.supports[0],
                                        classified.supports[1]);
  }
  out.support_faces = std::move(classified.supports);
  out.recognition = std::move(classified.result);
  return out;
}

} // namespace onecad::kernel::fillet
