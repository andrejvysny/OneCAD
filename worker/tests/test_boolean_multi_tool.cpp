// Multi-tool / retained-tool standalone Boolean (SCHEMA §7.3, ONECAD-362).
//
// Analytic volumes against one target box 40×40×20 at the origin; every refusal
// asserts that bodies, partition entries and events are left untouched.
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include <BRepPrimAPI_MakeBox.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "ops/BooleanOp.h"
#include "ops/OpTypes.h"
#include "session/BodyStore.h"
#include "session/ShapeMetrics.h"
#include "util/Cancel.h"

using nlohmann::json;
namespace ops = onecad::ops;
namespace em = onecad::elementmap;
namespace km = onecad::kernel::elementmap;
using onecad::session::BodyStore;

namespace {
int g_failures = 0;

void check(bool condition, const std::string& message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message.c_str());
        ++g_failures;
    }
}

TopoDS_Shape box(double x, double y, double z, double dx, double dy, double dz) {
    return BRepPrimAPI_MakeBox(gp_Pnt(x, y, z), dx, dy, dz).Shape();
}

double volume_of(const TopoDS_Shape& shape) { return onecad::session::shape_volume(shape); }

struct Fixture {
    BodyStore bodies;
    em::ElementMapPartition partition;
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;

    // Every body carries one tracked face entry so partition changes are visible.
    void add(const std::string& id, const TopoDS_Shape& shape) {
        bodies.create(id, "seed_" + id, shape);
        TopTools_IndexedMapOfShape faces;
        TopExp::MapShapes(shape, TopAbs_FACE, faces);
        partition.mint(id, "el_" + id, km::ElementKind::Face, faces(1), shape);
    }

    ops::OpOutcome run(const json& params, const std::string& op_id = "op_bool") {
        ops::OpContext context{bodies, &sketches, partition, &last_sketch, false,
                               json::object(), &cancel};
        const json op = {{"opType", "Boolean"}, {"opId", op_id}, {"params", params}};
        return ops::execute_boolean(context, op, op_id);
    }
};

json array_params(const char* operation, std::vector<std::string> tools, bool keep) {
    json params = {{"operation", operation}, {"targetBodyId", "body_t"}, {"toolBodyIds", tools}};
    if (keep) params["keepTools"] = true;
    return params;
}

void seed(Fixture& f, const std::vector<std::pair<std::string, TopoDS_Shape>>& tools) {
    f.add("body_t", box(0, 0, 0, 40, 40, 20));
    for (const auto& [id, shape] : tools) f.add(id, shape);
}

std::vector<std::string> event_list(const ops::OpOutcome& out) {
    std::vector<std::string> events;
    for (const auto& e : out.body_events) events.push_back(e.kind + ":" + e.body_id);
    return events;
}

json diagnostic_evidence(const ops::OpOutcome& out) {
    if (out.diagnostics.empty()) return json::object();
    return out.diagnostics.back().value("evidence", json::object()).value("boolean", json::object());
}

std::string diagnostic_code(const ops::OpOutcome& out) {
    return out.diagnostics.empty() ? "" : out.diagnostics.back().value("code", "");
}

// A refusal leaves every body, its geometry and its partition entry in place.
void check_untouched(const Fixture& f, const std::vector<std::pair<std::string, TopoDS_Shape>>& all,
                     const ops::OpOutcome& out, const std::string& name) {
    check(out.status == ops::OpOutcome::Status::Failed, name + ": refused");
    check(out.body_events.empty(), name + ": no body event");
    for (const auto& [id, shape] : all) {
        const auto* rec = f.bodies.get(id);
        check(rec && rec->geom.IsSame(shape), name + ": " + id + " intact");
        check(f.partition.entries_for_body(id).size() == 1, name + ": " + id + " partition intact");
    }
}

void test_union_two_tools_consumed() {
    Fixture f;
    // a overlaps the target edge; b touches only a.
    seed(f, {{"body_a", box(35, 10, 5, 10, 10, 10)}, {"body_b", box(42, 12, 7, 10, 6, 6)}});
    const ops::OpOutcome out = f.run(array_params("Union", {"body_a", "body_b"}, false));
    check(out.status == ops::OpOutcome::Status::Ok, "union2: succeeds");
    check(event_list(out) ==
              std::vector<std::string>{"modified:body_t", "deleted:body_a", "deleted:body_b"},
          "union2: modified target then deletes in tool order");
    check(std::abs(volume_of(f.bodies.get("body_t")->geom) - 32752.0) < 1e-6,
          "union2: volume 32000 + 500 + 252");
    check(!f.bodies.contains("body_a") && !f.bodies.contains("body_b"), "union2: tools consumed");
    check(f.partition.entries_for_body("body_a").empty(), "union2: tool partition dropped");
}

