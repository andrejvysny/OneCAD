// Oversized edge-op refusal and cross-version safety characterization.
#include <cmath>
#include <cstdio>
#include <iostream>
#include <string>
#include <tuple>
#include <utility>
#include <vector>

#include <BRepCheck_Analyzer.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Shape.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "session/HistoryHash.h"
#include "session/PlanExecutor.h"
#include "session/Session.h"
#include "session/ShapeMetrics.h"
#include "session/Signatures.h"

using nlohmann::json;
namespace em = onecad::elementmap;
namespace km = onecad::kernel::elementmap;
namespace session = onecad::session;

namespace {

int g_failures = 0;

void check(bool condition, const std::string &message) {
  if (condition)
    return;
  std::fprintf(stderr, "FAIL: %s\n", message.c_str());
  ++g_failures;
}

TopoDS_Shape first_edge(const TopoDS_Shape &shape) {
  TopTools_IndexedMapOfShape edges;
  TopExp::MapShapes(shape, TopAbs_EDGE, edges);
  return edges.Extent() > 0 ? edges(1) : TopoDS_Shape();
}

json edge_input(const TopoDS_Shape &edge) {
  const km::ElementDescriptor descriptor =
      em::ElementMapPartition::describe(edge);
  return json{
      {"primary",
       {{"bodyId", "body_box"}, {"elementId", "el_edge"}, {"kind", "edge"}}},
      {"intent",
       {{"kind", "edge"},
        {"descriptor",
         em::ElementMapPartition::descriptor_to_json(descriptor)}}},
      {"anchor",
       {{"worldPoint",
         {descriptor.center.X(), descriptor.center.Y(),
          descriptor.center.Z()}}}}};
}

json edge_op(const char *op_type, double value) {
  const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
  return json{{"opType", op_type},
              {"opId", std::string("op_") + op_type},
              {"stepIndex", 0},
              {"inputs", json::array({edge_input(first_edge(box))})},
              {"params",
               {{"mode", op_type},
                {"radius", value},
                {"edgeIds", json::array({"el_edge"})}}}};
}

std::string status_name(session::CandidateResult::Status status) {
  switch (status) {
  case session::CandidateResult::Status::Ok:
    return "ok";
  case session::CandidateResult::Status::Failed:
    return "opFailed";
  case session::CandidateResult::Status::Unsupported:
    return "unsupported";
  case session::CandidateResult::Status::NeedsRepair:
    return "needsRepair";
  case session::CandidateResult::Status::Cancelled:
    return "cancelled";
  }
  return "unknown";
}

bool one_valid_solid(const TopoDS_Shape &shape) {
  if (shape.IsNull() || !BRepCheck_Analyzer(shape).IsValid())
    return false;
  TopTools_IndexedMapOfShape solids;
  TopExp::MapShapes(shape, TopAbs_SOLID, solids);
  const double volume = session::shape_volume(shape);
  return solids.Extent() == 1 && std::isfinite(volume) && volume > 0.0;
}

struct PartitionEvidence {
  std::size_t size = 0;
  std::string body_id;
  km::ElementKind kind = km::ElementKind::Unknown;
  std::string topo_key;
  TopoDS_Shape shape;
  json descriptor;
  json anchor;
};

void seed_edge_binding(em::ElementMapPartition &partition,
                       const TopoDS_Shape &body) {
  const TopoDS_Shape edge = first_edge(body);
  partition.mint("body_box", "el_edge", km::ElementKind::Edge, edge, body,
                 edge_input(edge)["anchor"]);
}

PartitionEvidence partition_evidence(const em::ElementMapPartition &partition) {
  PartitionEvidence evidence;
  evidence.size = partition.size();
  const em::PartitionEntry *entry = partition.find("el_edge");
  if (!entry)
    return evidence;
  evidence.body_id = entry->body_id;
  evidence.kind = entry->kind;
  evidence.topo_key = entry->topo_key;
  evidence.shape = entry->shape;
  evidence.descriptor =
      em::ElementMapPartition::descriptor_to_json(entry->descriptor);
  evidence.anchor = entry->anchor;
  return evidence;
}

bool partition_matches(const em::ElementMapPartition &partition,
                       const PartitionEvidence &before) {
  const PartitionEvidence after = partition_evidence(partition);
  return after.size == before.size && after.body_id == before.body_id &&
         after.kind == before.kind && after.topo_key == before.topo_key &&
         !after.shape.IsNull() && after.shape.IsSame(before.shape) &&
         after.descriptor == before.descriptor && after.anchor == before.anchor;
}

json run_candidate(const char *case_id, const char *op_type, double value,
                   bool expected_primary_success) {
  session::ScratchJob job;
  job.history_prefix_hash =
      "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
  const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
  job.bodies.create("body_box", "op_box", box);
  seed_edge_binding(job.partition, box);

  const session::BodyRecord *before_record = job.bodies.get("body_box");
  const TopoDS_Shape before_shape = before_record->geom;
  const std::string before_provenance = before_record->provenance;
  const std::string before_signature = session::geometry_signature(job.bodies);
  const double before_volume = session::shape_volume(before_shape);
  const PartitionEvidence before_partition = partition_evidence(job.partition);
  const std::string before_history = job.history_prefix_hash;

  json op = edge_op(op_type, value);
  std::string last_sketch_id;
  const session::CandidateResult candidate =
      session::execute_candidate_op(job, op, op["opId"].get<std::string>(),
                                    last_sketch_id, onecad::CancelToken{});
  const json diagnostics = session::candidate_diagnostics(candidate);

  const session::BodyRecord *after_record = job.bodies.get("body_box");
  const TopoDS_Shape after_shape = after_record->geom;
  const std::string after_signature = session::geometry_signature(job.bodies);
  const double after_volume = session::shape_volume(after_shape);
  const bool shape_unchanged = after_shape.IsSame(before_shape);
  const bool geometry_unchanged = after_signature == before_signature;
  const bool partition_unchanged =
      partition_matches(job.partition, before_partition);
  const bool history_unchanged = job.history_prefix_hash == before_history;
  const bool volume_unchanged = after_volume == before_volume;
  const bool provenance_unchanged =
      after_record->provenance == before_provenance;
  const bool event_free = candidate.body_events.empty() &&
                          candidate.body_ids.empty() && candidate.delta.empty();
  const bool published =
      candidate.status == session::CandidateResult::Status::Ok;
  const bool state_unchanged = shape_unchanged && geometry_unchanged &&
                               partition_unchanged && history_unchanged &&
                               volume_unchanged && provenance_unchanged;
  const bool failure_safe =
      published || (state_unchanged && event_free && !diagnostics.empty());
  const bool primary = std::string(ONECAD_OCCT_VERSION) == "8.0.1";
  const bool primary_success =
      candidate.status == session::CandidateResult::Status::Ok;
  const bool primary_refusal =
      candidate.status == session::CandidateResult::Status::Failed &&
      candidate.error_code == "OP_FAILED";
  const bool boundary_expected =
      !primary ||
      (expected_primary_success ? primary_success : primary_refusal);
  const bool output_valid = one_valid_solid(after_shape);
  const bool safety_passed = one_valid_solid(before_shape) && output_valid &&
                             failure_safe && boundary_expected;

  check(safety_passed, std::string(case_id) + " safety contract");
  return json{
      {"kind", "case"},
      {"caseId", case_id},
      {"status", status_name(candidate.status)},
      {"errorCode", candidate.error_code},
      {"diagnostic", diagnostics.empty() ? json(nullptr) : diagnostics.back()},
      {"diagnostics", diagnostics},
      {"published", published},
      {"inputValid", one_valid_solid(before_shape)},
      {"outputValid", output_valid},
      {"shapeUnchanged", shape_unchanged},
      {"geometryUnchanged", geometry_unchanged},
      {"partitionUnchanged", partition_unchanged},
      {"historyUnchanged", history_unchanged},
      {"volumeUnchanged", volume_unchanged},
      {"stateUnchanged", state_unchanged},
      {"beforeSignature", before_signature},
      {"afterSignature", after_signature},
      {"beforeVolume", before_volume},
      {"afterVolume", after_volume},
      {"safetyPassed", safety_passed}};
}

bool same_repeat(const json &first, const json &second) {
  json a = first;
  json b = second;
  a.erase("repeatStable");
  b.erase("repeatStable");
  return a == b;
}

void seed_session(session::Session &live) {
  session::FenceOutcome fence =
      live.fence_and_clone(1, 1, 1, session::kEmptyPrefixHash);
  session::ScratchJob seed;
  seed.job_id = 1;
  seed.plan_document_revision = 1;
  seed.bodies = std::move(fence.cloned_bodies);
  seed.partition = std::move(fence.cloned_partition);
  seed.prepared_snapshot_id = fence.prepared_snapshot_id;
  seed.history_prefix_hash = "seed-history";
  const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
  seed.bodies.create("body_box", "op_box", box);
  seed_edge_binding(seed.partition, box);
  live.store_prepared(std::move(seed));
  const session::AcceptOutcome accepted = live.accept_prepared(1, 1, 1);
  check(accepted.ok, "seed session accepted");
}

json run_session_refusal(const char *op_type) {
  session::Session live;
  live.open("doc", 1, 1, "determinism");
  seed_session(live);
  const session::WorkerHead before = live.head();
  const session::BodyStore before_bodies = live.bodies_copy();
  const TopoDS_Shape before_shape = before_bodies.get("body_box")->geom;
  const std::string before_signature =
      session::geometry_signature(before_bodies);
  const double before_volume = session::shape_volume(before_shape);
  const PartitionEvidence before_partition =
      partition_evidence(live.partition_copy());

  const json args = {{"jobId", 77},
                     {"documentRevision", 2},
                     {"workerEpoch", 1},
                     {"expectedBaseHash", "seed-history"},
                     {"prefixHashes", json::array({"after-edge-op"})},
                     {"targetStep", 0},
                     {"ops", json::array({edge_op(op_type, 11.0)})}};
  onecad::CancelToken cancel;
  onecad::protocol::HandlerContext ctx{cancel, [](int) {},
                                       [](onecad::protocol::Envelope &) {}};
  const onecad::protocol::Envelope response = session::handle_execute_plan(
      live, onecad::protocol::Envelope::request(77, "ExecutePlan", args), ctx);
  const json terminal = response.result["perStepResults"][0];
  const session::WorkerHead prepared = live.head();
  const session::BodyStore prepared_bodies = live.bodies_copy();
  const TopoDS_Shape after_shape = prepared_bodies.get("body_box")->geom;
  const std::string after_signature =
      session::geometry_signature(prepared_bodies);
  const double after_volume = session::shape_volume(after_shape);
  const bool partition_unchanged =
      partition_matches(live.partition_copy(), before_partition);
  const bool unchanged =
      prepared.snapshot_id == before.snapshot_id &&
      prepared.history_prefix_hash == before.history_prefix_hash &&
      after_shape.IsSame(before_shape) && after_signature == before_signature &&
      after_volume == before_volume && partition_unchanged;
  const bool diagnostic_present =
      terminal.contains("diagnostics") && !terminal["diagnostics"].empty();
  const bool discarded = live.discard_prepared(77);
  const session::WorkerHead after_discard = live.head();
  const bool head_restored =
      discarded && !after_discard.has_scratch &&
      after_discard.snapshot_id == before.snapshot_id &&
      after_discard.history_prefix_hash == before.history_prefix_hash;
  const bool safety_passed =
      !response.error.has_value() &&
      response.result["stoppedReason"] == "opFailed" &&
      response.result["historyPrefixHash"] == "seed-history" && unchanged &&
      diagnostic_present && head_restored;
  check(safety_passed,
        std::string("session oversized ") + op_type + " refusal is atomic");
  const std::string case_id =
      std::string(op_type == std::string("Fillet") ? "fillet" : "chamfer") +
      "-oversized-session";
  return json{
      {"kind", "case"},
      {"caseId", case_id},
      {"status", terminal.value("status", "")},
      {"errorCode", "OP_FAILED"},
      {"diagnosticCode", terminal["diagnostics"].back().value("code", "")},
      {"diagnostic", terminal["diagnostics"].back()},
      {"diagnostics", terminal["diagnostics"]},
      {"published", false},
      {"inputValid", one_valid_solid(before_shape)},
      {"outputValid", one_valid_solid(after_shape)},
      {"shapeUnchanged", after_shape.IsSame(before_shape)},
      {"geometryUnchanged", after_signature == before_signature},
      {"partitionUnchanged",
       partition_matches(live.partition_copy(), before_partition)},
      {"historyUnchanged",
       after_discard.history_prefix_hash == before.history_prefix_hash},
      {"volumeUnchanged", after_volume == before_volume},
      {"stateUnchanged", unchanged && head_restored},
      {"beforeSignature", before_signature},
      {"afterSignature", after_signature},
      {"beforeVolume", before_volume},
      {"afterVolume", after_volume},
      {"safetyPassed", safety_passed}};
}

} // namespace

