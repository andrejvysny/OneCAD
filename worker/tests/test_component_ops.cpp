// test_component_ops.cpp — Component Library WP-0.2/WP-1.2 `PlaceComponent`/
// `DetachComponent` numerics, in-process via `ops::execute_place_component`/
// `execute_detach_component` (real OCCT). No framework: exit code == failure
// count. Mirrors `test_m6a_ops.cpp`'s shape.
#include <cmath>
#include <cstdio>
#include <string>

#include "nlohmann/json.hpp"
#include "ops/ComponentOp.h"
#include "ops/OpTypes.h"
#include "session/BodyStore.h"
#include "session/ShapeMetrics.h"
#include "util/Cancel.h"

using nlohmann::json;
namespace ops = onecad::ops;
using onecad::session::BodyStore;

namespace {
int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) { std::fprintf(stderr, "FAIL: %s\n", msg.c_str()); ++g_failures; }
}
void check_near(double got, double want, double tol, const std::string& msg) {
    if (std::abs(got - want) > tol) {
        std::fprintf(stderr, "FAIL: %s (got %.4f want %.4f)\n", msg.c_str(), got, want);
        ++g_failures;
    }
}

double vol(const TopoDS_Shape& s) { return onecad::session::shape_volume(s); }

// The hardcoded ISO 4762 M6x20 solid's exact analytic volume: head cylinder
// (r=5, h=6) fused to shank cylinder (r=3, h=20) — coaxial, no overlap.
constexpr double kPi = 3.14159265358979323846;
constexpr double kM6ShcsVolume = kPi * 5.0 * 5.0 * 6.0 + kPi * 3.0 * 3.0 * 20.0;  // ~1036.73

struct Ctx {
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;
    ops::OpContext make(BodyStore& bodies, onecad::elementmap::ElementMapPartition& part) {
        return ops::OpContext{bodies, &sketches, part, &last_sketch, false, json::object(), &cancel};
    }
};

json generator_source(const std::string& generator_id = "iso4762") {
    return json{{"kind", "generator"}, {"generatorId", generator_id}, {"generatorVersion", 1}};
}

// WP-2.1: a generator source carrying `params.thread`/`.length` — the
// table-driven path. Omitting `params` entirely (as `generator_source()`
// does above) exercises the M6×20 DEFAULT path instead — both must agree.
json generator_source_sized(const std::string& thread, double length_mm,
                            const std::string& generator_id = "iso4762") {
    return json{{"kind", "generator"},
                {"generatorId", generator_id},
                {"generatorVersion", 1},
                {"params", {{"thread", thread}, {"length", length_mm}}}};
}

json identity_placement() {
    return json{{"translate", {0.0, 0.0, 0.0}}};
}

