// test_hole_points.cpp — ONECAD-361 ordered multi-point Hole (SCHEMA §7.3
// `Hole.points`, `resultPolicyVersion: 3`) on real OCCT, in-process.
//
// Exact analytic volumes (GProp, never a mesh) for many holes in one cut, the
// all-or-nothing preflight with its reasonCodes, per-point through-all, the
// single-solid fence, determinism, cancellation, the per-point topology-ledger
// slot, the slot-narrowed ladder that keeps a downstream Chamfer on ITS hole
// across point edits, and FeaturePattern transforming every point.
//
// No framework: exit code == failure count.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepAdaptor_Curve.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <GeomAbs_CurveType.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Shape.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "session/FeaturePattern.h"
#include "session/PlanExecutor.h"
#include "session/ScratchJob.h"
#include "session/ShapeMetrics.h"
#include "session/Signatures.h"
#include "session/TopologyOrigins.h"
#include "util/Cancel.h"

using nlohmann::json;
namespace em = onecad::elementmap;
namespace session = onecad::session;

namespace {

constexpr double kPi = 3.14159265358979323846;
int g_failures = 0;

void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}

void check_rel(double got, double want, double rel, const std::string& msg) {
    if (std::abs(got - want) > std::abs(want) * rel) {
        std::fprintf(stderr, "FAIL: %s (got %.9f want %.9f)\n", msg.c_str(), got, want);
        ++g_failures;
    }
}

TopoDS_Shape top_face(const TopoDS_Shape& body, double z) {
    for (TopExp_Explorer it(body, TopAbs_FACE); it.More(); it.Next()) {
        const auto d = em::ElementMapPartition::describe(it.Current(), body);
        if (std::abs(d.center.Z() - z) < 1e-9 && d.hasNormal && d.normal.Z() > 0.5)
            return it.Current();
    }
    return {};
}

// The circular edge whose circle centre is (x,y,z) with radius r, if any.
TopoDS_Shape circle_at(const TopoDS_Shape& body, double x, double y, double z, double r) {
    for (TopExp_Explorer it(body, TopAbs_EDGE); it.More(); it.Next()) {
        BRepAdaptor_Curve curve(TopoDS::Edge(it.Current()));
        if (curve.GetType() != GeomAbs_Circle) continue;
        const gp_Circ c = curve.Circle();
        if (c.Location().Distance(gp_Pnt(x, y, z)) < 1e-6 && std::abs(c.Radius() - r) < 1e-6)
            return it.Current();
    }
    return {};
}

json input_ref(const std::string& id, const char* kind, const TopoDS_Shape& shape,
               const TopoDS_Shape& body, double x, double y, double z) {
    return {{"primary", {{"bodyId", "body_1"}, {"elementId", id}, {"kind", kind}}},
            {"intent", {{"kind", kind}, {"descriptor",
                em::ElementMapPartition::descriptor_to_json(
                    em::ElementMapPartition::describe(shape, body))}}},
            {"anchor", {{"worldPoint", {x, y, z}}}}};
}

struct Pt { std::string id; double x, y; };

// A V3 Hole (Ø`d`, through-all unless `extra` says otherwise) on the face at z=`top`.
json hole_v3(const TopoDS_Shape& host, double top, const std::vector<Pt>& pts, double d,
             const json& extra = json::object()) {
    json points = json::array();
    for (const Pt& p : pts) points.push_back({{"id", p.id}, {"at", {p.x, p.y, top}}});
    json params = {{"targetBodyId", "body_1"}, {"point", {pts[0].x, pts[0].y, top}},
                   {"points", points}, {"holeType", "simple"}, {"diameter", d},
                   {"depth", nullptr}, {"resultPolicyVersion", 3}};
    params.update(extra);
    return {{"opType", "Hole"}, {"opId", "record_hole"}, {"stepIndex", 1},
            {"inputs", json::array({{{"primary", {{"bodyId", "body_1"}, {"elementId", "body_1"},
                                                   {"kind", "body"}}}},
                                    input_ref("face_top", "face", top_face(host, top), host,
                                              pts[0].x, pts[0].y, top)})},
            {"params", params}};
}

