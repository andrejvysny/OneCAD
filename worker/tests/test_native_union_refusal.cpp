#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <memory>
#include <sstream>
#include <string>
#include <utility>
#include <vector>

#include <BRepTools.hxx>
#include <BinTools.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Shape.hxx>

#include "elementmap/ElementMapPartition.h"
#include "kernel/validation/GeometryPrecision.h"
#include "kernel/validation/ShapeAudit.h"
#include "nlohmann/json.hpp"
#include "ops/BooleanOp.h"
#include "ops/BooleanParticipation.h"
#include "ops/OpCommon.h"
#include "session/BodyStore.h"
#include "util/Cancel.h"

using nlohmann::json;
namespace ops = onecad::ops;
namespace val = onecad::kernel::validation;
namespace em = onecad::elementmap;
namespace km = onecad::kernel::elementmap;
namespace {
int failures = 0;
void check(bool condition, const std::string& message) {
    if (!condition) { std::fprintf(stderr, "FAIL: %s\n", message.c_str()); ++failures; }
}

TopoDS_Shape read_fixture(int index) {
    const auto path = std::filesystem::path(__FILE__).parent_path() / "fixtures" / "union-internals" /
                      ("input-" + std::to_string(index) + ".brep");
    std::ifstream stream(path, std::ios::binary);
    if (!stream) { std::fprintf(stderr, "FAIL: missing %s\n", path.string().c_str()); std::exit(1); }
    TopoDS_Shape shape;
    BinTools::Read(shape, stream);
    if (shape.IsNull()) { std::fprintf(stderr, "FAIL: invalid %s\n", path.string().c_str()); std::exit(1); }
    return shape;
}

std::string serialize(const TopoDS_Shape& shape) {
    std::ostringstream output;
    BRepTools::Write(shape, output);
    return output.str();
}

struct Union {
    TopoDS_Shape target = read_fixture(0);
    std::vector<TopoDS_Shape> tools;
    std::shared_ptr<BRepBuilderAPI_MakeShape> history;
    ops::BooleanResult result;
    Union() {
        for (int i = 1; i <= 9; ++i) tools.push_back(read_fixture(i));
        result = ops::checked_boolean(target, tools, onecad::app::BooleanMode::Add,
                                      false, json::object(), nullptr, history);
        if (!result.error_code.empty() || result.shape.IsNull() || !history) {
            std::fprintf(stderr, "FAIL: checked Union: %s %s\n", result.error_code.c_str(), result.error_message.c_str());
            std::exit(1);
        }
    }
};

val::PublicationDecision strict(const Union& input, const TopoDS_Shape& result) {
    val::PublicationPolicy policy;
    policy.name = "Boolean";
    policy.tier = val::PublicationTier::TierB;
    policy.allowed_top_level_shapes = val::TopLevelShapePolicy::SolidSet;
    policy.max_solid_count = -1;
    policy.require_closed_manifold = true;
    policy.allow_empty_lifecycle = true;
    const auto precision = val::precision_of(input.target);
    double worst = precision.input_tolerance;
    for (const auto& tool : input.tools) worst = std::max(worst, val::precision_of(tool).input_tolerance);
    policy.maximum_tolerance = precision.tolerance_ceiling(worst, 2.0, 1.0e-6);
    return ops::publication_decision(result, policy, nullptr);
}

void test_raw_union() {
    Union input;
    const auto before = serialize(input.result.shape);
    const auto decision = strict(input, input.result.shape);
    const auto idle = ops::first_tool_without_effect(*input.history, input.result.shape,
                                                    input.target, input.tools);
    check(idle && *idle == 1, "raw participation identifies tool-2");
    check(serialize(input.result.shape) == before, "participation query leaves raw result unchanged");
    check(!decision.publishable() && decision.reason_code == "PUBLICATION_OPEN_MANIFOLD" &&
              decision.evidence.open_edge_count == 6 && decision.evidence.self_interference_count == 1,
          "raw Union independently fails unchanged Boolean publication policy: " + decision.to_json().dump());
}

struct Fixture {
    onecad::session::BodyStore bodies;
    em::ElementMapPartition partition;
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;

