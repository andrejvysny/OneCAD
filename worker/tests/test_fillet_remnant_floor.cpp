// SCHEMA §7.6 remnant floor (kernel-hardening WP-G, finding fillet-chamfer-2).
//
// `AnalyzeEdgeOpRange` classifies a probe with the tier a commit uses, and Tier
// B publishes a fillet that BUILDS and is a single valid solid. That is not
// enough: the search refines a bracket only until it is narrower than the
// authoring resolution, so the last SUCCESS probe sits within 1e-3 mm of the
// true frontier — and when the frontier is "the blend consumes a whole adjacent
// face", the leftover land is exactly that distance wide. `bestKnownMax` then
// offers the user a radius whose result carries sub-resolution topology.
//
// The fixture makes that structural, not incidental: a 4.0002 mm-wide end face
// blended from BOTH its corners has its frontier at 2.0001 mm, so any probe in
// [1.9995, 2.0001) leaves a strip under 1e-3 mm — the brief's "two parallel edges
// r + 0.0004 mm apart", built.
//
// The assertions are the CONTRACT, not pinned literals: the pinned numbers live
// in `test_fillet_range.cpp`, which measures the same rule on the box fixtures.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRep_Tool.hxx>
#include <GProp_GProps.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Vertex.hxx>

#include "fillet_test_utils.h"
#include "kernel/fillet/FilletBuilder.h"
#include "kernel/fillet/FilletRangeAnalyzer.h"
#include "kernel/validation/GeometryPrecision.h"

namespace ft = onecad::tests::fillet;
namespace kf = onecad::kernel::fillet;
namespace validation = onecad::kernel::validation;

namespace {

int failures = 0;

void check(bool condition, const std::string &message) {
  if (condition)
    return;
  ++failures;
  std::fprintf(stderr, "FAIL: %s\n", message.c_str());
}

// An INDEPENDENT re-measurement of the rule, so the test is not simply asking
// the production code whether it agrees with itself.
struct Smallest {
  double face_area = -1.0;
  double edge_length = -1.0;
};

Smallest smallest_topology(const TopoDS_Shape &shape) {
  Smallest out;
  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(shape, TopAbs_FACE, faces);
  for (int i = 1; i <= faces.Extent(); ++i) {
    GProp_GProps properties;
    BRepGProp::SurfaceProperties(TopoDS::Face(faces(i)), properties);
    if (out.face_area < 0.0 || properties.Mass() < out.face_area)
      out.face_area = properties.Mass();
  }
  TopTools_IndexedMapOfShape edges;
  TopExp::MapShapes(shape, TopAbs_EDGE, edges);
  for (int i = 1; i <= edges.Extent(); ++i) {
    const TopoDS_Edge edge = TopoDS::Edge(edges(i));
    if (BRep_Tool::Degenerated(edge))
      continue;
    GProp_GProps properties;
    BRepGProp::LinearProperties(edge, properties);
    if (out.edge_length < 0.0 || properties.Mass() < out.edge_length)
      out.edge_length = properties.Mass();
  }
  return out;
}

// A 20 x 4.0002 x 10 bar. Its +X end face is 4.0002 mm wide; the two vertical
// edges bounding that face roll a blend onto it from opposite corners, so they
// meet — and the face vanishes — at r = 2.0001 mm.
TopoDS_Shape bar() {
  return BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 20.0, 4.0002, 10.0).Shape();
}

std::vector<TopoDS_Edge> end_face_vertical_edges(const TopoDS_Shape &body) {
  std::vector<TopoDS_Edge> picked;
  for (const TopoDS_Edge &edge : ft::vertical_edges(body)) {
    TopoDS_Vertex first;
    TopoDS_Vertex last;
    TopExp::Vertices(edge, first, last);
    if (first.IsNull())
      continue;
    if (std::abs(BRep_Tool::Pnt(first).X() - 20.0) > 1.0e-9)
      continue;
    picked.push_back(edge);
  }
  return picked;
}

