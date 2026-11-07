#include <algorithm>
#include <cstdio>
#include <set>
#include <filesystem>
#include <fstream>
#include <memory>
#include <sstream>
#include <string>
#include <vector>

#include <BRepBuilderAPI_Copy.hxx>
#include <BRepTools.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <gp_Pnt.hxx>
#include <BinTools.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>

#include "elementmap/ElementMapPartition.h"
#include "kernel/validation/GeometryPrecision.h"
#include "ops/BooleanInternals.h"
#include "ops/BooleanOp.h"
#include "ops/BooleanParticipation.h"
#include "ops/OpCommon.h"
#include "session/BodyStore.h"

namespace ops = onecad::ops;
namespace em = onecad::elementmap;
namespace km = onecad::kernel::elementmap;
namespace val = onecad::kernel::validation;
using nlohmann::json;
namespace {
int failures = 0;
void check(bool condition, const std::string& message) {
    if (!condition) { std::fprintf(stderr, "FAIL: %s\n", message.c_str()); ++failures; }
}
std::string bytes(const TopoDS_Shape& shape) {
    std::ostringstream out;
    BRepTools::Write(shape, out);
    return out.str();
}
TopoDS_Shape operand(int index) {
    const auto path = std::filesystem::path(__FILE__).parent_path() / "fixtures" /
                      "union-internals" / ("input-" + std::to_string(index) + ".brep");
    std::ifstream in(path, std::ios::binary);
    TopoDS_Shape shape;
    BinTools::Read(shape, in);
    check(!shape.IsNull(), "fixture is readable: " + path.string());
    return shape;
}
struct Inputs {
    TopoDS_Shape target = operand(0);
    std::vector<TopoDS_Shape> tools;
    Inputs() {
        // Deliberate diagnostic subset, not proof that operand 2 is redundant.
        for (int i : {1, 3, 4, 5, 6, 7, 8, 9}) tools.push_back(operand(i));
    }
};
val::PublicationDecision decision(const Inputs& in, const TopoDS_Shape& result) {
    val::PublicationPolicy policy;
    policy.name = "Boolean";
    policy.allowed_top_level_shapes = val::TopLevelShapePolicy::SolidSet;
    policy.max_solid_count = -1;
    policy.tier = val::PublicationTier::TierB;
    policy.require_closed_manifold = true;
    policy.allow_empty_lifecycle = true;
    auto precision = val::precision_of(in.target);
    double worst = precision.input_tolerance;
    for (const auto& tool : in.tools) worst = std::max(worst, val::precision_of(tool).input_tolerance);
    policy.maximum_tolerance = precision.tolerance_ceiling(worst, 2.0, 1e-6);
    return ops::publication_decision(result, policy);
}
void test_candidate() {
    Inputs in;
    auto target_before = bytes(in.target);
    std::vector<std::string> before;
    for (const auto& tool : in.tools) before.push_back(bytes(tool));
    std::shared_ptr<BRepBuilderAPI_MakeShape> history;
    auto raw = ops::checked_boolean(in.target, in.tools, onecad::app::BooleanMode::Add,
                                   false, json::object(), nullptr, history);
    check(raw.error_code.empty() && history, "raw checked Union builds");
    if (!history) return;
    auto raw_shape = raw.shape;
    auto raw_before = bytes(raw_shape);
    auto original = decision(in, raw_shape);
    check(original.reason_code == "PUBLICATION_OPEN_MANIFOLD" && original.evidence.open_edge_count == 5,
          "subset needs auxiliary topology normalization");
    check(!ops::first_tool_without_effect(*history, raw_shape, in.target, in.tools), "all raw tools contribute");
    check(ops::union_anchoring(*history, raw_shape, in.target, in.tools).unanchored_solids == 0,
          "raw result is target anchored");
    check(ops::normalize_boolean_internals(in.target, raw.shape, history, nullptr, in.tools), "normalize subset");
    auto final = decision(in, raw.shape);
    check(final.publishable() && final.evidence.open_edge_count == 0 && final.evidence.self_interference_count == 0,
          "candidate passes unchanged Boolean policy: " + final.to_json().dump());
    check(!ops::first_tool_without_effect(*history, raw.shape, in.target, in.tools), "all copied tools contribute");
    check(ops::union_anchoring(*history, raw.shape, in.target, in.tools).unanchored_solids == 0,
          "copied result remains target anchored");
    check(bytes(raw_shape) == raw_before && bytes(in.target) == target_before, "target and raw unchanged");
    for (std::size_t i = 0; i < in.tools.size(); ++i) check(bytes(in.tools[i]) == before[i], "tool unchanged");
}
struct Fixture {
    onecad::session::BodyStore bodies;
    em::ElementMapPartition partition;
    std::vector<std::pair<std::string, json>> sketches;
    std::string last;
    onecad::CancelToken cancel;
    std::vector<em::PartitionEntry> target_refs;
    std::vector<em::PartitionEntry> tool_refs;
    void seed(const std::string& id, const TopoDS_Shape& shape) {
        bodies.create(id, "seed_" + id, shape);
        for (const auto& [kind, type] : std::vector<std::pair<km::ElementKind, TopAbs_ShapeEnum>>{
                 {km::ElementKind::Face, TopAbs_FACE}, {km::ElementKind::Edge, TopAbs_EDGE},
                 {km::ElementKind::Vertex, TopAbs_VERTEX}}) {
            TopTools_IndexedMapOfShape shapes;
            TopExp::MapShapes(shape, type, shapes);
            for (int i = 1; i <= shapes.Extent(); ++i) {
                auto key = id + "-" + std::to_string(static_cast<int>(kind)) + "-" + std::to_string(i);
                partition.mint(id, key, kind, shapes(i), shape);
                if (id == "target") target_refs.push_back(*partition.find(key));
                else tool_refs.push_back(*partition.find(key));
            }
        }
    }
    ops::OpOutcome run(bool keep) {
        std::vector<std::string> tools;
        for (int i = 0; i < 8; ++i) tools.push_back("tool-" + std::to_string(i));
        json op = {{"params", {{"operation", "Union"}, {"targetBodyId", "target"},
                               {"toolBodyIds", tools}, {"keepTools", keep}}}};
        ops::OpContext ctx{bodies, &sketches, partition, &last, false, json::object(), &cancel};
        return ops::execute_boolean(ctx, op, "subset-union");
    }
};
struct Expected {
    TopoDS_Shape result;
    std::shared_ptr<BRepBuilderAPI_MakeShape> raw;
    std::shared_ptr<BRepBuilderAPI_MakeShape> composed;
    Expected(const Inputs& in, bool keep) {
        std::vector<TopoDS_Shape> tools;
        for (const auto& tool : in.tools) tools.push_back(keep ? BRepBuilderAPI_Copy(tool).Shape() : tool);
        auto built = ops::checked_boolean(in.target, tools, onecad::app::BooleanMode::Add,
                                         false, json::object(), nullptr, raw);
        result = built.shape;
        composed = raw;
        check(ops::normalize_boolean_internals(in.target, result, composed, nullptr, tools),
              "independent production-equivalent history is normalized");
    }
};
std::set<std::string> successor_keys(Expected& expected, const em::PartitionEntry& source) {
    std::set<std::string> keys;
    auto collect = [&](const TopTools_ListOfShape& images) {
        for (const auto& image : images) {
            auto key = em::ElementMapPartition::topokey_for_shape(expected.result, image, source.kind);
            if (!key.empty()) keys.insert(key);
        }
    };
    collect(expected.composed->Modified(source.shape));
    collect(expected.composed->Generated(source.shape));
    return keys;
}
void target_lineage(const Fixture& f, const ops::OpOutcome& out, Expected& expected) {
    check(bytes(f.bodies.get("target")->geom) == bytes(expected.result),
          "actual topology/geometry serialization matches independent copied-tool history replay");
    int bound = 0;
    for (const auto& source : f.target_refs) {
        const auto& id = source.element_id;
        const auto* entry = f.partition.find(id);
        auto keys = successor_keys(expected, source);
        if (entry) {
            auto resolved = em::ElementMapPartition::shape_for_topokey(f.bodies.get("target")->geom, entry->topo_key);
            check(entry->body_id == "target" && !resolved.IsNull() && resolved.IsSame(entry->shape), "reference belongs to final target");
            check(keys.contains(entry->topo_key), "bound key is a composed successor of this exact source");
            ++bound;
            continue;
        }
        bool removed = std::find(out.delta.removed.begin(), out.delta.removed.end(), id) != out.delta.removed.end();
        bool repair = std::any_of(out.needs_repair.begin(), out.needs_repair.end(), [&](const json& item) {
            return item.value("elementId", "") == id;
        });
        check(removed || repair, "unbound reference explicitly removed or NeedsRepair");
        if (removed) check(keys.empty() && expected.raw->IsDeleted(source.shape) &&
                               expected.raw->Modified(source.shape).IsEmpty() &&
                               expected.raw->Generated(source.shape).IsEmpty(),
                           "removal has positive raw deletion evidence and no successor");
    }
    check(bound > 0, "target lineage retains nonvacuous references");
}
void unchanged_reference(const Fixture& f, const em::PartitionEntry& before) {
    const auto* after = f.partition.find(before.element_id);
    check(after && after->body_id == before.body_id && after->kind == before.kind &&
              after->topo_key == before.topo_key && after->shape.IsEqual(before.shape) &&
              after->anchor == before.anchor &&
              em::ElementMapPartition::descriptor_to_json(after->descriptor) ==
                  em::ElementMapPartition::descriptor_to_json(before.descriptor),
          "unaffected reference preserves identity, orientation, descriptor and anchor");
}
void test_execute(bool keep, bool cancelled) {
    Inputs in;
    Fixture f;
    f.seed("target", in.target);
    std::vector<std::string> before;
    for (std::size_t i = 0; i < in.tools.size(); ++i) {
        f.seed("tool-" + std::to_string(i), in.tools[i]);
        before.push_back(bytes(in.tools[i]));
    }
    auto target_before = bytes(in.target);
    auto partition_size = f.partition.size();
    std::unique_ptr<Expected> expected;
    if (!cancelled) expected = std::make_unique<Expected>(in, keep);
    if (cancelled) f.cancel.cancel();
    auto out = f.run(keep);
    if (cancelled) {
        check(out.status == ops::OpOutcome::Status::Cancelled, "cancelled operation is not refusal");
        check(out.body_events.empty() && out.delta.empty() && out.needs_repair.empty(), "cancel emits no mutation");
        check(f.partition.size() == partition_size && bytes(f.bodies.get("target")->geom) == target_before,
              "cancel preserves target and partition");
    } else {
        check(out.status == ops::OpOutcome::Status::Ok, "actual Union publishes: " + json(out.diagnostics).dump());
        if (out.status != ops::OpOutcome::Status::Ok) return;
        check(f.bodies.contains("target") && decision(in, f.bodies.get("target")->geom).publishable(), "target identity and valid result");
        target_lineage(f, out, *expected);
    }
    if (cancelled) for (const auto& entry : f.target_refs) unchanged_reference(f, entry);
    if (keep || cancelled) for (const auto& entry : f.tool_refs) unchanged_reference(f, entry);
    for (std::size_t i = 0; i < in.tools.size(); ++i) {
        auto id = "tool-" + std::to_string(i);
        check(f.bodies.contains(id) == (keep || cancelled), "tool lifetime follows keep/cancel");
        if (keep || cancelled) {
            check(bytes(f.bodies.get(id)->geom) == before[i], "retained tool is unchanged");
            check(!f.partition.entries_for_body(id).empty(), "retained tool keeps references");
        } else check(f.partition.entries_for_body(id).empty(), "consumed tool references retire");
    }
}
class CancellingHistory final : public BRepBuilderAPI_MakeShape {
public:
    CancellingHistory(std::shared_ptr<BRepBuilderAPI_MakeShape> base, onecad::CancelToken& cancel)
        : base_(std::move(base)), cancel_(cancel) {}
    const TopTools_ListOfShape& Modified(const TopoDS_Shape& source) override {
        cancel_.cancel();
        return base_->Modified(source);
    }
    const TopTools_ListOfShape& Generated(const TopoDS_Shape& source) override { return base_->Generated(source); }
    Standard_Boolean IsDeleted(const TopoDS_Shape& source) override { return base_->IsDeleted(source); }
private:
    std::shared_ptr<BRepBuilderAPI_MakeShape> base_;
    onecad::CancelToken& cancel_;
};
void test_cancel_during_history() {
    Inputs in;
    std::shared_ptr<BRepBuilderAPI_MakeShape> base;
    auto raw = ops::checked_boolean(in.target, in.tools, onecad::app::BooleanMode::Add,
                                   false, json::object(), nullptr, base);
    onecad::CancelToken cancel;
    std::shared_ptr<BRepBuilderAPI_MakeShape> history = std::make_shared<CancellingHistory>(base, cancel);
    auto original_history = history;
    auto before = bytes(raw.shape);
    check(!ops::normalize_boolean_internals(in.target, raw.shape, history, &cancel, in.tools),
          "cancellation during lineage construction refuses candidate adoption");
    check(cancel.cancelled() && history == original_history && bytes(raw.shape) == before,
          "late cancellation preserves raw shape and history owner");
}
void test_accepted_raw_parity() {
    Fixture f;
    auto target = BRepPrimAPI_MakeBox(20., 10., 10.).Shape();
    f.seed("target", target);
    std::vector<TopoDS_Shape> tools;
    for (int i = 0; i < 8; ++i) {
        auto tool = BRepPrimAPI_MakeBox(gp_Pnt(i * 2., 0., 8.), 1., 1., 4.).Shape();
        f.seed("tool-" + std::to_string(i), tool);
        tools.push_back(tool);
    }
    std::shared_ptr<BRepBuilderAPI_MakeShape> history;
    auto raw = ops::checked_boolean(target, tools, onecad::app::BooleanMode::Add,
                                   false, json::object(), nullptr, history);
    auto out = f.run(false);
    check(out.status == ops::OpOutcome::Status::Ok, "ordinary raw Union still publishes");
    check(bytes(raw.shape) == bytes(f.bodies.get("target")->geom),
          "already accepted raw Union is byte-identical; normalization not introduced");
}
} // namespace
int main() {
    test_candidate();
    test_cancel_during_history();
    test_accepted_raw_parity();
    test_execute(false, false);
    test_execute(true, false);
    test_execute(false, true);
    test_execute(true, true);
    if (!failures) std::fprintf(stderr, "union_internals: OK\n");
    return failures;
}
