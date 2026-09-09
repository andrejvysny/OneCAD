#include "kernel/fillet/FilletSemanticChecks.h"

#include <algorithm>
#include <cmath>
#include <vector>

#include <NCollection_IndexedDataMap.hxx>
#include <NCollection_List.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ShapeMapHasher.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Vertex.hxx>

#include "kernel/validation/GeometryPrecision.h"

namespace onecad::kernel::fillet {

namespace {

using VertexEdges = NCollection_IndexedDataMap<
    TopoDS_Shape, NCollection_List<TopoDS_Shape>, TopTools_ShapeMapHasher>;

// Largest edge count at any END vertex of `contour` — an end being a vertex the
// contour's own edge chain touches exactly once, so a CLOSED contour has none
// and reports 0. The count itself is taken in the INPUT shape, because the
// question the evidence answers is how many edges met at the corner OCCT had to
// blend, not how many survived it.
int contour_end_valence(BRepFilletAPI_MakeFillet &builder, int contour_index,
                        const VertexEdges &input_vertex_edges) {
  TopTools_IndexedMapOfShape vertices;
  std::vector<int> occurrences;
  for (int i = 1; i <= builder.NbEdges(contour_index); ++i) {
    TopoDS_Vertex first;
    TopoDS_Vertex last;
    TopExp::Vertices(builder.Edge(contour_index, i), first, last);
    for (const TopoDS_Vertex &vertex : {first, last}) {
      if (vertex.IsNull())
        continue;
      const int index = vertices.Add(vertex);
      if (static_cast<std::size_t>(index) > occurrences.size())
        occurrences.resize(index, 0);
      ++occurrences[index - 1];
    }
  }
  int maximum = 0;
  for (int i = 1; i <= vertices.Extent(); ++i) {
    if (occurrences[i - 1] != 1)
      continue;
    if (!input_vertex_edges.Contains(vertices(i)))
      continue;
    maximum = std::max(maximum,
                       input_vertex_edges.FindFromKey(vertices(i)).Extent());
  }
  return maximum;
}

// The adaptive budget for one approximated contour, SCHEMA §7.3 rules 2 and 3.
struct AdaptiveBudget {
  double profile = 0.0;
  double tangency = 0.0;
};

AdaptiveBudget adaptive_budget_for(double radius, double tolerance) {
  return {std::min(radius * kRelSection, kSectionCeiling),
          std::min(std::max(kTangencyRadians,
                            kTangencyFactor * tolerance /
                                std::max(radius, 1.0e-9)),
                   kTangencyBudgetCeiling)};
}

// Copy one contour's measurements into the result's reported evidence, so a
// refusal or a warning describes the contour it is actually about rather than
// the whole-result aggregate.
void report_contour(const FilletContourEvidence &contour, double profile_budget,
                    double tangency_budget, FilletSemanticResult &out) {
  out.evidence.blend.maximum_profile_error = contour.blend.maximum_profile_error;
  out.evidence.blend.maximum_tangency_radians =
      contour.blend.maximum_tangency_radians;
  out.evidence.surface_class = contour.surface_class;
  out.evidence.approximation_tolerance = contour.approximation_tolerance;
  out.allowed_profile_error = profile_budget;
  out.allowed_tangency_radians = tangency_budget;
}

} // namespace

// The whole acceptance decision for ONE contour, as a pure function of its
// measured evidence. `judge_contours` is only the loop around this; a unit test
// drives it directly, because constructing geometry that lands on each branch
// (a fit coarser than the authoring resolution, a 1 %-of-radius curvature
// error) is not something OCCT can be asked for on demand.
ContourVerdict judge_contour_evidence(const FilletContourEvidence &contour,
                                      double requested_radius,
                                      double authoring_resolution) {
  ContourVerdict verdict;
  const double radius = std::abs(requested_radius);
  // EXACT FIRST, whatever the representation.
  verdict.allowed_profile_error = fillet_section_radius_limit(
      requested_radius, contour.blend.coordinate_magnitude);
  verdict.allowed_tangency_radians = fillet_tangency_limit(
      requested_radius, contour.blend.coordinate_magnitude);
  if (contour.blend.maximum_profile_error <= verdict.allowed_profile_error &&
      contour.blend.maximum_tangency_radians <= verdict.allowed_tangency_radians)
    return verdict;

  if (contour.surface_class != BlendSurfaceClass::Approximated) {
    // Analytic and outside the exact budgets: refused exactly as before.
    verdict.refusal_code = "FILLET_SEMANTIC_CHECK_FAILED";
    verdict.message =
        contour.blend.maximum_profile_error > verdict.allowed_profile_error
            ? "measured fillet section radius differs from requested radius"
            : "measured fillet boundary is not G1 tangent to its supports";
    return verdict;
  }

  const AdaptiveBudget budget =
      adaptive_budget_for(radius, contour.approximation_tolerance);
  verdict.allowed_profile_error = budget.profile;
  verdict.allowed_tangency_radians = budget.tangency;
  // Normative order: the fit bound, then the section budget, then tangency.
  // Only the first two re-label the refusal.
  if (contour.approximation_tolerance > authoring_resolution) {
    verdict.refusal_code = "FILLET_BLEND_TOO_COARSE";
    verdict.message = "fillet blend was approximated coarser than the authoring resolution";
    return verdict;
  }
  if (contour.blend.maximum_profile_error > budget.profile) {
    verdict.refusal_code = "FILLET_BLEND_TOO_COARSE";
    verdict.message = "measured fillet section radius differs from requested radius";
    return verdict;
  }
  if (contour.blend.maximum_tangency_radians > budget.tangency) {
    verdict.refusal_code = "FILLET_SEMANTIC_CHECK_FAILED";
    verdict.message = "measured fillet boundary is not G1 tangent to its supports";
    return verdict;
  }
  verdict.used_adaptive_budget = true;
  return verdict;
}

// Reached only when the whole result failed the exact budgets. Judges each
// contour on its OWN blend faces: exact budgets first, then — for an
// APPROXIMATED contour only — the adaptive budget.
FilletSemanticResult judge_contours(BRepFilletAPI_MakeFillet &builder,
                                    const FilletAnalysis &analysis,
                                    const TopoDS_Shape &input,
                                    const TopoDS_Shape &output,
                                    double requested_radius,
                                    FilletSemanticResult out) {
  std::vector<int> indices;
  indices.reserve(analysis.contours.size());
  for (const FilletContour &contour : analysis.contours)
    indices.push_back(contour.index);
  const std::vector<FilletContourEvidence> contours =
      collect_fillet_contour_evidence(builder, indices, input, output,
                                      requested_radius);
  const double resolution =
      validation::precision_of(input).authoring_resolution();

  // Counted from zero here: the aggregate collector already filled these from
  // ALL approximated blend faces, but the warning's counts mean "needed the
  // adaptive budget", which only this loop can know.
  out.approximated_contour_count = 0;
  out.evidence.approximated_face_count = 0;

  const FilletContourEvidence *worst = nullptr;
  ContourVerdict worst_verdict;
  for (const FilletContourEvidence &contour : contours) {
    // A contour that measured nothing is not judged here. The aggregate
    // sufficiency gate is what refuses a result with no evidence; splitting it
    // per contour would newly refuse multi-contour ops that publish today.
    if (contour.blend.samples == 0 && contour.blend.boundaries == 0)
      continue;
    const ContourVerdict verdict =
        judge_contour_evidence(contour, requested_radius, resolution);
    if (!verdict.refusal_code.empty()) {
      report_contour(contour, verdict.allowed_profile_error,
                     verdict.allowed_tangency_radians, out);
      out.refusal_code = verdict.refusal_code;
      out.message = verdict.message;
      return out;
    }
    if (!verdict.used_adaptive_budget)
      continue;
    ++out.approximated_contour_count;
    out.evidence.approximated_face_count += contour.approximated_face_count;
    // The step reports the WORST contour that needed the adaptive budget, so
    // one warning describes the least exact blend the step published.
    if (worst == nullptr ||
        contour.blend.maximum_profile_error >
            worst->blend.maximum_profile_error) {
      worst = &contour;
      worst_verdict = verdict;
    }
  }

  if (worst != nullptr) {
    report_contour(*worst, worst_verdict.allowed_profile_error,
                   worst_verdict.allowed_tangency_radians, out);
    out.adaptive_budget_used = true;
  }
  out.ok = true;
  return out;
}

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

// SCHEMA §7.3 "Two blend classes, two budgets": EXACT budgets first for every
// contour whatever its representation; only an APPROXIMATED contour that FAILS
// them is judged by the adaptive budget, and only such a contour makes the step
// warn. Classification, measurement and budget are all PER CONTOUR.
FilletSemanticResult validate_result(
    BRepFilletAPI_MakeFillet &builder, const FilletAnalysis &analysis,
    const TopoDS_Shape &input, const TopoDS_Shape &output,
    const std::vector<ResolvedEdge> &requested, double requested_radius) {
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

    out.evidence = collect_fillet_result_evidence(
        builder, input, output, requested, requested_radius);

    VertexEdges input_vertex_edges;
    TopExp::MapShapesAndAncestors(input, TopAbs_VERTEX, TopAbs_EDGE,
                                  input_vertex_edges);
    for (const FilletContour &contour : analysis.contours) {
      out.max_contour_vertex_valence =
          std::max(out.max_contour_vertex_valence,
                   contour_end_valence(builder, contour.index,
                                       input_vertex_edges));
    }

    out.allowed_profile_error = fillet_section_radius_limit(
        requested_radius, out.evidence.blend.coordinate_magnitude);
    out.allowed_tangency_radians = fillet_tangency_limit(
        requested_radius, out.evidence.blend.coordinate_magnitude);

    if (out.evidence.generated_face_count == 0 ||
        out.evidence.support_face_count < 2 ||
        out.evidence.blend.boundaries < 2 || out.evidence.blend.samples == 0) {
      out.message = "fillet result has insufficient geometric radius/tangency evidence";
      return out;
    }

    // EXACT FIRST. The fast path tests the WHOLE result against the
    // conditioning-FREE exact budgets. Passing that implies every contour
    // passes its own exact budget — an aggregate maximum bounds each contour's
    // maximum, and dropping the conditioning term only shrinks the budget, so
    // no per-contour coordinate magnitude can turn this into a failure. Exact
    // geometry therefore never pays for the second measurement pass, never
    // warns, and its evidence is bit-identical to the pre-WP-G kernel's.
    const double radius = std::abs(requested_radius);
    if (out.evidence.blend.maximum_profile_error <=
            std::max(1.0e-9, radius * kSectionRelative) &&
        out.evidence.blend.maximum_tangency_radians <= kTangencyRadians) {
      out.ok = true;
      return out;
    }
    return judge_contours(builder, analysis, input, output, requested_radius,
                          std::move(out));
  } catch (const Standard_Failure &failure) {
    const char *message = failure.GetMessageString();
    out.message = message ? message : "fillet result query failed";
  } catch (...) {
    out.message = "unknown fillet result query failure";
  }
  return out;
}

} // namespace onecad::kernel::fillet
