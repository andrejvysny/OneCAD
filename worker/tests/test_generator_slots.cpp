// test_generator_slots.cpp — Extrude generator-provenance slots in the topology
// ledger (SlotScope::Generator). Ledger only: nothing here asserts resolution.
//
// Real OCCT, in-process via execute_extrude. The MakePrism oracle is used by the
// TEST only, to name which wall a profile edge generated.
//
// No framework: exit code == failure count.
#include <cmath>
#include <cstdio>
#include <map>
#include <set>
#include <string>
#include <utility>
#include <vector>

#include <BRepAdaptor_Curve.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <GProp_GProps.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Shape.hxx>

#include "elementmap/ElementMapPartition.h"
#include "elementmap/Ladder.h"
#include "nlohmann/json.hpp"
#include "ops/ExtrudeOp.h"
#include "ops/GeneratorSlots.h"
#include "ops/OpTypes.h"
#include "protocol/Envelope.h"
#include "session/BodyStore.h"
#include "session/ElementIdentity.h"
#include "session/FeaturePatternRefs.h"
#include "session/HistoryHash.h"
#include "session/ScratchJob.h"
#include "session/Session.h"
#include "session/TopologyOrigins.h"
#include "util/Cancel.h"

using nlohmann::json;
namespace ops = onecad::ops;
namespace em = onecad::elementmap;
namespace session = onecad::session;

namespace {
int g_failures = 0;

void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}

// World map of an XY sketch: (u,v) -> (-v, u, 0).
json line(const std::string& id, double x0, double y0, double x1, double y1) {
    return {{"id", id}, {"type", "Line"}, {"p0", {x0, y0}}, {"p1", {x1, y1}}};
}

json sketch_of(json entities) {
    return {{"sketchId", "sk1"}, {"plane", {{"kind", "XY"}}}, {"entities", std::move(entities)},
            {"constraints", json::array()}};
}

json rect_sketch(double w = 10, double h = 6) {
    return sketch_of(json::array({line("e1", 0, 0, w, 0), line("e2", w, 0, w, h),
                                  line("e3", w, h, 0, h), line("e4", 0, h, 0, 0)}));
}

// What one Extrude left behind: the body store, the history it emitted and the
// slot -> shape table read out of its births (Generator scope only).
struct Run {
    session::BodyStore bodies;
    em::ElementMapPartition partition;
    ops::OpOutcome outcome;
    std::vector<std::pair<TopoDS_Shape, std::string>> slotted;
    std::string body;
    TopoDS_Shape shape() const { return bodies.get(body)->geom; }
    std::map<std::string, int> counts() const {
        std::map<std::string, int> out;
        for (const auto& s : slotted) ++out[s.second];
        return out;
    }
    TopoDS_Shape find(const std::string& slot) const {
        for (const auto& s : slotted)
            if (s.second == slot) return s.first;
        return {};
    }
};

void collect(Run& run) {
    for (const session::TopologyBodyHistory& history : run.outcome.topology_history)
        for (const session::TopologyBirth& birth : history.births)
            if (!birth.slot.empty() && birth.scope == session::SlotScope::Generator)
                run.slotted.emplace_back(birth.after, birth.slot);
}

// Slots need profile labels, which only regionIdentityVersion 2/3 profiles carry
// (legacy profiles stamp none), so drive the V3 identity path real documents
// use. The single region id is read back from the refusal that lists the
// available ones.
std::string only_region_id(const json& sketch) {
    std::string err;
    ops::build_profile_face(sketch, "probe", 3, err);
    const auto open = err.find("[");
    const auto close = err.find(']', open);
    if (open == std::string::npos || close == std::string::npos) return {};
    return err.substr(open + 1, close - open - 1);
}

Run extrude(const json& sketch, const json& extra = json::object(),
            const TopoDS_Shape& target = {}) {
    Run run;
    std::vector<std::pair<std::string, json>> sketches{{"sk1", sketch}};
    std::string last = "sk1";
    onecad::CancelToken cancel;
    if (!target.IsNull()) run.bodies.create("base", "box", target);
    ops::OpContext ctx{run.bodies, &sketches, run.partition, &last, false, json::object(), &cancel};
    json params = {{"sketchId", "sk1"}, {"distance", 8.0}, {"extrudeMode", "Blind"},
                   {"booleanMode", "NewBody"}, {"regionIdentityVersion", 3},
                   {"regionId", only_region_id(sketch)}};
    params.update(extra);
    run.outcome = ops::execute_extrude(
        ctx, {{"opType", "Extrude"}, {"opId", "ope"}, {"params", params}}, "ope");
    run.body = target.IsNull() ? "body_ope" : "base";
    check(run.outcome.status == ops::OpOutcome::Status::Ok, "extrude Ok");
    collect(run);
    return run;
}

gp_Pnt centre(const TopoDS_Shape& face) {
    GProp_GProps props;
    BRepGProp::SurfaceProperties(face, props);
    return props.CentreOfMass();
}

bool has(const Run& run, const std::string& slot) { return run.counts().count(slot) != 0; }

int count_prefix(const Run& run, const std::string& prefix) {
    int n = 0;
    for (const auto& s : run.slotted) n += s.second.rfind(prefix, 0) == 0 ? 1 : 0;
    return n;
}

