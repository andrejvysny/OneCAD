#include <cstdio>
#include <string>

#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeShape.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRep_Builder.hxx>
#include <gp_Pnt.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>

#include "ops/TopologyHistory.h"
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

// A builder whose reported history is deliberately incomplete: it names a new
// face as Generated by one contour edge, but says NOTHING about a before-FACE
// that is gone from the result. That is exactly the unreported-replacement
// adversary the certificate's vanishing guard exists to refuse — on IsSame,
// Modified, Generated and containment alone it is indistinguishable from a
// genuinely new boundary.
class PartialHistoryBuilder : public BRepBuilderAPI_MakeShape {
public:
    explicit PartialHistoryBuilder(const TopoDS_Shape& result) {
        myShape = result;
        Done();
    }
    void report_generated(const TopoDS_Shape& from, const TopoDS_Shape& to) {
        report(generated_, from, to);
    }
    void report_modified(const TopoDS_Shape& from, const TopoDS_Shape& to) {
        report(modified_, from, to);
    }
    const TopTools_ListOfShape& Generated(const TopoDS_Shape& shape) override {
        return lookup(generated_, shape);
    }
    const TopTools_ListOfShape& Modified(const TopoDS_Shape& shape) override {
        return lookup(modified_, shape);
    }

private:
    static void report(TopTools_IndexedDataMapOfShapeListOfShape& map,
                       const TopoDS_Shape& from, const TopoDS_Shape& to) {
        const int index = map.FindIndex(from);
        if (index > 0) {
            map.ChangeFromIndex(index).Append(to);
            return;
        }
        TopTools_ListOfShape list;
        list.Append(to);
        map.Add(from, list);
    }
    const TopTools_ListOfShape& lookup(const TopTools_IndexedDataMapOfShapeListOfShape& map,
                                       const TopoDS_Shape& shape) const {
        const int index = map.FindIndex(shape);
        return index > 0 ? map(index) : empty_;
    }
    TopTools_IndexedDataMapOfShapeListOfShape generated_;
    TopTools_IndexedDataMapOfShapeListOfShape modified_;
    TopTools_ListOfShape empty_;
};