session::CandidateResult run(session::ScratchJob& job, const json& op) {
    std::string last;
    onecad::CancelToken cancel;
    return session::execute_candidate_op(job, op, op.value("opId", "op"), last, cancel);
}

session::CandidateResult run_on(const TopoDS_Shape& host, const json& op,
                                session::ScratchJob& job) {
    job.bodies.create("body_1", "box", host);
    return run(job, op);
}

double body_volume(const session::ScratchJob& job) {
    return session::shape_volume(job.bodies.get("body_1")->geom);
}

// ── many holes, one cut: exact N·π·r²·t; 1-point V3 == legacy V2 ─────────────
void test_points_many() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(60, 20, 25).Shape();
    session::ScratchJob job;
    const auto out = run_on(box, hole_v3(box, 25, {{"a", 10, 10}, {"b", 30, 10}, {"c", 50, 10}}, 6),
                            job);
    check(out.status == session::CandidateResult::Status::Ok, "3 points: Ok");
    check(out.body_events.size() == 1 && out.body_events[0].kind == "modified",
          "3 points: ONE modified event");
    check_rel(body_volume(job), 30000 - 3 * kPi * 9 * 25, 1e-6, "3 points: 3·π·3²·25");

    session::ScratchJob v3_job, v2_job;
    run_on(box, hole_v3(box, 25, {{"a", 30, 10}}, 6), v3_job);
    json legacy = hole_v3(box, 25, {{"a", 30, 10}}, 6);
    legacy["params"].erase("points");
    legacy["params"]["resultPolicyVersion"] = 2;
    run_on(box, legacy, v2_job);
    check(session::geometry_signature(v3_job.bodies) == session::geometry_signature(v2_job.bodies),
          "1-point V3 is bit-identical to the legacy V2 hole");

    session::ScratchJob cb_job;
    const json cb = {{"holeType", "counterbore"}, {"depth", 20.0}, {"cbDiameter", 10.0},
                     {"cbDepth", 5.0}};
    run_on(box, hole_v3(box, 25, {{"a", 10, 10}, {"b", 40, 10}}, 6, cb), cb_job);
    check_rel(body_volume(cb_job), 30000 - 2 * (kPi * 9 * 15 + kPi * 25 * 5), 1e-6,
              "2 counterbores: analytic volume");
}

// ── through-all is measured per point (stepped underside) ─────────────────────
void test_points_through_all_per_point() {
    const TopoDS_Shape full = BRepPrimAPI_MakeBox(60, 20, 25).Shape();
    const TopoDS_Shape step = BRepPrimAPI_MakeBox(gp_Pnt(30, 0, 0), 30, 20, 10).Shape();
    const TopoDS_Shape host = BRepAlgoAPI_Cut(full, step).Shape();
    session::ScratchJob job;
    const auto out = run_on(host, hole_v3(host, 25, {{"a", 10, 10}, {"b", 50, 10}}, 6), job);
    check(out.status == session::CandidateResult::Status::Ok, "stepped: Ok");
    check_rel(body_volume(job), session::shape_volume(host) - kPi * 9 * (25 + 15), 1e-6,
              "stepped: each hole drills its own local thickness");
}

// ── every refusal names its reason, carries evidence, touches nothing ─────────
void expect_refusal(const json& op, const char* reason, const char* key, int index,
                    const std::string& what) {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(60, 20, 25).Shape();
    session::ScratchJob job;
    const auto out = run_on(box, op, job);
    const json diag = out.diagnostics.empty() ? json::object() : out.diagnostics.front();
    check(out.status == session::CandidateResult::Status::Failed &&
              out.error_code == "OP_FAILED" && diag.value("reasonCode", "") == reason &&
              diag.value("stage", "") == "preflight",
          what + ": OP_FAILED " + reason + " got " + json(out.diagnostics).dump());
    if (key) check(diag["evidence"].value(key, -1) == index, what + ": evidence " + key);
    check(out.body_events.empty() && job.bodies.get("body_1")->geom.IsSame(box),
          what + ": all-or-nothing, host untouched");
}