void remnant_floor_bounds_the_range() {
  const TopoDS_Shape body = bar();
  const std::vector<TopoDS_Edge> edges = end_face_vertical_edges(body);
  check(edges.size() == 2,
        "fixture: exactly the two vertical edges of the 4.0002 mm end face");
  if (edges.size() != 2)
    return;

  const std::vector<kf::ResolvedEdge> resolved = ft::resolved(body, edges);
  const kf::FilletRangeResult result =
      kf::FilletRangeAnalyzer(body, resolved, kf::EdgeOpMode::Fillet).analyze();
  const double resolution =
      validation::precision_of(body).authoring_resolution();

  std::fprintf(stderr,
               "REMNANT bar/end-face probes=%d bestMax=%.17g provenUpper=%.17g "
               "hit=%d kind=%s value=%.17g radius=%.17g res=%g\n",
               result.probes_used, result.best_known_max,
               result.proven_upper_bound, result.remnant_floor_hit ? 1 : 0,
               result.remnant_floor_kind.c_str(), result.remnant_floor_value,
               result.remnant_floor_radius, resolution);

  check(result.remnant_floor_hit,
        "the analyzer reports remnantFloorHit for this fixture");
  if (!result.remnant_floor_hit)
    return;
  check(result.remnant_floor_kind == "face" || result.remnant_floor_kind == "edge",
        "remnantFloorMeasure.kind is \"face\" or \"edge\", got \"" +
            result.remnant_floor_kind + "\"");
  const double floor_limit = result.remnant_floor_kind == "face"
                                 ? resolution * resolution
                                 : resolution;
  check(result.remnant_floor_value < floor_limit,
        "remnantFloorMeasure.value is below the authoring floor");
  check(result.remnant_floor_radius > 0.0,
        "remnantFloorMeasure.radius names the probe that was classified");

  // THE point of the rule: the reported max is the last CLEAN probe, strictly
  // below the radius that produced the remnant.
  check(result.has_best_known_max,
        "the search still found a feasible radius below the remnant");
  check(result.best_known_max < result.remnant_floor_radius,
        "bestKnownMax never advanced onto the remnant probe");
  check(result.contiguous_success_max <= result.best_known_max,
        "the remnant probe never joined a feasible interval");

  // §7.6 ordering is unchanged by the new classification.
  check(result.has_proven_upper_bound &&
            result.best_known_max < result.proven_upper_bound,
        "bestKnownMax < provenUpperBound still holds");
  check(result.proven_upper_bound <= result.remnant_floor_radius,
        "the remnant probe is a provenUpperBound candidate like any non-success");

  // ATTRIBUTION. A remnant probe has no OCCT diagnostic behind it — OCCT built
  // the thing happily — so without an explicit attribution `limitingEntities`
  // would be `[]` on a remnant-bounded run and the user would be told the range
  // ends here but not which edge ends it. The requested closure IS the answer.
  check(result.limiting.diagnostic_code == "FILLET_REMNANT_FLOOR",
        "the bounding refusal is named FILLET_REMNANT_FLOOR, got \"" +
            result.limiting.diagnostic_code + "\"");
  check(result.limiting.probe_value == result.remnant_floor_radius,
        "the limiting evidence is the remnant probe's own value");
  check(result.limiting.edge_topo_keys.size() == resolved.size(),
        "limiting.edgeTopoKeys names every requested edge (" +
            std::to_string(result.limiting.edge_topo_keys.size()) + " of " +
            std::to_string(resolved.size()) + ")");
  check(result.limiting.element_ids.size() == resolved.size(),
        "limiting.elementIds names every requested edge");
  check(!result.limiting.empty(),
        "a remnant-bounded run still attributes the bound");

  // The remnant probe is the one non-success `FilletBuilder` alone cannot see:
  // it BUILDS and publishes. If it ever stops building, this fixture has
  // stopped testing the floor and is testing an ordinary refusal instead.
  const kf::FilletBuildResult at_remnant =
      kf::FilletBuilder(body, resolved, result.remnant_floor_radius).build();
  check(at_remnant.ok,
        "the remnant probe BUILDS — only the resolution floor rejected it");

  const kf::FilletBuildResult at_max =
      kf::FilletBuilder(body, resolved, result.best_known_max).build();
  check(at_max.ok, "the reported max builds");
  if (!at_max.ok)
    return;
  const Smallest clean = smallest_topology(at_max.shape);
  std::fprintf(stderr,
               "REMNANT bar/end-face at bestMax: smallestFaceArea=%.17g "
               "smallestEdgeLength=%.17g\n",
               clean.face_area, clean.edge_length);
  check(clean.face_area >= resolution * resolution,
        "the reported max leaves no face below res^2");
  check(clean.edge_length >= resolution,
        "the reported max leaves no edge below res");
}