// ── A generator-source PlaceComponent mints one body of the exact M6 SHCS volume. ─
void test_place_component_generator_source_exact_volume() {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "PlaceComponent"}, {"opId", "opc1"},
               {"params", {{"componentId", "onecad.std.iso4762"},
                           {"componentVersion", "1.0.0"},
                           {"componentRevision", "sha256:" + std::string(64, '0')},
                           {"source", generator_source()},
                           {"placement", identity_placement()}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opc1");
    check(oc.status == ops::OpOutcome::Status::Ok, "placeComponent: Ok");
    check(oc.body_events.size() == 1 && oc.body_events[0].kind == "created",
          "placeComponent: one body CREATED (NewBody lineage)");
    check(oc.body_ids.size() == 1 && oc.body_ids[0] == "body_opc1",
          "placeComponent: body id is body_<opId>");
    const onecad::session::BodyRecord* rec = bodies.get("body_opc1");
    check(rec != nullptr, "placeComponent: body published in BodyStore");
    if (rec != nullptr) {
        check_near(vol(rec->geom), kM6ShcsVolume, 1.0, "placeComponent: exact M6 SHCS volume");
    }
}

// ── A rigid placement transform preserves volume (translate + rotate). ────────────
void test_place_component_placement_transform_preserves_volume() {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "PlaceComponent"}, {"opId", "opc2"},
               {"params", {{"componentId", "onecad.std.iso4762"},
                           {"componentVersion", "1.0.0"},
                           {"componentRevision", "sha256:" + std::string(64, '0')},
                           {"source", generator_source()},
                           {"placement",
                            {{"translate", {10.0, -5.0, 25.0}},
                             {"rotate", {{"center", {10.0, -5.0, 25.0}},
                                         {"axis", {0.0, 1.0, 0.0}},
                                         {"angleDeg", 90.0}}}}}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opc2");
    check(oc.status == ops::OpOutcome::Status::Ok, "placeComponent(placed): Ok");
    const onecad::session::BodyRecord* rec = bodies.get("body_opc2");
    check(rec != nullptr, "placeComponent(placed): body published");
    if (rec != nullptr) {
        check_near(vol(rec->geom), kM6ShcsVolume, 1.0,
                   "placeComponent(placed): rigid transform preserves volume");
    }
}

// ── WP-2.1: an explicit M6×20 via `source.params` matches the DEFAULT path
// exactly — proves the table lookup for the default size agrees with the
// old hardcoded constants byte-for-byte. ─────────────────────────────────────
void test_place_component_explicit_m6_matches_default() {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "PlaceComponent"}, {"opId", "opc5"},
               {"params", {{"componentId", "onecad.std.iso4762"},
                           {"componentVersion", "1.0.0"},
                           {"componentRevision", "sha256:" + std::string(64, '0')},
                           {"source", generator_source_sized("M6", 20.0)},
                           {"placement", identity_placement()}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opc5");
    check(oc.status == ops::OpOutcome::Status::Ok, "placeComponent(M6 explicit): Ok");
    const onecad::session::BodyRecord* rec = bodies.get("body_opc5");
    check(rec != nullptr, "placeComponent(M6 explicit): body published");
    if (rec != nullptr) {
        check_near(vol(rec->geom), kM6ShcsVolume, 1.0,
                   "placeComponent(M6 explicit): matches the default-path volume exactly");
    }
}

// ── WP-2.1: M2 (the seed range's smallest size) — BOLTS-sourced dims. ─────────────
void test_place_component_m2_exact_volume() {
    // d2=3.8 (head Ø), k=2.0 (head height), d1=2.0 (shank Ø), length=16.
    constexpr double kM2Volume = kPi * 1.9 * 1.9 * 2.0 + kPi * 1.0 * 1.0 * 16.0;  // ~72.94
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "PlaceComponent"}, {"opId", "opc6"},
               {"params", {{"componentId", "onecad.std.iso4762"},
                           {"componentVersion", "1.0.0"},
                           {"componentRevision", "sha256:" + std::string(64, '0')},
                           {"source", generator_source_sized("M2", 16.0)},
                           {"placement", identity_placement()}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opc6");
    check(oc.status == ops::OpOutcome::Status::Ok, "placeComponent(M2): Ok");
    const onecad::session::BodyRecord* rec = bodies.get("body_opc6");
    check(rec != nullptr, "placeComponent(M2): body published");
    if (rec != nullptr) {
        check_near(vol(rec->geom), kM2Volume, 0.1, "placeComponent(M2): exact BOLTS-sourced volume");
    }
}

// ── WP-2.1: M12 (the seed range's largest size) — BOLTS-sourced dims. ─────────────
void test_place_component_m12_exact_volume() {
    // d2=18 (head Ø), k=12 (head height), d1=12 (shank Ø), length=50.
    constexpr double kM12Volume = kPi * 9.0 * 9.0 * 12.0 + kPi * 6.0 * 6.0 * 50.0;  // ~8709.75
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "PlaceComponent"}, {"opId", "opc7"},
               {"params", {{"componentId", "onecad.std.iso4762"},
                           {"componentVersion", "1.0.0"},
                           {"componentRevision", "sha256:" + std::string(64, '0')},
                           {"source", generator_source_sized("M12", 50.0)},
                           {"placement", identity_placement()}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opc7");
    check(oc.status == ops::OpOutcome::Status::Ok, "placeComponent(M12): Ok");
    const onecad::session::BodyRecord* rec = bodies.get("body_opc7");
    check(rec != nullptr, "placeComponent(M12): body published");
    if (rec != nullptr) {
        check_near(vol(rec->geom), kM12Volume, 1.0, "placeComponent(M12): exact BOLTS-sourced volume");
    }
}

// ── WP-2.1: an unknown thread designation fails loudly, never substitutes. ────────
void test_place_component_unknown_thread_fails_loud() {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "PlaceComponent"}, {"opId", "opc8"},
               {"params", {{"componentId", "onecad.std.iso4762"},
                           {"componentVersion", "1.0.0"},
                           {"componentRevision", "sha256:" + std::string(64, '0')},
                           {"source", generator_source_sized("M99", 20.0)},
                           {"placement", identity_placement()}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opc8");
    check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "OP_FAILED",
          "placeComponent: unknown thread 'M99' -> OP_FAILED, not a silent M6 substitution");
    check(bodies.get("body_opc8") == nullptr, "placeComponent: no body published on unknown thread");
}

// ── WP-2.1: a non-positive length fails loudly. ────────────────────────────────────
void test_place_component_non_positive_length_fails_loud() {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "PlaceComponent"}, {"opId", "opc9"},
               {"params", {{"componentId", "onecad.std.iso4762"},
                           {"componentVersion", "1.0.0"},
                           {"componentRevision", "sha256:" + std::string(64, '0')},
                           {"source", generator_source_sized("M6", 0.0)},
                           {"placement", identity_placement()}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opc9");
    check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "OP_FAILED",
          "placeComponent: length=0 -> OP_FAILED");
}

// ── An unsupported source kind refuses loudly (P0/WP-0.2: generator only). ────────
void test_place_component_embedded_source_unsupported() {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "PlaceComponent"}, {"opId", "opc3"},
               {"params", {{"componentId", "onecad.std.iso4762"},
                           {"componentVersion", "1.0.0"},
                           {"componentRevision", "sha256:" + std::string(64, '0')},
                           {"source", {{"kind", "embedded"}, {"sha256", std::string(64, 'a')}}},
                           {"placement", identity_placement()}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opc3");
    // A recognized-but-not-yet-wired PARAMETER VALUE is a recoverable
    // `Status::Failed` with code UNSUPPORTED — distinct from `Status::Unsupported`,
    // which PlanExecutor reserves for an entirely unrecognized opType.
    check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "UNSUPPORTED",
          "placeComponent: embedded source is UNSUPPORTED in this build");
    check(bodies.get("body_opc3") == nullptr, "placeComponent: no body published on refusal");
}

// ── An empty generatorId fails loudly, recoverably. ────────────────────────────────
void test_place_component_empty_generator_id_fails_loud() {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "PlaceComponent"}, {"opId", "opc4"},
               {"params", {{"componentId", "onecad.std.iso4762"},
                           {"componentVersion", "1.0.0"},
                           {"componentRevision", "sha256:" + std::string(64, '0')},
                           {"source", generator_source("")},
                           {"placement", identity_placement()}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opc4");
    check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "OP_FAILED",
          "placeComponent: empty generatorId → OP_FAILED");
}

// ── DetachComponent builds the IDENTICAL geometry PlaceComponent would (WP-1.2). ──
void test_detach_component_exact_volume_matches_place_component() {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    // NO componentId/componentVersion/componentRevision/mate — spec §3.4:
    // "no component_* fields remain."
    json op = {{"opType", "DetachComponent"}, {"opId", "opd1"},
               {"params", {{"source", generator_source()},
                           {"placement", identity_placement()}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_detach_component(ctx, op, "opd1");
    check(oc.status == ops::OpOutcome::Status::Ok, "detachComponent: Ok");
    check(oc.body_events.size() == 1 && oc.body_events[0].kind == "created",
          "detachComponent: one body CREATED (mirrors PlaceComponent's lineage)");
    check(oc.body_ids.size() == 1 && oc.body_ids[0] == "body_opd1",
          "detachComponent: body id is body_<opId>");
    const onecad::session::BodyRecord* rec = bodies.get("body_opd1");
    check(rec != nullptr, "detachComponent: body published in BodyStore");
    if (rec != nullptr) {
        check_near(vol(rec->geom), kM6ShcsVolume, 1.0,
                   "detachComponent: exact M6 SHCS volume, same as PlaceComponent");
    }
}

// ── DetachComponent honors placement too (same shared pipeline). ──────────────────
void test_detach_component_placement_transform_preserves_volume() {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "DetachComponent"}, {"opId", "opd2"},
               {"params", {{"source", generator_source()},
                           {"placement",
                            {{"translate", {1.0, 2.0, 3.0}},
                             {"rotate", {{"center", {1.0, 2.0, 3.0}},
                                         {"axis", {1.0, 0.0, 0.0}},
                                         {"angleDeg", 45.0}}}}}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_detach_component(ctx, op, "opd2");
    check(oc.status == ops::OpOutcome::Status::Ok, "detachComponent(placed): Ok");
    const onecad::session::BodyRecord* rec = bodies.get("body_opd2");
    if (rec != nullptr) {
        check_near(vol(rec->geom), kM6ShcsVolume, 1.0,
                   "detachComponent(placed): rigid transform preserves volume");
    }
}

// ── DetachComponent refuses an unsupported source exactly like PlaceComponent. ────
void test_detach_component_embedded_source_unsupported() {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "DetachComponent"}, {"opId", "opd3"},
               {"params", {{"source", {{"kind", "embedded"}, {"sha256", std::string(64, 'a')}}},
                           {"placement", identity_placement()}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_detach_component(ctx, op, "opd3");
    check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "UNSUPPORTED",
          "detachComponent: embedded source is UNSUPPORTED in this build");
    check(bodies.get("body_opd3") == nullptr, "detachComponent: no body published on refusal");
}

}  // namespace

int main() {
    test_place_component_generator_source_exact_volume();
    test_place_component_placement_transform_preserves_volume();
    test_place_component_explicit_m6_matches_default();
    test_place_component_m2_exact_volume();
    test_place_component_m12_exact_volume();
    test_place_component_unknown_thread_fails_loud();
    test_place_component_non_positive_length_fails_loud();
    test_place_component_embedded_source_unsupported();
    test_place_component_empty_generator_id_fails_loud();
    test_detach_component_exact_volume_matches_place_component();
    test_detach_component_placement_transform_preserves_volume();
    test_detach_component_embedded_source_unsupported();
    if (g_failures == 0) std::fprintf(stderr, "component_ops: OK\n");
    return g_failures;
}