void test_points_refusals() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(60, 20, 25).Shape();
    const std::vector<Pt> ok = {{"a", 10, 10}, {"b", 30, 10}, {"c", 50, 10}};
    json op = hole_v3(box, 25, ok, 6);
    op["params"]["points"][1]["at"] = {30.0, 90.0, 25.0};
    expect_refusal(op, "HOLE_POINT_OUTSIDE_FACE", "pointIndex", 1, "outside face");
    op = hole_v3(box, 25, ok, 6);
    op["params"]["points"][2]["at"] = {50.0, 10.0, 26.0};
    expect_refusal(op, "HOLE_POINT_OFF_FACE", "pointIndex", 2, "off plane");
    expect_refusal(hole_v3(box, 25, {{"a", 10, 10}, {"b", 10.0005, 10}}, 6),
                   "HOLE_POINTS_COINCIDENT", "otherPointIndex", 1, "coincident");
    expect_refusal(hole_v3(box, 25, {{"a", 10, 10}, {"b", 20, 10}, {"c", 15.5, 10}}, 6),
                   "HOLE_POINTS_OVERLAP", "otherPointIndex", 2, "overlap");
    // Countersink 12 @ 90°: tool radius 6.01, so 12.015 apart still overlaps.
    const json cs = {{"holeType", "countersink"}, {"depth", 10.0}, {"csDiameter", 12.0},
                     {"csAngleDeg", 90.0}};
    expect_refusal(hole_v3(box, 25, {{"a", 10, 10}, {"b", 22.015, 10}}, 6, cs),
                   "HOLE_POINTS_OVERLAP", "pointIndex", 0, "countersink overshoot");
    op = hole_v3(box, 25, ok, 6);
    op["params"]["points"][2]["id"] = "a";
    expect_refusal(op, "HOLE_POINT_ID_DUPLICATE", "pointIndex", 2, "duplicate id");
    op = hole_v3(box, 25, ok, 6);
    op["params"]["point"] = {10.0, 10.000001, 25.0};
    expect_refusal(op, "HOLE_POINT_MISMATCH", "pointIndex", 0, "point != points[0]");
    op = hole_v3(box, 25, ok, 6);
    op["params"]["points"][1] = json::array({30.0, 10.0, 25.0});
    expect_refusal(op, "HOLE_POINTS_MALFORMED", "pointIndex", 1, "bare triple entry");
    op = hole_v3(box, 25, ok, 6);
    op["params"].erase("points");
    expect_refusal(op, "HOLE_POINTS_MALFORMED", nullptr, 0, "V3 without points");
    op = hole_v3(box, 25, ok, 6);
    op["params"]["points"] = json::array();
    expect_refusal(op, "HOLE_POINTS_COUNT", nullptr, 0, "zero points");
    json many = json::array();
    for (int k = 0; k < 65; ++k)
        many.push_back({{"id", std::to_string(k)}, {"at", {1.0, 1.0, 25.0}}});
    op["params"]["points"] = many;
    expect_refusal(op, "HOLE_POINTS_COUNT", nullptr, 0, "65 points");
    for (const json version : {json(2), json()}) {
        op = hole_v3(box, 25, ok, 6);
        if (version.is_null()) op["params"].erase("resultPolicyVersion");
        else op["params"]["resultPolicyVersion"] = version;
        expect_refusal(op, "HOLE_POINTS_REQUIRE_V3", nullptr, 0, "points without V3");
    }
}