// A body that ALREADY carries a sub-resolution edge, 25 mm from the edge being
// filleted: two 50 x 25 slabs fused, the second 0.0005 mm taller, so the step at
// y = 25 has 5e-4 mm vertical edges. Scanning the whole result for remnants
// makes EVERY probe a non-success here and the verb reports `confidence: none`
// on geometry whose range is perfectly well defined. The scan must see only the
// faces the probe generated or modified.
TopoDS_Shape stepped_slab() {
  const TopoDS_Shape lower =
      BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 50.0, 25.0, 10.0).Shape();
  const TopoDS_Shape upper =
      BRepPrimAPI_MakeBox(gp_Pnt(0, 25, 0), 50.0, 25.0, 10.0005).Shape();
  return BRepAlgoAPI_Fuse(lower, upper).Shape();
}

void pre_existing_sliver_is_not_a_remnant() {
  const TopoDS_Shape body = stepped_slab();
  const double resolution =
      validation::precision_of(body).authoring_resolution();

  // The fixture only means something if the input really does carry a
  // sub-resolution edge; otherwise this test would pass vacuously.
  const Smallest input_topology = smallest_topology(body);
  check(input_topology.edge_length < resolution,
        "fixture: the INPUT already carries a sub-resolution edge (" +
            std::to_string(input_topology.edge_length) + " mm)");

  // A vertical edge at the far corner (x = 0, y = 0) — 25 mm from the step.
  std::vector<TopoDS_Edge> picked;
  for (const TopoDS_Edge &edge : ft::vertical_edges(body)) {
    TopoDS_Vertex first;
    TopoDS_Vertex last;
    TopExp::Vertices(edge, first, last);
    if (first.IsNull())
      continue;
    const gp_Pnt point = BRep_Tool::Pnt(first);
    if (std::abs(point.X()) > 1.0e-9 || std::abs(point.Y()) > 1.0e-9)
      continue;
    picked.push_back(edge);
  }
  check(picked.size() == 1,
        "fixture: exactly one vertical edge at the far corner, got " +
            std::to_string(picked.size()));
  if (picked.size() != 1)
    return;

  const std::vector<kf::ResolvedEdge> resolved = ft::resolved(body, picked);
  const kf::FilletRangeResult result =
      kf::FilletRangeAnalyzer(body, resolved, kf::EdgeOpMode::Fillet).analyze();
  std::fprintf(stderr,
               "REMNANT slab/far-corner probes=%d ok=%d refuse=%d bestMax=%.17g "
               "hit=%d inputSmallestEdge=%.17g\n",
               result.probes_used, result.success_probes, result.refusal_probes,
               result.best_known_max, result.remnant_floor_hit ? 1 : 0,
               input_topology.edge_length);

  // THE regression guard. With a whole-result scan the pre-existing edge is in
  // EVERY probe's output, so even the smallest probe is a non-success and the
  // verb reports no range at all (measured before the fix: 0 successes,
  // `confidence: none`). The smallest probe succeeding is exactly the claim
  // that the scan ignored topology the blend never touched.
  check(result.success_probes > 0,
        "the search still finds feasible radii (got " +
            std::to_string(result.success_probes) + " successes)");
  check(result.has_lower_bound && result.lower_bound == resolution,
        "the SMALLEST probe still succeeds — a 1e-3 mm fillet cannot be blamed "
        "for a 5e-4 mm edge 25 mm away");
  check(result.has_best_known_max && result.best_known_max > 1.0,
        "the analyzer still reports a usable range, got " +
            std::to_string(result.best_known_max));

  // The floor may still fire at the TOP of the range, where the blend consumes
  // the 25 mm face and leaves its OWN leftover. What it must never do is
  // attribute the input's sliver: any remnant reported here belongs to the
  // frontier, not to the step at y = 25.
  if (result.remnant_floor_hit) {
    check(result.remnant_floor_radius == result.proven_upper_bound,
          "a remnant here is the frontier's own leftover, not the input's "
          "sliver");
    check(result.remnant_floor_value != input_topology.edge_length,
          "the reported remnant is not the pre-existing 5e-4 mm edge");
  }
}

} // namespace

int main() {
  try {
    remnant_floor_bounds_the_range();
    pre_existing_sliver_is_not_a_remnant();
  } catch (const Standard_Failure &failure) {
    const char *message = failure.GetMessageString();
    ++failures;
    std::fprintf(stderr, "FAIL: setup threw: %s\n", message ? message : "OCCT");
  }
  if (failures > 0)
    std::fprintf(stderr, "%d assertion(s) failed\n", failures);
  return failures == 0 ? 0 : 1;
}