// Oracle: the wall MakePrism generates from the profile edge whose midpoint is `mid`.
TopoDS_Shape oracle_wall(const std::vector<gp_Pnt>& corners, const gp_Pnt& mid, double height) {
    BRepBuilderAPI_MakePolygon poly;
    for (const gp_Pnt& p : corners) poly.Add(p);
    poly.Close();
    BRepBuilderAPI_MakeFace face(poly.Wire());
    BRepPrimAPI_MakePrism prism(face.Face(), gp_Vec(0, 0, height));
    TopTools_IndexedMapOfShape edges;
    TopExp::MapShapes(face.Face(), TopAbs_EDGE, edges);
    for (int i = 1; i <= edges.Extent(); ++i) {
        GProp_GProps props;
        BRepGProp::LinearProperties(edges(i), props);
        if (props.CentreOfMass().Distance(mid) > 1e-6) continue;
        for (const TopoDS_Shape& generated : prism.Generated(edges(i))) return generated;
    }
    return {};
}

void check_rect_sides(const Run& run, double height, double z0, const std::string& label) {
    // e1..e4 world midpoints for the 10x6 rect: (u,v)->(-v,u).
    const std::vector<gp_Pnt> corners{gp_Pnt(0, 0, z0), gp_Pnt(0, 10, z0), gp_Pnt(-6, 10, z0),
                                      gp_Pnt(-6, 0, z0)};
    const std::vector<std::pair<std::string, gp_Pnt>> mids{
        {"e1", gp_Pnt(0, 5, z0)}, {"e2", gp_Pnt(-3, 10, z0)},
        {"e3", gp_Pnt(-6, 5, z0)}, {"e4", gp_Pnt(-3, 0, z0)}};
    for (const auto& [id, mid] : mids) {
        const TopoDS_Shape got = run.find("side:" + id);
        const TopoDS_Shape want = oracle_wall(corners, mid, height);
        check(!got.IsNull() && !want.IsNull() && centre(got).Distance(centre(want)) < 1e-6,
              label + ": side:" + id + " is the MakePrism wall of " + id);
    }
}

void test_blind_rectangle() {
    const Run run = extrude(rect_sketch());
    check_rect_sides(run, 8, 0, "blind");
    check(has(run, "cap:0") && has(run, "cap:+") && !has(run, "cap:-"), "blind: caps 0 and +");
    check(count_prefix(run, "edge:") == 12, "blind: all 12 edges slotted, got " +
                                                std::to_string(count_prefix(run, "edge:")));
    for (const auto& [slot, n] : run.counts()) check(n == 1, "blind: slot unique: " + slot);
    check(count_prefix(run, "side:") == 4, "blind: 4 side slots");
}

void test_symmetric_and_two_direction() {
    const Run sym = extrude(rect_sketch(), {{"extrudeMode", "Symmetric"}});
    check_rect_sides(sym, 8, -4, "symmetric");
    check(has(sym, "cap:+") && has(sym, "cap:-") && !has(sym, "cap:0"), "symmetric: caps + and -");
    const Run two = extrude(rect_sketch(), {{"twoDirections", true}, {"distance2", 3.0}});
    check_rect_sides(two, 11, -3, "two-direction");
    check(has(two, "cap:+") && has(two, "cap:-"), "two-direction: caps + and -");
    check(count_prefix(two, "side:") == 4, "two-direction: 4 side slots");
}

void test_draft() {
    const Run run = extrude(rect_sketch(), {{"draftAngleDeg", 5.0}});
    for (const char* id : {"e1", "e2", "e3", "e4"})
        check(has(run, std::string("side:") + id), std::string("draft: side:") + id);
    check(has(run, "cap:0") && has(run, "cap:+"), "draft: caps survive");
}

void test_circle() {
    const json circle = sketch_of(json::array(
        {json{{"id", "c1"}, {"type", "Circle"}, {"center", {0, 0}}, {"radius", 5}}}));
    const Run run = extrude(circle);
    check(has(run, "side:c1"), "circle: side:c1");
    check(has(run, "cap:0") && has(run, "cap:+"), "circle: caps are cap slots");
    check(!run.find("cap:0").IsNull() && std::abs(centre(run.find("cap:0")).Z()) < 1e-6,
          "circle: cap:0 is the base face, not the wall");
    check(count_prefix(run, "side:") == 1, "circle: exactly one side slot");
}

void test_two_arcs() {
    auto arc = [](const char* id, double a0, double a1) {
        return json{{"id", id}, {"type", "Arc"}, {"center", {0, 0}}, {"radius", 5},
                    {"startAngle", a0}, {"endAngle", a1}};
    };
    const Run run = extrude(sketch_of(json::array({arc("a1", 0, M_PI), arc("a2", M_PI, 2 * M_PI)})));
    check(has(run, "side:a1") && has(run, "side:a2"), "arcs: both halves slotted");
    check(!has(run, "edge:side:a1|side:a2"),
          "arcs: the two a1|a2 joints share a slot, so both are dropped");
    check(has(run, "edge:cap:+|side:a1") && has(run, "edge:cap:0|side:a2"), "arcs: rim edges kept");
}

