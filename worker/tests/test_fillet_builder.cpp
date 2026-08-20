#include <algorithm>
#include <cmath>
#include <cstdio>
#include <limits>
#include <string>
#include <vector>

#include <BRep_Builder.hxx>
#include <BRepBuilderAPI_MakeSolid.hxx>
#include <BRepBuilderAPI_Sewing.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shell.hxx>
#include <TopoDS_Solid.hxx>

#include "fillet_test_utils.h"
#include "kernel/fillet/FilletBuilder.h"
#include "kernel/fillet/FilletSemanticChecks.h"
#include "session/PlanExecutor.h"

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

void test_radius_contract() {
  const TopoDS_Shape body = ft::box();
  const TopoDS_Edge edge = ft::vertical_edges(body).front();
  check(!kf::valid_constant_radius(0.0), "zero radius rejected");
  check(!kf::valid_constant_radius(-1.0), "negative radius rejected");
  check(!kf::valid_constant_radius(0.0009), "sub-floor radius rejected");
  check(!kf::valid_constant_radius(std::numeric_limits<double>::quiet_NaN()),
        "NaN radius rejected");
  check(!kf::valid_constant_radius(std::numeric_limits<double>::infinity()),
        "infinite radius rejected");
  check(kf::valid_constant_radius(0.001), "radius floor accepted");
  for (double radius :
       {0.0, -1.0, 0.0009, std::numeric_limits<double>::quiet_NaN(),
        std::numeric_limits<double>::infinity()}) {
    kf::FilletBuilder builder(body, {ft::resolved(body, edge)}, radius);
    const kf::FilletBuildResult result = builder.build();
    check(!result.ok && !result.diagnostics.empty() &&
              result.diagnostics.front().code == "FILLET_RADIUS_INVALID",
          "builder enforces radius contract at its boundary");
  }
}

void test_single_and_duplicate_contour() {
  const TopoDS_Shape body = ft::box();
  const TopoDS_Edge edge = ft::vertical_edges(body).front();
  kf::FilletBuilder single(body, {ft::resolved(body, edge)}, 1.0);
  const kf::FilletBuildResult one = single.build();
  check(one.ok && one.output_audit.publishable(),
        "single edge fillet publishes valid solid");
  check(one.analysis.contours.size() == 1, "single edge owns one contour");
  check(one.fillet_evidence.generated_face_count > 0 &&
            one.fillet_evidence.support_face_count >= 2 &&
            one.fillet_evidence.blend.boundaries >= 2 &&
            one.fillet_evidence.blend.samples > 0,
        "single edge fillet carries measured blend/support evidence");
  check(one.fillet_evidence.blend.maximum_profile_error <=
            kf::fillet_section_radius_limit(
                1.0, one.fillet_evidence.blend.coordinate_magnitude),
        "single edge fillet proves the resulting section radius");
  check(one.fillet_evidence.blend.maximum_tangency_radians <=
            kf::fillet_tangency_limit(
                1.0, one.fillet_evidence.blend.coordinate_magnitude),
        "single edge fillet proves G1 support tangency");
  check(one.output_audit.tolerances.face <= 1.0e-6 &&
            one.output_audit.tolerances.edge <= 1.0e-6 &&
            one.output_audit.tolerances.vertex <= 1.0e-6,
        "analytic fillet tolerances stay within measured ceiling");

  const kf::ResolvedEdge picked = ft::resolved(body, edge);
  kf::FilletBuilder duplicate(body, {picked, picked}, 1.0);
  const kf::FilletBuildResult twice = duplicate.build();
  check(twice.ok, "duplicate edge selection deduplicates");
  check(twice.analysis.contours.size() == 1,
        "duplicate selection builds contour once");
}

void test_disconnected_multi_edge() {
  const TopoDS_Shape body = ft::box();
  const std::vector<TopoDS_Edge> edges = ft::vertical_edges(body);
  check(edges.size() == 4, "box has four vertical edges");
  kf::FilletBuilder builder(body, ft::resolved(body, edges), 1.0);
  const kf::FilletBuildResult result = builder.build();
  check(result.ok && result.output_audit.publishable(),
        "four disconnected contours publish valid solid");
  check(result.analysis.contours.size() == 4,
        "four vertical edges remain four contours");
}

void test_supported_scales() {
  for (double scale : {1.0e-3, 1.0, 1.0e3}) {
    const TopoDS_Shape body = ft::box(10.0 * scale);
    const TopoDS_Edge edge = ft::vertical_edges(body).front();
    const double radius = std::max(1.0e-3, scale);
    kf::FilletBuilder builder(body, {ft::resolved(body, edge)}, radius);
    const kf::FilletBuildResult result = builder.build();
    check(result.ok ? result.output_audit.publishable()
                    : !result.diagnostics.empty(),
          "scale either validates or refuses with diagnostics");
  }
}

