#include <cstdio>
#include <string>

#include <TopExp.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>

#include "fillet_test_utils.h"
#include "session/PlanExecutor.h"
#include "session/Signatures.h"

namespace ft = onecad::tests::fillet;
namespace em = onecad::elementmap;
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

struct RunResult {
  session::CandidateResult candidate;
  std::string signature;
  std::string support_topo_key;
  bool selected_removed = false;
  bool support_survived = false;
};

RunResult run_once(const std::string &provenance) {
  session::ScratchJob job;
  job.history_prefix_hash = "history-before-fillet";
  const TopoDS_Shape body = ft::box();
  job.bodies.create("body_box", provenance, body);
  const TopoDS_Edge selected = ft::vertical_edges(body).front();

  TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
  TopExp::MapShapesAndAncestors(body, TopAbs_EDGE, TopAbs_FACE, edge_faces);
  const int edge_index = edge_faces.FindIndex(selected);
  check(edge_index > 0 && !edge_faces(edge_index).IsEmpty(),
        "selected edge has support face");
  const TopoDS_Face support = TopoDS::Face(edge_faces(edge_index).First());
  job.partition.mint("body_box", "el_0", km::ElementKind::Edge, selected, body,
                     ft::input(body, selected, "el_0")["anchor"]);
  job.partition.mint("body_box", "el_support", km::ElementKind::Face, support,
                     body, {{"worldPoint", {5.0, 0.0, 5.0}}});

  std::string last_sketch_id;
  const auto candidate = session::execute_candidate_op(
      job, ft::op(body, {selected}, 1.0), "op_fillet", last_sketch_id,
      onecad::CancelToken{});
  const em::PartitionEntry *rebound_support = job.partition.find("el_support");
  bool removed = false;
  for (const std::string &id : candidate.delta.removed)
    removed = removed || id == "el_0";
  return {candidate, session::geometry_signature(job.bodies),
          rebound_support ? rebound_support->topo_key : "",
          removed && !job.partition.contains("el_0"),
          rebound_support != nullptr && !rebound_support->shape.IsNull()};
}

void test_history_and_replay() {
  const RunResult first = run_once("op_native_box");
  check(first.candidate.status == session::CandidateResult::Status::Ok,
        "history fillet succeeds");
  check(first.selected_removed, "consumed selected edge is removed");
  check(first.support_survived, "support face survives history rebind");

  const RunResult second = run_once("op_native_box");
  check(second.candidate.status == first.candidate.status,
        "replay status stable");
  check(second.signature == first.signature,
        "replay geometry signature stable");
  check(second.candidate.delta.to_json() == first.candidate.delta.to_json(),
        "replay ElementId delta stable");
  check(second.support_topo_key == first.support_topo_key,
        "replay support ElementId binding stable");
}

void test_imported_body_supported() {
  const RunResult imported = run_once("ImportStep");
  check(imported.candidate.status == session::CandidateResult::Status::Ok,
        "imported body fillet remains supported");
  check(imported.selected_removed && imported.support_survived,
        "imported body history remains complete");
}

} // namespace

int main() {
  test_history_and_replay();
  test_imported_body_supported();
  return failures;
}