void test_split_entity_faces_unslotted() {
    // Two walls claimed by the SAME base id (what two "#seg" pieces of one entity
    // become after FaceBuilder strips the suffix) must both stay unslotted.
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10, 6, 8).Shape();
    const std::vector<ops::ProfileEdgeLabel> labels{
        {gp_Pnt(5, 0, 0), "e1"}, {gp_Pnt(10, 3, 0), "e1"},
        {gp_Pnt(5, 6, 0), "e3"}, {gp_Pnt(0, 3, 0), "e4"}};
    const ops::GeneratorSlotMap map = ops::attribute_extrude_slots(
        box, labels, gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1), 1e-3);
    std::set<std::string> slots;
    for (int i = 1; i <= map.Extent(); ++i) slots.insert(map.FindFromIndex(i));
    check(slots.count("side:e1") == 0, "split: faces sharing a base id carry no side:e1");
    check(slots.count("side:e3") == 1 && slots.count("side:e4") == 1, "split: unshared walls slotted");
    check(slots.count("cap:0") == 1 && slots.count("cap:+") == 1, "split: caps slotted");
    check(ops::attribute_extrude_slots(box, {}, gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1), 1e-3).Extent() == 0,
          "no labels: empty map");
}

void test_boolean_add() {
    const TopoDS_Shape base = BRepPrimAPI_MakeBox(gp_Pnt(-50, -50, -10), 100, 100, 10).Shape();
    const Run run = extrude(rect_sketch(), {{"booleanMode", "Add"}, {"targetBodyId", "base"}}, base);
    int sides = 0;
    for (const char* id : {"e1", "e2", "e3", "e4"}) sides += has(run, std::string("side:") + id);
    check(sides == 4, "boss: 4 walls slotted through Modified/IsSame, got " + std::to_string(sides));
    check(has(run, "cap:+"), "boss: top cap slotted");
    check(!has(run, "cap:0"), "boss: bottom cap is internal, not in the result");
    // No slot ever lands on a pre-existing base shape (root_slots only covers the
    // tool's own topology); Generated section topology is likewise unslotted.
    TopTools_IndexedMapOfShape base_shapes;
    TopExp::MapShapes(base, base_shapes);
    for (const auto& [shape, slot] : run.slotted)
        check(!base_shapes.Contains(shape), "boss: base shape carries no slot: " + slot);
    check(count_prefix(run, "side:") == 4, "boss: no extra side slots");
}

void test_ledger() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10, 10, 10).Shape();
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(box, TopAbs_FACE, faces);
    session::TopologyBodyHistory history;
    history.body_id = "b";
    history.coverage = session::HistoryCoverage::Proven;
    history.births.push_back({faces(1), "side:e1", session::SlotScope::Generator});
    history.births.push_back({faces(2), "side:e1", session::SlotScope::Point});
    session::TopologyOwnerLedger ledger;
    session::apply_topology_history(ledger, "op1", {"b"}, {history});
    const session::OriginClaim gen = ledger.lookup("b", faces(1));
    check(gen.state == session::OriginState::Known && gen.producer_record_id == "op1" &&
              gen.slot == "side:e1" && gen.scope == session::SlotScope::Generator,
          "ledger: Known + slot + Generator");
    const auto filter = session::origin_slot_filter(ledger, "b", "op1", "side:e1");
    check(!filter(faces(1)), "ledger: Hole filter rejects a Generator claim with an equal slot");
    check(filter(faces(2)), "ledger: Hole filter still admits the Point claim");
    // Rows that agree on slot text but not on scope drop the slot.
    const session::OriginClaim merged = session::merge_origin(
        {session::OriginState::Known, "op1", "x", session::SlotScope::Point},
        {session::OriginState::Known, "op1", "x", session::SlotScope::Generator});
    check(merged.state == session::OriginState::Known && merged.slot.empty(),
          "ledger: scope disagreement clears the slot");
}

// Brief amendment: a Generator slot must never be frozen as descriptor.originSlot.
void test_element_identity_origin_slot() {
    session::Session s;
    s.open("doc_1", 0, 3, "determinism");
    auto fence = s.fence_and_clone(1, 0, 3, session::kEmptyPrefixHash);
    session::ScratchJob job;
    job.job_id = 1;
    job.bodies = fence.cloned_bodies;
    job.partition = fence.cloned_partition;
    job.prepared_snapshot_id = fence.prepared_snapshot_id;
    job.history_prefix_hash = std::string(64, 'a');
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10, 10, 10).Shape();
    job.bodies.create("body_a", "op_a", box);
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(box, TopAbs_FACE, faces);
    job.topology_owners.add({"body_a", faces(1),
                             {session::OriginState::Known, "op_a", "p0", session::SlotScope::Point}});
    job.topology_owners.add({"body_a", faces(2),
                             {session::OriginState::Known, "op_a", "side:e1",
                              session::SlotScope::Generator}});
    s.store_prepared(std::move(job));
    const auto accepted = s.accept_prepared(1, 0, 3);
    check(accepted.ok, "identity: setup published");
    const auto resp = session::handle_acquire_element_ids(
        s, onecad::protocol::Envelope::request(
               7, "AcquireElementIds",
               json{{"snapshotId", accepted.snapshot_id}, {"bodyId", "body_a"},
                    {"picks", json::array({json{{"topoKey", "f:1"}}, json{{"topoKey", "f:2"}}})}}));
    const bool ok = resp.ok.has_value() && *resp.ok && resp.result["ids"].size() == 2;
    check(ok, "identity: both picks resolve");
    if (!ok) return;
    const json& point = resp.result["ids"][0]["descriptor"];
    const json& generator = resp.result["ids"][1]["descriptor"];
    check(point.contains("originSlot") && point["originSlot"]["slot"] == "p0",
          "identity: Point claim still stamps originSlot");
    check(!generator.contains("originSlot"), "identity: Generator claim stamps no originSlot");
    check(generator.contains("generatorSlot") &&
              generator["generatorSlot"]["recordId"] == "op_a" &&
              generator["generatorSlot"]["slot"] == "side:e1" &&
              !point.contains("generatorSlot"),
          "identity: Generator claim stamps generatorSlot, Point claim does not");
}

