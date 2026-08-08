#include <cstdio>
#include <string>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>

#include "fillet_test_utils.h"
#include "kernel/fillet/FilletBuilder.h"
#include "protocol/Envelope.h"
#include "session/HistoryHash.h"
#include "session/PlanExecutor.h"
#include "session/PreviewOp.h"
#include "session/Session.h"
#include "session/ShapeMetrics.h"
#include "session/Signatures.h"

namespace ft = onecad::tests::fillet;
namespace kf = onecad::kernel::fillet;
namespace km = onecad::kernel::elementmap;
namespace session = onecad::session;

namespace {

int failures = 0;

void check(bool condition, const std::string &message) {
  if (condition)
    return;
  std::fprintf(stderr, "FAIL: %s\n", message.c_str());
  ++failures;
}

void test_oversized_diagnostic() {
  const TopoDS_Shape body = ft::box();
  const TopoDS_Edge edge = ft::vertical_edges(body).front();
  kf::FilletBuilder builder(body, {ft::resolved(body, edge)}, 11.0);
  const kf::FilletBuildResult result = builder.build();
  check(!result.ok && result.error_code == "OP_FAILED",
        "oversized radius refuses");
  check(result.diagnostics.size() == 1,
        "oversized radius has primary diagnostic");
  if (result.diagnostics.empty())
    return;
  const auto json = result.diagnostics.front().to_json();
  check(json["code"].get<std::string>().rfind("FILLET_", 0) == 0,
        "kernel diagnostic has FILLET code");
  check(json["stage"] == "build", "kernel diagnostic carries build stage");
  check(json["evidence"]["refs"][0]["topoKey"].is_string(),
        "kernel diagnostic carries typed ref evidence");
  check(json["evidence"]["metrics"]["requestedRadius"] == 11.0,
        "kernel diagnostic carries requested radius");
  check(json["evidence"]["metrics"]["inputMaxTolerance"].is_number(),
        "kernel diagnostic carries input tolerance");
}

struct CandidateRun {
  session::CandidateResult::Status status;
  nlohmann::json diagnostics;
  std::string signature;
  nlohmann::json audit;
  nlohmann::json delta;
  std::vector<double> cylinder_radii;
  bool failure_state_unchanged = false;
};

struct RunBaseline {
  std::string signature;
  double volume = 0.0;
  TopoDS_Shape binding;
  std::string topo_key;
};

RunBaseline baseline(const session::ScratchJob &job, const TopoDS_Shape &body) {
  const auto *entry = job.partition.find("el_0");
  return {session::geometry_signature(job.bodies), session::shape_volume(body),
          entry->shape, entry->topo_key};
}

bool state_unchanged(const session::ScratchJob &job,
                     const session::CandidateResult &candidate,
                     const TopoDS_Shape &body, const RunBaseline &before) {
  const session::BodyRecord *record = job.bodies.get("body_box");
  const auto *entry = job.partition.find("el_0");
  return record && record->geom.IsSame(body) &&
         session::geometry_signature(job.bodies) == before.signature &&
         session::shape_volume(record->geom) == before.volume && entry &&
         entry->shape.IsSame(before.binding) &&
         entry->topo_key == before.topo_key &&
         job.history_prefix_hash == "history-before" &&
         candidate.delta.empty() && candidate.body_events.empty() &&
         candidate.body_ids.empty();
}

std::vector<double> cylinder_radii(const TopoDS_Shape &shape) {
  std::vector<double> radii;
  for (TopExp_Explorer ex(shape, TopAbs_FACE); ex.More(); ex.Next()) {
    const BRepAdaptor_Surface surface(TopoDS::Face(ex.Current()));
    if (surface.GetType() == GeomAbs_Cylinder)
      radii.push_back(surface.Cylinder().Radius());
  }
  return radii;
}

CandidateRun run_candidate(const TopoDS_Shape &body, const TopoDS_Edge &edge,
                           double radius) {
  session::ScratchJob job;
  job.history_prefix_hash = "history-before";
  job.bodies.create("body_box", "op_box", body);
  job.partition.mint("body_box", "el_0", km::ElementKind::Edge, edge, body,
                     ft::input(body, edge, "el_0")["anchor"]);
  const RunBaseline before = baseline(job, body);

  std::string last_sketch;
  const session::CandidateResult candidate = session::execute_candidate_op(
      job, ft::op(body, {edge}, radius), "op_fillet", last_sketch,
      onecad::CancelToken{});
  const session::BodyRecord *after_record = job.bodies.get("body_box");
  const TopoDS_Shape after = after_record ? after_record->geom : TopoDS_Shape{};
  const auto audit = onecad::kernel::validation::audit_shape(after);
  return {candidate.status,
          session::candidate_diagnostics(candidate),
          session::geometry_signature(job.bodies),
          audit.to_json(),
          candidate.delta.to_json(),
          cylinder_radii(after),
          state_unchanged(job, candidate, body, before)};
}

void test_near_boundary_sweep() {
  const TopoDS_Shape body = ft::box();
  const TopoDS_Edge edge = ft::vertical_edges(body).front();
  for (double radius : {4.5, 4.9, 5.0, 5.1, 5.5, 8.0, 11.0}) {
    const CandidateRun result = run_candidate(body, edge, radius);
    if (result.status == session::CandidateResult::Status::Ok) {
      check(!result.audit.value("nullShape", true) &&
                result.audit.value("brepValid", false) &&
                result.audit.value("solidCount", 0) == 1 &&
                std::isfinite(result.audit.value("volume", 0.0)) &&
                result.audit.value("volume", 0.0) > 0.0 &&
                result.audit.value("selfInterferenceChecked", false) &&
                result.audit.value("selfInterferenceCount", -1) == 0 &&
                !result.audit.contains("auditError"),
            "boundary success passes shape audit");
    } else {
      check(!result.diagnostics.empty(), "boundary refusal carries diagnostic");
      check(result.failure_state_unchanged,
            "boundary refusal preserves full predecessor state");
    }
  }
}

void test_deterministic_perturbations() {
  for (double delta : {-1.0e-6, 0.0, 1.0e-6}) {
    const TopoDS_Shape body =
        BRepPrimAPI_MakeBox(10.0 + delta, 10.0, 10.0).Shape();
    const TopoDS_Edge edge = ft::vertical_edges(body).front();
    const CandidateRun first = run_candidate(body, edge, 4.9);
    const CandidateRun second = run_candidate(body, edge, 4.9);
    check(first.status == second.status,
          "perturbed case repeats same classification");
    check(first.signature == second.signature,
          "perturbed case repeats geometry signature");
    check(first.audit == second.audit, "perturbed case repeats audit metrics");
    check(first.delta == second.delta,
          "perturbed case repeats ElementMap delta");
    check(first.cylinder_radii == second.cylinder_radii,
          "perturbed case repeats generated-face evidence");
    check(first.diagnostics == second.diagnostics,
          "perturbed case repeats diagnostics");
  }
}

kf::PartialResultProbe injected_partial(BRepFilletAPI_MakeFillet &) {
  kf::PartialResultProbe probe;
  probe.query_ok = true;
  probe.available = true;
  probe.bad_shape = ft::box();
  return probe;
}

void test_partial_result_rejected() {
  const TopoDS_Shape body = ft::box();
  const TopoDS_Edge edge = ft::vertical_edges(body).front();
  kf::FilletBuilder builder(body, {ft::resolved(body, edge)}, 1.0,
                            &injected_partial);
  const kf::FilletBuildResult result = builder.build();
  check(!result.ok && result.shape.IsNull(),
        "partial result is never returned for publication");
  check(result.error_code == "OP_FAILED" && !result.diagnostics.empty(),
        "partial result refuses with structured diagnostic");
  check(result.diagnostics.front().to_json()["evidence"]["metrics"]
                                            ["partialResultAvailable"] == true,
        "partial result diagnostic records availability");
}

void seed_head(session::Session &live, const TopoDS_Shape &body,
               const TopoDS_Edge &edge) {
  session::FenceOutcome fence =
      live.fence_and_clone(1, 1, 1, session::kEmptyPrefixHash);
  session::ScratchJob seed;
  seed.job_id = 1;
  seed.plan_document_revision = 1;
  seed.bodies = std::move(fence.cloned_bodies);
  seed.partition = std::move(fence.cloned_partition);
  seed.prepared_snapshot_id = fence.prepared_snapshot_id;
  seed.history_prefix_hash = "seed-history";
  seed.bodies.create("body_box", "op_box", body);
  seed.partition.mint("body_box", "el_0", km::ElementKind::Edge, edge, body,
                      ft::input(body, edge, "el_0")["anchor"]);
  live.store_prepared(std::move(seed));
  check(live.accept_prepared(1, 1, 1).ok, "diagnostic session seed accepted");
}

void test_preview_commit_diagnostics_match() {
  const TopoDS_Shape body = ft::box();
  const TopoDS_Edge edge = ft::vertical_edges(body).front();
  const nlohmann::json op = ft::op(body, {edge}, 11.0);
  session::Session live;
  live.open("doc", 1, 1, "diagnostic-test");
  seed_head(live, body, edge);

  const auto preview = session::handle_preview_op(
      live,
      onecad::protocol::Envelope::request(
          50, "PreviewOp",
          {{"op", op}, {"expectedSnapshotId", live.current_snapshot_id()}}));
  check(preview.error.has_value() && preview.error->code == "OP_FAILED",
        "oversized preview keeps top-level taxonomy");
  const nlohmann::json preview_diagnostics =
      preview.error && preview.error->detail
          ? preview.error->detail->value("diagnostics", nlohmann::json::array())
          : nlohmann::json::array();

  const nlohmann::json args = {
      {"jobId", 77},
      {"documentRevision", 2},
      {"workerEpoch", 1},
      {"expectedBaseHash", "seed-history"},
      {"prefixHashes", nlohmann::json::array({"after-fillet"})},
      {"targetStep", 0},
      {"ops", nlohmann::json::array({op})}};
  onecad::CancelToken cancel;
  onecad::protocol::HandlerContext ctx{cancel, [](int) {},
                                       [](onecad::protocol::Envelope &) {}};
  const auto plan = session::handle_execute_plan(
      live, onecad::protocol::Envelope::request(77, "ExecutePlan", args), ctx);
  const nlohmann::json commit_diagnostics =
      plan.result["perStepResults"][0]["diagnostics"];
  check(!preview_diagnostics.empty(),
        "oversized preview carries kernel diagnostic");
  check(preview_diagnostics[0].value("code", "").rfind("FILLET_", 0) == 0,
        "oversized preview retains kernel diagnostic code");
  check(plan.result["stoppedReason"] == "opFailed",
        "oversized plan stops as opFailed");
  check(preview_diagnostics.dump() == commit_diagnostics.dump(),
        "preview and commit diagnostics are byte-equivalent");
  check(live.discard_prepared(77), "failed diagnostic plan discarded");
}

} // namespace

int main() {
  test_oversized_diagnostic();
  test_near_boundary_sweep();
  test_deterministic_perturbations();
  test_partial_result_rejected();
  test_preview_commit_diagnostics_match();
  return failures;
}