int main() {
  std::cout << json{{"kind", "environment"},
                    {"occtVersion", ONECAD_OCCT_VERSION},
                    {"buildId", ONECAD_OCCT_BUILD_ID}}
                   .dump()
            << '\n';
  const std::vector<std::tuple<const char *, const char *, double, bool>>
      cases = {
          {"fillet-safe", "Fillet", 1.0, true},
          {"chamfer-safe", "Chamfer", 1.0, true},
          {"fillet-oversized", "Fillet", 11.0, false},
          {"chamfer-oversized", "Chamfer", 11.0, false},
      };
  for (const auto &[case_id, op_type, value, expected_success] : cases) {
    json first = run_candidate(case_id, op_type, value, expected_success);
    const json second =
        run_candidate(case_id, op_type, value, expected_success);
    first["repeatStable"] = same_repeat(first, second);
    check(first["repeatStable"].get<bool>(),
          std::string(case_id) + " repeat stability");
    std::cout << first.dump() << '\n';
  }
  for (const char *op_type : {"Fillet", "Chamfer"}) {
    json session_first = run_session_refusal(op_type);
    const json session_second = run_session_refusal(op_type);
    session_first["repeatStable"] = same_repeat(session_first, session_second);
    check(session_first["repeatStable"].get<bool>(),
          std::string(op_type) + " session repeat stability");
    std::cout << session_first.dump() << '\n';
  }
  return g_failures;
}
