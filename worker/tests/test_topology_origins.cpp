#include <cstdio>

#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <gp_Pnt.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp_Explorer.hxx>

#include "session/Session.h"
#include "session/TopologyOrigins.h"

namespace s = onecad::session;
namespace {
int failures = 0;
void check(bool condition, const char* message) {
    if (!condition) { std::fprintf(stderr, "FAIL: %s\n", message); ++failures; }
}
TopoDS_Shape vertex(double x) { return BRepBuilderAPI_MakeVertex(gp_Pnt(x, 0, 0)); }

void relation_policy() {
    const TopoDS_Shape host = vertex(0), tool = vertex(1), survivor = vertex(2);
    const TopoDS_Shape successor = vertex(3), intersection = vertex(4);
    s::TopologyOwnerLedger ledger;
    ledger.add({"body", host, {s::OriginState::Known, "host-op"}});
    ledger.add({"body", tool, {s::OriginState::Known, "tool-op"}});
    s::TopologyBodyHistory history;
    history.body_id = "body";
    history.coverage = s::HistoryCoverage::Proven;
    history.survivors.push_back({host, survivor});
    history.successors.push_back({{tool}, successor});
    history.births.push_back({intersection});
    s::apply_topology_history(ledger, "boolean-op", {"body"}, {history});
    check(ledger.lookup("body", survivor).producer_record_id == "host-op",
          "survivor inherits host producer");
    check(ledger.lookup("body", successor).producer_record_id == "tool-op",
          "Modified successor inherits source producer");
    check(ledger.lookup("body", intersection).producer_record_id == "boolean-op",
          "certified Boolean intersection is current-operation birth");
}

void conflicts_fail_closed() {
    const TopoDS_Shape a = vertex(10), b = vertex(11), mixed = vertex(12);
    s::TopologyOwnerLedger ledger;
    ledger.add({"body", a, {s::OriginState::Known, "a"}});
    ledger.add({"body", b, {s::OriginState::Known, "b"}});
    s::TopologyBodyHistory history;
    history.body_id = "body";
    history.coverage = s::HistoryCoverage::Proven;
    history.successors.push_back({{a, b}, mixed});
    s::apply_topology_history(ledger, "op", {"body"}, {history});
    check(ledger.lookup("body", mixed).state == s::OriginState::Ambiguous,
          "distinct inherited producers are ambiguous");

    s::TopologyOwnerLedger unknown;
    history.successors = {{{vertex(99)}, mixed}};
    history.births = {{mixed}};
    s::apply_topology_history(unknown, "op", {"body"}, {history});
    check(unknown.lookup("body", mixed).state == s::OriginState::Ambiguous,
          "Unknown inherited plus Birth is conflicting");

    s::TopologyOwnerLedger duplicate;
    duplicate.add({"body", a, {s::OriginState::Known, "a"}});
    s::apply_topology_history(duplicate, "op", {"body"}, {history, history});
    check(!duplicate.contains("body", mixed), "duplicate body histories invalidate ownership");
}

void stale_and_rename() {
    const TopoDS_Shape stale = BRepBuilderAPI_MakeEdge(gp_Pnt(0, 0, 0), gp_Pnt(1, 0, 0));
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(2, 2, 2).Shape();
    s::TopologyOwnerLedger ledger;
    ledger.add({"old", stale, {s::OriginState::Known, "producer"}});
    ledger.rename_body("old", "body");
    check(ledger.contains("body", stale), "body rename preserves exact ownership");
    ledger.retain_body_members("body", box);
    check(!ledger.contains("body", stale), "claim absent from final body is removed");
}

void session_lifecycle() {
    onecad::session::Session session;
    session.open("doc", 0, 7, "determinism");
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(2, 2, 2).Shape();
    TopExp_Explorer edges(box, TopAbs_EDGE);
    const TopoDS_Shape edge = edges.Current();
    s::ScratchJob first;
    first.job_id = 1;
    first.prepared_snapshot_id = 1;
    first.history_prefix_hash = "head-a";
    first.bodies.create("body", "producer", box);
    first.topology_owners.add({"body", edge, {s::OriginState::Known, "producer"}});
    session.store_prepared(std::move(first));
    check(session.accept_prepared(1, 0, 7).ok, "prepared ownership publishes atomically");
    check(session.topology_owners_copy().lookup("body", edge).producer_record_id == "producer",
          "accepted head retains ownership");

    const s::CheckpointState checkpoint = session.save_checkpoint(1);
    check(checkpoint.topology_owners.lookup("body", edge).producer_record_id == "producer",
          "checkpoint clones ownership");

    s::ScratchJob discarded;
    discarded.job_id = 2;
    discarded.topology_owners.add({"body", edge, {s::OriginState::Known, "wrong"}});
    session.store_prepared(std::move(discarded));
    check(session.discard_prepared(2), "discard removes candidate scratch");
    check(session.topology_owners_copy().lookup("body", edge).producer_record_id == "producer",
          "discard cannot leak candidate ownership");

    check(session.restore_checkpoint(1, "head-a", "checkpoint-a").restored,
          "checkpoint enters restored-base slot");
    const s::BaseCheckpointRef base{1, "checkpoint-a"};
    const s::FenceOutcome clone = session.fence_and_clone(3, 0, 7, "head-a", &base);
    check(clone.status == s::FenceOutcome::Status::Ok &&
              clone.cloned_topology_owners.lookup("body", edge).producer_record_id == "producer",
          "restored-base fence clones checkpoint ownership");

    session.reset();
    check(session.topology_owners_copy().entries().empty(), "reset clears ownership ledger");
}
}  // namespace

int main() {
    relation_policy();
    conflicts_fail_closed();
    stale_and_rename();
    session_lifecycle();
    return failures;
}