// SCHEMA §7.5 generatorSlot: parse + gated filter.
void test_generator_slot_filter() {
    const json input = {{"elementId", "e1"},
                        {"intent", {{"kind", "edge"},
                                    {"descriptor", {{"generatorSlot",
                                                     {{"recordId", "op1"},
                                                      {"slot", "edge:side:a|side:b"}}}}}}}};
    const em::LadderRef ref = em::ladder_ref_from_input(input, "r1");
    check(ref.generator_record_id == "op1" && ref.generator_slot == "edge:side:a|side:b",
          "parse: generatorSlot -> LadderRef");
    check(ref.origin_slot.empty(), "parse: generatorSlot leaves originSlot empty");
    const json bad = {{"intent", {{"descriptor", {{"generatorSlot", {{"recordId", 3}}}}}}}};
    check(em::ladder_ref_from_input(bad, "r2").generator_slot.empty(),
          "parse: malformed generatorSlot ignored");

    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10, 10, 10).Shape();
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(box, TopAbs_FACE, faces);
    // Gate closed: only a Point claim and an other-producer Generator claim.
    session::TopologyOwnerLedger closed;
    closed.add({"b", faces(1), {session::OriginState::Known, "op1", "s", session::SlotScope::Point}});
    closed.add({"b", faces(2), {session::OriginState::Known, "op2", "s",
                                session::SlotScope::Generator}});
    check(!session::generator_slot_filter(closed, "b", "op1", "s"),
          "filter: gate closed -> empty function");
    check(!session::generator_slot_filter(closed, "other", "op2", "s"),
          "filter: gate closed for another body");
    // Gate open.
    session::TopologyOwnerLedger open = closed;
    open.add({"b", faces(3), {session::OriginState::Known, "op1", "s",
                              session::SlotScope::Generator}});
    open.add({"b", faces(4), {session::OriginState::Known, "op1", "t",
                              session::SlotScope::Generator}});
    const auto filter = session::generator_slot_filter(open, "b", "op1", "s");
    check(static_cast<bool>(filter), "filter: gate open -> non-empty");
    if (!filter) return;
    check(filter(faces(3)), "filter: admits matching Known Generator claim");
    check(!filter(faces(1)), "filter: rejects Point claim with the same slot string");
    check(!filter(faces(2)), "filter: rejects other producer");
    check(!filter(faces(4)), "filter: rejects other slot");
    check(!filter(faces(5)), "filter: rejects unclaimed shape");
}

// ---- split / legacy / pattern regressions ----------------------------------

gp_Pnt edge_mid(const TopoDS_Shape& e) {
    BRepAdaptor_Curve c(TopoDS::Edge(e));
    return c.Value(0.5 * (c.FirstParameter() + c.LastParameter()));
}

// U-shaped profile (entities a..h) added onto a base whose fin splits the tool's
// +x top edge.
json u_sketch() {
    return sketch_of(json::array({line("a", 0, 0, 10, 0), line("b", 10, 0, 10, 6),
                                  line("c", 10, 6, 20, 6), line("e", 20, 6, 20, 0),
                                  line("d", 20, 0, 30, 0), line("f", 30, 0, 30, 12),
                                  line("g", 30, 12, 0, 12), line("h", 0, 12, 0, 0)}));
}

TopoDS_Shape u_base(double y1, double y2) {
    const TopoDS_Shape f1 = BRepPrimAPI_MakeBox(gp_Pnt(-10, y1, -1), 14, 5.5 - y1, 21).Shape();
    const TopoDS_Shape f2 = BRepPrimAPI_MakeBox(gp_Pnt(-10, y2, -1), 14, 25.5 - y2, 21).Shape();
    const TopoDS_Shape slab = BRepPrimAPI_MakeBox(gp_Pnt(-14, -2, -2), 20, 34, 1).Shape();
    return BRepAlgoAPI_Fuse(BRepAlgoAPI_Fuse(f1, f2).Shape(), slab).Shape();
}