    void add(const std::string& id, const TopoDS_Shape& shape) {
        bodies.create(id, "seed_" + id, shape);
        TopTools_IndexedMapOfShape faces;
        TopExp::MapShapes(shape, TopAbs_FACE, faces);
        if (faces.IsEmpty()) { std::fprintf(stderr, "FAIL: %s has no faces\n", id.c_str()); std::exit(1); }
        partition.mint(id, "tracked_" + id, km::ElementKind::Face, faces(1), shape);
    }

    ops::OpOutcome run(bool keep) {
        std::vector<std::string> ids;
        for (int i = 1; i <= 9; ++i) ids.push_back("tool-" + std::to_string(i));
        const json params = {{"operation", "Union"}, {"targetBodyId", "target"},
                             {"toolBodyIds", ids}, {"keepTools", keep}};
        const json op = {{"opType", "Boolean"}, {"opId", "native-union"}, {"params", params}};
        ops::OpContext context{bodies, &sketches, partition, &last_sketch, false,
                               json::object(), &cancel};
        return ops::execute_boolean(context, op, "native-union");
    }
};

void test_execute(bool keep) {
    Union input;
    Fixture fixture;
    fixture.add("target", input.target);
    for (std::size_t i = 0; i < input.tools.size(); ++i)
        fixture.add("tool-" + std::to_string(i + 1), input.tools[i]);
    std::vector<std::string> ids{"target"};
    for (std::size_t i = 0; i < input.tools.size(); ++i) ids.push_back("tool-" + std::to_string(i + 1));
    std::vector<std::string> geometry, keys;
    std::vector<TopoDS_Shape> bound;
    for (const auto& id : ids) {
        geometry.push_back(serialize(fixture.bodies.get(id)->geom));
        const auto* entry = fixture.partition.find("tracked_" + id);
        keys.push_back(entry->topo_key);
        bound.push_back(entry->shape);
    }
    const auto outcome = fixture.run(keep);
    const auto diagnostics = json(outcome.diagnostics);
    check(outcome.status == ops::OpOutcome::Status::Failed && outcome.error_code == "OP_FAILED",
          "array Union is refused: " + diagnostics.dump());
    check(!outcome.diagnostics.empty(), "refusal carries diagnostics");
    if (!outcome.diagnostics.empty()) {
        const auto& item = outcome.diagnostics.back();
        const auto evidence = item.value("evidence", json::object()).value("boolean", json::object());
        check(item.value("code", "") == "BOOLEAN_TOOL_NO_EFFECT" &&
                  evidence.value("noEffectToolBodyId", "") == "tool-2" &&
                  evidence.value("toolIndex", -1) == 1 && evidence.value("keepTools", !keep) == keep,
              "normalized refusal preserves original participation evidence: " + diagnostics.dump());
    }
    check(outcome.body_events.empty() && outcome.delta.empty() && outcome.needs_repair.empty(),
          "refusal emits no lifecycle or partition changes");
    check(fixture.partition.size() == ids.size(), "refusal preserves partition size");
    for (std::size_t i = 0; i < ids.size(); ++i) {
        const auto* body = fixture.bodies.get(ids[i]);
        const auto* entry = fixture.partition.find("tracked_" + ids[i]);
        check(body && serialize(body->geom) == geometry[i], "refusal preserves " + ids[i] + " geometry");
        check(entry && entry->body_id == ids[i] && entry->topo_key == keys[i] && entry->shape.IsSame(bound[i]),
              "refusal preserves " + ids[i] + " tracked reference");
    }
}
}  // namespace

int main() {
    test_raw_union();
    test_execute(false);
    test_execute(true);
    if (failures == 0) std::fprintf(stderr, "native_union_refusal: OK\n");
    return failures;
}
