#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepCheck_Analyzer.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRep_Builder.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRep_Tool.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>

#include "ops/BooleanOp.h"
#include "ops/MirrorOp.h"
#include "ops/ShellOp.h"
#include "ops/ShellOpenFacePatch.h"
#include "ops/TransformOp.h"
#include "session/ShapeMetrics.h"

namespace {
using nlohmann::json;
namespace ops = onecad::ops;
namespace em = onecad::elementmap;
int failures = 0;

void check(bool condition, const std::string& message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message.c_str());
        ++failures;
    }
}

bool succeeded(const ops::OpOutcome& outcome) {
    check(outcome.status == ops::OpOutcome::Status::Ok && outcome.needs_repair.empty(),
          "operation succeeded: " + outcome.error_message);
    return outcome.status == ops::OpOutcome::Status::Ok && outcome.needs_repair.empty();
}

bool mirrored_plate(ops::OpContext& ctx) {
    ctx.bodies.create("plate", "source", BRepPrimAPI_MakeBox(60, 40, 4).Shape());
    if (!succeeded(ops::execute_transform_body(
            ctx, {{"params", {{"targets", {"plate"}}, {"translate", {0, 0, 6}},
                              {"copy", false}}}}, "move"))) return false;
    if (!succeeded(ops::execute_mirror_body(
            ctx, {{"params", {{"sourceBodyId", "plate"}, {"planePoint", {0, 0, 0}},
                              {"planeNormal", {1, 0, 0}}, {"fuseWithOriginal", false}}}},
            "mirror"))) return false;
    return succeeded(ops::execute_boolean(
        ctx, {{"params", {{"targetBodyId", "plate"}, {"toolBodyId", "body_mirror"},
                          {"operation", "Union"}}}}, "union"));
}

void check_opening_history(const em::ElementMapPartition& partition, const TopoDS_Shape& result) {
    for (const std::string id : {"picked", "other_top"}) {
        const auto* entry = partition.find(id);
        check(entry && entry->body_id == "plate", "opening history preserves target ownership");
        if (!entry) continue;
        const auto rebound = em::ElementMapPartition::shape_for_topokey(result, entry->topo_key);
        check(!rebound.IsNull() && rebound.IsEqual(entry->shape),
              "opening history binds a real output face");
        check(std::abs(em::ElementMapPartition::describe(rebound).center.Z() - 10) < 1e-7,
              "opening face history follows the top rim");
    }
}

TopoDS_Face top_half(const TopoDS_Shape& shape, bool left) {
    for (TopExp_Explorer it(shape, TopAbs_FACE); it.More(); it.Next()) {
        const auto descriptor = em::ElementMapPartition::describe(it.Current());
        if (std::abs(descriptor.center.Z() - 10) < 1e-7 &&
            (descriptor.center.X() < 0) == left) return TopoDS::Face(it.Current());
    }
    return {};
}

void test_merged_plate(double thickness, bool left, ops::ValidationMode mode) {
    onecad::session::BodyStore bodies;
    em::ElementMapPartition partition;
    onecad::CancelToken cancel;
    ops::OpContext ctx{bodies, nullptr, partition, nullptr, false, json::object(), &cancel};
    if (!mirrored_plate(ctx)) return;
    const TopoDS_Shape before = bodies.get("plate")->geom;
    check(onecad::session::compute_shape_metrics(before).face_count == 10,
          "Boolean keeps the two top patches and original topology");
    const TopoDS_Face top = top_half(before, left);
    check(!top.IsNull(), "picked top half exists");
    if (top.IsNull()) return;
    partition.mint("plate", "picked", em::km::ElementKind::Face, top, before, json::object());
    partition.mint("plate", "other_top", em::km::ElementKind::Face, top_half(before, !left),
                   before, json::object());
    ctx.validation_mode = mode;
    const auto result = ops::execute_shell(
        ctx, {{"inputs", {{{"primary", {{"bodyId", "plate"}, {"elementId", "picked"},
                                         {"kind", "face"}}}}}},
              {"params", {{"targetBodyId", "plate"}, {"thickness", thickness}}}}, "shell");
    if (!succeeded(result)) return;
    check(result.body_events.size() == 1 && result.body_events[0].kind == "modified" &&
          result.body_ids == std::vector<std::string>{"plate"}, "Shell preserves target identity");
    const auto after = bodies.get("plate")->geom;
    check_opening_history(partition, after);
    const double expected = 19200 - (120 - 2 * thickness) * (40 - 2 * thickness) * (4 - thickness);
    check(std::abs(onecad::session::shape_volume(after) - expected) < 1e-6,
          "Shell opens both top patches: exact tray volume " + std::to_string(expected));
    check(BRepCheck_Analyzer(after).IsValid(), "Shell BRep is valid");
    TopTools_IndexedMapOfShape solids;
    TopExp::MapShapes(after, TopAbs_SOLID, solids);
    check(solids.Extent() == 1, "Shell publishes one solid");
    for (TopExp_Explorer it(after, TopAbs_SHELL); it.More(); it.Next()) {
        check(BRep_Tool::IsClosed(it.Current()), "Shell boundary is closed");
    }
}

TopoDS_Face rectangular_face(double x0, double x1, double z0, double z1,
                             const TopoDS_Edge& shared = {}) {
    const gp_Pnt a(x0, 0, z0), b(x1, 0, z1), c(x1, 1, z1), d(x0, 1, z0);
    BRepBuilderAPI_MakeWire wire;
    if (!shared.IsNull()) wire.Add(shared);
    wire.Add(BRepBuilderAPI_MakeEdge(a, b).Edge());
    wire.Add(BRepBuilderAPI_MakeEdge(b, c).Edge());
    wire.Add(BRepBuilderAPI_MakeEdge(c, d).Edge());
    if (shared.IsNull()) wire.Add(BRepBuilderAPI_MakeEdge(d, a).Edge());
    return BRepBuilderAPI_MakeFace(wire.Wire()).Face();
}