// ── V3 keeps V2's single-solid fence over the WHOLE result ───────────────────
void test_points_v3_disjoint() {
    // An H: one top face whose 4 mm neck a Ø6 hole at (10,10) severs.
    const TopoDS_Shape full = BRepPrimAPI_MakeBox(20, 20, 25).Shape();
    const TopoDS_Shape low = BRepPrimAPI_MakeBox(gp_Pnt(8, 0, 0), 4, 8, 25).Shape();
    const TopoDS_Shape high = BRepPrimAPI_MakeBox(gp_Pnt(8, 12, 0), 4, 8, 25).Shape();
    const TopoDS_Shape host =
        BRepAlgoAPI_Cut(BRepAlgoAPI_Cut(full, low).Shape(), high).Shape();
    session::ScratchJob job;
    const auto out = run_on(host, hole_v3(host, 25, {{"a", 4, 4}, {"b", 10, 10}}, 6), job);
    check(out.status == session::CandidateResult::Status::Failed &&
              !out.diagnostics.empty() &&
              out.diagnostics.front().value("code", "") == "HOLE_DISJOINT_RESULT",
          "V3 severing the bridge is HOLE_DISJOINT_RESULT");
    check(job.bodies.get("body_1")->geom.IsSame(host), "V3 disjoint: host untouched");
}

void test_points_determinism_and_cancel() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(60, 20, 25).Shape();
    const json op = hole_v3(box, 25, {{"a", 10, 10}, {"b", 30, 10}, {"c", 50, 10}}, 6);
    session::ScratchJob a, b;
    run_on(box, op, a);
    run_on(box, op, b);
    check(session::geometry_signature(a.bodies) == session::geometry_signature(b.bodies),
          "determinism: equal signatures");
    session::ScratchJob c;
    c.bodies.create("body_1", "box", box);
    std::string last;
    onecad::CancelToken cancel;
    cancel.cancel();
    const auto out = session::execute_candidate_op(c, op, "record_hole", last, cancel);
    check(out.status == session::CandidateResult::Status::Cancelled, "cancel: Cancelled");
    check(c.bodies.get("body_1")->geom.IsSame(box), "cancel: host untouched");
}

// ── every hole-born shape is claimed by its own point id ─────────────────────
void test_points_ledger_slots() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(60, 20, 10).Shape();
    session::ScratchJob job;
    run_on(box, hole_v3(box, 10, {{"p0", 10, 10}, {"p1", 30, 10}, {"p2", 50, 10}}, 4), job);
    const TopoDS_Shape body = job.bodies.get("body_1")->geom;
    for (const auto& [id, x] : std::vector<std::pair<std::string, double>>{
             {"p0", 10}, {"p1", 30}, {"p2", 50}}) {
        for (const double z : {10.0, 0.0}) {
            const auto claim = job.topology_owners.lookup("body_1", circle_at(body, x, 10, z, 2));
            check(claim.state == session::OriginState::Known &&
                      claim.producer_record_id == "record_hole" && claim.slot == id,
                  "rim of " + id + " is claimed by slot " + id);
        }
    }
    const auto host_top = job.topology_owners.lookup("body_1", top_face(body, 10));
    check(host_top.slot.empty(), "the modified host face carries no slot");
    const session::OriginClaim a{session::OriginState::Known, "r", "p0"};
    const session::OriginClaim b{session::OriginState::Known, "r", "p1"};
    const session::OriginClaim merged = session::merge_origin(a, b);
    check(merged.state == session::OriginState::Known && merged.producer_record_id == "r" &&
              merged.slot.empty(),
          "slot disagreement keeps the producer and clears the slot");
}