// A root split into two images (the base fin cuts the tool's top edge in two)
// hands its slot to neither piece. Replays the confirmed wrong bind: the ref's
// authored piece is consumed by an upstream edit and the sibling, once the only
// slot holder, used to auto-bind through a single-candidate pool.
void test_split_root_never_binds_sibling() {
    const std::string slot = "edge:cap:+|side:a";
    const Run r1 = extrude(u_sketch(), {{"booleanMode", "Add"}, {"targetBodyId", "base"}},
                           u_base(4.5, 24.5));
    check(!has(r1, slot), "split: neither fragment of a split root holds the slot (state 1)");
    TopoDS_Shape authored;
    TopTools_IndexedMapOfShape edges;
    TopExp::MapShapes(r1.shape(), TopAbs_EDGE, edges);
    for (int i = 1; i <= edges.Extent(); ++i) {
        const gp_Pnt m = edge_mid(edges(i));
        if (std::abs(m.X()) < 1e-3 && std::abs(m.Y() - 2.25) < 1e-3 && std::abs(m.Z() - 8) < 1e-3)
            authored = edges(i);
    }
    check(!authored.IsNull(), "split: authored piece located by geometry");
    if (authored.IsNull()) return;
    // No piece holds the slot, so AcquireElementIds stamps no generatorSlot on
    // the authored piece or its sibling: a later edit that consumes the authored
    // piece can never narrow the pool onto the sibling.
    session::TopologyOwnerLedger ledger;
    session::apply_topology_history(ledger, "ope", {"base"}, r1.outcome.topology_history);
    int pieces = 0;
    const auto filter = session::generator_slot_filter(ledger, "base", "ope", slot);
    for (int i = 1; i <= edges.Extent(); ++i) {
        const gp_Pnt m = edge_mid(edges(i));
        if (std::abs(m.X()) > 1e-3 || std::abs(m.Z() - 8) > 1e-3 ||
            (std::abs(m.Y() - 2.25) > 1e-3 && std::abs(m.Y() - 7.75) > 1e-3))
            continue;
        ++pieces;
        const session::OriginClaim claim = ledger.lookup("base", edges(i));
        check(claim.slot.empty(), "split: fragment claim carries no slot");
        check(!filter || !filter(edges(i)), "split: the split slot admits no fragment");
    }
    check(pieces == 2, "split: both fragments of the root located");
    // State 2 (an upstream edit swallows the authored piece): the unslotted ref
    // resolves as at HEAD and is refused, never bound to the sibling.
    const json desc = em::ElementMapPartition::descriptor_to_json(
        em::ElementMapPartition::describe(authored, r1.shape()));
    const gp_Pnt a = edge_mid(authored);
    const json input = {{"primary", {{"elementId", "el1"}, {"kind", "edge"}, {"bodyId", "base"}}},
                        {"intent", {{"kind", "edge"}, {"descriptor", desc}}},
                        {"anchor", {{"worldPoint", {a.X(), a.Y(), a.Z()}}}}};
    const Run r2 = extrude(u_sketch(), {{"booleanMode", "Add"}, {"targetBodyId", "base"}},
                           u_base(-1, 19));
    em::LadderEditContext edit;
    edit.post_upstream_edit = true;
    const auto res = em::resolve_descriptor_stage(
        r2.shape(), "base", {em::ladder_ref_from_input(input, "r")}, edit, {}, {});
    check(res.size() == 1 && res[0].outcome != em::LadderOutcome::AutoBind,
          "split: the sibling is never auto-bound (NeedsRepair)");
}

// A later Cut that splits a slotted wall leaves neither piece holding the slot.
void test_later_cut_clears_split_slots() {
    const json s1 = rect_sketch();
    // Notch through wall e1 (world x = 0), u in [4.5, 5.5].
    json s2 = sketch_of(json::array({line("n1", 4.5, -1, 5.5, -1), line("n2", 5.5, -1, 5.5, 1),
                                     line("n3", 5.5, 1, 4.5, 1), line("n4", 4.5, 1, 4.5, -1)}));
    s2["sketchId"] = "sk2";
    std::vector<std::pair<std::string, json>> sketches{{"sk1", s1}, {"sk2", s2}};
    std::string last = "sk1";
    onecad::CancelToken cancel;
    session::BodyStore bodies;
    em::ElementMapPartition partition;
    ops::OpContext ctx{bodies, &sketches, partition, &last, false, json::object(), &cancel};
    session::TopologyOwnerLedger ledger;
    const json p1 = {{"sketchId", "sk1"}, {"distance", 8.0}, {"extrudeMode", "Blind"},
                     {"booleanMode", "NewBody"}, {"regionIdentityVersion", 3},
                     {"regionId", only_region_id(s1)}};
    auto o1 = ops::execute_extrude(ctx, {{"opType", "Extrude"}, {"opId", "opA"}, {"params", p1}},
                                   "opA");
    std::vector<std::string> touched;
    for (const auto& e : o1.body_events) touched.push_back(e.body_id);
    session::apply_topology_history(ledger, "opA", touched, o1.topology_history);
    const std::string bid = o1.topology_history.at(0).body_id;
    last = "sk2";
    const json p2 = {{"sketchId", "sk2"}, {"distance", 8.0}, {"extrudeMode", "Blind"},
                     {"booleanMode", "Cut"}, {"targetBodyId", bid}, {"regionIdentityVersion", 3},
                     {"regionId", only_region_id(s2)}};
    auto o2 = ops::execute_extrude(ctx, {{"opType", "Extrude"}, {"opId", "opB"}, {"params", p2}},
                                   "opB");
    check(o2.status == ops::OpOutcome::Status::Ok, "cut: notch Ok");
    touched.clear();
    for (const auto& e : o2.body_events) touched.push_back(e.body_id);
    session::apply_topology_history(ledger, "opB", touched, o2.topology_history);
    const auto claims = ledger.effective_claims(bid);
    std::map<std::string, int> held;
    for (int i = 1; i <= claims.Extent(); ++i) {
        const session::OriginClaim& c = claims.FindFromIndex(i);
        if (c.state == session::OriginState::Known && c.scope == session::SlotScope::Generator &&
            !c.slot.empty())
            ++held[c.producer_record_id + " " + c.slot];
    }
    check(held.count("opA side:e1") == 0, "cut: the split wall's slot is held by no piece");
    for (const auto& [key, n] : held) check(n == 1, "cut: slot unique per body: " + key);
}