void boundary_completion_is_certified() {
    const TopoDS_Shape before = BRepPrimAPI_MakeBox(2, 2, 2).Shape();
    TopTools_IndexedMapOfShape before_faces;
    TopTools_IndexedMapOfShape before_edges;
    TopExp::MapShapes(before, TopAbs_FACE, before_faces);
    TopExp::MapShapes(before, TopAbs_EDGE, before_edges);
    const TopoDS_Shape dropped = before_faces(1);
    const TopoDS_Shape contour = before_edges(1);
    // A separate box supplies a face with a fresh TShape, so none of its four
    // edges and four vertices is IsSame anything in `before`: they are precisely
    // the orphans boundary completion exists to attribute. Dropping one before
    // face keeps every before EDGE alive (each is shared with a retained face),
    // so the vanishing guard turns on the FACE alone.
    TopTools_IndexedMapOfShape born_faces;
    TopExp::MapShapes(BRepPrimAPI_MakeBox(gp_Pnt(10, 10, 10), 1, 1, 1).Shape(),
                      TopAbs_FACE, born_faces);
    const TopoDS_Shape born = born_faces(1);
    // A third box supplies the CORNER patch: a face a real blend reports under
    // `Generated(vertex)` rather than `Generated(edge)` when three blends meet.
    // Its boundary is exclusive to it, so an edge-only witness rule leaves those
    // orphans Unknown (Astra break F4).
    TopTools_IndexedMapOfShape corner_faces;
    TopExp::MapShapes(BRepPrimAPI_MakeBox(gp_Pnt(20, 20, 20), 1, 1, 1).Shape(),
                      TopAbs_FACE, corner_faces);
    const TopoDS_Shape corner = corner_faces(1);
    // A fourth box supplies a shape the builder can name in a history list while
    // it is absent from `result` — the "reported but dead" vanishing adversary.
    TopTools_IndexedMapOfShape ghost_faces;
    TopExp::MapShapes(BRepPrimAPI_MakeBox(gp_Pnt(30, 30, 30), 1, 1, 1).Shape(),
                      TopAbs_FACE, ghost_faces);
    const TopoDS_Shape ghost = ghost_faces(1);
    BRep_Builder compound_builder;
    TopoDS_Compound result;
    compound_builder.MakeCompound(result);
    for (int i = 2; i <= before_faces.Extent(); ++i) compound_builder.Add(result, before_faces(i));
    compound_builder.Add(result, born);
    compound_builder.Add(result, corner);

    const auto face_births = [&](const s::TopologyBodyHistory& history,
                                 const TopoDS_Shape& face, TopAbs_ShapeEnum kind) {
        TopTools_IndexedMapOfShape members;
        TopExp::MapShapes(face, kind, members);
        int count = 0;
        for (const s::TopologyBirth& birth : history.births)
            if (members.Contains(birth.after)) ++count;
        return count;
    };
    const auto born_births = [&](const s::TopologyBodyHistory& history, TopAbs_ShapeEnum kind) {
        return face_births(history, born, kind);
    };
    const onecad::ops::BoundaryCompletionCertificate certificate{{contour}};

    PartialHistoryBuilder incomplete(result);
    incomplete.report_generated(contour, born);
    const s::TopologyBodyHistory refused = onecad::ops::modified_body_history(
        "body", before, result, incomplete, {}, &certificate);
    check(born_births(refused, TopAbs_EDGE) == 0 && born_births(refused, TopAbs_VERTEX) == 0,
          "a vanished before face with no reported history refuses all completion");
    s::TopologyOwnerLedger refused_ledger;
    s::apply_topology_history(refused_ledger, "blend-op", {"body"}, {refused});
    TopTools_IndexedMapOfShape born_edges;
    TopExp::MapShapes(born, TopAbs_EDGE, born_edges);
    bool all_unknown = true;
    for (int i = 1; i <= born_edges.Extent(); ++i)
        if (refused_ledger.lookup("body", born_edges(i)).state != s::OriginState::Unknown)
            all_unknown = false;
    check(all_unknown, "orphans of a refused certificate stay Unknown, never inferred");

    // Same geometry, same witness face: only the vanished face's history differs,
    // so the certificate is what decides, not the containment.
    PartialHistoryBuilder accounted(result);
    accounted.report_generated(contour, born);
    accounted.report_generated(dropped, born);
    const s::TopologyBodyHistory certified = onecad::ops::modified_body_history(
        "body", before, result, accounted, {}, &certificate);
    check(born_births(certified, TopAbs_EDGE) == 4 && born_births(certified, TopAbs_VERTEX) == 4,
          "a fully accounted blend completes its witness face boundary");
    check(onecad::ops::modified_body_history("body", before, result, accounted).births.size() ==
              certified.births.size() - 8,
          "no certificate means no completion at all");

    // Non-empty history is NOT proof of accounting. A vanished before face whose
    // only report is ITSELF says nothing about where its boundary went, and one
    // whose successor is absent from `result` says even less — both are the
    // unreported-replacement adversary wearing a history list.
    PartialHistoryBuilder self_modified(result);
    self_modified.report_generated(contour, born);
    self_modified.report_modified(dropped, dropped);
    const s::TopologyBodyHistory self_refused = onecad::ops::modified_body_history(
        "body", before, result, self_modified, {}, &certificate);
    check(born_births(self_refused, TopAbs_EDGE) == 0 &&
              born_births(self_refused, TopAbs_VERTEX) == 0,
          "a vanished before face reported only as itself refuses all completion");

    PartialHistoryBuilder dead_successor(result);
    dead_successor.report_generated(contour, born);
    dead_successor.report_modified(dropped, ghost);
    const s::TopologyBodyHistory dead_refused = onecad::ops::modified_body_history(
        "body", before, result, dead_successor, {}, &certificate);
    check(born_births(dead_refused, TopAbs_EDGE) == 0 &&
              born_births(dead_refused, TopAbs_VERTEX) == 0,
          "a vanished before face whose successor is absent from the result refuses completion");
    s::TopologyOwnerLedger dead_ledger;
    s::apply_topology_history(dead_ledger, "blend-op", {"body"}, {dead_refused});
    bool dead_unknown = true;
    for (int i = 1; i <= born_edges.Extent(); ++i)
        if (dead_ledger.lookup("body", born_edges(i)).state != s::OriginState::Unknown)
            dead_unknown = false;
    check(dead_unknown, "orphans of a dead-successor certificate stay Unknown");

    // A corner patch the builder attributes to a before VERTEX is as much an
    // operation-born face as an edge-generated blend: it satisfies the
    // derivation's F = { D(f) and P(f) = empty }, so its exclusive boundary
    // completes too.
    TopTools_IndexedMapOfShape before_vertices;
    TopExp::MapShapes(before, TopAbs_VERTEX, before_vertices);
    PartialHistoryBuilder with_corner(result);
    with_corner.report_generated(contour, born);
    with_corner.report_generated(dropped, born);
    with_corner.report_generated(before_vertices(1), corner);
    const s::TopologyBodyHistory corner_certified = onecad::ops::modified_body_history(
        "body", before, result, with_corner, {}, &certificate);
    check(face_births(corner_certified, corner, TopAbs_EDGE) == 4 &&
              face_births(corner_certified, corner, TopAbs_VERTEX) == 4,
          "a vertex-generated corner patch is a witness and completes its own boundary");
    check(face_births(certified, corner, TopAbs_EDGE) == 0 &&
              face_births(certified, corner, TopAbs_VERTEX) == 0,
          "an after face the builder never reports is no witness at all");
}

