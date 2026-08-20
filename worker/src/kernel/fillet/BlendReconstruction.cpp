#include "kernel/fillet/BlendReconstruction.h"

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <map>
#include <sstream>
#include <utility>

#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Defeaturing.hxx>
#include <BRepGProp.hxx>
#include <BRep_Tool.hxx>
#include <GProp_GProps.hxx>
#include <GeomAbs_CurveType.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <Geom_Curve.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <gp_Ax1.hxx>
#include <gp_Cylinder.hxx>
#include <gp_Lin.hxx>
#include <gp_Pln.hxx>

#include "kernel/fillet/BlendRecognizer.h"
#include "kernel/fillet/FilletBuilder.h"
#include "kernel/validation/GeometryPrecision.h"
#include "kernel/validation/ShapeAudit.h"

namespace onecad::kernel::fillet {
namespace {

namespace validation = onecad::kernel::validation;

std::string num(double value) {
  char buffer[64];
  std::snprintf(buffer, sizeof(buffer), "%.12g", value);
  return buffer;
}

TopTools_ListOfShape one(const TopoDS_Shape &shape) {
  TopTools_ListOfShape list;
  list.Append(shape);
  return list;
}

double edge_length(const TopoDS_Edge &edge) {
  GProp_GProps properties;
  BRepGProp::LinearProperties(edge, properties);
  return properties.Mass();
}

double shape_volume(const TopoDS_Shape &shape) {
  GProp_GProps properties;
  BRepGProp::VolumeProperties(shape, properties);
  return properties.Mass();
}

bool has_curve(const TopoDS_Edge &edge) {
  if (BRep_Tool::Degenerated(edge))
    return false;
  double first = 0.0;
  double last = 0.0;
  return !BRep_Tool::Curve(edge, first, last).IsNull();
}

// Edges belonging to BOTH faces, by `IsSame` (the hasher `TopTools_IndexedMapOfShape`
// uses), so orientation in the two faces does not split one shared edge into two.
std::vector<TopoDS_Edge> shared_edges(const TopoDS_Face &a, const TopoDS_Face &b) {
  TopTools_IndexedMapOfShape a_edges;
  TopExp::MapShapes(a, TopAbs_EDGE, a_edges);
  TopTools_IndexedMapOfShape b_edges;
  TopExp::MapShapes(b, TopAbs_EDGE, b_edges);
  std::vector<TopoDS_Edge> out;
  for (int i = 1; i <= a_edges.Extent(); ++i) {
    if (b_edges.Contains(a_edges(i)))
      out.push_back(TopoDS::Edge(a_edges(i)));
  }
  return out;
}

bool contains_same(const std::vector<TopoDS_Face> &faces, const TopoDS_Face &face) {
  return std::any_of(faces.begin(), faces.end(), [&face](const TopoDS_Face &known) {
    return known.IsSame(face);
  });
}

// Postcondition 6. Defeaturing must RE-TRIM a retained support, never re-fit it:
// the surviving face carries the same analytic surface, only bounded differently.
// A support whose plane moved or whose cylinder changed radius means the kernel
// healed the body into something else, and the blend we are about to rebuild would
// be rebuilt against geometry the user never had.
bool same_surface(const TopoDS_Face &before, const TopoDS_Face &after,
                  const validation::GeometryPrecisionContext &precision,
                  std::string &detail) {
  BRepAdaptor_Surface a(before);
  BRepAdaptor_Surface b(after);
  if (a.GetType() != b.GetType()) {
    detail = "support surface type changed";
    return false;
  }
  switch (a.GetType()) {
  case GeomAbs_Plane: {
    const gp_Pln before_plane = a.Plane();
    const gp_Pln after_plane = b.Plane();
    if (!before_plane.Axis().Direction().IsParallel(after_plane.Axis().Direction(),
                                                    precision.semantic_angular())) {
      detail = "support plane normal rotated";
      return false;
    }
    const double drift = after_plane.Distance(before_plane.Location());
    if (drift > precision.semantic_length()) {
      detail = "support plane moved by " + num(drift);
      return false;
    }
    return true;
  }
  case GeomAbs_Cylinder: {
    const gp_Cylinder before_cylinder = a.Cylinder();
    const gp_Cylinder after_cylinder = b.Cylinder();
    if (!before_cylinder.Axis().Direction().IsParallel(after_cylinder.Axis().Direction(),
                                                       precision.semantic_angular())) {
      detail = "support cylinder axis rotated";
      return false;
    }
    const double axis_drift =
        gp_Lin(after_cylinder.Axis()).Distance(before_cylinder.Axis().Location());
    const double radius_drift =
        std::abs(before_cylinder.Radius() - after_cylinder.Radius());
    if (axis_drift > precision.semantic_length() ||
        radius_drift > precision.semantic_length()) {
      detail = "support cylinder axis moved by " + num(axis_drift) + " mm, radius by " +
               num(radius_drift) + " mm";
      return false;
    }
    return true;
  }
  default:
    // Unreachable through `certify_blend` (layer 1 refuses any support that is
    // neither plane nor cylinder), reachable through a forged `RecognizedBlend`.
    detail = "support surface is neither a plane nor a cylinder";
    return false;
  }
}

// The RESULT BODY's own instance of a face, which is what every orientation-
// sensitive query has to read. A face handed back by an OCCT history carries the
// orientation the algorithm recorded it with, not the one the body assigns it, and
// `blend_convexity` reverses on `TopAbs_REVERSED`: reading convexity off a history
// instance answers about the wrong side of the surface. `recognize_blend_at` makes
// the same normalization for the same reason.
TopoDS_Face in_result(const TopTools_IndexedMapOfShape &result_faces,
                      const TopoDS_Shape &face) {
  const int ordinal = result_faces.FindIndex(face);
  return ordinal == 0 ? TopoDS_Face() : TopoDS::Face(result_faces(ordinal));
}

// The successors of `face` that are actually faces of `result`. A face the stage
// did not touch reports no `Modified` image and survives verbatim, so it is its
// own image; a face the stage removed has none.
std::vector<TopoDS_Face> face_images(const BRepTools_History &history,
                                     const TopoDS_Face &face,
                                     const TopTools_IndexedMapOfShape &result_faces) {
  std::vector<TopoDS_Face> out;
  for (const TopoDS_Shape &modified : history.Modified(face)) {
    if (modified.ShapeType() != TopAbs_FACE)
      continue;
    const TopoDS_Face image = in_result(result_faces, modified);
    if (!image.IsNull())
      out.push_back(image);
  }
  if (out.empty() && !history.IsRemoved(face)) {
    const TopoDS_Face survivor = in_result(result_faces, face);
    if (!survivor.IsNull())
      out.push_back(survivor);
  }
  return out;
}

// Everything the kernel said about the run, collapsed to one line for a reason
// string. `BOPAlgo_RemoveFeatures` reports an inability to remove a face as a
// WARNING and still answers `IsDone()`, so this text is the only place the kernel's
// own account of a no-op survives.
std::string kernel_alerts(BRepAlgoAPI_Defeaturing &defeature) {
  std::ostringstream stream;
  defeature.DumpErrors(stream);
  defeature.DumpWarnings(stream);
  std::string text = stream.str();
  std::replace(text.begin(), text.end(), '\n', ' ');
  const std::size_t last = text.find_last_not_of(' ');
  return last == std::string::npos ? std::string() : text.substr(0, last + 1);
}

const char *convexity_name(BlendConvexity convexity) {
  switch (convexity) {
  case BlendConvexity::Convex:
    return "Convex";
  case BlendConvexity::Concave:
    return "Concave";
  case BlendConvexity::Unknown:
    break;
  }
  return "Unknown";
}

const char *proof_name(AnalyticBlendProof proof) {
  switch (proof) {
  case AnalyticBlendProof::Proved:
    return "Proved";
  case AnalyticBlendProof::Failed:
    return "Failed";
  case AnalyticBlendProof::Unknown:
    break;
  }
  return "Unknown";
}

} // namespace

BlendCertification certify_blend(const TopoDS_Face &face, const TopoDS_Shape &body) {
  BlendCertification out;

  // ── Layer 1: sampled recognition + the exact tangency certificate ──────────
  const TargetedBlendRecognition targeted = recognize_blend_at(face, body);
  out.recognition_status = targeted.recognition.status;
  if (!targeted.recognized) {
    out.reason = "BLEND_NOT_RECOGNIZED: " + targeted.recognition.reason;
    return out;
  }
  if (targeted.proof != AnalyticBlendProof::Proved) {
    // `Failed` and `Unknown` are equivalent here by `AnalyticBlend.h`'s contract:
    // both mean "do not engage the destructive path".
    out.reason =
        std::string("BLEND_NOT_PROVED: analytic tangency is ") + proof_name(targeted.proof);
    return out;
  }
  if (targeted.convexity == BlendConvexity::Unknown) {
    out.reason = "BLEND_CONVEXITY_UNKNOWN: the blend face has no orientable radial normal";
    return out;
  }

  // `Proved` implies `support_faces.size() == 2` and an exact `GeomAbs_Cylinder`
  // (`recognize_blend_at` computes the proof only for two supports, and
  // `analytic_cylinder_blend` returns `Unknown` for a non-cylinder).
  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(body, TopAbs_FACE, faces);
  const TopoDS_Face candidate = TopoDS::Face(faces(faces.FindIndex(face)));

  // Built locally and published only on success, so a refused certification never
  // hands back a half-filled blend a caller could act on.
  RecognizedBlend blend;
  blend.face = candidate;
  blend.support_a = targeted.support_faces[0];
  blend.support_b = targeted.support_faces[1];
  blend.convexity = targeted.convexity;
  // The EXACT radius, not the sampled mean. See `RecognizedBlend::radius`.
  const gp_Cylinder cylinder = BRepAdaptor_Surface(candidate).Cylinder();
  blend.radius = cylinder.Radius();

  // ── Layer 2: re-measure at the raised budget ──────────────────────────────
  out.evidence = measure_blend_evidence(body, {candidate}, {blend.support_a, blend.support_b},
                                        blend.radius, kReconstructionBudget);
  const double profile_limit =
      fillet_section_radius_limit(blend.radius, out.evidence.coordinate_magnitude);
  const double tangency_limit =
      fillet_tangency_limit(blend.radius, out.evidence.coordinate_magnitude);
  if (out.evidence.samples < kMinimumReconstructionSamples || out.evidence.boundaries != 2 ||
      out.evidence.maximum_profile_error > profile_limit ||
      out.evidence.maximum_tangency_radians > tangency_limit) {
    out.reason = "BLEND_EVIDENCE_INSUFFICIENT: samples=" + std::to_string(out.evidence.samples) +
                 " (need " + std::to_string(kMinimumReconstructionSamples) +
                 "), boundaries=" + std::to_string(out.evidence.boundaries) +
                 " (need 2), profileError=" + num(out.evidence.maximum_profile_error) + "/" +
                 num(profile_limit) + ", tangency=" + num(out.evidence.maximum_tangency_radians) +
                 "/" + num(tangency_limit);
    return out;
  }

  // ── Layer 3: the trim/extent proof ────────────────────────────────────────
  // Both supports touch the blend along exactly one straight edge parallel to the
  // blend axis, and the two are the same length. That length L is the extent
  // suppression is allowed to expose; postcondition 7 holds the exposed sharp edge
  // to it, which is what refuses a partially-trimmed round instead of silently
  // rounding an edge the user never rounded.
  const validation::GeometryPrecisionContext precision = validation::precision_of(body);
  const TopoDS_Face *supports[2] = {&blend.support_a, &blend.support_b};
  double lengths[2] = {0.0, 0.0};
  for (int i = 0; i < 2; ++i) {
    const std::vector<TopoDS_Edge> shared = shared_edges(candidate, *supports[i]);
    if (shared.size() != 1) {
      out.reason = "BLEND_BOUNDARY_NOT_SIMPLE: support " + std::to_string(i) + " shares " +
                   std::to_string(shared.size()) + " edges with the blend face";
      return out;
    }
    if (!has_curve(shared.front())) {
      out.reason = "BLEND_BOUNDARY_NOT_LINEAR: boundary edge " + std::to_string(i) +
                   " has no 3D curve";
      return out;
    }
    const BRepAdaptor_Curve curve(shared.front());
    if (curve.GetType() != GeomAbs_Line) {
      out.reason = "BLEND_BOUNDARY_NOT_LINEAR: boundary edge " + std::to_string(i) +
                   " is not a line";
      return out;
    }
    if (!curve.Line().Direction().IsParallel(cylinder.Axis().Direction(),
                                             precision.semantic_angular())) {
      out.reason = "BLEND_BOUNDARY_NOT_LINEAR: boundary edge " + std::to_string(i) +
                   " is not parallel to the blend axis";
      return out;
    }
    lengths[i] = edge_length(shared.front());
  }
  if (std::abs(lengths[0] - lengths[1]) > precision.semantic_length()) {
    out.reason = "BLEND_BOUNDARY_LENGTH_MISMATCH: " + num(lengths[0]) + " mm vs " +
                 num(lengths[1]) + " mm";
    return out;
  }
  blend.boundary_length = 0.5 * (lengths[0] + lengths[1]);
  out.blend = std::move(blend);
  out.ok = true;
  return out;
}

SuppressionResult suppress_blends(const TopoDS_Shape &body,
                                  const std::vector<RecognizedBlend> &blends) {
  SuppressionResult out;
  if (body.IsNull() || blends.empty()) {
    out.reason = "SUPPRESSION_INPUT_INVALID: null body or empty blend set";
    return out;
  }

  const validation::GeometryPrecisionContext precision = validation::precision_of(body);
  TopTools_IndexedMapOfShape input_faces;
  TopExp::MapShapes(body, TopAbs_FACE, input_faces);
  const BlendConvexity convexity = blends.front().convexity;
  const double radius = blends.front().radius;
  const double radius_limit =
      fillet_section_radius_limit(radius, precision.coordinate_magnitude);
  TopTools_ListOfShape to_remove;
  for (const RecognizedBlend &blend : blends) {
    if (blend.face.IsNull() || blend.support_a.IsNull() || blend.support_b.IsNull() ||
        input_faces.FindIndex(blend.face) == 0 ||
        input_faces.FindIndex(blend.support_a) == 0 ||
        input_faces.FindIndex(blend.support_b) == 0) {
      out.reason = "SUPPRESSION_INPUT_INVALID: a blend face or support does not belong to the body";
      return out;
    }
    if (!(blend.radius > 0.0) || !(blend.boundary_length > 0.0)) {
      out.reason = "SUPPRESSION_INPUT_INVALID: blend radius and boundary length must be positive";
      return out;
    }
    if (blend.convexity == BlendConvexity::Unknown) {
      out.reason = "SUPPRESSION_INPUT_INVALID: blend convexity is Unknown";
      return out;
    }
    if (blend.convexity != convexity) {
      // Postcondition 8 has no single expected direction for a mixed set: one
      // blend adding material and another removing it can cancel to any sign.
      out.reason = "SUPPRESSION_INPUT_INVALID: mixed blend convexity has no single expected "
                   "volume direction";
      return out;
    }
    if (std::abs(blend.radius - radius) > radius_limit) {
      // The SAME shape of hole as mixed convexity, and it is the more dangerous of
      // the two because NONE of the eight postconditions below reads a radius: two
      // rounds of different radii and the same convexity pass every one of them.
      // The rebuild is a single-radius fillet, so a mixed set would come back with
      // one of its blends silently re-cut to the other's radius — a reconstruction
      // that differs from the original, which is the one outcome this module's
      // safety argument does not allow.
      out.reason = "SUPPRESSION_INPUT_INVALID: mixed blend radii (" + num(radius) + " mm and " +
                   num(blend.radius) + " mm) cannot be rebuilt by one single-radius fillet";
      return out;
    }
    to_remove.Append(blend.face);
  }

  BRepAlgoAPI_Defeaturing defeature;
  defeature.SetShape(body);
  defeature.AddFacesToRemove(to_remove);
  defeature.SetToFillHistory(Standard_True);
  // A destructive geometric decision must not depend on thread scheduling.
  defeature.SetRunParallel(Standard_False);
  try {
    defeature.Build();
  } catch (const Standard_Failure &failure) {
    const char *message = failure.GetMessageString();
    out.reason = std::string("SUPPRESSION_NOT_DONE: ") +
                 (message ? message : "OCCT defeaturing threw");
    return out;
  }

  // ── Postcondition 1 ───────────────────────────────────────────────────────
  // MEASURED OCCT BEHAVIOUR (8.0.1): when `BOPAlgo_RemoveFeatures` cannot remove a
  // requested face it does NOT fail. It reports `IsDone()`, returns the input body
  // unchanged, and records the refusal as a warning alert. `IsDone()` alone is
  // therefore not evidence that anything happened — the face-count postcondition
  // would catch the no-op, but a warning is the kernel saying so in its own words,
  // and a destructive path should quote it rather than infer it.
  if (!defeature.IsDone() || defeature.Shape().IsNull()) {
    out.reason = "SUPPRESSION_NOT_DONE: defeaturing did not complete: " +
                 kernel_alerts(defeature);
    return out;
  }
  if (defeature.HasWarnings()) {
    // Deliberately generic. A warning is the kernel declining to vouch for the run;
    // WHICH reservation it has is `kernel_alerts`' job to say, and inventing a
    // specific claim here would be this module asserting something it did not
    // measure. `BOPAlgo_AlertUnableToRemoveTheFeature` is the one observed on this
    // corpus, but it is not the only alert the algorithm can raise.
    out.reason = "SUPPRESSION_NOT_DONE: the kernel raised an alert on the removal: " +
                 kernel_alerts(defeature);
    return out;
  }
  const TopoDS_Shape suppressed = defeature.Shape();
  // Ingested over the input body's sub-shapes, which is the form
  // `elementmap::ComposedHistory::add_stage` consumes.
  const occ::handle<BRepTools_History> history =
      new BRepTools_History(one(body), defeature);

  out.input_volume = shape_volume(body);
  out.volume = shape_volume(suppressed);

  // ── Postcondition 2 ───────────────────────────────────────────────────────
  const validation::ShapeAuditResult input_audit = validation::audit_shape(body);
  const validation::ShapeAuditResult audit = validation::audit_shape(suppressed);
  out.maximum_tolerance = audit.tolerances.maximum();
  validation::PublicationPolicy policy = validation::single_solid_policy(
      "Blend suppression", validation::PublicationTier::TierB);
  // Same growth budget Fillet publishes under, from the same base. Suppression is
  // half a round trip; letting it inflate further than a fillet does would make the
  // characterized round-trip cap meaningless.
  policy.maximum_tolerance =
      precision.tolerance_ceiling(input_audit.tolerances.maximum(), 2.0, 1.0e-6);
  const validation::PublicationDecision decision =
      validation::evaluate_publication_policy(audit, policy);
  if (!decision.publishable()) {
    out.reason = "SUPPRESSION_NOT_PUBLISHABLE: " + decision.reason_code + ": " + decision.message;
    return out;
  }
  // The CHARACTERIZED cap, enforced. `policy.maximum_tolerance` above is the
  // generic Fillet growth budget and lands near 1.2e-3 on a body at
  // `Precision::Confusion()` — three orders looser than what this round trip was
  // actually measured at, so it would never fire. A cap nothing is held to is
  // documentation, not a cap.
  out.input_tolerance = input_audit.tolerances.maximum();
  if (out.maximum_tolerance - out.input_tolerance > kRoundTripToleranceCapMm) {
    out.reason = "SUPPRESSION_TOLERANCE_CAP: suppression added " +
                 num(out.maximum_tolerance - out.input_tolerance) + " mm of B-Rep tolerance (" +
                 num(out.input_tolerance) + " -> " + num(out.maximum_tolerance) +
                 "), over the characterized cap " + num(kRoundTripToleranceCapMm) + " mm";
    return out;
  }

  // ── Postcondition 3 ───────────────────────────────────────────────────────
  TopTools_IndexedMapOfShape result_faces;
  TopExp::MapShapes(suppressed, TopAbs_FACE, result_faces);
  const int expected_faces = input_faces.Extent() - static_cast<int>(blends.size());
  if (result_faces.Extent() != expected_faces) {
    out.reason = "SUPPRESSION_FACE_COUNT: expected " + std::to_string(expected_faces) +
                 " faces (" + std::to_string(input_faces.Extent()) + " - " +
                 std::to_string(blends.size()) + "), got " +
                 std::to_string(result_faces.Extent());
    return out;
  }

  // ── Postcondition 4 ───────────────────────────────────────────────────────
  // Every surviving face traces back to exactly one input face. Together with
  // postcondition 3 that is a bijection from the non-blend input faces onto the
  // result, so nothing was split, merged or invented.
  const auto is_blend_face = [&blends](const TopoDS_Shape &shape) {
    return std::any_of(blends.begin(), blends.end(), [&shape](const RecognizedBlend &blend) {
      return blend.face.IsSame(shape);
    });
  };
  std::vector<int> ancestors(static_cast<std::size_t>(result_faces.Extent()) + 1, 0);
  std::map<int, TopoDS_Face> image_of_input;
  for (int i = 1; i <= input_faces.Extent(); ++i) {
    const TopoDS_Face input_face = TopoDS::Face(input_faces(i));
    if (is_blend_face(input_face))
      continue;
    const std::vector<TopoDS_Face> images = face_images(*history, input_face, result_faces);
    if (images.size() == 1)
      image_of_input.emplace(i, images.front());
    for (const TopoDS_Face &image : images)
      ++ancestors[static_cast<std::size_t>(result_faces.FindIndex(image))];
  }
  for (int i = 1; i <= result_faces.Extent(); ++i) {
    if (ancestors[static_cast<std::size_t>(i)] != 1) {
      out.reason = "SUPPRESSION_ANCESTRY_NOT_UNIQUE: result face " + std::to_string(i) +
                   " has " + std::to_string(ancestors[static_cast<std::size_t>(i)]) +
                   " input-face ancestors";
      return out;
    }
  }

  // ── Postcondition 5 ───────────────────────────────────────────────────────
  for (const RecognizedBlend &blend : blends) {
    if (!history->IsRemoved(blend.face) || result_faces.FindIndex(blend.face) != 0) {
      out.reason = "SUPPRESSION_BLEND_SURVIVED: a suppressed blend face is not reported deleted";
      return out;
    }
  }

  // ── Postconditions 6 and 7, per blend ─────────────────────────────────────
  TopTools_IndexedMapOfShape input_edges;
  TopExp::MapShapes(body, TopAbs_EDGE, input_edges);
  std::vector<SuppressedBlendSeed> seeds;
  for (const RecognizedBlend &blend : blends) {
    const TopoDS_Face *supports[2] = {&blend.support_a, &blend.support_b};
    TopoDS_Face support_image[2];
    for (int i = 0; i < 2; ++i) {
      const auto found = image_of_input.find(input_faces.FindIndex(*supports[i]));
      if (found == image_of_input.end()) {
        out.reason = "SUPPRESSION_ANCESTRY_NOT_UNIQUE: blend support " + std::to_string(i) +
                     " has no unique image in the suppressed body";
        return out;
      }
      support_image[i] = found->second;
      std::string detail;
      if (!same_surface(*supports[i], support_image[i], precision, detail)) {
        out.reason = "SUPPRESSION_SUPPORT_SURFACE_DRIFT: " + detail;
        return out;
      }
    }

    // The seed is a NEW edge — the supports were re-trimmed to meet, so the edge
    // where they now meet cannot be one the input already had. Requiring newness is
    // what stops a pre-existing sharp edge between the same two supports (the
    // partially-trimmed round) from being adopted as the seed.
    std::vector<TopoDS_Edge> candidates;
    for (const TopoDS_Edge &edge : shared_edges(support_image[0], support_image[1])) {
      if (input_edges.FindIndex(edge) != 0 || !has_curve(edge))
        continue;
      if (BRepAdaptor_Curve(edge).GetType() != GeomAbs_Line)
        continue;
      if (std::abs(edge_length(edge) - blend.boundary_length) > precision.semantic_length())
        continue;
      candidates.push_back(edge);
    }
    if (candidates.size() != 1) {
      out.reason = std::string(candidates.empty() ? "SUPPRESSION_SEED_EDGE_NOT_FOUND"
                                                  : "SUPPRESSION_SEED_EDGE_AMBIGUOUS") +
                   ": " + std::to_string(candidates.size()) +
                   " new straight edges of length " + num(blend.boundary_length) +
                   " mm are shared by the two support images";
      return out;
    }
    if (std::any_of(seeds.begin(), seeds.end(), [&candidates](const SuppressedBlendSeed &seed) {
          return seed.edge.IsSame(candidates.front());
        })) {
      out.reason = "SUPPRESSION_SEED_EDGE_AMBIGUOUS: two blends resolved to the same seed edge";
      return out;
    }
    seeds.push_back({candidates.front(), support_image[0], support_image[1],
                     blend.boundary_length, blend.radius, blend.convexity});
  }

  // ── Postcondition 8 ───────────────────────────────────────────────────────
  // DERIVATION, not a guess. `blend_convexity` reports Convex when the face's
  // outward normal points AWAY from the cylinder axis, i.e. the axis lies inside
  // the material: the rolling ball ran along the OUTSIDE of a convex edge, and the
  // rounded body is the sharp body MINUS the region between the sharp corner and
  // the arc. Restoring the sharp corner therefore ADDS exactly that region, so
  // suppressing a convex round must INCREASE volume. Concave is the mirror: the
  // normal points toward the axis, the axis lies outside the material, the fillet
  // FILLED the reentrant notch, and suppressing it must REMOVE that fill.
  //
  // (Both directions are the same magnitude for a 90-degree corner of radius R and
  // run L: L * (R^2 - pi*R^2/4). `test_blend_reconstruction.cpp` asserts the exact
  // number in both signs.)
  //
  // KNOWN REFUSAL BOUNDARY at the low extremity. `minimum_volume()` is
  // `authoring_resolution()` cubed = 1e-9 mm^3, and the corner a blend moves is
  // L*(R^2 - pi*R^2/4). At the smallest radius Fillet will build at all
  // (`valid_constant_radius`, R = 0.001 mm) over a 1 mm run that is ~2.1e-10 mm^3,
  // UNDER the floor — so suppressing a legitimate 0.001 mm blend refuses here. That
  // is the fail-closed direction and it is deliberate: at that scale the volume is
  // no longer evidence of anything, and refusing beats inferring a direction from a
  // number the kernel cannot resolve. It is also unmeasured territory — no fixture
  // in this corpus reaches it, so the boundary is stated, not characterized.
  const double delta = out.volume - out.input_volume;
  const double floor = precision.minimum_volume();
  const bool expect_gain = convexity == BlendConvexity::Convex;
  if ((expect_gain && delta < floor) || (!expect_gain && delta > -floor)) {
    out.reason = std::string("SUPPRESSION_VOLUME_DIRECTION: suppressing a ") +
                 (expect_gain ? "convex round must ADD material"
                              : "concave fillet must REMOVE material") +
                 ", but volume moved by " + num(delta) + " mm^3";
    return out;
  }

  out.ok = true;
  out.suppressed_shape = suppressed;
  out.history = history;
  out.seed_edges = std::move(seeds);
  return out;
}

ReblendResult reblend(const TopoDS_Shape &suppressed_shape,
                      const std::vector<SuppressedBlendSeed> &seeds, double radius) {
  ReblendResult out;
  if (suppressed_shape.IsNull() || seeds.empty()) {
    out.reason = "REBLEND_INPUT_INVALID: null body or empty seed set";
    return out;
  }

  // The seed's own radius is the authority on what it was. `radius` is a parameter
  // because one fillet operation carries one radius, and holding it to the seed is
  // what stops an R3 round being rebuilt as an R2 one: every rung of the
  // reproduction ladder below would pass on that rebuild, because each rung asks
  // whether the new face is a good blend of `radius`, not whether `radius` is the
  // radius that was destroyed.
  const validation::GeometryPrecisionContext input_precision =
      validation::precision_of(suppressed_shape);
  const double seed_radius_limit =
      fillet_section_radius_limit(radius, input_precision.coordinate_magnitude);

  std::vector<ResolvedEdge> requested;
  for (std::size_t i = 0; i < seeds.size(); ++i) {
    if (seeds[i].edge.IsNull() || seeds[i].support_a.IsNull() || seeds[i].support_b.IsNull()) {
      out.reason = "REBLEND_INPUT_INVALID: seed " + std::to_string(i) + " is incomplete";
      return out;
    }
    if (std::abs(seeds[i].radius - radius) > seed_radius_limit) {
      out.reason = "REBLEND_INPUT_INVALID: seed " + std::to_string(i) + " was an R" +
                   num(seeds[i].radius) + " blend, the rebuild was asked for R" + num(radius);
      return out;
    }
    ResolvedEdge resolved;
    resolved.element_id = "seed_" + std::to_string(i);
    resolved.edge = seeds[i].edge;
    requested.push_back(std::move(resolved));
  }

  // `FilletBuilder` brings contour analysis, radius-law validation, the partial-
  // result probe, the TierB publication decision and its own measured radius /
  // tangency verification. The reproduction proof below is what it does NOT do:
  // prove the rebuilt face is THE blend that was suppressed.
  FilletBuilder builder(suppressed_shape, requested, radius);
  const FilletBuildResult built = builder.build();
  if (!built.ok) {
    out.reason = "REBLEND_FAILED: " + built.message;
    return out;
  }
  out.maximum_tolerance = built.output_audit.tolerances.maximum();
  out.input_tolerance = built.input_audit.tolerances.maximum();
  // The second half of the characterized round-trip cap. `FilletBuilder` publishes
  // under its own generic growth budget, which is orders looser than what this round
  // trip measures at; this is the tighter number and it is the one enforced.
  if (out.maximum_tolerance - out.input_tolerance > kRoundTripToleranceCapMm) {
    out.reason = "REBLEND_TOLERANCE_CAP: the rebuild added " +
                 num(out.maximum_tolerance - out.input_tolerance) + " mm of B-Rep tolerance (" +
                 num(out.input_tolerance) + " -> " + num(out.maximum_tolerance) +
                 "), over the characterized cap " + num(kRoundTripToleranceCapMm) + " mm";
    return out;
  }
  const TopoDS_Shape shape = built.shape;
  const occ::handle<BRepTools_History> history =
      new BRepTools_History(one(suppressed_shape), builder.history());

  TopTools_IndexedMapOfShape result_faces;
  TopExp::MapShapes(shape, TopAbs_FACE, result_faces);
  const validation::GeometryPrecisionContext precision = validation::precision_of(shape);
  const double radius_limit =
      fillet_section_radius_limit(radius, precision.coordinate_magnitude);

  std::vector<TopoDS_Face> blend_faces;
  std::vector<TopoDS_Face> support_faces;
  for (std::size_t i = 0; i < seeds.size(); ++i) {
    const SuppressedBlendSeed &seed = seeds[i];
    const std::string at = " (seed " + std::to_string(i) + ")";

    std::vector<TopoDS_Face> generated;
    for (const TopoDS_Shape &successor : history->Generated(seed.edge)) {
      if (successor.ShapeType() != TopAbs_FACE)
        continue;
      const TopoDS_Face image = in_result(result_faces, successor);
      if (!image.IsNull())
        generated.push_back(image);
    }
    if (generated.size() != 1) {
      out.reason = "REBLEND_NOT_GENERATED: the seed edge generated " +
                   std::to_string(generated.size()) + " faces present in the result" + at;
      return out;
    }
    const TopoDS_Face face = generated.front();
    if (contains_same(blend_faces, face)) {
      out.reason = "REBLEND_FACE_NOT_UNIQUE: two seeds generated the same blend face" + at;
      return out;
    }

    TopoDS_Face support_image[2];
    const TopoDS_Face *supports[2] = {&seed.support_a, &seed.support_b};
    for (int s = 0; s < 2; ++s) {
      const std::vector<TopoDS_Face> images = face_images(*history, *supports[s], result_faces);
      if (images.size() != 1) {
        out.reason = "REBLEND_SUPPORTS_MISMATCH: support " + std::to_string(s) + " has " +
                     std::to_string(images.size()) + " images in the result" + at;
        return out;
      }
      support_image[s] = images.front();
    }

    const BRepAdaptor_Surface surface(face);
    if (surface.GetType() != GeomAbs_Cylinder) {
      out.reason = "REBLEND_NOT_CYLINDER: the rebuilt face is not an exact cylinder" + at;
      return out;
    }
    const double built_radius = surface.Cylinder().Radius();
    if (std::abs(built_radius - radius) > radius_limit) {
      out.reason = "REBLEND_RADIUS: rebuilt radius " + num(built_radius) + " mm differs from " +
                   num(radius) + " mm by more than " + num(radius_limit) + " mm" + at;
      return out;
    }

    const TargetedBlendRecognition targeted = recognize_blend_at(face, shape);
    if (!targeted.recognized) {
      out.reason = "REBLEND_NOT_RECOGNIZED: " + targeted.recognition.reason + at;
      return out;
    }
    if (targeted.support_faces.size() != 2 ||
        !contains_same(targeted.support_faces, support_image[0]) ||
        !contains_same(targeted.support_faces, support_image[1])) {
      out.reason = "REBLEND_SUPPORTS_MISMATCH: the rebuilt blend is not bounded by the two "
                   "tracked support images" +
                   at;
      return out;
    }
    const AnalyticBlendProof proof =
        analytic_cylinder_blend(face, support_image[0], support_image[1]);
    if (proof != AnalyticBlendProof::Proved) {
      out.reason = std::string("REBLEND_NOT_PROVED: analytic tangency against the tracked "
                               "support images is ") +
                   proof_name(proof) + at;
      return out;
    }
    // A round and a fillet of the same radius on the same support pair are tangent
    // in exactly the same way, so the analytic proof above cannot separate them —
    // only the material side can, and that is what convexity reads.
    const BlendConvexity rebuilt_convexity = blend_convexity(face);
    if (rebuilt_convexity != seed.convexity) {
      out.reason = std::string("REBLEND_CONVEXITY: the rebuilt blend is ") +
                   convexity_name(rebuilt_convexity) + ", the suppressed one was " +
                   convexity_name(seed.convexity) + at;
      return out;
    }

    for (int s = 0; s < 2; ++s) {
      const std::vector<TopoDS_Edge> shared = shared_edges(face, support_image[s]);
      if (shared.size() != 1) {
        out.reason = "REBLEND_BOUNDARY_NOT_SIMPLE: support " + std::to_string(s) + " shares " +
                     std::to_string(shared.size()) + " edges with the rebuilt blend" + at;
        return out;
      }
      const double length = edge_length(shared.front());
      if (std::abs(length - seed.boundary_length) > precision.semantic_length()) {
        out.reason = "REBLEND_BOUNDARY_LENGTH: rebuilt boundary is " + num(length) +
                     " mm, the original was " + num(seed.boundary_length) + " mm" + at;
        return out;
      }
    }

    blend_faces.push_back(face);
    for (int s = 0; s < 2; ++s) {
      if (!contains_same(support_faces, support_image[s]))
        support_faces.push_back(support_image[s]);
    }
  }

  const BlendEvidence evidence =
      measure_blend_evidence(shape, blend_faces, support_faces, radius, kReconstructionBudget);
  const int expected_boundaries = 2 * static_cast<int>(seeds.size());
  const int minimum_samples = kMinimumReconstructionSamples * static_cast<int>(seeds.size());
  const double profile_limit =
      fillet_section_radius_limit(radius, evidence.coordinate_magnitude);
  const double tangency_limit = fillet_tangency_limit(radius, evidence.coordinate_magnitude);
  if (evidence.boundaries != expected_boundaries || evidence.samples < minimum_samples ||
      evidence.maximum_profile_error > profile_limit ||
      evidence.maximum_tangency_radians > tangency_limit) {
    out.reason = "REBLEND_EVIDENCE: boundaries=" + std::to_string(evidence.boundaries) +
                 " (need " + std::to_string(expected_boundaries) +
                 "), samples=" + std::to_string(evidence.samples) + " (need " +
                 std::to_string(minimum_samples) +
                 "), profileError=" + num(evidence.maximum_profile_error) + "/" +
                 num(profile_limit) + ", tangency=" + num(evidence.maximum_tangency_radians) +
                 "/" + num(tangency_limit);
    return out;
  }

  out.ok = true;
  out.shape = shape;
  out.history = history;
  out.blend_faces = std::move(blend_faces);
  out.evidence = evidence;
  out.volume = shape_volume(shape);
  return out;
}

double RoundTripTolerance::inflation() const {
  return input > 0.0 ? after_reblend / input : 0.0;
}

double RoundTripTolerance::contribution() const {
  return after_reblend - input;
}

bool RoundTripTolerance::within_cap() const {
  return contribution() <= kRoundTripToleranceCapMm;
}

RoundTripTolerance roundtrip_tolerance_cap(const TopoDS_Shape &input,
                                           const SuppressionResult &suppressed,
                                           const ReblendResult &reblended) {
  RoundTripTolerance out;
  out.input = validation::audit_shape(input).tolerances.maximum();
  out.after_suppress = suppressed.maximum_tolerance;
  out.after_reblend = reblended.maximum_tolerance;
  return out;
}

} // namespace onecad::kernel::fillet
