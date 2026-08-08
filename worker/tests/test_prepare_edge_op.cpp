#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>

#include "kernel/fillet/EdgeContour.h"
#include "fillet_test_utils.h"
#include "protocol/Envelope.h"
#include "session/HistoryHash.h"
#include "session/PrepareEdgeOp.h"
#include "session/PlanExecutor.h"
#include "session/Session.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::protocol::Envelope;
using onecad::session::Session;
namespace kf = onecad::kernel::fillet;
namespace ft = onecad::tests::fillet;

namespace {

int failures = 0;

void check(bool condition, const std::string &message) {
  if (condition)
    return;
  std::fprintf(stderr, "FAIL: %s\n", message.c_str());
  ++failures;
}

TopoDS_Shape split_prism() {
  BRepBuilderAPI_MakePolygon polygon;
  for (const gp_Pnt &point : {gp_Pnt(0, 0, 0), gp_Pnt(5, 0, 0),
                              gp_Pnt(10, 0, 0), gp_Pnt(10, 10, 0),
                              gp_Pnt(0, 10, 0)})
    polygon.Add(point);
  polygon.Close();
  return BRepPrimAPI_MakePrism(BRepBuilderAPI_MakeFace(polygon.Wire()).Face(),
                               gp_Vec(0, 0, 10))
      .Shape();
}

void seed_head(Session &session,
               const std::vector<std::pair<std::string, TopoDS_Shape>> &seeds) {
  session.open("doc", 1, 1, "determinism");
  auto fence = session.fence_and_clone(1, 1, 1,
                                       onecad::session::kEmptyPrefixHash);
  onecad::session::ScratchJob job;
  job.job_id = 1;
  job.plan_document_revision = 1;
  job.bodies = std::move(fence.cloned_bodies);
  job.partition = std::move(fence.cloned_partition);
  job.prepared_snapshot_id = fence.prepared_snapshot_id;
  for (const auto &[id, shape] : seeds)
    job.bodies.create(id, "op_seed", shape);
  session.store_prepared(std::move(job));
  session.accept_prepared(1, 1, 1);
}

std::string tangent_seed(const TopoDS_Shape &body) {
  TopTools_IndexedMapOfShape edges;
  TopExp::MapShapes(body, TopAbs_EDGE, edges);
  for (int ordinal = 1; ordinal <= edges.Extent(); ++ordinal) {
    const auto contour = kf::analyze_edge_contours(
        body, {TopoDS::Edge(edges(ordinal))}, kf::EdgeOpMode::Fillet);
    if (contour.ok && contour.closure_ordinals.size() > 1)
      return "e:" + std::to_string(ordinal);
  }
  return {};
}

Envelope request(const std::string &mode, json picks, bool chain,
                 std::uint64_t snapshot = 1) {
  Envelope req;
  req.id = 22;
  req.verb = "PrepareEdgeOp";
  req.args = {{"snapshotId", snapshot},
              {"mode", mode},
              {"pickedEdges", std::move(picks)},
              {"chainTangentEdges", chain}};
  return req;
}

json pick(const std::string &body, const std::string &topo) {
  return {{"bodyId", body}, {"topoKey", topo}};
}

std::string refusal_code(const Envelope &response) {
  return response.ok.value_or(false) && response.result.contains("refusal") &&
                 !response.result["refusal"].is_null()
             ? response.result["refusal"].value("code", "")
             : std::string{};
}

std::vector<TopoDS_Edge> response_edges(const TopoDS_Shape &body,
                                        const Envelope &response) {
  std::vector<TopoDS_Edge> edges;
  for (const json &entry : response.result["edges"]) {
    const TopoDS_Shape shape =
        onecad::elementmap::ElementMapPartition::shape_for_topokey(
            body, entry.value("topoKey", ""));
    if (!shape.IsNull() && shape.ShapeType() == TopAbs_EDGE)
      edges.push_back(TopoDS::Edge(shape));
  }
  return edges;
}

bool execute_prepared(const TopoDS_Shape &body,
                      const std::vector<TopoDS_Edge> &edges,
                      const std::string &mode) {
  onecad::session::ScratchJob job;
  job.bodies.create("body_box", "op_box", body);
  json op = ft::op(body, edges, 1.0);
  op["opType"] = mode;
  op["params"]["mode"] = mode;
  op["params"]["tangentClosureVersion"] = 1;
  std::string last_sketch;
  const auto result = onecad::session::execute_candidate_op(
      job, op, "op_prepared", last_sketch, onecad::CancelToken{});
  return result.status == onecad::session::CandidateResult::Status::Ok;
}

void test_closure_and_chain_off() {
  const TopoDS_Shape body = split_prism();
  const std::string seed = tangent_seed(body);
  check(!seed.empty(), "split prism has operable tangent contour");
  Session session;
  seed_head(session, {{"body_1", body}});
  const json picks = json::array({pick("body_1", seed)});
  const Envelope fillet = onecad::session::handle_prepare_edge_op(
      session, request("Fillet", picks, true));
  check(fillet.ok.value_or(false) && refusal_code(fillet).empty(),
        "Fillet prepare accepts tangent closure");
  check(fillet.result["edges"].size() > 1,
        "Fillet prepare expands tangent contour");
  int picked_count = 0;
  for (const json &edge : fillet.result["edges"]) {
    picked_count += edge.value("picked", false) ? 1 : 0;
    check(edge.contains("anchor") && edge.contains("descriptor") &&
              !edge.contains("elementId"),
          "prepare returns evidence without minting");
  }
  check(picked_count == 1, "exactly one closure edge marked picked");

  const Envelope chamfer = onecad::session::handle_prepare_edge_op(
      session, request("Chamfer", picks, true));
  check(chamfer.result["edges"] == fillet.result["edges"],
        "Fillet and Chamfer freeze identical closure");
  const Envelope exact = onecad::session::handle_prepare_edge_op(
      session, request("Fillet", picks, false));
  check(refusal_code(exact) == "chainMismatch",
        "chain off refuses implicit contour expansion");
  check(exact.result["edges"].empty(),
        "chain mismatch refusal exposes no unpromoted closure");
  const Envelope repeat = onecad::session::handle_prepare_edge_op(
      session, request("Fillet", picks, true));
  check(repeat.result == fillet.result, "prepare response deterministic");

  const TopoDS_Shape box = BRepPrimAPI_MakeBox(10, 10, 10).Shape();
  Session legacy_session;
  seed_head(legacy_session, {{"body_1", box}});
  const Envelope legacy = onecad::session::handle_prepare_edge_op(
      legacy_session, request("Fillet", json::array({pick("body_1", "e:1")}), false));
  check(legacy.ok.value_or(false) && refusal_code(legacy).empty(),
        "Fillet prepare accepts seed-only chain-off");
  check(legacy.result["edges"].size() == 1,
        "seed-only chain-off keeps the legacy single-edge result");
  check(legacy.result["edges"][0]["picked"].get<bool>(),
        "seed-only chain-off keeps the picked edge");
}

void test_fence_and_cross_body() {
  const TopoDS_Shape first = BRepPrimAPI_MakeBox(10, 10, 10).Shape();
  const TopoDS_Shape second =
      BRepPrimAPI_MakeBox(gp_Pnt(20, 0, 0), 10, 10, 10).Shape();
  Session session;
  seed_head(session, {{"body_1", first}, {"body_2", second}});
  const Envelope stale = onecad::session::handle_prepare_edge_op(
      session,
      request("Fillet", json::array({pick("body_1", "e:1")}), true, 99));
  check(!stale.ok.value_or(true) && stale.error &&
            stale.error->code == "STALE_PREVIEW",
        "stale snapshot refused");
  const Envelope cross = onecad::session::handle_prepare_edge_op(
      session, request("Fillet",
                       json::array({pick("body_1", "e:1"),
                                    pick("body_2", "e:1")}),
                       true));
  check(cross.ok.value_or(false) && refusal_code(cross) == "crossBody",
        "cross-body selection refused as answer");
}

void test_disconnected_contours() {
  const TopoDS_Shape body = BRepPrimAPI_MakeBox(10, 10, 10).Shape();
  const std::vector<TopoDS_Edge> vertical = ft::vertical_edges(body);
  check(vertical.size() == 4, "box exposes four disconnected vertical contours");
  const std::string first = onecad::elementmap::ElementMapPartition::topokey_for_shape(
      body, vertical[0], onecad::kernel::elementmap::ElementKind::Edge);
  const std::string second = onecad::elementmap::ElementMapPartition::topokey_for_shape(
      body, vertical[2], onecad::kernel::elementmap::ElementKind::Edge);
  Session session;
  seed_head(session, {{"body_1", body}});
  const Envelope fillet = onecad::session::handle_prepare_edge_op(
      session, request("Fillet", json::array({pick("body_1", first),
                                              pick("body_1", second)}),
                       true));
  check(fillet.ok.value_or(false) && refusal_code(fillet).empty(),
        "multiple seed edges prepare successfully");
  check(fillet.result["edges"].size() >= 2,
        "multiple seed edges keep multiple contour entries");
  int picked_count = 0;
  for (const json &edge : fillet.result["edges"])
    picked_count += edge.value("picked", false) ? 1 : 0;
  check(picked_count == 2, "multiple seed edges preserve both picked seeds");
  const std::vector<TopoDS_Edge> closure = response_edges(body, fillet);
  check(closure.size() == 2, "box picks remain two disconnected contours");
  check(execute_prepared(body, closure, "Fillet"),
        "version-1 Fillet executes two disconnected contours");
  check(execute_prepared(body, closure, "Chamfer"),
        "version-1 Chamfer executes two disconnected contours");
}

void test_complete_tangent_closure_executes() {
  const TopoDS_Shape body = split_prism();
  const std::string seed = tangent_seed(body);
  Session session;
  seed_head(session, {{"body_1", body}});
  const Envelope prepared = onecad::session::handle_prepare_edge_op(
      session, request("Fillet", json::array({pick("body_1", seed)}), true));
  const std::vector<TopoDS_Edge> closure = response_edges(body, prepared);
  check(closure.size() > 1, "prepared tangent closure contains every contour edge");
  check(execute_prepared(body, closure, "Fillet"),
        "complete version-1 tangent Fillet executes");
  check(execute_prepared(body, closure, "Chamfer"),
        "complete version-1 tangent Chamfer executes");
}

void test_versioned_closure_drift() {
  const TopoDS_Shape body = split_prism();
  const std::string seed = tangent_seed(body);
  const TopoDS_Edge edge = TopoDS::Edge(
      onecad::elementmap::ElementMapPartition::shape_for_topokey(body, seed));
  onecad::session::ScratchJob job;
  job.bodies.create("body_box", "op_box", body);
  json op = ft::op(body, {edge}, 1.0);
  op["params"]["tangentClosureVersion"] = 1;
  std::string last_sketch;
  const auto drift = onecad::session::execute_candidate_op(
      job, op, "op_fillet", last_sketch, onecad::CancelToken{});
  const json diagnostics = onecad::session::candidate_diagnostics(drift);
  check(drift.status == onecad::session::CandidateResult::Status::Failed &&
            !diagnostics.empty() &&
            diagnostics[0].value("code", "") ==
                "EDGE_OP_TANGENT_CLOSURE_CHANGED",
        "versioned incomplete closure fails with drift diagnostic");
  check(drift.needs_repair.empty() &&
            job.bodies.get("body_box")->geom.IsSame(body),
        "closure drift is not NeedsRepair and leaves body unchanged");

  op["params"].erase("tangentClosureVersion");
  const auto legacy = onecad::session::execute_candidate_op(
      job, op, "op_legacy", last_sketch, onecad::CancelToken{});
  const json legacy_diagnostics = onecad::session::candidate_diagnostics(legacy);
  check(legacy.status == onecad::session::CandidateResult::Status::Ok &&
            legacy_diagnostics.empty(),
        "legacy record executes current seed-only behavior without auto-upgrade");
}

void test_upstream_edit_rebind_detects_closure_drift() {
  const TopoDS_Shape predecessor = BRepPrimAPI_MakeBox(10, 10, 10).Shape();
  const TopoDS_Edge stored_edge = ft::all_edges(predecessor).front();
  const TopoDS_Shape edited = split_prism();
  const std::string rebound_key = tangent_seed(edited);
  const TopoDS_Edge rebound = TopoDS::Edge(
      onecad::elementmap::ElementMapPartition::shape_for_topokey(edited,
                                                                 rebound_key));

  onecad::session::ScratchJob job;
  job.edited_from = 0;
  job.bodies.create("body_box", "op_upstream", edited);
  job.partition.mint("body_box", "el_0",
                     onecad::kernel::elementmap::ElementKind::Edge, rebound,
                     edited);
  const auto *before_binding = job.partition.find("el_0");
  const TopoDS_Shape before_shape = before_binding->shape;
  const std::string before_key = before_binding->topo_key;

  json op = ft::op(predecessor, {stored_edge}, 1.0);
  op["stepIndex"] = 1;
  op["params"]["tangentClosureVersion"] = 1;
  std::string last_sketch;
  const auto drift = onecad::session::execute_candidate_op(
      job, op, "op_after_edit", last_sketch, onecad::CancelToken{});
  const json diagnostics = onecad::session::candidate_diagnostics(drift);
  const auto *after_binding = job.partition.find("el_0");
  check(drift.status == onecad::session::CandidateResult::Status::Failed &&
            !diagnostics.empty() &&
            diagnostics[0].value("code", "") ==
                "EDGE_OP_TANGENT_CLOSURE_CHANGED",
        "upstream split after persistent-id rebind detects closure drift");
  check(job.bodies.get("body_box")->geom.IsSame(edited) && after_binding &&
            after_binding->topo_key == before_key &&
            after_binding->shape.IsSame(before_shape),
        "upstream closure drift rolls body and rebound partition back exactly");
}

void test_identity_fallback_collision_refuses() {
  const TopoDS_Shape body = BRepPrimAPI_MakeBox(10, 10, 10).Shape();
  const TopoDS_Edge edge = ft::all_edges(body).front();
  onecad::session::ScratchJob job;
  job.bodies.create("body_box", "op_box", body);
  job.partition.mint("body_box", "el_0",
                     onecad::kernel::elementmap::ElementKind::Edge, edge, body);
  json op = ft::op(body, {edge, edge}, 1.0);
  op["params"]["tangentClosureVersion"] = 1;
  std::string last_sketch;
  const auto result = onecad::session::execute_candidate_op(
      job, op, "op_collision", last_sketch, onecad::CancelToken{});
  const json diagnostics = onecad::session::candidate_diagnostics(result);
  check(result.status == onecad::session::CandidateResult::Status::Failed &&
            !diagnostics.empty() &&
            diagnostics[0].value("code", "") ==
                "EDGE_OP_TANGENT_CLOSURE_CHANGED",
        "mixed identity and descriptor fallback cannot collapse onto one edge");
  check(job.bodies.get("body_box")->geom.IsSame(body) &&
            job.partition.size() == 1 && job.partition.contains("el_0") &&
            !job.partition.contains("el_1"),
        "collision refusal rolls descriptor fallback promotion back");
}

} // namespace

int main() {
  test_closure_and_chain_off();
  test_fence_and_cross_body();
  test_disconnected_contours();
  test_complete_tangent_closure_executes();
  test_versioned_closure_drift();
  test_upstream_edit_rebind_detects_closure_drift();
  test_identity_fallback_collision_refuses();
  return failures;
}