// Real OCCT, not a synthetic history: three blends meeting at one cube corner is
// the configuration that produces a face reported under `Generated(vertex)`.
void blend_corner_completes_every_orphan() {
    const TopoDS_Shape cube = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    TopTools_IndexedDataMapOfShapeListOfShape vertex_edges;
    TopExp::MapShapesAndAncestors(cube, TopAbs_VERTEX, TopAbs_EDGE, vertex_edges);
    // The ancestor list repeats each edge once per incident face, and feeding a
    // duplicate to `Add` would leave the contour count to OCCT's coalescing.
    TopTools_IndexedMapOfShape corner_edges;
    for (const TopoDS_Shape& edge : vertex_edges(1)) corner_edges.Add(edge);
    BRepFilletAPI_MakeFillet blend(cube);
    for (int i = 1; i <= corner_edges.Extent(); ++i)
        blend.Add(1.0, TopoDS::Edge(corner_edges(i)));
    blend.Build();
    check(corner_edges.Extent() == 3 && blend.IsDone(),
          "three edges meet at the chosen cube corner and the fillet builds");
    if (!blend.IsDone()) return;
    const TopoDS_Shape result = blend.Shape();
    onecad::ops::BoundaryCompletionCertificate certificate;
    for (int contour = 1; contour <= blend.NbContours(); ++contour)
        for (int i = 1; i <= blend.NbEdges(contour); ++i)
            certificate.contour_edges.push_back(blend.Edge(contour, i));

    const auto omitted = [&](const s::TopologyBodyHistory& history, TopAbs_ShapeEnum kind) {
        TopTools_IndexedMapOfShape outputs;
        TopExp::MapShapes(result, kind, outputs);
        int classified = 0;
        for (const auto& item : history.survivors) if (outputs.Contains(item.after)) ++classified;
        for (const auto& item : history.successors) if (outputs.Contains(item.after)) ++classified;
        for (const auto& item : history.births) if (outputs.Contains(item.after)) ++classified;
        for (const auto& item : history.conflicts) if (outputs.Contains(item.after)) ++classified;
        return outputs.Extent() - classified;
    };
    const s::TopologyBodyHistory bare =
        onecad::ops::modified_body_history("body", cube, result, blend);
    const s::TopologyBodyHistory certified = onecad::ops::modified_body_history(
        "body", cube, result, blend, {}, &certificate);
    // Count the vertex-ONLY witness faces the same way the ledger does, so the
    // check message records the measured corner-patch count.
    TopTools_IndexedMapOfShape before_faces, after_faces, before_edges, before_vertices;
    TopExp::MapShapes(cube, TopAbs_FACE, before_faces);
    TopExp::MapShapes(result, TopAbs_FACE, after_faces);
    TopExp::MapShapes(cube, TopAbs_EDGE, before_edges);
    TopExp::MapShapes(cube, TopAbs_VERTEX, before_vertices);
    TopTools_IndexedMapOfShape modified_faces;
    for (int i = 1; i <= before_faces.Extent(); ++i)
        for (const TopoDS_Shape& x : blend.Modified(before_faces(i)))
            if (!x.IsSame(before_faces(i))) modified_faces.Add(x);
    const auto is_witness = [&](const TopoDS_Shape& face) {
        return face.ShapeType() == TopAbs_FACE && after_faces.Contains(face) &&
               !before_faces.Contains(face) && !modified_faces.Contains(face);
    };
    TopTools_IndexedMapOfShape from_edge, from_vertex;
    for (int i = 1; i <= before_edges.Extent(); ++i)
        for (const TopoDS_Shape& g : blend.Generated(before_edges(i)))
            if (is_witness(g)) from_edge.Add(g);
    for (int i = 1; i <= before_vertices.Extent(); ++i)
        for (const TopoDS_Shape& g : blend.Generated(before_vertices(i)))
            if (is_witness(g)) from_vertex.Add(g);
    int vertex_only = 0;
    for (int i = 1; i <= from_vertex.Extent(); ++i)
        if (!from_edge.Contains(from_vertex(i))) ++vertex_only;
    char message[224];
    std::snprintf(message, sizeof(message),
                  "three-edge corner blend leaves no orphan: omitted edges %d->%d, "
                  "vertices %d->%d; witness faces edge=%d vertexOnly=%d",
                  omitted(bare, TopAbs_EDGE), omitted(certified, TopAbs_EDGE),
                  omitted(bare, TopAbs_VERTEX), omitted(certified, TopAbs_VERTEX),
                  from_edge.Extent(), vertex_only);
    check(omitted(bare, TopAbs_EDGE) > 0 && vertex_only == 1 &&
              omitted(certified, TopAbs_EDGE) == 0 &&
              omitted(certified, TopAbs_VERTEX) == 0,
          message);
}