TopoDS_Shape open_box_solid() {
  BRepBuilderAPI_Sewing sewing;
  int face_index = 0;
  for (TopExp_Explorer it(ft::box(), TopAbs_FACE); it.More(); it.Next()) {
    if (face_index++ < 5)
      sewing.Add(it.Current());
  }
  sewing.Perform();
  for (TopExp_Explorer it(sewing.SewedShape(), TopAbs_SHELL); it.More();
       it.Next()) {
    return BRepBuilderAPI_MakeSolid(TopoDS::Shell(it.Current())).Solid();
  }
  return {};
}

void test_shape_audit_policy() {
  namespace validation = onecad::kernel::validation;
  const validation::ShapeAuditResult null_audit =
      validation::audit_shape(TopoDS_Shape{});
  check(null_audit.null_shape && !null_audit.publishable(),
        "null shape fails audit");

  const TopoDS_Shape box = ft::box();
  const validation::ShapeAuditResult box_audit = validation::audit_shape(box);
  check(box_audit.publishable(), "analytic box passes audit");
  check(box_audit.tolerances_checked && box_audit.structure_checked &&
            box_audit.stray_topology_count == 0 && box_audit.volume_checked,
        "analytic box carries complete structural/tolerance/volume evidence");
  check(box_audit.tolerances.face <= 1.0e-7 &&
            box_audit.tolerances.edge <= 1.0e-7 &&
            box_audit.tolerances.vertex <= 1.0e-7,
        "analytic box tolerances stay within measured ceiling");

  const validation::PublicationPolicy tier_a_policy =
      validation::single_solid_policy("Tier A box", validation::PublicationTier::TierA);
  const validation::ShapeEvidence fast_box =
      validation::collect_shape_evidence(box, validation::PublicationTier::TierA);
  const validation::PublicationDecision fast_decision =
      validation::evaluate_publication_policy(fast_box, tier_a_policy);
  check(fast_decision.publishable(),
        "Tier A policy accepts fast valid-solid evidence");
  const nlohmann::json decision_json = fast_decision.to_json();
  check(decision_json["disposition"] == "publishable" &&
            decision_json["evidence"]["topLevelShape"] == "solid" &&
            decision_json["evidence"]["solidCount"] == 1 &&
            decision_json["evidence"]["brepValid"] == true,
        "publication decision serializes verdict and core evidence");
  // WP1-G3 moved this pin from `== false`. Micro-topology collection used to be
  // inside the Tier B branch, but the BOUNDS that consume it live on the policy,
  // not on the tier — so a Tier A policy carrying one would have refused
  // everything as PUBLICATION_MICRO_TOPOLOGY_UNKNOWN, including every
  // `validate_modeling_input` call and every downgraded interactive preview.
  // Collection is GProp-only (no BOP), so it now runs at both tiers; what Tier A
  // still skips is manifold and self-interference.
  check(decision_json["evidence"]["microTopologyChecked"] == true,
        "Tier A carries micro topology (GProp-only, so not part of the Tier B cost)");
  check(decision_json["evidence"]["manifoldChecked"] == false &&
            decision_json["evidence"]["selfInterferenceChecked"] == false,
        "Tier A still omits the BOP-backed manifold and self-interference evidence");
  check(decision_json["timings"]["buildMs"] == 0.0 &&
            decision_json["timings"]["validatorMs"].get<double>() >= 0.0,
        "publication decision serializes build and validator timings");
  validation::PublicationPolicy tolerance_policy = tier_a_policy;
  tolerance_policy.maximum_tolerance = fast_box.tolerances.maximum();
  check(validation::evaluate_publication_policy(fast_box, tolerance_policy).publishable(),
        "measured tolerance at the policy ceiling passes");
  tolerance_policy.maximum_tolerance = fast_box.tolerances.maximum() * 0.5;
  check(!validation::evaluate_publication_policy(fast_box, tolerance_policy).publishable(),
        "tolerance growth beyond the owned ceiling refuses publication");
  const validation::PublicationPolicy tier_b_policy =
      validation::single_solid_policy("Tier B box", validation::PublicationTier::TierB);
  const validation::ShapeEvidence deep_box =
      validation::collect_shape_evidence(box, validation::PublicationTier::TierB);
  check(deep_box.micro_topology_checked && deep_box.micro_edge_count == 0 &&
            deep_box.sliver_face_count == 0 && deep_box.scale_diagonal > 0.0,
        "Tier B evidence includes scale-normalized micro topology");
  check(!validation::evaluate_publication_policy(fast_box, tier_b_policy).publishable(),
        "Tier B policy refuses missing self-interference evidence");

  const validation::ShapeAuditResult edge_audit =
      validation::audit_shape(ft::all_edges(box).front());
  check(edge_audit.solid_count == 0 && !edge_audit.publishable(),
        "non-solid shape fails audit");

  BRep_Builder builder;
  TopoDS_Compound solid_with_stray_face;
  builder.MakeCompound(solid_with_stray_face);
  builder.Add(solid_with_stray_face, box);
  for (TopExp_Explorer it(ft::box(), TopAbs_FACE); it.More(); it.Next()) {
    builder.Add(solid_with_stray_face, it.Current());
    break;
  }
  const validation::ShapeAuditResult stray_audit =
      validation::audit_shape(solid_with_stray_face);
  check(stray_audit.solid_count == 1 && stray_audit.stray_topology_count == 1 &&
            !stray_audit.publishable(),
        "one solid plus stray face is not a publishable Body");


  TopoDS_Solid empty_solid;
  builder.MakeSolid(empty_solid);
  const validation::ShapeAuditResult empty_audit =
      validation::audit_shape(empty_solid);
  check(empty_audit.volume == 0.0 && !empty_audit.publishable(),
        "zero-volume solid fails audit");

  const validation::ShapeAuditResult invalid_audit =
      validation::audit_shape(open_box_solid());
  check(!invalid_audit.brep_valid && !invalid_audit.publishable(),
        "invalid solid fails audit");
  check(invalid_audit.manifold_checked && invalid_audit.open_edge_count > 0 &&
            invalid_audit.non_manifold_edge_count == 0,
        "open solid reports open edge-use evidence");

  TopoDS_Compound overlapping;
  builder.MakeCompound(overlapping);
  builder.Add(overlapping, BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape());
  builder.Add(
      overlapping,
      BRepPrimAPI_MakeBox(gp_Pnt(5.0, 0.0, 0.0), 10.0, 10.0, 10.0).Shape());
  const validation::ShapeAuditResult overlap_audit =
      validation::audit_shape(overlapping);
  check(overlap_audit.solid_count == 2 && !overlap_audit.publishable(),
        "multi-solid shape fails audit");
  check(overlap_audit.self_interference_checked &&
            overlap_audit.self_interference_count > 0,
        "overlapping solids report self-interference");
  check(overlap_audit.manifold_checked && overlap_audit.open_edge_count == 0 &&
            overlap_audit.non_manifold_edge_count == 0,
        "Tier B evidence reports closed-manifold edge use");
  check(!validation::evaluate_publication_policy(overlap_audit, tier_b_policy).publishable(),
        "Tier B policy refuses overlapping solids");

  validation::PublicationPolicy empty_lifecycle;
  empty_lifecycle.name = "Boolean";
  empty_lifecycle.min_solid_count = 0;
  empty_lifecycle.max_solid_count = -1;
  empty_lifecycle.require_positive_volume = false;
  empty_lifecycle.allow_empty_lifecycle = true;
  TopoDS_Compound empty_compound;
  builder.MakeCompound(empty_compound);
  const validation::ShapeEvidence fast_empty = validation::collect_shape_evidence(
      empty_compound, validation::PublicationTier::TierA);
  check(validation::evaluate_publication_policy(fast_empty, empty_lifecycle).lifecycle_only(),
        "policy separates explicit empty lifecycle from refusal");
}