TopoDS_Edge right_edge(const TopoDS_Face& face, double x) {
    for (TopExp_Explorer it(face, TopAbs_EDGE); it.More(); it.Next()) {
        const auto descriptor = em::ElementMapPartition::describe(it.Current());
        if (std::abs(descriptor.center.X() - x) < 1e-9) {
            return TopoDS::Edge(it.Current().Reversed());
        }
    }
    return {};
}

TopoDS_Shape face_compound(const std::vector<TopoDS_Face>& faces) {
    BRep_Builder builder;
    TopoDS_Compound body;
    builder.MakeCompound(body);
    for (const auto& face : faces) builder.Add(body, face);
    return body;
}

void check_patch(const TopoDS_Shape& body, const TopTools_ListOfShape& seeds,
                 int expected, const std::string& message) {
    TopTools_ListOfShape patch;
    check(ops::shell_detail::expand_open_faces(body, seeds, nullptr, patch), message + ": collected");
    check(patch.Extent() == expected, message + ": exact patch size got " +
          std::to_string(patch.Extent()) + " expected " + std::to_string(expected));
    TopTools_IndexedMapOfShape originals;
    TopExp::MapShapes(body, TopAbs_FACE, originals);
    for (TopTools_ListIteratorOfListOfShape it(patch); it.More(); it.Next()) {
        const int index = originals.FindIndex(it.Value());
        check(index && originals(index).IsEqual(it.Value()), message + ": original oriented face");
    }
}

void test_patch_boundaries() {
    const auto seed = rectangular_face(0, 1, 0, 0);
    const auto edge = right_edge(seed, 1);
    check(!edge.IsNull(), "shared edge exists");
    const auto adjacent = rectangular_face(1, 2, 0, 0, edge);
    const auto disconnected = rectangular_face(4, 5, 0, 0);
    const auto body = face_compound({seed, adjacent, disconnected});
    TopTools_ListOfShape seeds;
    seeds.Append(seed);
    check_patch(body, seeds, 2, "edge-connected patch excludes disconnected coplanar face");
    seeds.Append(adjacent);
    seeds.Append(seed);
    check_patch(body, seeds, 2, "overlapping and repeated selections deduplicate");
    seeds.Clear();
    seeds.Append(seed);
    check_patch(face_compound({seed, TopoDS::Face(adjacent.Reversed())}), seeds, 1,
                "opposite outward normal excluded");
    const auto tilted = rectangular_face(1, 2, 0, 1e-5, edge);
    check_patch(face_compound({seed, tilted}), seeds, 1,
                "tilted neighbor excluded below projection-helper angular tolerance");
    const auto first = rectangular_face(1, 2, 0, 6e-8, edge);
    const auto second = rectangular_face(2, 3, 6e-8, 1.8e-7, right_edge(first, 2));
    check_patch(face_compound({seed, first, second}), seeds, 2,
                "angular tolerance never accumulates along neighbors");
    const auto offset = rectangular_face(4, 5, 2e-6, 2e-6);
    const auto precision = onecad::kernel::validation::precision_of(body);
    gp_Pln plane;
    gp_Dir outward;
    check(ops::shell_detail::planar_frame(seed, plane, outward), "seed planar frame");
    check(!ops::shell_detail::matches_seed(offset, plane, outward, precision),
          "parallel plane offset beyond semantic length excluded");
}

void test_nonplanar_selection() {
    const auto cylinder = BRepPrimAPI_MakeCylinder(2, 4).Shape();
    for (TopExp_Explorer it(cylinder, TopAbs_FACE); it.More(); it.Next()) {
        const auto face = TopoDS::Face(it.Current());
        gp_Pln plane;
        gp_Dir outward;
        if (ops::shell_detail::planar_frame(face, plane, outward)) continue;
        TopTools_ListOfShape seeds;
        seeds.Append(face);
        check_patch(cylinder, seeds, 1, "nonplanar selection opens explicit face only");
        return;
    }
    check(false, "nonplanar fixture exists");
}

void test_fail_closed_and_cancellation() {
    const auto seed = rectangular_face(0, 1, 0, 0);
    const auto body = face_compound({seed});
    TopTools_ListOfShape seeds, patch;
    seeds.Append(rectangular_face(0, 1, 0, 0));
    check(!ops::shell_detail::expand_open_faces(body, seeds, nullptr, patch),
          "geometrically identical foreign face refused");
    seeds.Clear();
    seeds.Append(seed.Reversed());
    check(!ops::shell_detail::expand_open_faces(body, seeds, nullptr, patch),
          "reversed replacement of original seed refused");
    seeds.Clear();
    seeds.Append(seed);
    onecad::CancelToken cancel;
    cancel.cancel();
    check(!ops::shell_detail::expand_open_faces(body, seeds, &cancel, patch) && patch.IsEmpty(),
          "cancelled collection produces no partial patch");
}
} // namespace

int main() {
    test_patch_boundaries();
    test_nonplanar_selection();
    test_fail_closed_and_cancellation();
    for (double thickness : {1., 2.}) {
        for (bool left : {false, true}) {
            for (auto mode : {ops::ValidationMode::PreviewInteractive,
                              ops::ValidationMode::CommitAuthoritative}) {
                test_merged_plate(thickness, left, mode);
            }
        }
    }
    return failures ? 1 : 0;
}
