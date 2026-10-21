// Multi-tool / retained-tool standalone Boolean (SCHEMA §7.3, ONECAD-362).
//
// Analytic volumes against one target box 40×40×20 at the origin; every refusal
// asserts that bodies, partition entries and events are left untouched.
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <string>
#include <thread>
#include <utility>
#include <vector>

#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Shape.hxx>
#include <gp.hxx>
#include <gp_Ax2.hxx>
#include <gp_Pnt.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "ops/BooleanOp.h"
#include "ops/OpTypes.h"
#include "session/BodyStore.h"
#include "session/ShapeMetrics.h"
#include "tess/Tessellate.h"
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

// Tools that share the sketch plane and a wall but each reach material of their
// own still participate: the shared images are nobody's evidence, the rest is.
void test_coplanar_tools_participate() {
    {
        Fixture f;
        seed(f, {{"body_a", box(5, 5, 0, 10, 10, 30)}, {"body_b", box(10, 5, 0, 10, 10, 30)}});
        const ops::OpOutcome out = f.run(array_params("Cut", {"body_a", "body_b"}, true));
        check(out.status == ops::OpOutcome::Status::Ok, "coplanar cut: succeeds");
        check(std::abs(volume_of(f.bodies.get("body_t")->geom) - 29000.0) < 1e-6,
              "coplanar cut: volume 32000 - 150*20");
    }
    {
        Fixture f;
        seed(f, {{"body_a", box(35, 5, 0, 15, 10, 10)}, {"body_b", box(35, 10, 0, 20, 10, 10)}});
        const ops::OpOutcome out = f.run(array_params("Union", {"body_a", "body_b"}, false));
        check(out.status == ops::OpOutcome::Status::Ok, "coplanar union: succeeds");
        // Outside the target: a [40,50]x[5,15] + b [40,55]x[10,20] - [40,50]x[10,15].
        check(std::abs(volume_of(f.bodies.get("body_t")->geom) - 34000.0) < 1e-6,
              "coplanar union: volume 32000 + (100 + 150 - 50)*10");
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
    // Same-sketch-plane extrudes (every box from z=0, like the UI makes them): the
    // nested tool shares walls and the base plane with the outer one, so all of
    // its result faces are images the outer tool owns too.
    const TopoDS_Shape plane_outer = box(5, 5, 0, 10, 10, 30);
    const TopoDS_Shape plane_nested = box(5, 5, 0, 5, 5, 30);
    const TopoDS_Shape edge_outer = box(35, 5, 0, 15, 10, 10);
    const TopoDS_Shape edge_nested = box(35, 5, 0, 10, 5, 10);
    const TopoDS_Shape edge_base = box(41, 7, 0, 4, 3, 5);
    struct Case {
        const char* name;
        const char* operation;
        TopoDS_Shape first, second;
        const char* code;
        const char* idle = "body_b";
    };
    const std::vector<Case> cases = {
        {"union disjoint", "Union", edge, far, "BOOLEAN_DISJOINT_RESULT"},
        {"union covered", "Union", edge, inside_target, "BOOLEAN_TOOL_NO_EFFECT"},
        {"cut far", "Cut", through, far, "BOOLEAN_TOOL_NO_EFFECT"},
        {"cut nested", "Cut", through, inside_through, "BOOLEAN_TOOL_NO_EFFECT"},
        {"cut coplanar nested", "Cut", plane_outer, plane_nested, "BOOLEAN_TOOL_NO_EFFECT"},
        {"cut identical", "Cut", plane_outer, box(5, 5, 0, 10, 10, 30), "BOOLEAN_TOOL_NO_EFFECT",
         "body_a"},
        {"union coplanar nested", "Union", edge_outer, edge_nested, "BOOLEAN_TOOL_NO_EFFECT"},
        {"union shared base", "Union", edge_outer, edge_base, "BOOLEAN_TOOL_NO_EFFECT"},
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
                const int index = std::string(c.idle) == "body_a" ? 0 : 1;
                check(ev.value("noEffectToolBodyId", "") == c.idle &&
                          ev.value("toolIndex", -1) == index,
                      name + ": names the idle tool " + c.idle + ", got " +
                          ev.value("noEffectToolBodyId", ""));
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

// A retained tool must not share face TShapes with the published target: BRepMesh
// reuses any fine-enough triangulation, so the target's MESH1 bytes would depend
// on whether (and at which LOD) the tool was meshed first.
void test_retained_tool_mesh_independent(const char* operation) {
    std::vector<std::vector<std::uint8_t>> blobs;
    for (const char* tool_lod : {"", "coarse", "fine"}) {
        Fixture f;
        // Union crosses the x=40 wall with its top cap (z=21) outside; Cut drills a
        // blind pocket whose bottom cap (z=15) lies untouched inside the target.
        const bool cut = std::string(operation) == "Cut";
        const gp_Ax2 axis(cut ? gp_Pnt(20, 20, 15) : gp_Pnt(40, 20, 19), gp::DZ());
        seed(f, {{"body_c", BRepPrimAPI_MakeCylinder(axis, 10, cut ? 10 : 2).Shape()}});
        const ops::OpOutcome out = f.run(array_params(operation, {"body_c"}, true));
        check(out.status == ops::OpOutcome::Status::Ok, std::string(operation) + " keep cyl: ok");
        if (*tool_lod) {
            onecad::tess::tessellate_body(f.bodies.get("body_c")->geom, "body_c", tool_lod, true,
                                          &f.partition);
        }
        blobs.push_back(onecad::tess::tessellate_body(f.bodies.get("body_t")->geom, "body_t",
                                                      "coarse", true, &f.partition)
                            .blob);
    }
    check(!blobs[0].empty() && blobs[0] == blobs[1] && blobs[0] == blobs[2],
          std::string(operation) + " keep cyl: target MESH1 independent of tool meshing");
}

// `toolBodyIds:[a]` without retention is the legacy op (SCHEMA §7.3): legacy
// evidence keys and the consumed-tool lifecycle, exactly like the app's reader.
void test_single_unkept_array_is_legacy() {
    for (const bool explicit_false : {false, true}) {
        const std::string name = explicit_false ? "single keep:false" : "single unkept";
        json params = array_params("Cut", {"body_a"}, false);
        if (explicit_false) params["keepTools"] = false;
        {
            Fixture f;
            seed(f, {{"body_a", box(100, 100, 100, 5, 5, 5)}});
            const ops::OpOutcome out = f.run(params);
            const json ev = diagnostic_evidence(out);
            check(out.status == ops::OpOutcome::Status::Failed, name + ": far tool refused");
            check(ev.value("toolBodyId", "") == "body_a" && !ev.contains("toolBodyIds") &&
                      !ev.contains("keepTools"),
                  name + ": legacy evidence keys");
        }
        Fixture f;
        seed(f, {{"body_a", box(5, 5, -5, 10, 10, 30)}});
        const ops::OpOutcome out = f.run(params);
        check(out.status == ops::OpOutcome::Status::Ok, name + ": succeeds");
        check(event_list(out) == std::vector<std::string>{"modified:body_t", "deleted:body_a"},
              name + ": consumes the tool");
    }
}

// The token trips while the 32-tool Intersect pre-combine is running (past the
// entry check), so CANCELLED must propagate out of the auxiliary builder loop.
void test_cancel_mid_precombine() {
    Fixture f;
    std::vector<std::pair<std::string, TopoDS_Shape>> all = {{"body_t", box(0, 0, 0, 40, 40, 20)}};
    f.add("body_t", all.front().second);
    std::vector<std::string> ids;
    for (int k = 0; k < 32; ++k) {
        const double a = 2.0 * M_PI * k / 32.0;
        const gp_Ax2 axis(gp_Pnt(20 + 3 * std::cos(a), 20 + 3 * std::sin(a), -5), gp::DZ());
        const std::string id = "body_c" + std::to_string(k);
        all.emplace_back(id, BRepPrimAPI_MakeCylinder(axis, 15, 30).Shape());
        f.add(id, all.back().second);
        ids.push_back(id);
    }
    std::thread trip([&f] {
        std::this_thread::sleep_for(std::chrono::milliseconds(20));
        f.cancel.cancel();
    });
    const ops::OpOutcome out = f.run(array_params("Intersect", ids, false));
    trip.join();
    check(out.status == ops::OpOutcome::Status::Cancelled, "cancel mid pre-combine: CANCELLED");
    check(out.body_events.empty(), "cancel mid pre-combine: no body event");
    for (const auto& [id, shape] : all) {
        const auto* rec = f.bodies.get(id);
        check(rec && rec->geom.IsSame(shape), "cancel mid pre-combine: " + id + " intact");
        check(f.partition.entries_for_body(id).size() == 1,
              "cancel mid pre-combine: " + id + " partition intact");
    }
}

void test_determinism() {
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
    test_coplanar_tools_participate();
    test_intersect_three_tools();
    test_cut_split_retained();
    test_refusals();
    test_param_refusals();
    test_retained_tool_mesh_independent("Union");
    test_retained_tool_mesh_independent("Cut");
    test_single_unkept_array_is_legacy();
    test_cancel_mid_precombine();
    test_determinism();
    if (g_failures == 0) std::fprintf(stderr, "boolean_multi_tool: OK\n");
    return g_failures;
}