// Direct ledger rule: a (producer, slot) held by two classes is cleared; a unique
// one and Point slots are untouched.
void test_ledger_uniqueness() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10, 10, 10).Shape();
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(box, TopAbs_FACE, faces);
    session::TopologyBodyHistory history;
    history.body_id = "b";
    history.coverage = session::HistoryCoverage::Proven;
    history.births.push_back({faces(1), "dup", session::SlotScope::Generator});
    history.births.push_back({faces(2), "dup", session::SlotScope::Generator});
    history.births.push_back({faces(3), "uniq", session::SlotScope::Generator});
    history.births.push_back({faces(4), "p", session::SlotScope::Point});
    history.births.push_back({faces(5), "p", session::SlotScope::Point});
    session::TopologyOwnerLedger ledger;
    session::apply_topology_history(ledger, "op1", {"b"}, {history});
    check(ledger.lookup("b", faces(1)).slot.empty() && ledger.lookup("b", faces(2)).slot.empty(),
          "uniqueness: duplicated Generator slot cleared on both holders");
    check(ledger.lookup("b", faces(1)).state == session::OriginState::Known,
          "uniqueness: a cleared claim keeps its producer");
    check(ledger.lookup("b", faces(3)).slot == "uniq", "uniqueness: unique slot kept");
    check(ledger.lookup("b", faces(4)).slot == "p" && ledger.lookup("b", faces(5)).slot == "p",
          "uniqueness: Point slots untouched");
    check(!session::generator_slot_filter(ledger, "b", "op1", "dup") ||
              !session::generator_slot_filter(ledger, "b", "op1", "dup")(faces(1)),
          "uniqueness: a cleared slot admits nothing");
}

// Pristine rules at the ledger: birth and IsSame survivor are pristine, a Modified
// successor is not, and merging is an AND.
void test_pristine_ledger() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10, 10, 10).Shape();
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(box, TopAbs_FACE, faces);
    session::TopologyBodyHistory first;
    first.body_id = "b";
    first.coverage = session::HistoryCoverage::Proven;
    first.births.push_back({faces(1), "s1", session::SlotScope::Generator});
    first.births.push_back({faces(2), "s2", session::SlotScope::Generator});
    session::TopologyOwnerLedger ledger;
    session::apply_topology_history(ledger, "op1", {"b"}, {first});
    check(ledger.lookup("b", faces(1)).pristine, "pristine: a birth is pristine");
    session::TopologyBodyHistory second;
    second.body_id = "b";
    second.coverage = session::HistoryCoverage::Proven;
    second.survivors.push_back({faces(1), faces(1)});
    second.successors.push_back({{faces(2)}, faces(3)});
    session::apply_topology_history(ledger, "op2", {"b"}, {second});
    const session::OriginClaim survivor = ledger.lookup("b", faces(1));
    const session::OriginClaim successor = ledger.lookup("b", faces(3));
    check(survivor.slot == "s1" && survivor.pristine, "pristine: IsSame survivor stays pristine");
    check(successor.slot == "s2" && !successor.pristine,
          "pristine: Modified successor keeps the slot, not pristine");
    session::OriginClaim a{session::OriginState::Known, "p", "x", session::SlotScope::Generator};
    session::OriginClaim b = a;
    b.pristine = false;
    check(!session::merge_origin(a, b).pristine && session::merge_origin(a, a).pristine,
          "pristine: merge is AND");
    const auto filter = session::generator_slot_filter(ledger, "b", "op1", "s1");
    check(static_cast<bool>(filter) && filter(faces(1)), "pristine: pristine holder narrows");
    // A non-pristine holder of the (record, slot) withholds narrowing.
    check(!session::generator_slot_filter(ledger, "b", "op1", "s2"),
          "pristine: a non-pristine holder yields no narrowing");
}

// probe2: a fin swallows one side of the slotted edge (one trimmed image keeps the
// slot), then an edit moves the fin so a DISJOINT remnant is the single image.
void test_trimmed_continuation_not_narrowed() {
    const std::string slot = "edge:cap:+|side:a";
    const auto fins = [](double a0, double a1, double b0, double b1) {
        const TopoDS_Shape f1 = BRepPrimAPI_MakeBox(gp_Pnt(-10, a0, -1), 14, a1 - a0, 21).Shape();
        const TopoDS_Shape f2 = BRepPrimAPI_MakeBox(gp_Pnt(-10, b0, -1), 14, b1 - b0, 21).Shape();
        const TopoDS_Shape slab = BRepPrimAPI_MakeBox(gp_Pnt(-14, -2, -2), 20, 34, 1).Shape();
        return BRepAlgoAPI_Fuse(BRepAlgoAPI_Fuse(f1, f2).Shape(), slab).Shape();
    };
    const Run r1 = extrude(u_sketch(), {{"booleanMode", "Add"}, {"targetBodyId", "base"}},
                           fins(-1, 5.5, 19, 25.5));
    check(r1.counts().count(slot) == 1, "probe2: one trimmed image keeps the slot (state 1)");
    const TopoDS_Shape authored = r1.find(slot);
    if (authored.IsNull()) return;
    session::TopologyOwnerLedger ledger1;
    session::apply_topology_history(ledger1, "ope", {"base"}, r1.outcome.topology_history);
    check(!ledger1.lookup("base", authored).pristine, "probe2: the trimmed holder is not pristine");
    json desc = em::ElementMapPartition::descriptor_to_json(
        em::ElementMapPartition::describe(authored, r1.shape()));
    desc["generatorSlot"] = {{"recordId", "ope"}, {"slot", slot}};
    const gp_Pnt a = edge_mid(authored);
    const json input = {{"primary", {{"elementId", "el1"}, {"kind", "edge"}, {"bodyId", "base"}}},
                        {"intent", {{"kind", "edge"}, {"descriptor", desc}}},
                        {"anchor", {{"worldPoint", {a.X(), a.Y(), a.Z()}}}}};
    const Run r2 = extrude(u_sketch(), {{"booleanMode", "Add"}, {"targetBodyId", "base"}},
                           fins(4.5, 11, 24.5, 31));
    session::TopologyOwnerLedger ledger2;
    session::apply_topology_history(ledger2, "ope", {"base"}, r2.outcome.topology_history);
    const auto filter = session::generator_slot_filter(ledger2, "base", "ope", slot);
    check(!filter, "probe2: non-pristine holder -> no narrowing");
    em::LadderEditContext edit;
    edit.post_upstream_edit = true;
    const em::LadderRef ref = em::ladder_ref_from_input(input, "r");
    const auto head = em::resolve_descriptor_stage(r2.shape(), "base", {ref}, edit, {}, {});
    std::vector<em::CandidateFilter> filters(1);
    filters[0] = filter;
    const auto slotted = em::resolve_descriptor_stage(r2.shape(), "base", {ref}, edit, {}, filters);
    check(head.size() == 1 && slotted.size() == 1 && head[0].outcome != em::LadderOutcome::AutoBind &&
              slotted[0].outcome == head[0].outcome && slotted[0].reason == head[0].reason,
          "probe2: not AutoBind, identical to HEAD (" + (head.empty() ? "" : head[0].reason) + ")");
}

