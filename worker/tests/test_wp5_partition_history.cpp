// test_wp5_partition_history.cpp — white-box test of ElementMap V2 history
// rebinding against REAL OCCT builder history. Demonstrates: a minted target face
// rebinds via Modified (relabeled), a consumed face is dropped via IsDeleted
// (removed), and a tool body's entries are removed. No framework: exit == failures.
#include <cstdio>
#include <string>
#include <vector>

#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>

#include "elementmap/ElementMapPartition.h"

namespace em = onecad::elementmap;
namespace km = onecad::kernel::elementmap;

namespace {
int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}

// The MapShapes face index whose bbox center has the greatest Z (the +Z "top").
int top_face_index(const TopoDS_Shape& shape) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    int best = 1;
    double best_z = -1e18;
    for (int i = 1; i <= faces.Extent(); ++i) {
        const km::ElementDescriptor d = em::ElementMapPartition::describe(faces(i));
        if (d.center.Z() > best_z) {
            best_z = d.center.Z();
            best = i;
        }
    }
    return best;
}

TopoDS_Shape face_by_center(const TopoDS_Shape& shape, double x, double y, double z) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    TopoDS_Shape best;
    double best_d2 = -1.0;
    for (int i = 1; i <= faces.Extent(); ++i) {
        const km::ElementDescriptor d = em::ElementMapPartition::describe(faces(i));
        const double dx = d.center.X() - x;
        const double dy = d.center.Y() - y;
        const double dz = d.center.Z() - z;
        const double d2 = dx * dx + dy * dy + dz * dz;
        if (best_d2 < 0.0 || d2 < best_d2) {
            best_d2 = d2;
            best = faces(i);
        }
    }
    return best;
}

class SyntheticHistory final : public BRepBuilderAPI_MakeShape {
public:
    TopTools_ListOfShape modified;
    TopTools_ListOfShape generated;
    bool deleted = false;

    const TopTools_ListOfShape& Modified(const TopoDS_Shape&) override { return modified; }
    const TopTools_ListOfShape& Generated(const TopoDS_Shape&) override { return generated; }
    Standard_Boolean IsDeleted(const TopoDS_Shape&) override { return deleted; }
};

void test_history_channel_union() {
    const TopoDS_Shape old_body = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    const TopoDS_Shape old_face = face_by_center(old_body, 0, 5, 5);
    const TopoDS_Shape new_body = BRepPrimAPI_MakeBox(12.0, 10.0, 10.0).Shape();
    const TopoDS_Shape successor = face_by_center(new_body, 0, 5, 5);

    {  // Generated-only history is authoritative.
        em::ElementMapPartition part;
        part.mint("A", "generated", km::ElementKind::Face, old_face, old_body);
        SyntheticHistory history;
        history.generated.Append(successor);
        em::ElementMapDelta delta;
        std::vector<nlohmann::json> needs_repair;
        part.apply_history("A", new_body, history, delta, &needs_repair);
        const em::PartitionEntry* entry = part.find("generated");
        check(entry != nullptr && entry->shape.IsSame(successor),
              "history union: Generated-only successor auto-binds");
        check(needs_repair.empty(), "history union: Generated-only needs no repair");
    }

    {  // One shape echoed through both channels is still one successor.
        em::ElementMapPartition part;
        part.mint("A", "dedup", km::ElementKind::Face, old_face, old_body);
        SyntheticHistory history;
        history.modified.Append(successor);
        history.generated.Append(successor);
        em::ElementMapDelta delta;
        std::vector<nlohmann::json> needs_repair;
        part.apply_history("A", new_body, history, delta, &needs_repair);
        const em::PartitionEntry* entry = part.find("dedup");
        check(entry != nullptr && entry->shape.IsSame(successor),
              "history union: identical Modified/Generated image is deduplicated");
        check(needs_repair.empty(), "history union: deduped unique image needs no repair");
    }
}

void test_union_ambiguity_and_deletion_precedence() {
    const TopoDS_Shape body = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    const TopoDS_Shape left = face_by_center(body, 0, 5, 5);
    const TopoDS_Shape right = face_by_center(body, 10, 5, 5);

    {  // Distinct images across the two channels enter the same confidence gate.
        em::ElementMapPartition part;
        part.mint("A", "ambiguous", km::ElementKind::Face, left, body,
                  nlohmann::json{{"worldPoint", {5.0, 5.0, 5.0}}});
        SyntheticHistory history;
        history.modified.Append(left);
        history.generated.Append(right);
        em::ElementMapDelta delta;
        std::vector<nlohmann::json> needs_repair;
        part.apply_history("A", body, history, delta, &needs_repair);
        check(!part.contains("ambiguous"),
              "history union: ambiguous multi-image lineage never guesses");
        check(needs_repair.size() == 1 && needs_repair[0].value("reason", "") == "ambiguous",
              "history union: ambiguous multi-image lineage emits NeedsRepair");
    }

    {  // Positive deletion evidence wins even if a history image is also reported.
        em::ElementMapPartition part;
        part.mint("A", "deleted", km::ElementKind::Face, left, body);
        SyntheticHistory history;
        history.generated.Append(right);
        history.deleted = true;
        em::ElementMapDelta delta;
        std::vector<nlohmann::json> needs_repair;
        part.apply_history("A", body, history, delta, &needs_repair);
        check(!part.contains("deleted"), "history union: IsDeleted remains definitive");
        check(delta.removed.size() == 1 && delta.removed[0] == "deleted",
              "history union: IsDeleted records removal");
        check(needs_repair.empty(), "history union: deleted entry never needs repair");
    }
}

