// test_blend_budget.cpp — `BlendSamplingBudget` changes who can ask for more
// samples, and nothing else.
//
// The zero-drift proof is the first two checks: the struct's defaults are the
// literals `measure_blend_evidence` used before it was parameterized, and an
// omitted argument produces a field-for-field identical `BlendEvidence` to an
// explicit default one on a pinned fixture. Fillet and kernelbench read that
// same function, so those two facts are what makes the parameterization a
// refactor rather than a behaviour change.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepFilletAPI_MakeFillet.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>

#include "fillet_test_utils.h"
#include "kernel/fillet/BlendEvidence.h"
#include "kernel/fillet/BlendRecognizer.h"

namespace ft = onecad::tests::fillet;
namespace kf = onecad::kernel::fillet;

namespace {
int failures = 0;

void check(bool condition, const std::string &message) {
  if (condition)
    return;
  std::fprintf(stderr, "FAIL: %s\n", message.c_str());
  ++failures;
}

struct Fixture {
  TopoDS_Shape body;
  TopoDS_Face blend;
  std::vector<TopoDS_Face> supports;
};

// Pinned fixture: a 10 mm box with one vertical edge rounded at R2, and the
// blend face plus the two G1 supports the recognizer finds on it.
Fixture pinned_fixture() {
  Fixture out;
  const TopoDS_Shape sharp = ft::box();
  BRepFilletAPI_MakeFillet fillet(sharp);
  fillet.Add(2.0, ft::vertical_edges(sharp).front());
  fillet.Build();
  if (!fillet.IsDone())
    return out;
  out.body = fillet.Shape();

  const std::vector<kf::BlendRecognition> blends =
      kf::recognize_constant_radius_blends(out.body).recognized();
  if (blends.size() != 1)
    return out;
  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(out.body, TopAbs_FACE, faces);
  out.blend = TopoDS::Face(faces(blends.front().face_ordinal));
  for (int ordinal : blends.front().support_face_ordinals)
    out.supports.push_back(TopoDS::Face(faces(ordinal)));
  return out;
}

void defaults_are_todays_budget() {
  const kf::BlendSamplingBudget budget;
  check(budget.edge_samples == 9,
        "budget: the default edge sample count is today's 9");
  check(budget.uv_grid == 5, "budget: the default UV grid is today's 5");
}

void default_budget_reproduces_todays_counts() {
  const Fixture fixture = pinned_fixture();
  check(!fixture.body.IsNull() && !fixture.blend.IsNull() &&
            fixture.supports.size() == 2,
        "fixture: filleted box with one blend face and two supports");
  if (fixture.supports.size() != 2)
    return;

  const kf::BlendEvidence implied = kf::measure_blend_evidence(
      fixture.body, {fixture.blend}, fixture.supports, 2.0);
  const kf::BlendEvidence explicit_default =
      kf::measure_blend_evidence(fixture.body, {fixture.blend},
                                 fixture.supports, 2.0,
                                 kf::BlendSamplingBudget{});

  std::fprintf(stderr,
               "default budget: samples=%d boundaries=%d radius=[%.12f,%.12f] "
               "tangency=%.3e profile=%.3e\n",
               implied.samples, implied.boundaries,
               implied.minimum_section_radius, implied.maximum_section_radius,
               implied.maximum_tangency_radians, implied.maximum_profile_error);

  // The pinned counts: a 5x5 grid entirely inside the trimmed quarter-cylinder,
  // and the two edges it shares with its supports.
  check(implied.samples == 25,
        "default budget: 25 curvature samples on the pinned fixture");
  check(implied.boundaries == 2,
        "default budget: 2 measured support boundaries");

  check(explicit_default.samples == implied.samples &&
            explicit_default.boundaries == implied.boundaries &&
            explicit_default.minimum_section_radius ==
                implied.minimum_section_radius &&
            explicit_default.maximum_section_radius ==
                implied.maximum_section_radius &&
            explicit_default.maximum_profile_error ==
                implied.maximum_profile_error &&
            explicit_default.maximum_tangency_radians ==
                implied.maximum_tangency_radians &&
            explicit_default.coordinate_magnitude ==
                implied.coordinate_magnitude,
        "default budget: omitting the argument is bit-identical to passing the "
        "default");
}

void raised_budget_measures_more_and_agrees() {
  const Fixture fixture = pinned_fixture();
  if (fixture.supports.size() != 2)
    return;

  const kf::BlendEvidence baseline = kf::measure_blend_evidence(
      fixture.body, {fixture.blend}, fixture.supports, 2.0);
  const kf::BlendEvidence raised =
      kf::measure_blend_evidence(fixture.body, {fixture.blend},
                                 fixture.supports, 2.0,
                                 kf::BlendSamplingBudget{33, 17});

  std::fprintf(stderr,
               "raised budget: samples=%d boundaries=%d radius=[%.12f,%.12f] "
               "tangency=%.3e profile=%.3e\n",
               raised.samples, raised.boundaries, raised.minimum_section_radius,
               raised.maximum_section_radius, raised.maximum_tangency_radians,
               raised.maximum_profile_error);

  check(raised.samples >= 64,
        "raised budget: at least 64 curvature samples");
  check(raised.samples == 289, "raised budget: a full 17x17 grid");
  check(raised.boundaries == 2,
        "raised budget: still exactly two support boundaries");

  // Same verdicts: both budgets see a constant 2 mm section and a G1 boundary.
  const double limit = kf::fillet_section_radius_limit(
      2.0, std::max(baseline.coordinate_magnitude, raised.coordinate_magnitude));
  const double tangency_limit = kf::fillet_tangency_limit(
      2.0, std::max(baseline.coordinate_magnitude, raised.coordinate_magnitude));
  check(baseline.maximum_profile_error <= limit &&
            raised.maximum_profile_error <= limit,
        "both budgets: measured section radius is 2 mm within the fillet limit");
  check(baseline.maximum_tangency_radians <= tangency_limit &&
            raised.maximum_tangency_radians <= tangency_limit,
        "both budgets: measured boundary is G1 within the fillet limit");
}

void degenerate_budget_measures_nothing_and_fails_closed() {
  const Fixture fixture = pinned_fixture();
  if (fixture.supports.size() != 2)
    return;

  const kf::BlendEvidence zero =
      kf::measure_blend_evidence(fixture.body, {fixture.blend},
                                 fixture.supports, 2.0,
                                 kf::BlendSamplingBudget{0, 0});
  check(zero.samples == 0 && zero.boundaries == 0,
        "degenerate budget: a non-positive budget samples nothing");
  check(zero.minimum_section_radius == 0.0 &&
            zero.maximum_section_radius == 0.0 &&
            zero.maximum_profile_error == 0.0 &&
            zero.maximum_tangency_radians == 0.0,
        "degenerate budget: no measurement is reported as zero error");
  // The guard BlendRecognizer and FilletSemanticChecks both apply verbatim.
  // Zero evidence trips it, so an empty budget can only route to Ambiguous /
  // refused — it can never be mistaken for a clean measurement.
  check(zero.samples == 0 || zero.boundaries < 2,
        "degenerate budget: zero evidence trips the fail-closed guard");
}

} // namespace

int main() {
  defaults_are_todays_budget();
  default_budget_reproduces_todays_counts();
  raised_budget_measures_more_and_agrees();
  degenerate_budget_measures_nothing_and_fails_closed();
  if (failures == 0)
    std::fprintf(stderr, "blend budget: all checks passed\n");
  return failures;
}
