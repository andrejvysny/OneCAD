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
  check(decision_json["evidence"]["microTopologyChecked"] == false,
        "Tier A omits scale-normalized micro topology");
  check(decision_json["timings"]["buildMs"] == 0.0 &&
            decision_json["timings"]["validatorMs"].get<double>() >= 0.0,
        "publication decision serializes build and validator timings");
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
  test_executor_radius_contract();
  return failures;
}