void effective_claims_agree_with_lookup() {
    const TopoDS_Shape survivor = vertex(30), conflicted = vertex(31);
    const TopoDS_Shape elsewhere = vertex(32), absent = vertex(33);
    s::TopologyOwnerLedger ledger;
    ledger.add({"body", survivor, {s::OriginState::Known, "producer"}});
    ledger.add({"body", survivor, {s::OriginState::Known, "producer"}});
    ledger.add({"body", conflicted, {s::OriginState::Known, "a"}});
    ledger.add({"body", conflicted, {s::OriginState::Known, "b"}});
    ledger.add({"body", conflicted, {s::OriginState::Known, "a"}});
    ledger.add({"other", elsewhere, {s::OriginState::Known, "elsewhere"}});
    const s::TopologyOwnerLedger::EffectiveClaims claims = ledger.effective_claims("body");
    const auto effective = [&](const TopoDS_Shape& shape) {
        const int index = claims.FindIndex(shape);
        return index > 0 ? claims.FindFromIndex(index) : s::OriginClaim{};
    };
    bool agrees = true;
    for (const TopoDS_Shape& shape : {survivor, conflicted, elsewhere, absent}) {
        const s::OriginClaim direct = ledger.lookup("body", shape);
        const s::OriginClaim indexed = effective(shape);
        if (direct.state != indexed.state ||
            direct.producer_record_id != indexed.producer_record_id) agrees = false;
    }
    check(agrees,
          "effective_claims agrees with lookup on survivors, collapse and other bodies");
    check(effective(survivor).state == s::OriginState::Known &&
              effective(survivor).producer_record_id == "producer" &&
              effective(conflicted).state == s::OriginState::Ambiguous &&
              effective(conflicted).producer_record_id.empty() &&
              claims.FindIndex(elsewhere) == 0 && claims.FindIndex(absent) == 0,
          "effective_claims collapses disagreeing rows and never crosses a body");
}

void inherit_origin_is_order_insensitive() {
    const TopoDS_Shape a = vertex(20), b = vertex(21), unknown = vertex(22);
    s::TopologyOwnerLedger ledger;
    ledger.add({"body", a, {s::OriginState::Known, "A"}});
    ledger.add({"body", b, {s::OriginState::Known, "B"}});
    check(s::inherit_origin(ledger, "body", {a, b, unknown}).state == s::OriginState::Ambiguous &&
              s::inherit_origin(ledger, "body", {unknown, a, b}).state ==
                  s::OriginState::Ambiguous,
          "distinct Known producers are ambiguous wherever Unknown sits in the list");
    check(s::inherit_origin(ledger, "body", {unknown, a}).state == s::OriginState::Unknown &&
              s::inherit_origin(ledger, "body", {a, unknown}).state == s::OriginState::Unknown,
          "one Unknown source blocks inheritance in either order");
    const s::OriginClaim repeated = s::inherit_origin(ledger, "body", {a, a});
    check(repeated.state == s::OriginState::Known && repeated.producer_record_id == "A",
          "one producer named twice inherits without ambiguity");
}

}  // namespace

int main() {
    relation_policy();
    conflicts_fail_closed();
    stale_and_rename();
    session_lifecycle();
    boundary_completion_is_certified();
    blend_corner_completes_every_orphan();
    effective_claims_agree_with_lookup();
    inherit_origin_is_order_insensitive();
    return failures;
}
