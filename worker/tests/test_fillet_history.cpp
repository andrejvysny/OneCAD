#include <cmath>
#include <cstdio>
#include <string>

#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <GProp_GProps.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRep_Tool.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>

#include "fillet_test_utils.h"
#include "ops/FilletChamferOp.h"
#include "session/PlanExecutor.h"
#include "session/Signatures.h"

namespace ft = onecad::tests::fillet;
namespace em = onecad::elementmap;
namespace km = onecad::kernel::elementmap;
namespace session = onecad::session;
namespace ops = onecad::ops;

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

// SCHEMA §7.3 `overflow: "auto"` (2026-11-01): the native build refuses r=11.2 on
// the top edge of a 20x30x4 box; the opted-in fallback consumes the 4 mm side face
// and keeps the top face as a history successor.
struct OverflowRun {
  ops::OpOutcome outcome;
  double volume = 0.0;
  bool consumed_gone = false;
  bool top_survived = false;
  TopoDS_Shape top_before;
};

OverflowRun run_overflow(const std::string &op_type, double value,
                         const nlohmann::json &overflow_params) {
  session::ScratchJob job;
  const TopoDS_Shape body = BRepPrimAPI_MakeBox(20.0, 30.0, 4.0).Shape();
  job.bodies.create("body_box", "op_native_box", body);
  TopoDS_Edge selected;
  for (const TopoDS_Edge &edge : ft::all_edges(body)) {
    TopoDS_Vertex a, b;
    TopExp::Vertices(edge, a, b);
    const gp_Pnt pa = BRep_Tool::Pnt(a), pb = BRep_Tool::Pnt(b);
    if (std::abs(pa.X()) < 1e-9 && std::abs(pb.X()) < 1e-9 &&
        std::abs(pa.Z() - 4.0) < 1e-9 && std::abs(pb.Z() - 4.0) < 1e-9)
      selected = edge;
  }
  check(!selected.IsNull(), "overflow seed edge found");
  TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
  TopExp::MapShapesAndAncestors(body, TopAbs_EDGE, TopAbs_FACE, edge_faces);
  job.partition.mint("body_box", "el_0", km::ElementKind::Edge, selected, body,
                     ft::input(body, selected, "el_0")["anchor"]);
  OverflowRun run;
  for (const TopoDS_Shape &face : edge_faces.FindFromKey(selected)) {
    const BRepAdaptor_Surface surface(TopoDS::Face(face));
    const bool top = surface.GetType() == GeomAbs_Plane &&
                     std::abs(surface.Plane().Axis().Direction().Z()) > 0.5;
    if (top) run.top_before = face;
    job.partition.mint("body_box", top ? "el_top" : "el_side", km::ElementKind::Face,
                       TopoDS::Face(face), body,
                       {{"worldPoint", top ? nlohmann::json{10.0, 15.0, 4.0}
                                           : nlohmann::json{0.0, 15.0, 2.0}}});
  }
  nlohmann::json op = ft::op(body, {selected}, value);
  op["opType"] = op_type;
  op["params"]["mode"] = op_type;
  for (auto it = overflow_params.begin(); it != overflow_params.end(); ++it)
    op["params"][it.key()] = it.value();
  std::string last_sketch_id;
  onecad::CancelToken cancel;
  ops::OpContext ctx{job.bodies, nullptr, job.partition, &last_sketch_id, false,
                     nlohmann::json::object(), &cancel};
  run.outcome = op_type == "Fillet" ? ops::execute_fillet(ctx, op, "op_fillet")
                                    : ops::execute_chamfer(ctx, op, "op_fillet");
  if (const auto *result = job.bodies.get("body_box")) {
    GProp_GProps props;
    BRepGProp::VolumeProperties(result->geom, props);
    run.volume = props.Mass();
  }
  const em::PartitionEntry *side = job.partition.find("el_side");
  const em::PartitionEntry *top = job.partition.find("el_top");
  run.consumed_gone = side == nullptr || side->shape.IsNull();
  run.top_survived = top != nullptr && !top->shape.IsNull();
  return run;
}

bool has_diagnostic(const ops::OpOutcome &outcome, const char *code) {
  for (const nlohmann::json &d : outcome.diagnostics)
    if (d.value("code", "") == code)
      return true;
  return false;
}

void check_overflow_history(const OverflowRun &run, bool fillet, const char *label) {
  const std::string tag = label;
  check(run.outcome.topology_history.size() == 1, tag + ": one topology history");
  if (run.outcome.topology_history.size() != 1)
    return;
  const session::TopologyBodyHistory &h = run.outcome.topology_history[0];
  check(h.conflicts.empty(), tag + ": zero ledger conflicts (got " +
                                 std::to_string(h.conflicts.size()) + ")");
  bool blend_birth = false;
  for (const session::TopologyBirth &birth : h.births) {
    if (birth.after.ShapeType() != TopAbs_FACE)
      continue;
    const BRepAdaptor_Surface s(TopoDS::Face(birth.after));
    blend_birth = blend_birth || s.GetType() == (fillet ? GeomAbs_Cylinder : GeomAbs_Plane);
  }
  check(blend_birth, tag + ": blend face is a birth");
  bool top_successor = false;
  for (const session::TopologySuccessor &succ : h.successors)
    for (const TopoDS_Shape &before : succ.before)
      top_successor = top_successor || before.IsSame(run.top_before);
  check(top_successor, tag + ": top face is a successor");
}

void test_overflow_auto() {
  const OverflowRun off = run_overflow("Fillet", 11.2, nlohmann::json::object());
  check(off.outcome.status == ops::OpOutcome::Status::Failed,
        "native-only r=11.2 still fails without overflow");
  const OverflowRun on = run_overflow("Fillet", 11.2, {{"overflow", "auto"}, {"overflowVersion", 1}});
  check(on.outcome.status == ops::OpOutcome::Status::Ok, "overflow auto publishes r=11.2");
  check(std::abs(on.volume - 2085.812877) < 1e-6, "overflow retained volume");
  check(on.consumed_gone, "consumed side face leaves the partition");
  check(on.top_survived, "top face survives as a history successor");
  check(has_diagnostic(on.outcome, "FILLET_OVERFLOW_TRIMMED"),
        "FILLET_OVERFLOW_TRIMMED warning rides the Ok step");
  check_overflow_history(on, true, "fillet r=11.2");
  const OverflowRun chamfer =
      run_overflow("Chamfer", 6.2, {{"overflow", "auto"}, {"overflowVersion", 1}});
  check(chamfer.outcome.status == ops::OpOutcome::Status::Ok, "overflow auto publishes chamfer 6.2");
  check(has_diagnostic(chamfer.outcome, "CHAMFER_OVERFLOW_TRIMMED"), "chamfer warning rides the Ok step");
  check_overflow_history(chamfer, false, "chamfer 6.2");
  const OverflowRun bad = run_overflow("Fillet", 11.2, {{"overflow", "bogus"}});
  check(bad.outcome.status == ops::OpOutcome::Status::Failed &&
            bad.outcome.error_message.find("overflow") != std::string::npos,
        "bogus overflow value is refused by name");
  const OverflowRun wide = run_overflow(
      "Fillet", 11.2, {{"overflow", "auto"}, {"overflowVersion", 4294967297LL}});
  check(wide.outcome.status == ops::OpOutcome::Status::Failed &&
            wide.outcome.error_message.find("overflowVersion") != std::string::npos,
        "overflowVersion 2^32+1 is not narrowed to 1");
}

} // namespace

int main() {
  test_history_and_replay();
  test_imported_body_supported();
  test_overflow_auto();
  return failures;
}
