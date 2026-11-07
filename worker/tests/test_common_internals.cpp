#include <algorithm>
#include <cmath>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <cstdlib>
#include <memory>
#include <sstream>
#include <string>
#include <utility>
#include <vector>

#include <BRep_Builder.hxx>
#include <BRepBndLib.hxx>
#include <BRepClass3d_SolidClassifier.hxx>
#include <Bnd_Box.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepTools.hxx>
#include <BinTools.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Iterator.hxx>
#include <TopoDS_Wire.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>
#include <gp_Pnt.hxx>

#include "elementmap/ElementMapPartition.h"
#include "kernel/validation/GeometryPrecision.h"
#include "kernel/validation/ShapeAudit.h"
#include "nlohmann/json.hpp"
#include "ops/BooleanOp.h"
#include "ops/CommonInternals.h"
#include "ops/OpCommon.h"
#include "session/BodyStore.h"
#include "session/ShapeMetrics.h"
#include "util/Cancel.h"

namespace ops = onecad::ops;
namespace val = onecad::kernel::validation;
using nlohmann::json;
namespace {
int failures = 0;
void check(bool value, const char* message) {
    if (!value) { std::fprintf(stderr, "FAIL: %s\n", message); ++failures; }
}

TopoDS_Shape read_fixture(const char* name) {
    const auto path = std::filesystem::path(__FILE__).parent_path() / "fixtures" / "common-internals" / name;
    std::ifstream input(path, std::ios::binary);
    if (!input) {
        std::fprintf(stderr, "FAIL: cannot open fixture %s\n", path.string().c_str());
        std::exit(1);
    }
    TopoDS_Shape shape;
    BinTools::Read(shape, input);
    if (shape.IsNull()) {
        std::fprintf(stderr, "FAIL: invalid fixture %s\n", path.string().c_str());
        std::exit(1);
    }
    return shape;
}

std::string serialize(const TopoDS_Shape& shape) {
    std::ostringstream out;
    BRepTools::Write(shape, out);
    return out.str();
}

struct Common {
    TopoDS_Shape target;
    TopoDS_Shape tool;
    std::shared_ptr<BRepBuilderAPI_MakeShape> history;
    ops::BooleanResult result;
    Common() : Common(read_fixture("target.brep"), read_fixture("tool.brep")) {}
    Common(TopoDS_Shape a, TopoDS_Shape b)
        : target(std::move(a)), tool(std::move(b)),
          result(ops::checked_boolean(target, tool, onecad::app::BooleanMode::Intersect,
                                      false, json::object(), nullptr, history)) {
        check(result.error_code.empty() && !result.shape.IsNull() && history, "raw Common succeeds");
    }
};

val::PublicationDecision strict(const Common& common, const TopoDS_Shape& result) {
    auto policy = val::single_solid_policy("Boolean", val::PublicationTier::TierB);
    policy.allowed_top_level_shapes = val::TopLevelShapePolicy::SolidSet;
    policy.require_closed_manifold = true;
    const auto precision = val::precision_of(common.target);
    const double worst = std::max(precision.input_tolerance, val::precision_of(common.tool).input_tolerance);
    policy.maximum_tolerance = precision.tolerance_ceiling(worst, 2.0, 1.0e-6);
    return ops::publication_decision(result, policy, nullptr);
}

void check_history(Common& common) {
    TopTools_IndexedMapOfShape final_shapes;
    TopExp::MapShapes(common.result.shape, final_shapes);
    int images = 0;
    for (const auto kind : {TopAbs_FACE, TopAbs_EDGE}) {
        for (TopExp_Explorer it(common.target, kind); it.More(); it.Next()) {
            const TopoDS_Shape source = it.Current();
            for (const auto& image : common.history->Modified(source)) {
                ++images;
                check(final_shapes.Contains(image), "modified image belongs to final result");
            }
            for (const auto& image : common.history->Generated(source)) {
                ++images;
                check(final_shapes.Contains(image), "generated image belongs to final result");
            }
        }
    }
    check(images > 0, "history retains at least one predecessor image");
    const TopoDS_Shape unrelated = BRepBuilderAPI_MakeEdge(gp_Pnt(0, 0, 0), gp_Pnt(1, 0, 0)).Shape();
    check(common.history->Modified(unrelated).IsEmpty(), "unknown has no modified images");
    check(common.history->Generated(unrelated).IsEmpty(), "unknown has no generated images");
    check(!common.history->IsDeleted(unrelated), "unknown is not asserted deleted");
}

void check_material_grid(const TopoDS_Shape& raw, const TopoDS_Shape& normalized) {
    TopExp_Explorer raw_solids(raw, TopAbs_SOLID);
    TopExp_Explorer final_solids(normalized, TopAbs_SOLID);
    check(raw_solids.More() && final_solids.More(), "material grid has two solids");
    if (!raw_solids.More() || !final_solids.More()) return;
    Bnd_Box bounds;
    BRepBndLib::Add(raw, bounds);
    double xmin, ymin, zmin, xmax, ymax, zmax;
    bounds.Get(xmin, ymin, zmin, xmax, ymax, zmax);
    BRepClass3d_SolidClassifier before(raw_solids.Current());
    BRepClass3d_SolidClassifier after(final_solids.Current());
    int checked = 0;
    int inside = 0;
    int outside = 0;
    for (int x = 0; x < 9; ++x) for (int y = 0; y < 9; ++y) for (int z = 0; z < 9; ++z) {
        const gp_Pnt point(xmin + (x + 0.371) * (xmax - xmin) / 9,
                           ymin + (y + 0.371) * (ymax - ymin) / 9,
                           zmin + (z + 0.371) * (zmax - zmin) / 9);
        before.Perform(point, 1e-7);
        after.Perform(point, 1e-7);
        check(before.State() == after.State(), "material classification retained on fixed grid");
        inside += before.State() == TopAbs_IN;
        outside += before.State() == TopAbs_OUT;
        ++checked;
    }
    check(checked == 729 && inside > 0 && outside > 0, "material grid covers inside and outside");
}

void test_cancelled_normalization() {
    Common common;
    const std::string before = serialize(common.result.shape);
    const auto history = common.history;
    onecad::CancelToken cancel;
    cancel.cancel();
    check(!ops::normalize_common_internals(common.target, common.result.shape, common.history, &cancel),
          "cancelled normalization refused");
    check(serialize(common.result.shape) == before && common.history == history,
          "cancelled normalization retains shape and history");
}

void test_raw_normalization() {
    Common common;
    const std::string target_before = serialize(common.target);
    const std::string tool_before = serialize(common.tool);
    const TopoDS_Shape raw = common.result.shape;
    const std::string raw_before = serialize(raw);
    const auto initial = strict(common, raw);
    check(!initial.publishable() && initial.evidence.open_edge_count == 3 &&
              initial.evidence.self_interference_count == 0, "raw Common has three open edges and no self interference");
    const double volume = onecad::session::shape_volume(raw);
    check(ops::normalize_common_internals(common.target, common.result.shape, common.history), "normalize Common");
    const auto final = strict(common, common.result.shape);
    check(final.publishable() && final.evidence.solid_count == 1 && final.evidence.open_edge_count == 0 &&
              final.evidence.self_interference_count == 0, "normalized Common passes unchanged strict policy");
    check(std::abs(onecad::session::shape_volume(common.result.shape) - volume) < 1e-9, "normalization preserves material volume");
    check(serialize(raw) == raw_before, "raw result unchanged");
    check(serialize(common.target) == target_before && serialize(common.tool) == tool_before, "live inputs unchanged");
    TopTools_IndexedMapOfShape inputs, output;
    TopExp::MapShapes(common.target, inputs);
    TopExp::MapShapes(common.tool, inputs);
    TopExp::MapShapes(common.result.shape, output);
    for (int i = 1; i <= output.Extent(); ++i) check(!inputs.Contains(output(i)), "final topology detached from inputs");
    check_material_grid(raw, common.result.shape);
    check_history(common);
}

void test_repeatable() {
    Common a;
    Common b;
    check(ops::normalize_common_internals(a.target, a.result.shape, a.history), "first normalization");
    check(ops::normalize_common_internals(b.target, b.result.shape, b.history), "second normalization");
    check(serialize(a.result.shape) == serialize(b.result.shape), "deterministic final BRep");
    const double volume = onecad::session::shape_volume(a.result.shape);
    check(std::abs(volume - 0.025549018062697226) < 1e-9, "captured Common material regression");
}

std::shared_ptr<BRepBuilderAPI_MakeShape> box_history() {
    return std::make_shared<BRepPrimAPI_MakeBox>(1.0, 1.0, 1.0);
}

void test_located_and_reversed() {
    Common common;
    gp_Trsf translation;
    translation.SetTranslation(gp_Vec(11.0, -7.0, 3.0));
    for (bool reversed : {false, true}) {
        TopoDS_Shape transformed = BRepBuilderAPI_Transform(common.result.shape, translation, true).Shape();
        if (reversed) transformed.Reverse();
        const std::string before = serialize(transformed);
        const double volume = onecad::session::shape_volume(transformed);
        TopoDS_Shape retained = transformed;
        auto history = common.history;
        check(ops::normalize_common_internals(common.target, transformed, history),
              "located or reversed raw result normalizes");
        check(serialize(retained) == before, "located or reversed predecessor remains unchanged");
        check(std::abs(onecad::session::shape_volume(transformed) - volume) < 1e-9,
              "located or reversed material volume preserved");
    }
}

void test_refusals() {
    auto history = box_history();
    TopoDS_Shape box = history->Shape();
    const auto original_history = history;
    const std::string original = serialize(box);
    check(!ops::normalize_common_internals(box, box, history), "no internals unchanged");
    check(serialize(box) == original && history == original_history, "no-op retains shape and history");
    for (const auto orientation : {TopAbs_INTERNAL, TopAbs_EXTERNAL}) {
        BRep_Builder builder;
        TopoDS_Compound compound;
        builder.MakeCompound(compound);
        builder.Add(compound, box);
        TopExp_Explorer faces(box, TopAbs_FACE);
        TopoDS_Shape face = faces.Current();
        face.Orientation(orientation);
        builder.Add(compound, face);
        TopoDS_Shape result = compound;
        const std::string before = serialize(result);
        auto candidate_history = box_history();
        const auto saved_history = candidate_history;
        check(!ops::normalize_common_internals(box, result, candidate_history), "unsupported orientation refused");
        check(serialize(result) == before && candidate_history == saved_history, "refusal retains shape and history");
    }
}

void test_shared_edge_occurrences() {
    auto history = box_history();
    const TopoDS_Shape box = history->Shape();
    TopExp_Explorer edge(box, TopAbs_EDGE);
    TopoDS_Shape internal = edge.Current();
    internal.Orientation(TopAbs_INTERNAL);
    BRep_Builder builder;
    TopoDS_Compound compound;
    builder.MakeCompound(compound);
    builder.Add(compound, box);
    TopoDS_Wire auxiliary;
    builder.MakeWire(auxiliary);
    builder.Add(auxiliary, internal);
    builder.Add(compound, auxiliary);
    TopoDS_Shape result = compound;
    TopTools_IndexedMapOfShape before, after;
    TopExp::MapShapes(box, TopAbs_EDGE, before);
    check(ops::normalize_common_internals(box, result, history), "remove duplicate internal use");
    TopExp::MapShapes(result, TopAbs_EDGE, after);
    check(before.Extent() == after.Extent(), "external edge use survives");
    check(std::abs(onecad::session::shape_volume(result) - 1.0) < 1e-9, "external boundary survives");
}

struct Fixture {
    onecad::session::BodyStore bodies;
    onecad::elementmap::ElementMapPartition partition;
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;
    ops::OpContext context() {
        return {bodies, &sketches, partition, &last_sketch, false, json::object(), &cancel};
    }
};

void test_execute(bool keep, bool cancelled) {
    Fixture fixture;
    const TopoDS_Shape target = read_fixture("target.brep");
    const TopoDS_Shape tool = read_fixture("tool.brep");
    const std::string target_bytes = serialize(target);
    const std::string tool_bytes = serialize(tool);
    fixture.bodies.create("target", "seed_target", target);
    fixture.bodies.create("tool", "seed_tool", tool);
    if (cancelled) fixture.cancel.cancel();
    const json op = {{"opType", "Boolean"}, {"opId", "common"},
                     {"params", {{"operation", "Intersect"}, {"targetBodyId", "target"},
                                 {"toolBodyIds", {"tool"}}, {"keepTools", keep}}}};
    auto context = fixture.context();
    const auto outcome = ops::execute_boolean(context, op, "common");
    check(fixture.bodies.contains("target"), "target identity preserved");
    if (cancelled) {
        check(outcome.status == ops::OpOutcome::Status::Cancelled, "pre-cancelled operation cancelled");
        check(fixture.bodies.contains("tool"), "cancel retains tool");
        check(serialize(fixture.bodies.get("target")->geom) == target_bytes &&
                  serialize(fixture.bodies.get("tool")->geom) == tool_bytes, "cancel retains inputs");
    } else {
        check(outcome.status == ops::OpOutcome::Status::Ok, "fixture Common publishes");
        check(fixture.bodies.contains("tool") == keep, "tool keep/consume policy");
        if (keep) check(serialize(fixture.bodies.get("tool")->geom) == tool_bytes, "kept tool unchanged");
        check(std::abs(onecad::session::shape_volume(fixture.bodies.get("target")->geom) -
                       0.025549018062697226) < 1e-9, "published material regression");
    }
}
struct Tracked {
    std::string id;
    TopoDS_Shape source;
    onecad::kernel::elementmap::ElementKind kind;
};

std::vector<Tracked> mint_target(Fixture& fixture, const TopoDS_Shape& target) {
    std::vector<Tracked> tracked;
    for (const auto kind : {TopAbs_FACE, TopAbs_EDGE}) {
        TopTools_IndexedMapOfShape shapes;
        TopExp::MapShapes(target, kind, shapes);
        const auto element_kind = kind == TopAbs_FACE ? onecad::kernel::elementmap::ElementKind::Face
                                                      : onecad::kernel::elementmap::ElementKind::Edge;
        for (int i = 1; i <= shapes.Extent(); ++i) {
            const std::string id = (kind == TopAbs_FACE ? "face-" : "edge-") + std::to_string(i);
            fixture.partition.mint("target", id, element_kind, shapes(i), target);
            tracked.push_back({id, shapes(i), element_kind});
        }
    }
    return tracked;
}

void verify_tracked(const Tracked& tracked, const Fixture& fixture, const ops::OpOutcome& outcome,
                    Common& reference, BRepBuilderAPI_MakeShape& raw_history, int& survivors) {
    const auto* entry = fixture.partition.find(tracked.id);
    const bool removed = std::find(outcome.delta.removed.begin(), outcome.delta.removed.end(), tracked.id) != outcome.delta.removed.end();
    const bool repair = std::any_of(outcome.needs_repair.begin(), outcome.needs_repair.end(),
                                    [&](const json& item) { return item.value("elementId", "") == tracked.id; });
    if (!entry) {
        if (removed) {
            check(raw_history.IsDeleted(tracked.source) && raw_history.Modified(tracked.source).IsEmpty() &&
                      raw_history.Generated(tracked.source).IsEmpty(), "removed reference has positive raw deletion");
        } else {
            check(repair, "unresolved reference requires repair");
        }
        return;
    }
    ++survivors;
    check(!removed && !repair, "surviving reference is not removed or repaired");
    const auto& final = fixture.bodies.get("target")->geom;
    const auto resolved = onecad::elementmap::ElementMapPartition::shape_for_topokey(final, entry->topo_key);
    check(!resolved.IsNull() && resolved.IsSame(entry->shape), "tracked image belongs to actual final shape");
    bool supported = onecad::elementmap::ElementMapPartition::topokey_for_shape(
                         reference.result.shape, tracked.source, tracked.kind) == entry->topo_key;
    for (const auto& image : reference.history->Modified(tracked.source))
        supported |= onecad::elementmap::ElementMapPartition::topokey_for_shape(reference.result.shape, image, tracked.kind) == entry->topo_key;
    for (const auto& image : reference.history->Generated(tracked.source))
        supported |= onecad::elementmap::ElementMapPartition::topokey_for_shape(reference.result.shape, image, tracked.kind) == entry->topo_key;
    check(supported, "tracked image is supported by composed raw Common lineage");
}

void test_tracked_execute(Common& reference, bool normalize) {
    auto raw_history = reference.history;
    const bool changed = ops::normalize_common_internals(reference.target, reference.result.shape, reference.history);
    check(changed == normalize, "expected normalization policy");
    Fixture fixture;
    fixture.bodies.create("target", "seed_target", reference.target);
    fixture.bodies.create("tool", "seed_tool", reference.tool);
    const auto tracked = mint_target(fixture, reference.target);
    const json op = {{"opType", "Boolean"}, {"opId", "tracked-common"},
                     {"params", {{"operation", "Intersect"}, {"targetBodyId", "target"}, {"toolBodyId", "tool"}}}};
    auto context = fixture.context();
    const auto outcome = ops::execute_boolean(context, op, "tracked-common");
    check(outcome.status == ops::OpOutcome::Status::Ok, "tracked Common publishes");
    if (outcome.status != ops::OpOutcome::Status::Ok) return;
    check(serialize(fixture.bodies.get("target")->geom) == serialize(reference.result.shape), "reference and actual final boundaries match");
    int face_survivors = 0, edge_survivors = 0;
    int face_modified = 0, face_generated = 0, edge_modified = 0, edge_generated = 0;
    for (const auto& entry : tracked) {
        const bool face = entry.kind == onecad::kernel::elementmap::ElementKind::Face;
        (face ? face_modified : edge_modified) += raw_history->Modified(entry.source).Size();
        (face ? face_generated : edge_generated) += raw_history->Generated(entry.source).Size();
        int& count = entry.kind == onecad::kernel::elementmap::ElementKind::Face ? face_survivors : edge_survivors;
        verify_tracked(entry, fixture, outcome, reference, *raw_history, count);
    }
    std::fprintf(stderr, "%s: raw face modified=%d generated=%d edge modified=%d generated=%d; "
                         "surviving face=%d edge=%d removed=%zu repair=%zu\n",
                 normalize ? "captured" : "analytic", face_modified, face_generated,
                 edge_modified, edge_generated, face_survivors, edge_survivors,
                 outcome.delta.removed.size(), outcome.needs_repair.size());
    check(face_modified + face_generated + edge_modified + edge_generated > 0, "raw lineage is nonempty");
    if (!normalize) check(face_survivors > 0 && edge_survivors > 0, "analytic face and edge lineage survive");
}

void test_tracked_cases() {
    Common captured;
    test_tracked_execute(captured, true);
    Common boxes(BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape(),
                 BRepPrimAPI_MakeBox(gp_Pnt(5.0, 0.0, 0.0), 10.0, 10.0, 10.0).Shape());
    check(strict(boxes, boxes.result.shape).publishable(), "analytic Common is already publishable");
    check(std::abs(onecad::session::shape_volume(boxes.result.shape) - 500.0) < 1e-9,
          "analytic overlap has independently known volume");
    test_tracked_execute(boxes, false);
}
}  // namespace

int main() {
    test_raw_normalization();
    test_cancelled_normalization();
    test_repeatable();
    test_located_and_reversed();
    test_refusals();
    test_shared_edge_occurrences();
    test_execute(false, false);
    test_execute(true, false);
    test_execute(false, true);
    test_tracked_cases();
    if (failures == 0) std::fprintf(stderr, "common_internals: OK\n");
    return failures;
}