// split2: an end-notch Cut trims the slotted wall (one Modified image keeps the
// slot); moving the notch to the other end leaves a disjoint remnant.
void test_cut_continuation_not_narrowed() {
    const json s1 = rect_sketch();
    const auto notch = [](double u0, double u1) {
        json s = sketch_of(json::array({line("n1", u0, -1, u1, -1), line("n2", u1, -1, u1, 1),
                                        line("n3", u1, 1, u0, 1), line("n4", u0, 1, u0, -1)}));
        s["sketchId"] = "sk2";
        return s;
    };
    struct State { session::BodyStore bodies; session::TopologyOwnerLedger ledger; std::string bid; };
    const auto build = [&](const json& s2, State& st) {
        std::vector<std::pair<std::string, json>> sketches{{"sk1", s1}, {"sk2", s2}};
        std::string last = "sk1";
        onecad::CancelToken cancel;
        em::ElementMapPartition partition;
        ops::OpContext ctx{st.bodies, &sketches, partition, &last, false, json::object(), &cancel};
        const json p1 = {{"sketchId", "sk1"}, {"distance", 8.0}, {"extrudeMode", "Blind"},
                         {"booleanMode", "NewBody"}, {"regionIdentityVersion", 3},
                         {"regionId", only_region_id(s1)}};
        auto o1 = ops::execute_extrude(ctx, {{"opType", "Extrude"}, {"opId", "opA"}, {"params", p1}},
                                       "opA");
        std::vector<std::string> touched;
        for (const auto& e : o1.body_events) touched.push_back(e.body_id);
        session::apply_topology_history(st.ledger, "opA", touched, o1.topology_history);
        st.bid = o1.topology_history.at(0).body_id;
        last = "sk2";
        const json p2 = {{"sketchId", "sk2"}, {"distance", 8.0}, {"extrudeMode", "Blind"},
                         {"booleanMode", "Cut"}, {"targetBodyId", st.bid},
                         {"regionIdentityVersion", 3}, {"regionId", only_region_id(s2)}};
        auto o2 = ops::execute_extrude(ctx, {{"opType", "Extrude"}, {"opId", "opB"}, {"params", p2}},
                                       "opB");
        touched.clear();
        for (const auto& e : o2.body_events) touched.push_back(e.body_id);
        session::apply_topology_history(st.ledger, "opB", touched, o2.topology_history);
        check(o2.status == ops::OpOutcome::Status::Ok, "split2: cut Ok");
    };
    const auto wall = [](const State& st) {
        TopoDS_Shape found;
        const auto claims = st.ledger.effective_claims(st.bid);
        for (int i = 1; i <= claims.Extent(); ++i) {
            const session::OriginClaim& c = claims.FindFromIndex(i);
            if (c.producer_record_id == "opA" && c.slot == "side:e1" &&
                c.scope == session::SlotScope::Generator)
                found = claims.FindKey(i);
        }
        return found;
    };
    State a, b;
    build(notch(-1, 5.5), a);   // trims the start of the wall, remnant u in [5.5, 10]
    build(notch(4.5, 11), b);   // trims the end, remnant u in [0, 4.5]: disjoint
    const TopoDS_Shape authored = wall(a);
    check(!authored.IsNull(), "split2: the trimmed wall keeps its slot");
    if (authored.IsNull()) return;
    check(!a.ledger.lookup(a.bid, authored).pristine, "split2: the trimmed wall is not pristine");
    const TopoDS_Shape a_body = a.bodies.get(a.bid)->geom;
    const TopoDS_Shape b_body = b.bodies.get(b.bid)->geom;
    json desc = em::ElementMapPartition::descriptor_to_json(
        em::ElementMapPartition::describe(authored, a_body));
    desc["generatorSlot"] = {{"recordId", "opA"}, {"slot", "side:e1"}};
    const gp_Pnt c = centre(authored);
    const json input = {{"primary", {{"elementId", "el1"}, {"kind", "face"}, {"bodyId", a.bid}}},
                        {"intent", {{"kind", "face"}, {"descriptor", desc}}},
                        {"anchor", {{"worldPoint", {c.X(), c.Y(), c.Z()}}}}};
    const auto filter = session::generator_slot_filter(b.ledger, b.bid, "opA", "side:e1");
    check(!filter, "split2: non-pristine holder -> no narrowing");
    em::LadderEditContext edit;
    edit.post_upstream_edit = true;
    const em::LadderRef ref = em::ladder_ref_from_input(input, "r");
    std::vector<em::CandidateFilter> filters(1);
    filters[0] = filter;
    const auto head = em::resolve_descriptor_stage(b_body, b.bid, {ref}, edit, {}, {});
    const auto slotted = em::resolve_descriptor_stage(b_body, b.bid, {ref}, edit, {}, filters);
    check(head.size() == 1 && slotted.size() == 1 && slotted[0].outcome == head[0].outcome &&
              slotted[0].reason == head[0].reason,
          "split2: identical to the unnarrowed HEAD outcome (" +
              (head.empty() ? std::string() : head[0].reason) + ")");
    std::fprintf(stderr, "split2 HEAD outcome: %s %s ncand=%zu\n",
                 head[0].outcome == em::LadderOutcome::AutoBind ? "AutoBind" : "NeedsRepair",
                 head[0].reason.c_str(), head[0].candidates.size());
}