void test_cut_overlapping_tools(bool keep) {
    Fixture f;
    const TopoDS_Shape a = box(5, 5, -5, 10, 10, 30);
    const TopoDS_Shape b = box(10, 10, -5, 10, 10, 30);
    seed(f, {{"body_a", a}, {"body_b", b}});
    const std::string name = keep ? "cut2 keep" : "cut2 consume";
    const ops::OpOutcome out = f.run(array_params("Cut", {"body_a", "body_b"}, keep));
    check(out.status == ops::OpOutcome::Status::Ok, name + ": succeeds");
    check(std::abs(volume_of(f.bodies.get("body_t")->geom) - 28500.0) < 1e-6,
          name + ": volume 32000 - 175*20");
    if (keep) {
        check(event_list(out) == std::vector<std::string>{"modified:body_t"},
              name + ": only the target event");
        check(f.bodies.get("body_a")->geom.IsSame(a) && f.bodies.get("body_b")->geom.IsSame(b),
              name + ": tools unchanged");
        check(f.partition.entries_for_body("body_a").size() == 1 &&
                  f.partition.entries_for_body("body_b").size() == 1,
              name + ": tool partitions unchanged");
    } else {
        check(event_list(out) ==
                  std::vector<std::string>{"modified:body_t", "deleted:body_a", "deleted:body_b"},
              name + ": deletes in tool order");
    }
}

void test_intersect_three_tools() {
    Fixture f;
    seed(f, {{"body_p", box(0, 0, 0, 20, 40, 20)},
             {"body_q", box(0, 0, 0, 40, 20, 20)},
             {"body_r", box(0, 0, 0, 40, 40, 10)}});
    const ops::OpOutcome out = f.run(array_params("Intersect", {"body_p", "body_q", "body_r"}, false));
    check(out.status == ops::OpOutcome::Status::Ok, "intersect3: succeeds");
    const double v = volume_of(f.bodies.get("body_t")->geom);
    // Common of ALL tools (4000), never target ∩ (∪ tools) (28000).
    check(std::abs(v - 4000.0) < 1e-6, "intersect3: triple overlap volume 4000");
    check(event_list(out).size() == 4, "intersect3: modified + three deletes");
}

void test_cut_split_retained() {
    Fixture f;
    const TopoDS_Shape slab = box(18, -5, -5, 4, 50, 30);
    const TopoDS_Shape hole = box(5, 5, -5, 5, 5, 30);
    seed(f, {{"body_s", slab}, {"body_h", hole}});
    const ops::OpOutcome out = f.run(array_params("Cut", {"body_s", "body_h"}, true), "op_split");
    check(out.status == ops::OpOutcome::Status::Ok, "split keep: succeeds");
    check(event_list(out) == std::vector<std::string>{"deleted:body_t", "created:body_op_split:0",
                                                      "created:body_op_split:1"},
          "split keep: target split, tools emit nothing");
    check(f.bodies.contains("body_s") && f.bodies.contains("body_h"), "split keep: tools alive");
    const double v0 = volume_of(f.bodies.get("body_op_split:0")->geom);
    const double v1 = volume_of(f.bodies.get("body_op_split:1")->geom);
    check(std::abs(v0 - 13900.0) < 1e-6 && std::abs(v1 - 14400.0) < 1e-6,
          "split keep: halves 14400 - 500 and 14400 ranked by volume");
}

void test_refusals() {
    const TopoDS_Shape through = box(5, 5, -5, 10, 10, 30);
    const TopoDS_Shape edge = box(35, 10, 5, 10, 10, 10);
    const TopoDS_Shape far = box(100, 100, 100, 5, 5, 5);
    const TopoDS_Shape inside_target = box(10, 10, 5, 5, 5, 5);
    const TopoDS_Shape inside_through = box(7, 7, 0, 2, 2, 10);
    struct Case {
        const char* name;
        const char* operation;
        TopoDS_Shape first, second;
        const char* code;
    };
    const std::vector<Case> cases = {
        {"union disjoint", "Union", edge, far, "BOOLEAN_DISJOINT_RESULT"},
        {"union covered", "Union", edge, inside_target, "BOOLEAN_TOOL_NO_EFFECT"},
        {"cut far", "Cut", through, far, "BOOLEAN_TOOL_NO_EFFECT"},
        {"cut nested", "Cut", through, inside_through, "BOOLEAN_TOOL_NO_EFFECT"},
        {"intersect empty", "Intersect", through, far, "BOOLEAN_EMPTY_RESULT"},
    };
    for (const Case& c : cases) {
        for (bool keep : {false, true}) {
            Fixture f;
            const TopoDS_Shape target = box(0, 0, 0, 40, 40, 20);
            f.add("body_t", target);
            f.add("body_a", c.first);
            f.add("body_b", c.second);
            const ops::OpOutcome out = f.run(array_params(c.operation, {"body_a", "body_b"}, keep));
            const std::string name = std::string(c.name) + (keep ? " keep" : "");
            check_untouched(f, {{"body_t", target}, {"body_a", c.first}, {"body_b", c.second}},
                            out, name);
            check(out.error_code == "OP_FAILED", name + ": OP_FAILED");
            check(diagnostic_code(out) == c.code, name + ": code " + c.code + ", got " +
                                                      diagnostic_code(out));
            const json ev = diagnostic_evidence(out);
            check(ev.value("toolBodyIds", json()) == json{"body_a", "body_b"} &&
                      ev.value("keepTools", !keep) == keep && !ev.contains("toolBodyId"),
                  name + ": evidence names the tool set");
            if (std::string(c.code) == "BOOLEAN_TOOL_NO_EFFECT") {
                check(ev.value("noEffectToolBodyId", "") == "body_b" && ev.value("toolIndex", -1) == 1,
                      name + ": names the idle tool");
            }
            if (std::string(c.code) == "BOOLEAN_DISJOINT_RESULT") {
                check(ev.value("disjointToolBodyIds", json()) == json{"body_b"},
                      name + ": names the disjoint tool");
            }
            if (std::string(c.name) == "intersect empty") {
                check(ev.value("precombine", false), name + ": pre-combine stage");
            }
        }
    }
}