// --- rebind-via-Modified + survivor integrity ---
void test_rebind_via_modified() {
    const TopoDS_Shape box1 = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    const TopoDS_Shape box2 = BRepPrimAPI_MakeBox(gp_Pnt(5, 5, 5), 10.0, 10.0, 10.0).Shape();

    em::ElementMapPartition part;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(box1, TopAbs_FACE, faces);
    for (int i = 1; i <= faces.Extent(); ++i) {
        part.mint("A", "A_f" + std::to_string(i), km::ElementKind::Face, faces(i), box1);
    }
    const std::size_t minted = part.entries_for_body("A").size();
    check(minted == 6, "mint: 6 faces tracked on body A");

    BRepAlgoAPI_Fuse fuse(box1, box2);
    fuse.Build();
    check(fuse.IsDone(), "fuse built");
    const TopoDS_Shape result = fuse.Shape();

    em::ElementMapDelta delta;
    std::vector<nlohmann::json> needs_repair;
    part.apply_history("A", result, fuse, delta, &needs_repair);

    // Every relabeled entry carries bodyId "A" + a topoKey resolving to a real face
    // in the fused result.
    for (const auto& e : delta.relabeled) {
        check(e.body_id == "A", "relabeled bodyId is A");
        check(e.kind == "face", "relabeled kind face");
        check(!em::ElementMapPartition::shape_for_topokey(result, e.topo_key).IsNull(),
              "relabeled topoKey resolves in the new snapshot");
    }
    // Survivors still resolve; removed are gone.
    for (const em::PartitionEntry* e : part.entries_for_body("A")) {
        check(!em::ElementMapPartition::shape_for_topokey(result, e->topo_key).IsNull(),
              "surviving entry binds to a real face");
    }
    for (const std::string& id : delta.removed) check(!part.contains(id), "removed entry dropped");

    // Accounting: every tracked face survives, is deleted, or explicitly needs repair.
    const std::size_t survivors = part.entries_for_body("A").size();
    check(survivors + delta.removed.size() + needs_repair.size() == 6,
          "all 6 minted faces accounted for");
    // A box fused with an overlapping box modifies (reindexes) at least one face →
    // proves rebind via Modified actually fires.
    check(!delta.relabeled.empty(), "at least one face rebound via Modified (relabeled)");
}

// --- consumed face → removed via IsDeleted (same-footprint taller fuse) ---
void test_removed_via_deleted() {
    const TopoDS_Shape base = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();   // z 0..10
    const TopoDS_Shape tall = BRepPrimAPI_MakeBox(10.0, 10.0, 20.0).Shape();   // z 0..20, same xy

    em::ElementMapPartition part;
    const int top = top_face_index(base);
    part.mint("A", "A_top", km::ElementKind::Face, [&] {
        TopTools_IndexedMapOfShape faces;
        TopExp::MapShapes(base, TopAbs_FACE, faces);
        return faces(top);
    }(), base);

    BRepAlgoAPI_Fuse fuse(base, tall);
    fuse.Build();
    const TopoDS_Shape result = fuse.Shape();

    em::ElementMapDelta delta;
    part.apply_history("A", result, fuse, delta);
    // The base's top face (z=10) becomes an interior seam of the union → consumed.
    bool removed = false;
    for (const std::string& id : delta.removed)
        if (id == "A_top") removed = true;
    check(removed, "consumed top face → delta.removed");
    check(!part.contains("A_top"), "consumed face dropped from partition");
}

// --- tool body entries removed by remove_body ---
void test_remove_body() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(4.0, 4.0, 4.0).Shape();
    em::ElementMapPartition part;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(box, TopAbs_FACE, faces);
    part.mint("B", "B_f1", km::ElementKind::Face, faces(1), box);
    part.mint("B", "B_f2", km::ElementKind::Face, faces(2), box);

    em::ElementMapDelta delta;
    part.remove_body("B", delta);
    check(delta.removed.size() == 2, "remove_body: both tool entries removed");
    check(!part.contains("B_f1") && !part.contains("B_f2"), "tool entries dropped");
}

}  // namespace

int main() {
    test_history_channel_union();
    test_union_ambiguity_and_deletion_precedence();
    test_rebind_via_modified();
    test_removed_via_deleted();
    test_remove_body();
    if (g_failures == 0) std::fprintf(stderr, "wp5_partition_history: OK\n");
    return g_failures;
}