// Legacy profiles (no regionIdentityVersion) stamp no slots on any path.
void test_legacy_profiles_carry_no_slots() {
    const json circle = {{"id", "c1"}, {"type", "Circle"}, {"center", {5, 3}}, {"radius", 1.0}};
    const json disc = sketch_of(json::array({circle}));
    const json rect_hole = sketch_of(json::array({line("e1", 0, 0, 10, 0), line("e2", 10, 0, 10, 6),
                                                  line("e3", 10, 6, 0, 6), line("e4", 0, 6, 0, 0),
                                                  circle}));
    for (const json& sketch : {rect_sketch(), disc, rect_hole}) {
        std::vector<std::pair<std::string, json>> sketches{{"sk1", sketch}};
        std::string last = "sk1";
        onecad::CancelToken cancel;
        session::BodyStore bodies;
        em::ElementMapPartition partition;
        ops::OpContext ctx{bodies, &sketches, partition, &last, false, json::object(), &cancel};
        const json params = {{"sketchId", "sk1"}, {"distance", 8.0}, {"extrudeMode", "Blind"},
                             {"booleanMode", "NewBody"}};
        const auto out = ops::execute_extrude(
            ctx, {{"opType", "Extrude"}, {"opId", "ope"}, {"params", params}}, "ope");
        int slotted = 0;
        for (const auto& h : out.topology_history)
            for (const auto& b : h.births) slotted += b.slot.empty() ? 0 : 1;
        check(out.status == ops::OpOutcome::Status::Ok, "legacy: extrude Ok");
        check(slotted == 0, "legacy profile stamps no slots, got " + std::to_string(slotted));
    }
}

// FeaturePattern: selected producer -> virtual id; unselected -> erased; a shared
// host face is left verbatim.
void test_pattern_rewrite() {
    const json ref = {{"intent",
                       {{"descriptor",
                         {{"generatorSlot", {{"recordId", "src"}, {"slot", "edge:x"}}},
                          {"originSlot", {{"recordId", "src"}, {"slot", "p0"}}}}}}}};
    const std::string virtual_id = session::feature_pattern_virtual_id("pat", "src", 2);
    json selected = ref;
    session::rewrite_ref_slot_claims(selected, "pat", 2, false, {"src"});
    const json& d = selected["intent"]["descriptor"];
    check(d["generatorSlot"]["recordId"] == virtual_id && d["generatorSlot"]["slot"] == "edge:x",
          "pattern: selected producer rewritten to the virtual id");
    check(d["originSlot"]["recordId"] == virtual_id, "pattern: originSlot rewritten as before");
    json other = ref;
    session::rewrite_ref_slot_claims(other, "pat", 2, false, {"someone_else"});
    check(!other["intent"]["descriptor"].contains("generatorSlot"),
          "pattern: unselected producer ERASES generatorSlot");
    check(other["intent"]["descriptor"]["originSlot"]["recordId"] == "src",
          "pattern: unselected originSlot keeps its producer");
    json shared = ref;
    session::rewrite_ref_slot_claims(shared, "pat", 2, true, {"src"});
    check(shared == ref, "pattern: shared host face left unchanged");
    json malformed = {{"intent", {{"descriptor", {{"generatorSlot", 7}}}}}};
    session::rewrite_ref_slot_claims(malformed, "pat", 2, false, {"src"});
    check(!malformed["intent"]["descriptor"].contains("generatorSlot"),
          "pattern: malformed generatorSlot erased");
}

}  // namespace

int main() {
    test_blind_rectangle();
    test_symmetric_and_two_direction();
    test_draft();
    test_circle();
    test_two_arcs();
    test_split_entity_faces_unslotted();
    test_boolean_add();
    test_ledger();
    test_element_identity_origin_slot();
    test_generator_slot_filter();
    test_split_root_never_binds_sibling();
    test_later_cut_clears_split_slots();
    test_ledger_uniqueness();
    test_pristine_ledger();
    test_trimmed_continuation_not_narrowed();
    test_cut_continuation_not_narrowed();
    test_legacy_profiles_carry_no_slots();
    test_pattern_rewrite();
    if (g_failures == 0) std::printf("test_generator_slots: all passed\n");
    return g_failures;
}