void test_param_refusals() {
    const std::vector<std::string> many = [] {
        std::vector<std::string> ids;
        for (int i = 0; i < 33; ++i) ids.push_back("body_x" + std::to_string(i));
        return ids;
    }();
    struct Case {
        const char* name;
        json params;
        const char* code;
    };
    const std::vector<Case> cases = {
        {"missing tool", array_params("Cut", {"body_a", "body_missing"}, false), "REF_UNRESOLVED"},
        {"duplicate", array_params("Cut", {"body_a", "body_a"}, false), "OP_FAILED"},
        {"self", array_params("Cut", {"body_a", "body_t"}, false), "OP_FAILED"},
        {"empty", array_params("Cut", {}, false), "OP_FAILED"},
        {"33 tools", array_params("Cut", many, false), "OP_FAILED"},
        {"both fields",
         {{"operation", "Cut"}, {"targetBodyId", "body_t"}, {"toolBodyId", "body_a"},
          {"toolBodyIds", {"body_a"}}},
         "OP_FAILED"},
        {"legacy keep",
         {{"operation", "Cut"}, {"targetBodyId", "body_t"}, {"toolBodyId", "body_a"},
          {"keepTools", true}},
         "OP_FAILED"},
        {"keep not bool", {{"operation", "Cut"}, {"targetBodyId", "body_t"},
                           {"toolBodyIds", {"body_a"}}, {"keepTools", "yes"}},
         "OP_FAILED"},
    };
    for (const Case& c : cases) {
        Fixture f;
        const TopoDS_Shape target = box(0, 0, 0, 40, 40, 20);
        const TopoDS_Shape a = box(5, 5, -5, 10, 10, 30);
        f.add("body_t", target);
        f.add("body_a", a);
        const ops::OpOutcome out = f.run(c.params);
        check_untouched(f, {{"body_t", target}, {"body_a", a}}, out, c.name);
        check(out.error_code == c.code,
              std::string(c.name) + ": " + c.code + ", got " + out.error_code);
    }
}

void test_cancel_and_determinism() {
    Fixture cancelled;
    seed(cancelled, {{"body_p", box(0, 0, 0, 20, 40, 20)}, {"body_q", box(0, 0, 0, 40, 20, 20)}});
    cancelled.cancel.cancel();
    const ops::OpOutcome out =
        cancelled.run(array_params("Intersect", {"body_p", "body_q"}, true));
    check(out.status == ops::OpOutcome::Status::Cancelled, "cancel: CANCELLED");
    check(cancelled.bodies.size() == 3, "cancel: bodies intact");

    std::vector<std::vector<std::string>> runs;
    std::vector<std::vector<std::int64_t>> keys;
    for (int i = 0; i < 2; ++i) {
        Fixture f;
        seed(f, {{"body_s", box(18, -5, -5, 4, 50, 30)}, {"body_h", box(5, 5, -5, 5, 5, 30)}});
        const ops::OpOutcome run = f.run(array_params("Cut", {"body_s", "body_h"}, false), "op_d");
        runs.push_back(event_list(run));
        std::vector<std::int64_t> flat;
        for (const auto& e : run.body_events) {
            if (!e.rank_key) continue;
            flat.insert(flat.end(), e.rank_key->begin(), e.rank_key->end());
        }
        keys.push_back(flat);
    }
    check(runs[0] == runs[1] && keys[0] == keys[1] && !keys[0].empty(),
          "determinism: identical ids and rank keys");
}
}  // namespace

int main() {
    test_union_two_tools_consumed();
    test_cut_overlapping_tools(false);
    test_cut_overlapping_tools(true);
    test_intersect_three_tools();
    test_cut_split_retained();
    test_refusals();
    test_param_refusals();
    test_cancel_and_determinism();
    if (g_failures == 0) std::fprintf(stderr, "boolean_multi_tool: OK\n");
    return g_failures;
}