// A Chamfer frozen on hole `p2`'s top rim, replayed after an upstream edit to
// the Hole's points. Returns the candidate; `body` receives the result.
session::CandidateResult chamfer_after_edit(const std::vector<Pt>& edited, bool with_slot,
                                            TopoDS_Shape& body) {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(60, 20, 10).Shape();
    session::ScratchJob authored;
    run_on(box, hole_v3(box, 10, {{"p0", 10, 10}, {"p1", 30, 10}, {"p2", 50, 10}}, 4), authored);
    const TopoDS_Shape drilled = authored.bodies.get("body_1")->geom;
    const TopoDS_Shape rim = circle_at(drilled, 50, 10, 10, 2);
    json edge = input_ref("edge_rim", "edge", rim, drilled, 50, 12, 10);
    if (with_slot) {
        const auto claim = authored.topology_owners.lookup("body_1", rim);
        edge["intent"]["descriptor"]["originSlot"] = {{"recordId", claim.producer_record_id},
                                                      {"slot", claim.slot}};
    }
    const json chamfer = {{"opType", "Chamfer"}, {"opId", "record_chamfer"}, {"stepIndex", 2},
                          {"inputs", json::array({edge})},
                          {"params", {{"mode", "Chamfer"}, {"radius", 0.5},
                                      {"edgeIds", json::array({"edge_rim"})},
                                      {"chainTangentEdges", false}}}};
    session::ScratchJob replay;
    replay.edited_from = 1;
    const auto hole = run_on(box, hole_v3(box, 10, edited, 4), replay);
    check(hole.status == session::CandidateResult::Status::Ok, "edited hole: Ok");
    const auto out = run(replay, chamfer);
    body = replay.bodies.get("body_1")->geom;
    return out;
}

bool chamfered_at(const TopoDS_Shape& body, double x) {
    return circle_at(body, x, 10, 10, 2).IsNull() && !circle_at(body, x, 10, 10, 2.5).IsNull();
}

void test_downstream_ref_keeps_its_hole() {
    TopoDS_Shape body;
    auto out = chamfer_after_edit({{"p0", 15, 10}, {"p1", 30, 10}, {"p2", 50, 10}}, true, body);
    check(out.status == session::CandidateResult::Status::Ok && chamfered_at(body, 50) &&
              !chamfered_at(body, 15) && !chamfered_at(body, 30),
          "move points[0]: the chamfer stays on hole p2");
    out = chamfer_after_edit({{"p0", 10, 10}, {"p2", 50, 10}}, true, body);
    check(out.status == session::CandidateResult::Status::Ok && chamfered_at(body, 50),
          "remove points[1]: the chamfer stays on hole p2 (now index 1)");
    out = chamfer_after_edit({{"p0", 10, 10}, {"p2", 47, 10}}, true, body);
    check(out.status == session::CandidateResult::Status::Ok && chamfered_at(body, 47),
          "small move of p2: the chamfer follows its hole");
    out = chamfer_after_edit({{"p0", 10, 10}, {"p1", 30, 10}}, true, body);
    check(out.status == session::CandidateResult::Status::NeedsRepair &&
              !out.needs_repair.empty() &&
              out.needs_repair.front().value("reason", "") == "no-candidates" &&
              !chamfered_at(body, 10) && !chamfered_at(body, 30),
          "remove p2: NeedsRepair no-candidates, never a sibling");
    // Control: without the slot the congruent siblings tie after the edit and
    // the resolver fails closed — the slot is what keeps the ref bindable.
    out = chamfer_after_edit({{"p0", 15, 10}, {"p1", 30, 10}, {"p2", 50, 10}}, false, body);
    check(out.status == session::CandidateResult::Status::NeedsRepair &&
              !chamfered_at(body, 15) && !chamfered_at(body, 30),
          "no slot: NeedsRepair, a sibling is never chamfered");
}

