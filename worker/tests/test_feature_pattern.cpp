#include <cmath>
#include <cstdio>
#include <string>

#include <BRepAdaptor_Curve.hxx>
#include <BRepBuilderAPI_Copy.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <GeomAbs_CurveType.hxx>
#include <TopExp_Explorer.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>

#include "elementmap/ElementMapPartition.h"
#include "kernel/fillet/EdgeContour.h"
#include "nlohmann/json.hpp"
#include "session/PlanExecutor.h"
#include "session/FeaturePattern.h"
#include "session/ScratchJob.h"
#include "session/Session.h"
#include "session/ShapeMetrics.h"
#include "session/Signatures.h"
#include "util/Cancel.h"

using nlohmann::json;
namespace em = onecad::elementmap;
namespace km = onecad::kernel::elementmap;
namespace session = onecad::session;

namespace {
int failures = 0;
void check(bool value, const char* message) {
    if (!value) { std::fprintf(stderr, "FAIL: %s\n", message); ++failures; }
}

bool same_owner_ledger(const session::TopologyOwnerLedger& left,
                       const session::TopologyOwnerLedger& right) {
    if (left.entries().size() != right.entries().size()) return false;
    for (std::size_t i = 0; i < left.entries().size(); ++i) {
        const auto& a = left.entries()[i];
        const auto& b = right.entries()[i];
        if (a.body_id != b.body_id || !a.shape.IsSame(b.shape) ||
            a.claim.state != b.claim.state ||
            a.claim.producer_record_id != b.claim.producer_record_id) return false;
    }
    return true;
}

bool same_evidence(const session::ResolvedInputEvidenceLedger& left,
                   const session::ResolvedInputEvidenceLedger& right) {
    if (left.size() != right.size()) return false;
    for (const auto& [id, op] : left) {
        const auto found = right.find(id);
        if (found == right.end() || found->second.effective_hash != op.effective_hash ||
            found->second.inputs.size() != op.inputs.size()) return false;
        for (std::size_t i = 0; i < op.inputs.size(); ++i) {
            const auto& a = op.inputs[i]; const auto& b = found->second.inputs[i];
            if (a.input_index != b.input_index || a.element_id != b.element_id ||
                a.body_id != b.body_id || a.kind != b.kind || a.descriptor != b.descriptor ||
                a.anchor != b.anchor || a.origin_state != b.origin_state ||
                a.producer_record_id != b.producer_record_id) return false;
        }
    }
    return true;
}

bool same_partition_body(const em::ElementMapPartition& left,
                         const em::ElementMapPartition& right,
                         const std::string& body_id) {
    const auto entries = left.entries_for_body(body_id);
    if (entries.size() != right.entries_for_body(body_id).size()) return false;
    for (const auto* a : entries) {
        const auto* b = right.find(a->element_id);
        if (!b || a->body_id != b->body_id || a->kind != b->kind ||
            a->topo_key != b->topo_key || !a->shape.IsSame(b->shape) ||
            a->anchor != b->anchor) return false;
    }
    return true;
}

std::vector<TopoDS_Shape> new_topology_for_fixture(
    const TopoDS_Shape& before, const TopoDS_Shape& after) {
    TopTools_IndexedMapOfShape old_shapes;
    TopExp::MapShapes(before, old_shapes);
    std::vector<TopoDS_Shape> result;
    for (TopExp_Explorer it(after, TopAbs_EDGE); it.More(); it.Next())
        if (old_shapes.FindIndex(it.Current()) == 0) result.push_back(it.Current());
    return result;
}

TopoDS_Shape nearest_face(const TopoDS_Shape& body, double x, double y, double z) {
    TopoDS_Shape best; double distance = 1e100;
    for (TopExp_Explorer it(body, TopAbs_FACE); it.More(); it.Next()) {
        const auto d = em::ElementMapPartition::describe(it.Current(), body);
        const double dx = d.center.X() - x, dy = d.center.Y() - y, dz = d.center.Z() - z;
        const double candidate = dx * dx + dy * dy + dz * dz;
        if (candidate < distance) { distance = candidate; best = it.Current(); }
    }
    return best;
}

TopoDS_Shape circular_edge(const TopoDS_Shape& body, double x, double y, double z) {
    TopoDS_Shape best; double distance = 1e100;
    for (TopExp_Explorer it(body, TopAbs_EDGE); it.More(); it.Next()) {
        BRepAdaptor_Curve curve(TopoDS::Edge(it.Current()));
        if (curve.GetType() != GeomAbs_Circle) continue;
        const gp_Pnt center = curve.Circle().Location();
        const double dx = center.X() - x, dy = center.Y() - y, dz = center.Z() - z;
        const double candidate = dx * dx + dy * dy + dz * dz;
        if (candidate < distance) { distance = candidate; best = it.Current(); }
    }
    return best;
}

TopoDS_Shape straight_edge(const TopoDS_Shape& body, double x, double y, double z) {
    TopoDS_Shape best; double distance = 1e100;
    for (TopExp_Explorer it(body, TopAbs_EDGE); it.More(); it.Next()) {
        BRepAdaptor_Curve curve(TopoDS::Edge(it.Current()));
        if (curve.GetType() != GeomAbs_Line) continue;
        const auto d = em::ElementMapPartition::describe(it.Current(), body);
        const double candidate = std::pow(d.center.X() - x, 2) +
            std::pow(d.center.Y() - y, 2) + std::pow(d.center.Z() - z, 2);
        if (candidate < distance) { distance = candidate; best = it.Current(); }
    }
    return best;
}

json rectangle_sketch() {
    return {{"opType", "Sketch"}, {"opId", "record_sketch"},
        {"params", {{"sketchId", "source_sketch"}, {"plane", {{"kind", "XY"}}},
            {"entities", json::array({
                {{"id", "a"}, {"type", "Line"}, {"p0", {0, 0}}, {"p1", {10, 0}}},
                {{"id", "b"}, {"type", "Line"}, {"p0", {10, 0}}, {"p1", {10, 8}}},
                {{"id", "c"}, {"type", "Line"}, {"p0", {10, 8}}, {"p1", {0, 8}}},
                {{"id", "d"}, {"type", "Line"}, {"p0", {0, 8}}, {"p1", {0, 0}}}})},
            {"constraints", json::array()}}}};
}

json ref(const std::string& body_id, const std::string& id, const char* kind,
         const TopoDS_Shape& shape, const TopoDS_Shape& body,
         double x, double y, double z) {
    return {{"primary", {{"bodyId", body_id}, {"elementId", id}, {"kind", kind}}},
            {"intent", {{"kind", kind}, {"descriptor",
                em::ElementMapPartition::descriptor_to_json(
                    em::ElementMapPartition::describe(shape, body))}}},
            {"anchor", {{"worldPoint", {x, y, z}}}}};
}

json body_ref() {
    return {{"primary", {{"bodyId", "body_1"}, {"elementId", "body_1"}, {"kind", "body"}}}};
}

json hole(const TopoDS_Shape& body, double x, double y) {
    const TopoDS_Shape top = nearest_face(body, 20, 20, 10);
    return {{"opType", "Hole"}, {"opId", "hole_seed"},
            {"inputs", json::array({body_ref(), ref("body_1", "face_top", "face", top,
                                                       body, x, y, 10)})},
            {"params", {{"targetBodyId", "body_1"}, {"point", {x, y, 10.0}},
                        {"holeType", "simple"}, {"diameter", 4.0}, {"depth", nullptr},
                        {"resultPolicyVersion", 2}}}};
}

session::CandidateResult execute(session::ScratchJob& job, const json& op,
                                 std::string& last, onecad::CancelToken& cancel) {
    return session::execute_candidate_op(job, op, op.value("opId", "pattern"), last, cancel);
}

void test_hole_chamfer_circular() {
    session::ScratchJob job; std::string last; onecad::CancelToken cancel;
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(40, 40, 10).Shape();
    job.bodies.create("body_1", "box", box);
    json source_hole = hole(box, 30, 20);
    source_hole["opId"] = "record_hole";
    auto seed_hole = execute(job, source_hole, last, cancel);
    if (seed_hole.status != session::CandidateResult::Status::Ok)
        std::fprintf(stderr, "seed hole: %s %s repairs=%s\n", seed_hole.error_code.c_str(),
                     seed_hole.error_message.c_str(), json(seed_hole.needs_repair).dump().c_str());
    check(seed_hole.status == session::CandidateResult::Status::Ok, "seed hole succeeds");
    const TopoDS_Shape after_hole = job.bodies.get("body_1")->geom;
    const TopoDS_Shape rim = circular_edge(after_hole, 30, 20, 10);
    const TopoDS_Shape chamfer_reference = nearest_face(after_hole, 30, 20, 5);
    check(!rim.IsNull(), "seed hole rim found");
    const json edge_ref = ref("body_1", "edge_rim", "edge", rim, after_hole, 30, 20, 10);
    const TopoDS_Shape congruent_copy = BRepBuilderAPI_Copy(rim).Shape();
    TopoDS_Shape ambiguous_match;
    check(session::match_feature_pattern_produced_ref(
              after_hole, edge_ref, {rim, congruent_copy}, ambiguous_match) == 2 &&
              ambiguous_match.IsNull(),
          "distinct congruent OCCT producer candidates are ambiguous");
    const json chamfer_face_ref = ref("body_1", "face_hole_wall", "face",
                                      chamfer_reference, after_hole, 32, 20, 5);
    json source_chamfer = {{"opType", "Chamfer"}, {"opId", "record_chamfer"},
        {"inputs", json::array({edge_ref, chamfer_face_ref})},
        {"params", {{"mode", "Chamfer"}, {"radius", 0.5}, {"distance2", 0.5},
                    {"edgeIds", json::array({"edge_rim"})},
                    {"referenceFaces", json::array({{{"edgeId", "edge_rim"},
                                                        {"faceId", "face_hole_wall"}}})},
                    {"chainTangentEdges", false}, {"tangentClosureVersion", 1}}}};
    auto seed_chamfer = execute(job, source_chamfer, last, cancel);
    if (seed_chamfer.status != session::CandidateResult::Status::Ok)
        std::fprintf(stderr, "seed chamfer: %s %s repairs=%s\n", seed_chamfer.error_code.c_str(),
                     seed_chamfer.error_message.c_str(), json(seed_chamfer.needs_repair).dump().c_str());
    check(seed_chamfer.status == session::CandidateResult::Status::Ok, "seed chamfer succeeds");
    const double before = session::shape_volume(job.bodies.get("body_1")->geom);
    session::ScratchJob hole_only_job = job;

    source_hole["sourceRecordId"] = "record_hole";
    source_chamfer["sourceRecordId"] = "record_chamfer";
    json pattern = {{"opType", "FeaturePattern"}, {"opId", "pattern_circular"},
        {"inputs", json::array()},
        {"params", {{"semanticsVersion", 1}, {"count", 4},
            {"sourceRecordIds", json::array({"record_hole", "record_chamfer"})},
            {"layout", {{"kind", "Circular"}, {"axisOrigin", {20.0, 20.0, 0.0}},
                        {"axisDirection", {0.0, 0.0, 1.0}}, {"angleDeg", 360.0}}},
            {"sourceOps", json::array({source_hole, source_chamfer})}}}};
    auto candidate = execute(job, pattern, last, cancel);
    if (candidate.status != session::CandidateResult::Status::Ok)
        std::fprintf(stderr, "pattern: %s %s repairs=%s\n", candidate.error_code.c_str(),
                     candidate.error_message.c_str(), json(candidate.needs_repair).dump().c_str());
    check(candidate.status == session::CandidateResult::Status::Ok,
          "Hole to Chamfer circular FeaturePattern succeeds");
    check(candidate.body_events.size() == 1 && candidate.body_events[0].kind == "modified",
          "pattern publishes one host modification");
    int virtual_faces = 0;
    for (const auto& entry : candidate.delta.added)
        if (entry.kind == "face" && entry.element_id != "face_hole_wall") ++virtual_faces;
    check(virtual_faces >= 3, "each cylindrical reference face is remapped to its instance");
    check(session::shape_volume(job.bodies.get("body_1")->geom) < before,
          "three patterned hole and chamfer instances remove material");
    json hole_only = pattern;
    hole_only["opId"] = "pattern_holes_only";
    hole_only["params"]["sourceRecordIds"] = json::array({"record_hole"});
    hole_only["params"]["sourceOps"] = json::array({source_hole});
    session::ScratchJob missing_job = hole_only_job;
    auto hole_only_candidate = execute(hole_only_job, hole_only, last, cancel);
    check(hole_only_candidate.status == session::CandidateResult::Status::Ok,
          "comparison hole-only pattern succeeds");
    check(session::shape_volume(job.bodies.get("body_1")->geom) <
              session::shape_volume(hole_only_job.bodies.get("body_1")->geom),
          "each patterned Chamfer removes material beyond patterned Hole");

    json missing = pattern;
    missing["opId"] = "pattern_missing_producer";
    missing["params"]["sourceOps"][1]["inputs"][0]["intent"]["descriptor"]["center"] =
        {999.0, 999.0, 999.0};
    const std::string missing_before = session::geometry_signature(missing_job.bodies);
    auto missing_candidate = execute(missing_job, missing, last, cancel);
    check(missing_candidate.status == session::CandidateResult::Status::NeedsRepair,
          "zero producer match halts as NeedsRepair before generic ladder");
    check(!missing_candidate.needs_repair.empty() &&
              missing_candidate.needs_repair[0].value("instance", 0) == 1 &&
              missing_candidate.needs_repair[0].value("sourceRecordId", "") == "record_chamfer",
          "producer NeedsRepair identifies instance and source");
    check(session::geometry_signature(missing_job.bodies) == missing_before,
          "producer NeedsRepair restores outer pattern state");
}

void test_mid_instance_failure_rolls_back() {
    session::ScratchJob job; std::string last; onecad::CancelToken cancel;
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(40, 40, 10).Shape();
    job.bodies.create("body_1", "box", box);
    json source = hole(box, 10, 20); source["sourceRecordId"] = "record_hole";
    const std::string signature = session::geometry_signature(job.bodies);
    const std::size_t partition_size = job.partition.size();
    const std::size_t sketch_size = job.sketches.size();
    json pattern = {{"opType", "FeaturePattern"}, {"opId", "pattern_fail"},
        {"inputs", json::array()},
        {"params", {{"semanticsVersion", 1}, {"count", 4},
            {"sourceRecordIds", json::array({"record_hole"})},
            {"layout", {{"kind", "Linear"}, {"direction", {1.0, 0.0, 0.0}},
                        {"spacing", 15.0}}}, {"sourceOps", json::array({source})}}}};
    auto candidate = execute(job, pattern, last, cancel);
    check(candidate.status != session::CandidateResult::Status::Ok,
          "later out-of-face instance fails");
    check(session::geometry_signature(job.bodies) == signature,
          "pattern failure restores full body state");
    check(job.partition.size() == partition_size, "pattern failure restores identity state");
    check(job.sketches.size() == sketch_size && last.empty(),
          "pattern failure restores sketch state");
    check(candidate.body_events.empty() && candidate.delta.empty(),
          "failed pattern publishes no nested events or delta");
}

void test_malformed_source_is_recoverable() {
    session::ScratchJob job; std::string last; onecad::CancelToken cancel;
    json pattern = {{"opType", "FeaturePattern"}, {"opId", "pattern_malformed"},
        {"params", {{"semanticsVersion", 1}, {"count", 2},
                    {"sourceRecordIds", json::array({"record_hole"})},
                    {"layout", {{"kind", "Linear"}, {"direction", {1, 0, 0}},
                                {"spacing", -2.0}}},
                    {"sourceOps", json::array({"not-an-object"})}}}};
    auto candidate = execute(job, pattern, last, cancel);
    check(candidate.status == session::CandidateResult::Status::Failed,
          "malformed source is OP_FAILED, not an escaped exception");
    for (const json& invalid : {json(2.5), json(1e100)}) {
        pattern["params"]["sourceOps"] = json::array({hole(
            BRepPrimAPI_MakeBox(4, 4, 4).Shape(), 2, 2)});
        pattern["params"]["sourceOps"][0]["sourceRecordId"] = "record_hole";
        pattern["params"]["count"] = invalid;
        candidate = execute(job, pattern, last, cancel);
        check(candidate.status == session::CandidateResult::Status::Failed,
              "fractional or overflowing count is rejected");
    }
}

void test_sketch_extrude_fillet_linear() {
    session::ScratchJob job; std::string last; onecad::CancelToken cancel;
    json sketch = rectangle_sketch();
    json extrude = {{"opType", "Extrude"}, {"opId", "record_extrude"},
        {"params", {{"sketchId", "source_sketch"}, {"distance", 6.0},
                    {"extrudeMode", "Blind"}, {"booleanMode", "NewBody"}}}};
    check(execute(job, sketch, last, cancel).status == session::CandidateResult::Status::Ok,
          "seed sketch succeeds");
    check(execute(job, extrude, last, cancel).status == session::CandidateResult::Status::Ok,
          "seed extrude succeeds");
    const TopoDS_Shape seed = job.bodies.get("body_record_extrude")->geom;
    const TopoDS_Shape edge = straight_edge(seed, 0, 0, 3);
    check(!edge.IsNull(), "seed straight fillet edge found");
    json edge_input = ref("body_record_extrude", "seed_edge", "edge", edge, seed, 0, 0, 3);
    TopoDS_Shape ambiguous;
    check(session::match_feature_pattern_produced_ref(
              seed, edge_input, {edge, BRepBuilderAPI_Copy(edge).Shape()}, ambiguous) == 2 &&
              ambiguous.IsNull(),
          "congruent straight producer twins are rejected as ambiguous");
    job.partition.mint("body_record_extrude", "seed_edge", km::ElementKind::Edge,
                       edge, seed, edge_input["anchor"]);
    json fillet = {{"opType", "Fillet"}, {"opId", "record_fillet"},
        {"inputs", json::array({edge_input})},
        {"params", {{"mode", "Fillet"}, {"radius", 1.0},
                    {"edgeIds", json::array({"seed_edge"})}}}};
    // Simulate an upstream extent edit: the tracked ElementId is current, while
    // the record's frozen authoring evidence still describes the old edge.
    fillet["inputs"][0]["intent"]["descriptor"]["center"] = {999.0, 999.0, 999.0};
    fillet["inputs"][0]["intent"]["descriptor"]["magnitude"] = 10.0;
    check(execute(job, fillet, last, cancel).status == session::CandidateResult::Status::Ok,
          "seed fillet succeeds");
    check(job.resolved_input_evidence.contains("record_fillet") &&
              job.resolved_input_evidence.at("record_fillet").inputs[0].descriptor["center"] !=
                  fillet["inputs"][0]["intent"]["descriptor"]["center"],
          "pre-op ledger captures current topology instead of stale record evidence");
    const TopoDS_Shape once_filleted = job.bodies.get("body_record_extrude")->geom;
    const TopoDS_Shape second_edge = straight_edge(once_filleted, -8, 10, 3);
    check(!second_edge.IsNull(), "second surviving Extrude edge found");
    json second_input = ref("body_record_extrude", "seed_edge_2", "edge",
                            second_edge, once_filleted, -8, 10, 3);
    job.partition.mint("body_record_extrude", "seed_edge_2", km::ElementKind::Edge,
                       second_edge, once_filleted, second_input["anchor"]);
    json fillet2 = {{"opType", "Fillet"}, {"opId", "record_fillet_2"},
        {"inputs", json::array({second_input})},
        {"params", {{"mode", "Fillet"}, {"radius", 0.5},
                    {"edgeIds", json::array({"seed_edge_2"})}}}};
    check(execute(job, fillet2, last, cancel).status == session::CandidateResult::Status::Ok,
          "second seed fillet succeeds on a different creator-owned edge");
    for (json* source : {&sketch, &extrude, &fillet, &fillet2})
        (*source)["sourceRecordId"] = (*source)["opId"];
    json pattern = {{"opType", "FeaturePattern"}, {"opId", "solid_pattern"},
        {"params", {{"semanticsVersion", 1}, {"count", 3},
            {"sourceRecordIds", {"record_sketch", "record_extrude", "record_fillet",
                                  "record_fillet_2"}},
            {"layout", {{"kind", "Linear"}, {"direction", {1, 0, 0}}, {"spacing", 20.0}}},
            {"sourceOps", json::array({sketch, extrude, fillet, fillet2})}}}};
    session::ScratchJob stale_job = job;
    const TopoDS_Shape stale_body = BRepPrimAPI_MakeBox(2, 2, 2).Shape();
    const TopoDS_Shape stale_edge = straight_edge(stale_body, 0, 0, 0);
    json stale_input = ref("body_record_extrude", "virtual_stale", "edge",
                           stale_edge, stale_body, 0, 0, 0);
    stale_job.partition.mint("body_record_extrude", "virtual_stale",
                             km::ElementKind::Edge, stale_edge, stale_body,
                             stale_input["anchor"]);
    json stale_nested = fillet;
    stale_nested["inputs"] = json::array({stale_input});
    em::ElementMapDelta stale_delta;
    const auto stale_repairs = session::feature_pattern_bind_instance_output_refs(
        stale_job, stale_nested, fillet, {}, stale_delta, 1, "record_fillet",
        {"record_sketch", "record_extrude", "record_fillet", "record_fillet_2"},
        "body_record_extrude");
    check(stale_repairs.size() == 1 && stale_delta.added.empty(),
          "stale virtual partition entry absent from live body fails closed");
    session::ScratchJob bare_job = job;
    json bare = pattern;
    bare["opId"] = "bare_solid_pattern";
    bare["params"]["count"] = 2;
    bare["params"]["sourceRecordIds"] = {"record_sketch", "record_extrude"};
    bare["params"]["sourceOps"] = json::array({sketch, extrude});
    check(execute(bare_job, bare, last, cancel).status ==
              session::CandidateResult::Status::Ok,
          "Sketch to Blind NewBody Extrude with zero modifiers is executable");
    json unsupported_mode = bare;
    unsupported_mode["opId"] = "unsupported_solid_pattern";
    unsupported_mode["params"]["sourceOps"][1]["params"]["booleanMode"] = "Intersect";
    const auto unsupported_result = execute(bare_job, unsupported_mode, last, cancel);
    check(unsupported_result.status == session::CandidateResult::Status::Failed &&
              unsupported_result.error_code == "UNSUPPORTED_OP" &&
              unsupported_result.error_message.find("record_extrude") != std::string::npos,
          "unsupported Extrude mode is refused with source context");
    json new_body_with_target = bare;
    new_body_with_target["params"]["sourceOps"][1]["params"]["targetBodyId"] = "body_1";
    const auto target_result = execute(bare_job, new_body_with_target, last, cancel);
    check(target_result.status == session::CandidateResult::Status::Failed &&
              target_result.error_code == "UNSUPPORTED_OP" &&
              target_result.error_message.find("record_extrude") != std::string::npos,
          "NewBody Extrude with targetBodyId is refused in lockstep with core");
    json wrong_second = bare;
    wrong_second["params"]["sourceOps"][1]["opType"] = "Hole";
    const auto wrong_second_result = execute(bare_job, wrong_second, last, cancel);
    check(wrong_second_result.status == session::CandidateResult::Status::Failed &&
              wrong_second_result.error_code == "UNSUPPORTED_OP" &&
              wrong_second_result.error_message.find("record_extrude index 1 (Hole)") !=
                  std::string::npos,
          "unsupported Sketch successor identifies source index 1");
    wrong_second["params"]["sourceOps"][1]["opType"] = "Fillet";
    const auto wrong_modifier_result = execute(bare_job, wrong_second, last, cancel);
    check(wrong_modifier_result.status == session::CandidateResult::Status::Failed &&
              wrong_modifier_result.error_code == "UNSUPPORTED_OP" &&
              wrong_modifier_result.error_message.find("record_extrude index 1 (Fillet)") !=
                  std::string::npos,
          "modifier cannot replace required Extrude at source index 1");
    json missing_profile = pattern;
    missing_profile["params"]["sourceOps"][1]["params"].erase("sketchId");
    auto missing_profile_result = execute(job, missing_profile, last, cancel);
    check(missing_profile_result.status == session::CandidateResult::Status::Failed,
          "Extrude cannot fall back to lastSketch inside FeaturePattern");
    json wrong_profile = pattern;
    wrong_profile["params"]["sourceOps"][1]["params"]["sketchId"] = "other";
    auto wrong_profile_result = execute(job, wrong_profile, last, cancel);
    check(wrong_profile_result.status == session::CandidateResult::Status::Failed,
          "Extrude must name the selected source Sketch");
    session::ScratchJob missing_evidence_job = job;
    missing_evidence_job.resolved_input_evidence.erase("record_fillet");
    auto missing_evidence = execute(missing_evidence_job, pattern, last, cancel);
    check(missing_evidence.status == session::CandidateResult::Status::NeedsRepair,
          "missing pre-op evidence fails closed");
    session::ScratchJob hash_mismatch_job = job;
    json hash_mismatch = pattern;
    hash_mismatch["params"]["sourceOps"][2]["params"]["radius"] = 1.25;
    auto hash_mismatch_result = execute(hash_mismatch_job, hash_mismatch, last, cancel);
    check(hash_mismatch_result.status == session::CandidateResult::Status::NeedsRepair,
          "source edit without replay cannot consume old evidence");
    session::ScratchJob failure_job = job;
    failure_job.bodies.create("body_solid_failure:1", "collision",
                              BRepPrimAPI_MakeBox(1, 1, 1).Shape());
    const std::string failure_signature = session::geometry_signature(failure_job.bodies);
    const std::size_t failure_partition = failure_job.partition.size();
    json late_failure = pattern;
    late_failure["opId"] = "solid_failure";
    auto failed = execute(failure_job, late_failure, last, cancel);
    check(failed.status == session::CandidateResult::Status::Failed,
          "second independent instance identity collision fails atomically");
    check(session::geometry_signature(failure_job.bodies) == failure_signature &&
              failure_job.partition.size() == failure_partition,
          "complete second-instance failure restores bodies and partition");
    const std::size_t evidence_count = job.resolved_input_evidence.size();
    auto candidate = execute(job, pattern, last, cancel);
    if (candidate.status != session::CandidateResult::Status::Ok)
        std::fprintf(stderr, "solid pattern: %s %s repairs=%s\n", candidate.error_code.c_str(),
                     candidate.error_message.c_str(), json(candidate.needs_repair).dump().c_str());
    check(candidate.status == session::CandidateResult::Status::Ok,
          "Sketch to Extrude to Fillet pattern succeeds");
    check(job.resolved_input_evidence.size() == evidence_count &&
              job.resolved_input_evidence.contains("record_fillet"),
          "virtual nested operations do not leak into source evidence ledger");
    check(job.bodies.contains("body_solid_pattern:0") &&
              job.bodies.contains("body_solid_pattern:1"),
          "independent instances receive canonical ordinal body IDs");
    check(candidate.body_events.size() == 2 &&
              candidate.body_events[0].kind == "created" &&
              candidate.body_events[1].kind == "created",
          "each independent instance publishes one created event");
    const auto first = session::compute_shape_metrics(
        job.bodies.get("body_solid_pattern:0")->geom);
    const auto second = session::compute_shape_metrics(
        job.bodies.get("body_solid_pattern:1")->geom);
    check(std::abs(first.bbox_min[0] - 12.0) < 1e-6 &&
              std::abs(first.bbox_max[0] - 20.0) < 1e-6 &&
              std::abs(first.bbox_min[1]) < 1e-6 &&
              std::abs(first.bbox_max[1] - 10.0) < 1e-6 &&
              std::abs(first.bbox_min[2]) < 1e-6 &&
              std::abs(first.bbox_max[2] - 6.0) < 1e-6,
          "XY instance preserves OneCAD nonstandard basis and exact translated bounds");
    check(std::abs(second.bbox_min[0] - 32.0) < 1e-6 &&
              first.face_count > 6 && second.face_count == first.face_count &&
              first.volume < 480.0 && first.volume > 400.0,
          "second instance translation and real fillet topology/volume are metrologically visible");
    check(job.bodies.size() == 3, "virtual nested body IDs do not leak");
    for (const std::string& id : {"body_solid_pattern:0", "body_solid_pattern:1"}) {
        const auto entries = job.partition.entries_for_body(id);
        for (const auto* entry : entries)
            check(entry->body_id == id, "pattern partition follows canonical body rename");
    }
    for (const auto* entry : job.partition.entries_for_body("body_record_extrude"))
        check(entry->body_id == "body_record_extrude", "seed partition remains isolated");
    for (const auto& entry : candidate.delta.added)
        check(entry.body_id == "body_solid_pattern:0" ||
                  entry.body_id == "body_solid_pattern:1",
              "published delta contains canonical body IDs only");
}

void test_canonical_body_partition_migration() {
    session::ScratchJob job;
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(3, 4, 5).Shape();
    job.bodies.create("body_virtual", "virtual", box);
    TopoDS_Shape edge;
    for (TopExp_Explorer it(box, TopAbs_EDGE); it.More(); it.Next()) {
        edge = it.Current(); break;
    }
    job.partition.mint("body_virtual", "survivor", km::ElementKind::Edge, edge, box);
    check(job.bodies.rename("body_virtual", "body_pattern:0", "pattern") &&
              !job.bodies.contains("body_virtual"),
          "canonical body rename removes virtual body identity");
    job.partition.rename_body("body_virtual", "body_pattern:0");
    const auto entries = job.partition.entries_for_body("body_pattern:0");
    check(entries.size() == 1 && entries[0]->element_id == "survivor" &&
              entries[0]->topo_key == em::ElementMapPartition::topokey_for_shape(
                  box, edge, km::ElementKind::Edge),
          "surviving element identity and topology key migrate to canonical body");
}

void test_sketch_revolve_new_body_pattern() {
    session::ScratchJob job; std::string last; onecad::CancelToken cancel;
    json sketch = rectangle_sketch();
    sketch["params"]["entities"].push_back(
        {{"id", "axis"}, {"type", "Line"}, {"p0", {-5, -4}}, {"p1", {-5, 12}},
         {"construction", true}});
    json revolve = {{"opType", "Revolve"}, {"opId", "record_revolve"},
        {"params", {{"sketchId", "source_sketch"}, {"regionId", ""},
                    {"angleDeg", 180.0}, {"booleanMode", "NewBody"},
                    {"axis", {{"kind", "sketchLine"},
                              {"sketchId", "source_sketch"}, {"lineId", "axis"}}}}}};
    check(execute(job, sketch, last, cancel).status == session::CandidateResult::Status::Ok,
          "Revolve pattern seed Sketch succeeds");
    check(execute(job, revolve, last, cancel).status == session::CandidateResult::Status::Ok,
          "Revolve pattern seed NewBody succeeds");
    sketch["sourceRecordId"] = "record_sketch";
    revolve["sourceRecordId"] = "record_revolve";
    json pattern = {{"opType", "FeaturePattern"}, {"opId", "revolve_pattern"},
        {"params", {{"semanticsVersion", 1}, {"count", 3},
                    {"sourceRecordIds", {"record_sketch", "record_revolve"}},
                    {"layout", {{"kind", "Linear"}, {"direction", {1, 0, 0}},
                                {"spacing", 30.0}}},
                    {"sourceOps", json::array({sketch, revolve})}}}};
    const auto result = execute(job, pattern, last, cancel);
    check(result.status == session::CandidateResult::Status::Ok &&
              job.bodies.contains("body_revolve_pattern:0") &&
              job.bodies.contains("body_revolve_pattern:1"),
          "Sketch-line Revolve NewBody instances use canonical child IDs");
    json bad_tail = pattern;
    bad_tail["params"]["sourceRecordIds"].push_back("bad_hole");
    json bad_hole = revolve;
    bad_hole["sourceRecordId"] = "bad_hole";
    bad_hole["opType"] = "Hole";
    bad_tail["params"]["sourceOps"].push_back(bad_hole);
    const auto bad_tail_result = execute(job, bad_tail, last, cancel);
    check(bad_tail_result.status == session::CandidateResult::Status::Failed &&
              bad_tail_result.error_message.find("bad_hole index 2 (Hole)") !=
                  std::string::npos,
          "Sketch-Revolve invalid modifier reports the modifier source");

    session::ScratchJob two_job; std::string two_last;
    json axis_sketch = sketch;
    axis_sketch["opId"] = "record_axis_sketch";
    axis_sketch["params"]["sketchId"] = "axis_sketch";
    check(execute(two_job, sketch, two_last, cancel).status ==
              session::CandidateResult::Status::Ok &&
              execute(two_job, axis_sketch, two_last, cancel).status ==
              session::CandidateResult::Status::Ok,
          "two selected Revolve sketches seed");
    json two_revolve = revolve;
    two_revolve["params"]["axis"]["sketchId"] = "axis_sketch";
    check(execute(two_job, two_revolve, two_last, cancel).status ==
              session::CandidateResult::Status::Ok,
          "separate axis Sketch seed Revolve succeeds");
    axis_sketch["sourceRecordId"] = "record_axis_sketch";
    json two_pattern = pattern;
    two_pattern["opId"] = "two_sketch_revolve_pattern";
    two_pattern["params"]["sourceRecordIds"] =
        {"record_sketch", "record_axis_sketch", "record_revolve"};
    two_pattern["params"]["sourceOps"] =
        json::array({sketch, axis_sketch, two_revolve});
    check(execute(two_job, two_pattern, two_last, cancel).status ==
              session::CandidateResult::Status::Ok,
          "profile and separate sketch-line axis are both cloned and remapped");
    two_pattern["params"]["sourceRecordIds"].push_back("bad_hole");
    two_pattern["params"]["sourceOps"].push_back(bad_hole);
    const auto two_bad = execute(two_job, two_pattern, two_last, cancel);
    check(two_bad.status == session::CandidateResult::Status::Failed &&
              two_bad.error_message.find("bad_hole index 3 (Hole)") != std::string::npos,
          "two-Sketch Revolve invalid modifier reports source index 3");
    json edge_axis = pattern;
    edge_axis["params"]["sourceOps"][1]["params"]["axis"] =
        {{"kind", "edge"}, {"bodyId", "body_record_revolve"}, {"edgeId", "e:1"}};
    const auto refused = execute(job, edge_axis, last, cancel);
    check(refused.status == session::CandidateResult::Status::Failed &&
              refused.error_code == "UNSUPPORTED_OP" &&
              refused.error_message.find("record_revolve index 1") != std::string::npos,
          "external body-edge Revolve axis is a contextual future-adapter refusal");
}

void test_shared_host_extrude_add_pattern() {
    session::ScratchJob job; std::string last; onecad::CancelToken cancel;
    const TopoDS_Shape host = BRepPrimAPI_MakeBox(gp_Pnt(-100, -100, 0), 200, 200, 5).Shape();
    job.bodies.create("body_host", "host", host);
    json sketch = rectangle_sketch();
    json add = {{"opType", "Extrude"}, {"opId", "record_add"},
        {"params", {{"sketchId", "source_sketch"}, {"distance", 10.0},
                    {"extrudeMode", "Blind"}, {"booleanMode", "Add"},
                    {"targetBodyId", "body_host"}}}};
    check(execute(job, sketch, last, cancel).status == session::CandidateResult::Status::Ok &&
              execute(job, add, last, cancel).status == session::CandidateResult::Status::Ok,
          "shared-host seed Sketch and Extrude Add succeed");
    const double seed_volume = session::compute_shape_metrics(job.bodies.get("body_host")->geom).volume;
    sketch["sourceRecordId"] = "record_sketch";
    add["sourceRecordId"] = "record_add";
    session::ScratchJob modifier_job = job;
    std::vector<TopoDS_Shape> modifier_edges;
    for (const TopoDS_Shape& produced : new_topology_for_fixture(
             host, modifier_job.bodies.get("body_host")->geom)) {
        if (produced.ShapeType() != TopAbs_EDGE) continue;
        BRepAdaptor_Curve curve(TopoDS::Edge(produced));
        const auto descriptor = em::ElementMapPartition::describe(
            produced, modifier_job.bodies.get("body_host")->geom);
        if (curve.GetType() == GeomAbs_Line && descriptor.center.Z() > 5.0) {
            modifier_edges.push_back(produced);
        }
    }
    check(modifier_edges.size() >= 2,
          "shared-host Add exposes distinct straight modifier edges");
    const TopoDS_Shape modifier_edge = modifier_edges.front();
    const auto modifier_descriptor = em::ElementMapPartition::describe(
        modifier_edge, modifier_job.bodies.get("body_host")->geom);
    json modifier_input = ref(
        "body_host", "shared_add_edge", "edge", modifier_edge,
        modifier_job.bodies.get("body_host")->geom, modifier_descriptor.center.X(),
        modifier_descriptor.center.Y(), modifier_descriptor.center.Z());
    modifier_job.partition.mint(
        "body_host", "shared_add_edge", km::ElementKind::Edge, modifier_edge,
        modifier_job.bodies.get("body_host")->geom, modifier_input["anchor"]);
    json modifier = {{"opType", "Fillet"}, {"opId", "record_modifier"},
        {"inputs", json::array({modifier_input})},
        {"params", {{"mode", "Fillet"}, {"radius", 0.5},
                    {"edgeIds", json::array({"shared_add_edge"})}}}};
    TopTools_IndexedMapOfShape all_pre_fillet_edges;
    TopExp::MapShapes(modifier_job.bodies.get("body_host")->geom, TopAbs_EDGE,
                      all_pre_fillet_edges);
    check(execute(modifier_job, modifier, last, cancel).status ==
              session::CandidateResult::Status::Ok,
          "shared-host seed Add Fillet succeeds");
    TopoDS_Shape surviving_add_edge;
    json later_modifier;
    for (const auto& owned : modifier_job.topology_owners.entries()) {
        if (owned.body_id == "body_host" && owned.shape.ShapeType() == TopAbs_EDGE &&
            owned.claim.state == session::OriginState::Known &&
            owned.claim.producer_record_id == "record_add" &&
            all_pre_fillet_edges.FindIndex(owned.shape) == 0) {
            session::ScratchJob trial = modifier_job;
            const TopoDS_Shape trial_body = trial.bodies.get("body_host")->geom;
            const auto contour = onecad::kernel::fillet::analyze_edge_contours(
                trial_body, {TopoDS::Edge(owned.shape)},
                onecad::kernel::fillet::EdgeOpMode::Chamfer);
            if (!contour.ok || contour.closure_ordinals.empty()) continue;
            TopTools_IndexedMapOfShape trial_edges;
            TopExp::MapShapes(trial_body, TopAbs_EDGE, trial_edges);
            json inputs = json::array();
            json edge_ids = json::array();
            for (const int ordinal : contour.closure_ordinals) {
                if (ordinal <= 0 || ordinal > trial_edges.Extent()) continue;
                const TopoDS_Shape& edge = trial_edges(ordinal);
                const auto descriptor = em::ElementMapPartition::describe(edge, trial_body);
                const std::string edge_id =
                    "shared_add_edge_b_" + std::to_string(ordinal);
                json input = ref("body_host", edge_id, "edge", edge, trial_body,
                                 descriptor.center.X(), descriptor.center.Y(),
                                 descriptor.center.Z());
                trial.partition.mint("body_host", edge_id, km::ElementKind::Edge, edge,
                                     trial_body, input["anchor"]);
                inputs.push_back(std::move(input));
                edge_ids.push_back(edge_id);
            }
            json candidate = {{"opType", "Chamfer"},
                {"opId", "record_later_modifier"}, {"inputs", inputs},
                {"params", {{"mode", "Chamfer"}, {"radius", 0.05},
                            {"edgeIds", edge_ids},
                            {"chainTangentEdges", false},
                            {"tangentClosureVersion", 1}}}};
            const auto trial_result = execute(trial, candidate, last, cancel);
            if (trial_result.status != session::CandidateResult::Status::Ok) continue;
            surviving_add_edge = owned.shape;
            later_modifier = std::move(candidate);
            modifier_job = std::move(trial);
            break;
        }
    }
    check(!surviving_add_edge.IsNull(),
          "intervening Fillet retains a Modified descendant owned by Add");
    if (surviving_add_edge.IsNull()) return;
    check(!later_modifier.is_null(),
          "shared-host seed Chamfer consumes Add edge surviving an intervening Fillet");
    const double modifier_seed_volume = session::compute_shape_metrics(
        modifier_job.bodies.get("body_host")->geom).volume;
    modifier["sourceRecordId"] = "record_modifier";
    later_modifier["sourceRecordId"] = "record_later_modifier";
    json modifier_pattern = {{"opType", "FeaturePattern"},
        {"opId", "shared_add_modifier_pattern"},
        {"params", {{"semanticsVersion", 1}, {"count", 3},
                    {"sourceRecordIds", {"record_sketch", "record_add", "record_modifier",
                                         "record_later_modifier"}},
                    {"layout", {{"kind", "Linear"}, {"direction", {1, 0, 0}},
                                {"spacing", 20.0}}},
                    {"sourceOps", json::array({sketch, add, modifier, later_modifier})}}}};
    session::ScratchJob modifier_seed_job = modifier_job;
    session::ScratchJob unknown_origin_job = modifier_seed_job;
    unknown_origin_job.resolved_input_evidence["record_modifier"].inputs[0].origin_state =
        session::OriginState::Unknown;
    const auto unknown_origin =
        execute(unknown_origin_job, modifier_pattern, last, cancel);
    check(unknown_origin.status == session::CandidateResult::Status::NeedsRepair,
          "shared-host modifier refuses unknown source ownership");
    session::ScratchJob missing_producer_job = modifier_seed_job;
    missing_producer_job.resolved_input_evidence["record_modifier"]
        .inputs[0].producer_record_id = "record_modifier";
    const auto missing_producer =
        execute(missing_producer_job, modifier_pattern, last, cancel);
    check(missing_producer.status == session::CandidateResult::Status::NeedsRepair,
          "shared-host modifier never treats a missing selected producer as host support");
    const auto modifier_result =
        execute(modifier_job, modifier_pattern, last, cancel);
    const double modifier_pattern_volume = session::compute_shape_metrics(
        modifier_job.bodies.get("body_host")->geom).volume;
    const double host_volume = session::compute_shape_metrics(host).volume;
    const double expected_modifier_volume =
        host_volume + 3.0 * (modifier_seed_volume - host_volume);
    check(modifier_result.status == session::CandidateResult::Status::Ok &&
              modifier_result.body_ids == std::vector<std::string>{"body_host"} &&
              modifier_job.bodies.size() == 1 &&
              std::abs(modifier_pattern_volume - expected_modifier_volume) < 1e-5,
          "shared-host Add edges compose through Fillet then i-2 Chamfer");
    session::ScratchJob modifier_failure_job = modifier_seed_job;
    const std::string modifier_failure_signature =
        session::geometry_signature(modifier_failure_job.bodies);
    const std::size_t modifier_failure_partition = modifier_failure_job.partition.size();
    json modifier_failure = modifier_pattern;
    modifier_failure["params"]["layout"]["spacing"] = 90.0;
    const auto modifier_failure_result =
        execute(modifier_failure_job, modifier_failure, last, cancel);
    check(modifier_failure_result.status == session::CandidateResult::Status::Failed &&
              modifier_failure_result.error_message.find(
                  "instance 2 source record_add index 1") != std::string::npos &&
              session::geometry_signature(modifier_failure_job.bodies) ==
                  modifier_failure_signature &&
              modifier_failure_job.partition.size() == modifier_failure_partition,
          "shared-host Add modifier chain rolls back after a later instance fails");
    json pattern = {{"opType", "FeaturePattern"}, {"opId", "shared_add_pattern"},
        {"params", {{"semanticsVersion", 1}, {"count", 3},
                    {"sourceRecordIds", {"record_sketch", "record_add"}},
                    {"layout", {{"kind", "Linear"}, {"direction", {1, 0, 0}},
                                {"spacing", 20.0}}},
                    {"sourceOps", json::array({sketch, add})}}}};
    session::ScratchJob failure_job = job;
    json failure_pattern = pattern;
    failure_pattern["params"]["layout"]["spacing"] = 90.0;
    const std::size_t failure_partition = failure_job.partition.size();
    const auto failure = execute(failure_job, failure_pattern, last, cancel);
    check(failure.status == session::CandidateResult::Status::Failed &&
              failure_job.bodies.size() == 1 &&
              failure_job.partition.size() == failure_partition &&
              std::abs(session::compute_shape_metrics(
                  failure_job.bodies.get("body_host")->geom).volume - seed_volume) < 1e-6,
          "shared-host mid-instance failure restores host and identity state");
    const auto result = execute(job, pattern, last, cancel);
    const double patterned_volume =
        session::compute_shape_metrics(job.bodies.get("body_host")->geom).volume;
    check(result.status == session::CandidateResult::Status::Ok &&
              result.body_ids == std::vector<std::string>{"body_host"} &&
              job.bodies.size() == 1 && std::abs(patterned_volume - seed_volume - 800.0) < 1e-5,
          "shared-host Add modifies one host without pattern child bodies");
    json missing_host = pattern;
    missing_host["params"]["sourceOps"][1]["params"]["targetBodyId"] = "";
    const auto missing_host_result = execute(job, missing_host, last, cancel);
    check(missing_host_result.status == session::CandidateResult::Status::Failed &&
              missing_host_result.error_message.find("record_add index 1") != std::string::npos,
          "shared-host Add missing target identifies creator source");
    json unselected = pattern;
    unselected["params"]["sourceOps"][1]["params"]["sketchId"] = "other_sketch";
    const auto unselected_result = execute(job, unselected, last, cancel);
    check(unselected_result.status == session::CandidateResult::Status::Failed &&
              unselected_result.error_message.find("record_add index 1") != std::string::npos,
          "shared-host Add refuses unselected profile Sketch");
    session::ScratchJob circular_job;
    circular_job.bodies.create("body_host", "host", host);
    std::string circular_last;
    check(execute(circular_job, sketch, circular_last, cancel).status ==
              session::CandidateResult::Status::Ok &&
              execute(circular_job, add, circular_last, cancel).status ==
              session::CandidateResult::Status::Ok,
          "shared-host circular seed succeeds");
    const double circular_seed =
        session::compute_shape_metrics(circular_job.bodies.get("body_host")->geom).volume;
    pattern["params"]["layout"] = {{"kind", "Circular"}, {"axisOrigin", {-50, 0, 0}},
        {"axisDirection", {0, 0, 1}}, {"angleDeg", 360.0}};
    const auto circular = execute(circular_job, pattern, circular_last, cancel);
    const double circular_volume =
        session::compute_shape_metrics(circular_job.bodies.get("body_host")->geom).volume;
    check(circular.status == session::CandidateResult::Status::Ok &&
              circular_job.bodies.size() == 1 &&
              std::abs(circular_volume - circular_seed - 800.0) < 1e-5,
          "shared-host circular Add preserves host and exact added volume");

    const auto cut_case = [&](json layout, const char* label) {
        session::ScratchJob cut_job;
        cut_job.bodies.create("body_host", "host", host);
        std::string cut_last;
        json cut = add;
        cut["params"]["booleanMode"] = "Cut";
        check(execute(cut_job, sketch, cut_last, cancel).status ==
                  session::CandidateResult::Status::Ok &&
                  execute(cut_job, cut, cut_last, cancel).status ==
                      session::CandidateResult::Status::Ok,
              "shared-host Cut seed succeeds");
        const double cut_seed =
            session::compute_shape_metrics(cut_job.bodies.get("body_host")->geom).volume;
        check(session::feature_pattern_same_topology_set(host, host) &&
                  !session::feature_pattern_same_topology_set(
                      host, cut_job.bodies.get("body_host")->geom),
              "shared-host unchanged predicate distinguishes identical and removed topology");
        json cut_pattern = pattern;
        cut_pattern["opId"] = std::string("shared_cut_") + label;
        cut_pattern["params"]["layout"] = std::move(layout);
        cut_pattern["params"]["sourceOps"][1] = cut;
        const auto cut_result = execute(cut_job, cut_pattern, cut_last, cancel);
        const double cut_volume =
            session::compute_shape_metrics(cut_job.bodies.get("body_host")->geom).volume;
        check(cut_result.status == session::CandidateResult::Status::Ok &&
                  cut_result.body_ids == std::vector<std::string>{"body_host"} &&
                  cut_job.bodies.size() == 1 &&
                  std::abs(cut_seed - cut_volume - 800.0) < 1e-5,
              label);
    };
    cut_case({{"kind", "Linear"}, {"direction", {1, 0, 0}}, {"spacing", 20.0}},
             "shared-host linear Cut removes exact replica volume");
    cut_case({{"kind", "Circular"}, {"axisOrigin", {-50, 0, 0}},
              {"axisDirection", {0, 0, 1}}, {"angleDeg", 360.0}},
             "shared-host circular Cut removes exact replica volume");

    session::ScratchJob cut_modifier_job;
    cut_modifier_job.bodies.create("body_host", "host", host);
    std::string cut_modifier_last;
    json cut_modifier_source = add;
    cut_modifier_source["params"]["booleanMode"] = "Cut";
    check(execute(cut_modifier_job, sketch, cut_modifier_last, cancel).status ==
              session::CandidateResult::Status::Ok &&
              execute(cut_modifier_job, cut_modifier_source, cut_modifier_last, cancel).status ==
                  session::CandidateResult::Status::Ok,
          "shared-host Cut modifier seed succeeds");
    TopoDS_Shape cut_edge;
    for (const TopoDS_Shape& produced : new_topology_for_fixture(
             host, cut_modifier_job.bodies.get("body_host")->geom)) {
        if (produced.ShapeType() != TopAbs_EDGE) continue;
        BRepAdaptor_Curve curve(TopoDS::Edge(produced));
        if (curve.GetType() == GeomAbs_Line) {
            cut_edge = produced;
            break;
        }
    }
    check(!cut_edge.IsNull(), "shared-host Cut exposes a produced straight modifier edge");
    const auto cut_descriptor = em::ElementMapPartition::describe(
        cut_edge, cut_modifier_job.bodies.get("body_host")->geom);
    json cut_input = ref(
        "body_host", "shared_cut_edge", "edge", cut_edge,
        cut_modifier_job.bodies.get("body_host")->geom, cut_descriptor.center.X(),
        cut_descriptor.center.Y(), cut_descriptor.center.Z());
    cut_modifier_job.partition.mint(
        "body_host", "shared_cut_edge", km::ElementKind::Edge, cut_edge,
        cut_modifier_job.bodies.get("body_host")->geom, cut_input["anchor"]);
    json cut_fillet = {{"opType", "Fillet"}, {"opId", "record_cut_modifier"},
        {"inputs", json::array({cut_input})},
        {"params", {{"mode", "Fillet"}, {"radius", 0.5},
                    {"edgeIds", json::array({"shared_cut_edge"})}}}};
    check(execute(cut_modifier_job, cut_fillet, cut_modifier_last, cancel).status ==
              session::CandidateResult::Status::Ok,
          "shared-host seed Cut Fillet succeeds");
    const double cut_modifier_seed = session::compute_shape_metrics(
        cut_modifier_job.bodies.get("body_host")->geom).volume;
    cut_fillet["sourceRecordId"] = "record_cut_modifier";
    json cut_modifier_pattern = {{"opType", "FeaturePattern"},
        {"opId", "shared_cut_modifier_pattern"},
        {"params", {{"semanticsVersion", 1}, {"count", 3},
                    {"sourceRecordIds", {"record_sketch", "record_add",
                                         "record_cut_modifier"}},
                    {"layout", {{"kind", "Linear"}, {"direction", {1, 0, 0}},
                                {"spacing", 20.0}}},
                    {"sourceOps", json::array({sketch, cut_modifier_source, cut_fillet})}}}};
    const auto cut_modifier_result =
        execute(cut_modifier_job, cut_modifier_pattern, cut_modifier_last, cancel);
    const double cut_modifier_volume = session::compute_shape_metrics(
        cut_modifier_job.bodies.get("body_host")->geom).volume;
    const double expected_cut_modifier_volume =
        host_volume - 3.0 * (host_volume - cut_modifier_seed);
    check(cut_modifier_result.status == session::CandidateResult::Status::Ok &&
              cut_modifier_job.bodies.size() == 1 &&
              std::abs(cut_modifier_volume - expected_cut_modifier_volume) < 1e-5,
          "shared-host Cut followed by produced-edge Fillet patterns all instances");

    session::ScratchJob cut_failure_job;
    cut_failure_job.bodies.create("body_host", "host", host);
    std::string cut_failure_last;
    json cut = add;
    cut["params"]["booleanMode"] = "Cut";
    check(execute(cut_failure_job, sketch, cut_failure_last, cancel).status ==
              session::CandidateResult::Status::Ok &&
              execute(cut_failure_job, cut, cut_failure_last, cancel).status ==
                  session::CandidateResult::Status::Ok,
          "shared-host Cut failure seed succeeds");
    const double cut_failure_seed = session::compute_shape_metrics(
        cut_failure_job.bodies.get("body_host")->geom).volume;
    const std::size_t cut_failure_partition = cut_failure_job.partition.size();
    json cut_failure_pattern = pattern;
    cut_failure_pattern["params"]["sourceOps"][1] = cut;
    cut_failure_pattern["params"]["layout"] = {
        {"kind", "Linear"}, {"direction", {1, 0, 0}}, {"spacing", 90.0}};
    const auto cut_failure =
        execute(cut_failure_job, cut_failure_pattern, cut_failure_last, cancel);
    check(cut_failure.status == session::CandidateResult::Status::Failed &&
              cut_failure.error_message.find("instance 2 source record_add index 1") !=
                  std::string::npos &&
              cut_failure_job.bodies.size() == 1 &&
              cut_failure_job.partition.size() == cut_failure_partition &&
              std::abs(session::compute_shape_metrics(
                           cut_failure_job.bodies.get("body_host")->geom).volume -
                       cut_failure_seed) < 1e-6,
          "shared-host Cut mid-instance failure restores host and identity state");
}

void test_shared_add_hole_chamfer_pattern() {
    session::ScratchJob job; std::string last; onecad::CancelToken cancel;
    const TopoDS_Shape host = BRepPrimAPI_MakeBox(gp_Pnt(-100, -100, 0), 200, 200, 5).Shape();
    job.bodies.create("body_host", "root_host", host);
    const TopoDS_Shape top = nearest_face(host, 0, 0, 5);
    job.topology_owners.add({"body_host", top,
        {session::OriginState::Known, "root_host"}});
    json top_ref = ref("body_host", "retained_top", "face", top, host, -60, -60, 5);
    job.partition.mint("body_host", "retained_top", km::ElementKind::Face, top, host,
                       top_ref["anchor"]);
    json sketch = rectangle_sketch();
    json add = {{"opType", "Extrude"}, {"opId", "mixed_add"},
        {"params", {{"sketchId", "source_sketch"}, {"distance", 10.0},
                    {"extrudeMode", "Blind"}, {"booleanMode", "Add"},
                    {"targetBodyId", "body_host"}}}};
    check(execute(job, sketch, last, cancel).status == session::CandidateResult::Status::Ok &&
              execute(job, add, last, cancel).status == session::CandidateResult::Status::Ok,
          "mixed shared-host Add seed succeeds");
    const auto* retained = job.partition.find("retained_top");
    check(retained != nullptr, "Add preserves the retained host support face");
    if (!retained) return;
    const TopoDS_Shape after_add = job.bodies.get("body_host")->geom;
    top_ref = ref("body_host", "retained_top", "face", retained->shape,
                  after_add, -60, -60, 5);
    json hole_source = {{"opType", "Hole"}, {"opId", "mixed_hole"},
        {"inputs", json::array({
            json{{"primary", {{"bodyId", "body_host"},
                               {"elementId", "body_host"}, {"kind", "body"}}}},
            top_ref})},
        {"params", {{"targetBodyId", "body_host"}, {"face", top_ref},
                    {"point", {-60.0, -60.0, 5.0}}, {"holeType", "simple"},
                    {"diameter", 4.0}, {"depth", nullptr},
                    {"resultPolicyVersion", 2}}}};
    check(execute(job, hole_source, last, cancel).status ==
              session::CandidateResult::Status::Ok,
          "mixed retained-face Hole seed succeeds");
    const TopoDS_Shape after_hole = job.bodies.get("body_host")->geom;
    const TopoDS_Shape rim = circular_edge(after_hole, -60, -60, 5);
    check(!rim.IsNull(), "mixed Hole produces a local rim");
    json rim_ref = ref("body_host", "mixed_rim", "edge", rim, after_hole, -60, -60, 5);
    job.partition.mint("body_host", "mixed_rim", km::ElementKind::Edge, rim,
                       after_hole, rim_ref["anchor"]);
    json chamfer = {{"opType", "Chamfer"}, {"opId", "mixed_chamfer"},
        {"inputs", json::array({rim_ref})},
        {"params", {{"mode", "Chamfer"}, {"radius", 0.25},
                    {"edgeIds", json::array({"mixed_rim"})},
                    {"chainTangentEdges", false}, {"tangentClosureVersion", 1}}}};
    check(execute(job, chamfer, last, cancel).status ==
              session::CandidateResult::Status::Ok,
          "mixed Hole-local Chamfer seed succeeds");
    const auto& hole_evidence = job.resolved_input_evidence.at("mixed_hole").inputs;
    check(std::any_of(hole_evidence.begin(), hole_evidence.end(),
              [](const session::ResolvedInputEvidence& evidence) {
                  return evidence.kind == "face" &&
                      evidence.origin_state == session::OriginState::Known &&
                      evidence.producer_record_id == "root_host";
              }),
          "mixed Hole face is factual unselected retained-host support");
    for (json* source : {&sketch, &add, &hole_source, &chamfer})
        (*source)["sourceRecordId"] = (*source)["opId"];
    json pattern = {{"opType", "FeaturePattern"}, {"opId", "mixed_pattern"},
        {"params", {{"semanticsVersion", 1}, {"count", 3},
            {"sourceRecordIds", {"record_sketch", "mixed_add", "mixed_hole",
                                  "mixed_chamfer"}},
            {"layout", {{"kind", "Linear"}, {"direction", {1, 0, 0}},
                        {"spacing", 20.0}}},
            {"sourceOps", json::array({sketch, add, hole_source, chamfer})}}}};
    const auto before = session::geometry_signature(job.bodies);
    const double host_volume = session::compute_shape_metrics(host).volume;
    const double seed_volume = session::compute_shape_metrics(
        job.bodies.get("body_host")->geom).volume;
    session::ScratchJob failure_job = job;
    const std::size_t failure_partition = failure_job.partition.size();
    const std::size_t failure_owners = failure_job.topology_owners.entries().size();
    const session::ScratchJob failure_before = failure_job;
    json failure_pattern = pattern;
    failure_pattern["params"]["layout"]["spacing"] = 90.0;
    const auto failure = execute(failure_job, failure_pattern, last, cancel);
    check(failure.status == session::CandidateResult::Status::Failed &&
              failure.error_message.find("instance 2 source mixed_add index 1") !=
                  std::string::npos &&
              session::geometry_signature(failure_job.bodies) == before &&
              failure_job.partition.size() == failure_partition &&
              failure_job.topology_owners.entries().size() == failure_owners &&
              same_owner_ledger(failure_job.topology_owners,
                                failure_before.topology_owners) &&
              same_evidence(failure_job.resolved_input_evidence,
                            failure_before.resolved_input_evidence) &&
              same_partition_body(failure_job.partition, failure_before.partition,
                                  "body_host"),
          "mixed Add Hole Chamfer late-instance failure is atomic");
    const auto result = execute(job, pattern, last, cancel);
    const double patterned_volume = session::compute_shape_metrics(
        job.bodies.get("body_host")->geom).volume;
    const double expected_volume = host_volume + 3.0 * (seed_volume - host_volume);
    check(result.status == session::CandidateResult::Status::Ok &&
              result.body_ids == std::vector<std::string>{"body_host"} &&
              job.bodies.size() == 1 && session::geometry_signature(job.bodies) != before &&
              std::abs(patterned_volume - expected_volume) < 1e-5,
          "Add to retained-face Hole to local Chamfer patterns one host");
}

void test_solid_pattern_plane_frames() {
    const auto run = [](const json& plane, const json& layout, const char* label,
                        bool quarter_turn) {
        session::ScratchJob job; std::string last; onecad::CancelToken cancel;
        json sketch = rectangle_sketch();
        sketch["params"]["plane"] = plane;
        json extrude = {{"opType", "Extrude"}, {"opId", "record_extrude"},
            {"params", {{"sketchId", "source_sketch"}, {"distance", 6.0},
                        {"extrudeMode", "Blind"}, {"booleanMode", "NewBody"}}}};
        if (execute(job, sketch, last, cancel).status != session::CandidateResult::Status::Ok ||
            execute(job, extrude, last, cancel).status != session::CandidateResult::Status::Ok) {
            check(false, label); return;
        }
        const TopoDS_Shape seed = job.bodies.get("body_record_extrude")->geom;
        TopoDS_Shape edge;
        for (TopExp_Explorer it(seed, TopAbs_EDGE); it.More(); it.Next()) {
            BRepAdaptor_Curve curve(TopoDS::Edge(it.Current()));
            if (curve.GetType() == GeomAbs_Line) { edge = it.Current(); break; }
        }
        const auto edge_desc = em::ElementMapPartition::describe(edge, seed);
        json edge_input = ref("body_record_extrude", "seed_edge", "edge", edge, seed,
                              edge_desc.center.X(), edge_desc.center.Y(), edge_desc.center.Z());
        job.partition.mint("body_record_extrude", "seed_edge", km::ElementKind::Edge,
                           edge, seed, edge_input["anchor"]);
        json fillet = {{"opType", "Fillet"}, {"opId", "record_fillet"},
            {"inputs", json::array({edge_input})},
            {"params", {{"mode", "Fillet"}, {"radius", 0.75},
                        {"edgeIds", json::array({"seed_edge"})}}}};
        if (execute(job, fillet, last, cancel).status != session::CandidateResult::Status::Ok) {
            check(false, label); return;
        }
        const auto source_metrics = session::compute_shape_metrics(
            job.bodies.get("body_record_extrude")->geom);
        for (json* source : {&sketch, &extrude, &fillet})
            (*source)["sourceRecordId"] = (*source)["opId"];
        json pattern = {{"opType", "FeaturePattern"}, {"opId", "frame_pattern"},
            {"params", {{"semanticsVersion", 1}, {"count", 2},
                {"sourceRecordIds", {"record_sketch", "record_extrude", "record_fillet"}},
                {"layout", layout}, {"sourceOps", json::array({sketch, extrude, fillet})}}}};
        const auto result = execute(job, pattern, last, cancel);
        check(result.status == session::CandidateResult::Status::Ok, label);
        if (result.status != session::CandidateResult::Status::Ok) return;
        const auto child = session::compute_shape_metrics(
            job.bodies.get("body_frame_pattern:0")->geom);
        check(std::abs(child.volume - source_metrics.volume) < 1e-6 &&
                  child.face_count == source_metrics.face_count,
              "plane-frame instance preserves fillet topology and volume");
        if (quarter_turn) {
            const double ox = layout["axisOrigin"][0].get<double>();
            const double oy = layout["axisOrigin"][1].get<double>();
            check(std::abs(child.bbox_min[0] - (ox + oy - source_metrics.bbox_max[1])) < 1e-6 &&
                      std::abs(child.bbox_max[0] - (ox + oy - source_metrics.bbox_min[1])) < 1e-6 &&
                      std::abs(child.bbox_min[1] - (oy - ox + source_metrics.bbox_min[0])) < 1e-6 &&
                      std::abs(child.bbox_max[1] - (oy - ox + source_metrics.bbox_max[0])) < 1e-6,
                  "circular custom-plane instance rotates exact asymmetric bounds");
        } else {
            check(std::abs(child.bbox_min[0] - source_metrics.bbox_min[0] - 17.0) < 1e-6 &&
                      std::abs(child.bbox_max[0] - source_metrics.bbox_max[0] - 17.0) < 1e-6,
                  "canonical plane instance translates exact asymmetric bounds");
        }
    };
    const json linear = {{"kind", "Linear"}, {"direction", {1, 0, 0}}, {"spacing", 17.0}};
    run({{"kind", "XZ"}}, linear, "XZ canonical frame patterns", false);
    run({{"kind", "YZ"}}, linear, "YZ canonical frame patterns", false);
    run({{"kind", "custom"}, {"origin", {0, 0, 0}}, {"xAxis", {1, 0, 0}},
         {"yAxis", {0, 0, 1}}, {"normal", {0, -1, 0}}},
        {{"kind", "Circular"}, {"axisOrigin", {0, 0, 0}},
         {"axisDirection", {0, 0, 1}}, {"angleDeg", 180.0}},
        "custom frame circularly patterns", true);
    run({{"kind", "custom"}, {"origin", {2, 3, 0}}},
        {{"kind", "Circular"}, {"axisOrigin", {5, 7, 0}},
         {"axisDirection", {0, 0, 1}}, {"angleDeg", 180.0}},
        "partial custom frame uses XY defaults under off-origin rotation", true);
}

void test_evidence_survives_head_and_checkpoint_clones() {
    session::Session worker;
    worker.open("doc", 1, 7, "determinism");
    session::ScratchJob first;
    first.job_id = 1;
    first.plan_document_revision = 1;
    first.prepared_snapshot_id = 1;
    first.history_prefix_hash = "head-one";
    first.resolved_input_evidence["source"].effective_hash = "effective";
    worker.store_prepared(std::move(first));
    check(worker.accept_prepared(1, 1, 7).ok, "evidence head fixture accepts");
    auto head = worker.fence_and_clone(2, 1, 7, "head-one");
    check(head.status == session::FenceOutcome::Status::Ok &&
              head.cloned_input_evidence.contains("source"),
          "incremental head clone preserves source evidence");

    worker.save_checkpoint(3);
    session::ScratchJob second;
    second.job_id = 2;
    second.plan_document_revision = 2;
    second.prepared_snapshot_id = 2;
    second.history_prefix_hash = "head-two";
    worker.store_prepared(std::move(second));
    check(worker.accept_prepared(2, 2, 7).ok, "replacement head fixture accepts");
    const auto restored = worker.restore_checkpoint(3, "head-one", "checkpoint-three");
    session::BaseCheckpointRef base{3, "checkpoint-three"};
    auto checkpoint = worker.fence_and_clone(3, 2, 7, "head-one", &base);
    check(restored.restored && checkpoint.status == session::FenceOutcome::Status::Ok &&
              checkpoint.cloned_input_evidence.contains("source"),
          "in-memory checkpoint restore preserves source evidence");
}
}  // namespace

int main() {
    test_hole_chamfer_circular();
    test_mid_instance_failure_rolls_back();
    test_malformed_source_is_recoverable();
    test_sketch_extrude_fillet_linear();
    test_solid_pattern_plane_frames();
    test_canonical_body_partition_migration();
    test_sketch_revolve_new_body_pattern();
    test_shared_host_extrude_add_pattern();
    test_shared_add_hole_chamfer_pattern();
    test_evidence_survives_head_and_checkpoint_clones();
    return failures == 0 ? 0 : 1;
}