// WP1-G2/G3: every `refuse()` branch of `evaluate_publication_policy` carries
// its own `reason_code`, and the two non-refusal dispositions carry none. The
// table is the branch list in ORDER — a case that stops reaching its branch
// (because an earlier one started firing) surfaces as the earlier branch's code,
// not as a silent pass. G3 adds the four micro-topology bounds plus the
// missing-evidence refusal, and the negative case proving the bounds are inert
// at their disabled default.
void test_publication_reason_codes() {
  namespace validation = onecad::kernel::validation;

  const auto valid_solid = [] {
    validation::ShapeEvidence evidence;
    evidence.null_shape = false;
    evidence.brep_valid = true;
    evidence.top_level_shape = TopAbs_SOLID;
    evidence.solid_count = 1;
    evidence.structure_checked = true;
    evidence.volume = 1000.0;
    evidence.volume_checked = true;
    evidence.tolerances = {1.0e-7, 1.0e-7, 1.0e-7};
    evidence.tolerances_checked = true;
    evidence.manifold_checked = true;
    evidence.self_interference_checked = true;
    evidence.micro_topology_checked = true;
    evidence.minimum_edge_length = 1.0;
    evidence.minimum_face_width = 1.0;
    return evidence;
  };

  struct Case {
    const char *label;
    void (*mutate)(validation::ShapeEvidence &, validation::PublicationPolicy &);
    const char *reason_code;
    validation::PublicationDisposition disposition;
  };

  static const Case cases[] = {
      {"null shape",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &) { e.null_shape = true; },
       "PUBLICATION_NULL_SHAPE", validation::PublicationDisposition::Refused},
      {"audit error",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &) { e.error = "OCCT threw"; },
       "PUBLICATION_AUDIT_ERROR", validation::PublicationDisposition::Refused},
      {"invalid brep",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &) { e.brep_valid = false; },
       "PUBLICATION_BREP_INVALID", validation::PublicationDisposition::Refused},
      {"no structure evidence",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &) {
         e.structure_checked = false;
       },
       "PUBLICATION_NO_STRUCTURE_EVIDENCE", validation::PublicationDisposition::Refused},
      {"unsupported top level",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &) {
         e.top_level_shape = TopAbs_FACE;
       },
       "PUBLICATION_UNSUPPORTED_TOP_LEVEL", validation::PublicationDisposition::Refused},
      {"stray topology",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &) {
         e.top_level_shape = TopAbs_COMPOUND;
         e.stray_topology_count = 1;
       },
       "PUBLICATION_STRAY_TOPOLOGY", validation::PublicationDisposition::Refused},
      {"too few solids",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &) { e.solid_count = 0; },
       "PUBLICATION_TOO_FEW_SOLIDS", validation::PublicationDisposition::Refused},
      {"too many solids",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &) {
         e.top_level_shape = TopAbs_COMPOUND;
         e.solid_count = 2;
       },
       "PUBLICATION_TOO_MANY_SOLIDS", validation::PublicationDisposition::Refused},
      {"not a single connected solid",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &p) {
         e.top_level_shape = TopAbs_COMPOUND;
         e.solid_count = 2;
         p.max_solid_count = -1;  // the count gate is off; the SingleBody gate is not
       },
       "PUBLICATION_NOT_SINGLE_SOLID", validation::PublicationDisposition::Refused},
      {"non-positive volume",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &) { e.volume = 0.0; },
       "PUBLICATION_NON_POSITIVE_VOLUME", validation::PublicationDisposition::Refused},
      {"unknown tolerances",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &) {
         e.tolerances_checked = false;
       },
       "PUBLICATION_TOLERANCE_UNKNOWN", validation::PublicationDisposition::Refused},
      {"tolerance budget",
       [](validation::ShapeEvidence &, validation::PublicationPolicy &p) {
         p.maximum_tolerance = 1.0e-9;
       },
       "PUBLICATION_TOLERANCE_BUDGET", validation::PublicationDisposition::Refused},
      {"open manifold",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &) { e.open_edge_count = 1; },
       "PUBLICATION_OPEN_MANIFOLD", validation::PublicationDisposition::Refused},
      // WP1-G3. The micro-topology bounds default to -1 (disabled), so each of
      // these must ENABLE its bound before the branch is reachable at all — a
      // case that forgets to would land on "publishable" and red here.
      {"micro topology unknown",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &p) {
         e.micro_topology_checked = false;
         p.max_micro_edge_count = 0;
       },
       "PUBLICATION_MICRO_TOPOLOGY_UNKNOWN", validation::PublicationDisposition::Refused},
      {"micro edge count",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &p) {
         e.micro_edge_count = 1;
         p.max_micro_edge_count = 0;
       },
       "PUBLICATION_MICRO_EDGE", validation::PublicationDisposition::Refused},
      {"minimum edge length",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &p) {
         e.minimum_edge_length = 1.0e-4;
         p.minimum_edge_length = 1.0e-3;
       },
       "PUBLICATION_MICRO_EDGE", validation::PublicationDisposition::Refused},
      {"sliver face count",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &p) {
         e.sliver_face_count = 1;
         p.max_sliver_face_count = 0;
       },
       "PUBLICATION_SLIVER_FACE", validation::PublicationDisposition::Refused},
      {"minimum face width",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &p) {
         e.minimum_face_width = 1.0e-4;
         p.minimum_face_width = 1.0e-3;
       },
       "PUBLICATION_SLIVER_FACE", validation::PublicationDisposition::Refused},
      {"self interference",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &) {
         e.self_interference_count = 1;
       },
       "PUBLICATION_SELF_INTERFERENCE", validation::PublicationDisposition::Refused},
      // The disabled default is the load-bearing half: the SAME defective
      // evidence must publish under a DEFAULT-CONSTRUCTED policy, because that
      // is what the ungated sites (ExtrudeOp tool check, PlanExecutor global
      // invariant) build. Not single_solid_policy — G4 enables the sliver bound
      // there, deliberately.
      {"micro topology bounds disabled on a default-constructed policy",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &p) {
         e.micro_edge_count = 99;
         e.sliver_face_count = 99;
         e.minimum_edge_length = 0.0;
         e.minimum_face_width = 0.0;
         const std::string name = p.name;
         p = validation::PublicationPolicy{};
         p.name = name;
       },
       "", validation::PublicationDisposition::Publishable},
      // WP1-G4: the TierB single-solid policy carries the sliver bound OUT OF
      // THE BOX — one sliver face refuses with no per-case policy mutation.
      // The micro bound stays disabled pending a characterized dirty import.
      {"G4: one sliver face refuses under the stock TierB policy",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &) {
         e.sliver_face_count = 1;
       },
       "PUBLICATION_SLIVER_FACE", validation::PublicationDisposition::Refused},
      {"G4: micro edges still publish under the stock TierB policy",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &) {
         e.micro_edge_count = 99;
       },
       "", validation::PublicationDisposition::Publishable},
      {"publishable",
       [](validation::ShapeEvidence &, validation::PublicationPolicy &) {}, "",
       validation::PublicationDisposition::Publishable},
      {"lifecycle only",
       [](validation::ShapeEvidence &e, validation::PublicationPolicy &p) {
         e.top_level_shape = TopAbs_COMPOUND;
         e.solid_count = 0;
         p.allow_empty_lifecycle = true;
       },
       "", validation::PublicationDisposition::LifecycleOnly},
  };

  std::vector<std::string> seen;
  for (const Case &c : cases) {
    validation::ShapeEvidence evidence = valid_solid();
    validation::PublicationPolicy policy =
        validation::single_solid_policy("Probe", validation::PublicationTier::TierB);
    c.mutate(evidence, policy);
    const validation::PublicationDecision decision =
        validation::evaluate_publication_policy(evidence, policy);
    check(decision.disposition == c.disposition,
          std::string("reason codes: ") + c.label + " reaches its disposition");
    check(decision.reason_code == c.reason_code,
          std::string("reason codes: ") + c.label + " maps to '" + c.reason_code + "', got '" +
              decision.reason_code + "'");
    check(decision.to_json().value("reasonCode", std::string("<missing>")) == c.reason_code,
          std::string("reason codes: ") + c.label + " serializes reasonCode");
    if (c.disposition == validation::PublicationDisposition::Refused) {
      check(decision.code == "GEOMETRY_INVALID",
            std::string("reason codes: ") + c.label +
                " keeps the §8 taxonomy code, got '" + decision.code + "'");
      seen.push_back(decision.reason_code);
    }
  }
  std::sort(seen.begin(), seen.end());
  seen.erase(std::unique(seen.begin(), seen.end()), seen.end());
  check(seen.size() == 17,
        "reason codes: all seventeen refusal reasons are distinct and reachable, saw " +
            std::to_string(seen.size()));
}

void test_executor_radius_contract() {
  const TopoDS_Shape body = ft::box();
  const TopoDS_Edge edge = ft::vertical_edges(body).front();
  for (double radius :
       {0.0, -1.0, 0.0009, std::numeric_limits<double>::quiet_NaN(),
        std::numeric_limits<double>::infinity()}) {
    onecad::session::ScratchJob job;
    job.bodies.create("body_box", "op_box", body);
    std::string last_sketch;
    const auto result = onecad::session::execute_candidate_op(
        job, ft::op(body, {edge}, radius), "op_fillet", last_sketch,
        onecad::CancelToken{});
    check(result.status == onecad::session::CandidateResult::Status::Failed,
          "executor rejects non-finite or sub-floor radius");
    check(job.bodies.get("body_box")->geom.IsSame(body),
          "radius refusal leaves predecessor shape unchanged");
  }
}

} // namespace

int main() {
  test_radius_contract();
  test_single_and_duplicate_contour();
  test_disconnected_multi_edge();
  test_supported_scales();
  test_shape_audit_policy();
  test_publication_reason_codes();
  test_executor_radius_contract();
  return failures;
}