// Review finding (ONECAD-361): one op mixing a slotted and an unslotted ref on
// the SAME shape must keep the §10 distinct assignment joint. Before the fix the
// two refs were resolved in separate ladder calls and both bound p2's rim, so
// the Chamfer committed on an ambiguous binding instead of NeedsRepair.
void test_mixed_slotted_and_unslotted_refs_never_share_a_shape() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(60, 20, 10).Shape();
    session::ScratchJob authored;
    run_on(box, hole_v3(box, 10, {{"p0", 10, 10}, {"p1", 30, 10}, {"p2", 50, 10}}, 4), authored);
    const TopoDS_Shape drilled = authored.bodies.get("body_1")->geom;
    const TopoDS_Shape rim = circle_at(drilled, 50, 10, 10, 2);
    const auto claim = authored.topology_owners.lookup("body_1", rim);
    json slotted = input_ref("edge_a", "edge", rim, drilled, 50, 12, 10);
    slotted["intent"]["descriptor"]["originSlot"] = {{"recordId", claim.producer_record_id},
                                                     {"slot", claim.slot}};
    const json unslotted = input_ref("edge_b", "edge", rim, drilled, 50, 12, 10);
    for (const bool slotted_first : {true, false}) {
        const json inputs = slotted_first ? json::array({slotted, unslotted})
                                          : json::array({unslotted, slotted});
        const json chamfer = {{"opType", "Chamfer"}, {"opId", "record_chamfer"},
                              {"stepIndex", 2}, {"inputs", inputs},
                              {"params", {{"mode", "Chamfer"}, {"radius", 0.5},
                                          {"edgeIds", json::array({"edge_a", "edge_b"})},
                                          {"chainTangentEdges", false}}}};
        // Clean replay: each ref alone would AutoBind p2's rim, so only the
        // joint assignment can keep them apart.
        session::ScratchJob replay;
        run_on(box, hole_v3(box, 10, {{"p0", 10, 10}, {"p1", 30, 10}, {"p2", 50, 10}}, 4),
               replay);
        const auto out = run(replay, chamfer);
        const std::string order = slotted_first ? "slotted first" : "unslotted first";
        check(out.status == session::CandidateResult::Status::NeedsRepair,
              "mixed slotted/unslotted refs on one rim (" + order + "): NeedsRepair");
        for (std::size_t a = 0; a < out.delta.added.size(); ++a)
            for (std::size_t b = a + 1; b < out.delta.added.size(); ++b)
                check(out.delta.added[a].topo_key != out.delta.added[b].topo_key,
                      "mixed refs (" + order + "): never minted on one shape");
        check(out.needs_repair.size() == 1 ||
                  (out.needs_repair.size() == 2 &&
                   out.needs_repair[0].value("refId", "") == "record_chamfer.input0" &&
                   out.needs_repair[1].value("refId", "") == "record_chamfer.input1"),
              "mixed refs (" + order + "): needsRepair in input order");
    }
}

// ── FeaturePattern transforms every point; N holes × instances ───────────────
void test_feature_pattern_transforms_points() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(100, 20, 10).Shape();
    session::ScratchJob job;
    json source = hole_v3(box, 10, {{"p0", 10, 10}, {"p1", 20, 10}}, 4);
    run_on(box, source, job);
    const double seeded = body_volume(job);
    source["sourceRecordId"] = "record_hole";
    const json pattern = {{"opType", "FeaturePattern"}, {"opId", "pattern"}, {"stepIndex", 2},
        {"inputs", json::array()},
        {"params", {{"semanticsVersion", 1}, {"count", 3},
                    {"sourceRecordIds", json::array({"record_hole"})},
                    {"layout", {{"kind", "Linear"}, {"direction", {1.0, 0.0, 0.0}},
                                {"spacing", 30.0}}},
                    {"sourceOps", json::array({source})}}}};
    const auto out = run(job, pattern);
    check(out.status == session::CandidateResult::Status::Ok,
          "FeaturePattern of a V3 Hole: Ok " + out.error_message);
    check_rel(body_volume(job), seeded - 2 * 2 * kPi * 4 * 10, 1e-6,
              "FeaturePattern: 2 points × 2 more instances");
    const TopoDS_Shape body = job.bodies.get("body_1")->geom;
    check(!circle_at(body, 40, 10, 10, 2).IsNull() && !circle_at(body, 80, 10, 10, 2).IsNull(),
          "FeaturePattern: points[1] moved with points[0]");
}

// A Chamfer slotted on p1, patterned with its V3 Hole: `rewrite_origin_slot`
// must retarget each instance's slot to the instance's virtual producer, so
// every instance chamfers ITS copy of p1 and never p0.
void test_feature_pattern_rewrites_origin_slot() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(100, 20, 10).Shape();
    session::ScratchJob job;
    json source_hole = hole_v3(box, 10, {{"p0", 10, 10}, {"p1", 20, 10}}, 4);
    run_on(box, source_hole, job);
    const TopoDS_Shape drilled = job.bodies.get("body_1")->geom;
    const TopoDS_Shape rim = circle_at(drilled, 20, 10, 10, 2);
    const auto claim = job.topology_owners.lookup("body_1", rim);
    check(claim.slot == "p1", "seed: p1 rim carries slot p1");
    json edge = input_ref("edge_rim", "edge", rim, drilled, 20, 12, 10);
    edge["intent"]["descriptor"]["originSlot"] = {{"recordId", claim.producer_record_id},
                                                  {"slot", claim.slot}};
    json source_chamfer = {{"opType", "Chamfer"}, {"opId", "record_chamfer"}, {"stepIndex", 2},
                           {"inputs", json::array({edge})},
                           {"params", {{"mode", "Chamfer"}, {"radius", 0.5},
                                       {"edgeIds", json::array({"edge_rim"})},
                                       {"chainTangentEdges", false}}}};
    const auto seeded = run(job, source_chamfer);
    check(seeded.status == session::CandidateResult::Status::Ok, "seed slotted chamfer: Ok");
    source_hole["sourceRecordId"] = "record_hole";
    source_chamfer["sourceRecordId"] = "record_chamfer";
    const json pattern = {{"opType", "FeaturePattern"}, {"opId", "pattern"}, {"stepIndex", 3},
        {"inputs", json::array()},
        {"params", {{"semanticsVersion", 1}, {"count", 3},
                    {"sourceRecordIds", json::array({"record_hole", "record_chamfer"})},
                    {"layout", {{"kind", "Linear"}, {"direction", {1.0, 0.0, 0.0}},
                                {"spacing", 30.0}}},
                    {"sourceOps", json::array({source_hole, source_chamfer})}}}};
    const auto out = run(job, pattern);
    check(out.status == session::CandidateResult::Status::Ok,
          "FeaturePattern of [V3 Hole, slotted Chamfer]: Ok " + out.error_message +
              json(out.needs_repair).dump());
    const TopoDS_Shape body = job.bodies.get("body_1")->geom;
    for (const double x : {20.0, 50.0, 80.0})
        check(chamfered_at(body, x), "pattern: chamfer on its own p1 copy at x=" +
                                         std::to_string(x));
    for (const double x : {10.0, 40.0, 70.0})
        check(!chamfered_at(body, x) && !circle_at(body, x, 10, 10, 2).IsNull(),
              "pattern: p0 copy at x=" + std::to_string(x) + " is never chamfered");
    const std::string virtual_id = session::feature_pattern_virtual_id("pattern", "record_hole", 1);
    const auto instance_claim =
        job.topology_owners.lookup("body_1", circle_at(body, 40, 10, 0, 2));
    check(instance_claim.state == session::OriginState::Known &&
              instance_claim.producer_record_id == virtual_id && instance_claim.slot == "p0",
          "instance 1 p0 bottom rim: producer = virtual id, slot p0");
}

}  // namespace

int main() {
    test_points_many();
    test_points_through_all_per_point();
    test_points_refusals();
    test_points_v3_disjoint();
    test_points_determinism_and_cancel();
    test_points_ledger_slots();
    test_downstream_ref_keeps_its_hole();
    test_mixed_slotted_and_unslotted_refs_never_share_a_shape();
    test_feature_pattern_transforms_points();
    test_feature_pattern_rewrites_origin_slot();
    if (g_failures == 0) std::fprintf(stderr, "test_hole_points: all checks passed\n");
    return g_failures;
}
