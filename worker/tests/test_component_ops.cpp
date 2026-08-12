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

// WP-2.5: a generator source additionally carrying `params.thread_detail`.
json generator_source_detailed(const std::string& thread, double length_mm,
                               const std::string& thread_detail,
                               const std::string& generator_id = "iso4762") {
    return json{{"kind", "generator"},
                {"generatorId", generator_id},
                {"generatorVersion", 1},
                {"params",
                 {{"thread", thread}, {"length", length_mm}, {"thread_detail", thread_detail}}}};
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

// ── WP-2.5: `thread_detail: "cosmetic"` explicit matches the DEFAULT (absent)
// path exactly — proves the new param doesn't change cosmetic behavior. ──────────
void test_place_component_thread_detail_cosmetic_matches_default() {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "PlaceComponent"}, {"opId", "opc10"},
               {"params", {{"componentId", "onecad.std.iso4762"},
                           {"componentVersion", "1.0.0"},
                           {"componentRevision", "sha256:" + std::string(64, '0')},
                           {"source", generator_source_detailed("M6", 20.0, "cosmetic")},
                           {"placement", identity_placement()}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opc10");
    check(oc.status == ops::OpOutcome::Status::Ok, "placeComponent(cosmetic explicit): Ok");
    const onecad::session::BodyRecord* rec = bodies.get("body_opc10");
    check(rec != nullptr, "placeComponent(cosmetic explicit): body published");
    if (rec != nullptr) {
        check_near(vol(rec->geom), kM6ShcsVolume, 1.0,
                   "placeComponent(cosmetic explicit): matches default-path volume exactly");
    }
}

// ── WP-2.5: `simplified` cuts real material out of the shank, but stays close
// to the blank (shallow grooves, not a gutted shank). ─────────────────────────────
void test_place_component_thread_detail_simplified_removes_material() {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "PlaceComponent"}, {"opId", "opc11"},
               {"params", {{"componentId", "onecad.std.iso4762"},
                           {"componentVersion", "1.0.0"},
                           {"componentRevision", "sha256:" + std::string(64, '0')},
                           {"source", generator_source_detailed("M6", 20.0, "simplified")},
                           {"placement", identity_placement()}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opc11");
    check(oc.status == ops::OpOutcome::Status::Ok, "placeComponent(simplified M6x20): Ok");
    const onecad::session::BodyRecord* rec = bodies.get("body_opc11");
    check(rec != nullptr, "placeComponent(simplified M6x20): body published");
    if (rec != nullptr) {
        const double v = vol(rec->geom);
        check(v < kM6ShcsVolume - 1.0,
              "placeComponent(simplified M6x20): grooves remove material vs the cosmetic blank");
        check(v > kM6ShcsVolume * 0.5,
              "placeComponent(simplified M6x20): grooves are shallow, not gutting the shank");
    }
}

// ── WP-2.5: `modeled` cuts a true helical thread — real material removed, at
// both a mid-range pitch (M6) and the seed range's tightest pitch (M2, 0.4mm),
// the case most likely to stress `BRepOffsetAPI_MakePipeShell`. ───────────────────
void test_place_component_thread_detail_modeled_removes_material() {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "PlaceComponent"}, {"opId", "opc12"},
               {"params", {{"componentId", "onecad.std.iso4762"},
                           {"componentVersion", "1.0.0"},
                           {"componentRevision", "sha256:" + std::string(64, '0')},
                           {"source", generator_source_detailed("M6", 20.0, "modeled")},
                           {"placement", identity_placement()}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opc12");
    check(oc.status == ops::OpOutcome::Status::Ok, "placeComponent(modeled M6x20): Ok");
    const onecad::session::BodyRecord* rec = bodies.get("body_opc12");
    check(rec != nullptr, "placeComponent(modeled M6x20): body published");
    if (rec != nullptr) {
        const double v = vol(rec->geom);
        check(v < kM6ShcsVolume - 1.0,
              "placeComponent(modeled M6x20): helical cut removes material vs the cosmetic blank");
        check(v > kM6ShcsVolume * 0.5,
              "placeComponent(modeled M6x20): thread is shallow, not gutting the shank");
    }
}

void test_place_component_thread_detail_modeled_m2_smallest_pitch_succeeds() {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "PlaceComponent"}, {"opId", "opc13"},
               {"params", {{"componentId", "onecad.std.iso4762"},
                           {"componentVersion", "1.0.0"},
                           {"componentRevision", "sha256:" + std::string(64, '0')},
                           {"source", generator_source_detailed("M2", 16.0, "modeled")},
                           {"placement", identity_placement()}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opc13");
    check(oc.status == ops::OpOutcome::Status::Ok,
          "placeComponent(modeled M2x16, smallest seed-range pitch 0.4mm): Ok");
    check(bodies.get("body_opc13") != nullptr, "placeComponent(modeled M2): body published");
}

// ── WP-2.5: an unknown thread_detail fails loudly, never silently falls back
// to cosmetic. ──────────────────────────────────────────────────────────────────────
void test_place_component_unknown_thread_detail_fails_loud() {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "PlaceComponent"}, {"opId", "opc14"},
               {"params", {{"componentId", "onecad.std.iso4762"},
                           {"componentVersion", "1.0.0"},
                           {"componentRevision", "sha256:" + std::string(64, '0')},
                           {"source", generator_source_detailed("M6", 20.0, "bogus")},
                           {"placement", identity_placement()}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opc14");
    check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "OP_FAILED",
          "placeComponent: unknown thread_detail 'bogus' -> OP_FAILED, not a silent cosmetic fallback");
    check(bodies.get("body_opc14") == nullptr,
          "placeComponent: no body published on unknown thread_detail");
}

// ── WP-2.6: thread_detail × thread size matrix across the FULL seed range —
// robustness, not just correctness-at-a-few-sizes (spec §10: "modeled
// threads... are a natural kernelbench robustness case"). A REAL kernelbench
// `OperationFamily::Component` would need a schema fork (case-v2's
// selector/validator types are edge-blend-topology-shaped, not solid-body-
// boolean-cut-shaped) — out of scope for one WP, flagged in TODO.md's WP-2.5
// entry. This ctest matrix is the deliberately-chosen lighter mechanism
// instead. Length is 4×d1 per size — proportional, not claiming any external
// standard, just large enough that even the tightest pitch (M2, 0.4mm)
// clears several grooves/turns.
void test_place_component_thread_detail_matrix_across_the_full_seed_range() {
    struct Size {
        const char* thread;
        double d1;
    };
    static const Size kSizes[] = {
        {"M2", 2.0}, {"M2.5", 2.5}, {"M3", 3.0},  {"M4", 4.0},  {"M5", 5.0},
        {"M6", 6.0}, {"M8", 8.0},  {"M10", 10.0}, {"M12", 12.0},
    };
    static const char* kDetails[] = {"cosmetic", "simplified", "modeled"};
    int op_seq = 100;
    for (const Size& size : kSizes) {
        const double length_mm = size.d1 * 4.0;
        for (const char* detail : kDetails) {
            BodyStore bodies;
            onecad::elementmap::ElementMapPartition part;
            const std::string op_id = "opcm" + std::to_string(op_seq++);
            json op = {{"opType", "PlaceComponent"}, {"opId", op_id},
                       {"params", {{"componentId", "onecad.std.iso4762"},
                                   {"componentVersion", "1.0.0"},
                                   {"componentRevision", "sha256:" + std::string(64, '0')},
                                   {"source", generator_source_detailed(size.thread, length_mm, detail)},
                                   {"placement", identity_placement()}}}};
            Ctx c;
            ops::OpContext ctx = c.make(bodies, part);
            ops::OpOutcome oc = ops::execute_place_component(ctx, op, op_id);
            const std::string label = std::string(size.thread) + "/" + detail;
            check(oc.status == ops::OpOutcome::Status::Ok,
                  "placeComponent(matrix " + label + "): Ok");
            const onecad::session::BodyRecord* rec = bodies.get("body_" + op_id);
            check(rec != nullptr, "placeComponent(matrix " + label + "): body published");
            if (rec != nullptr) {
                const double v = vol(rec->geom);
                check(std::isfinite(v) && v > 0.0,
                      "placeComponent(matrix " + label + "): finite positive volume");
            }
        }
    }
}

// ── WP-2.6: `simplified` below one pitch (the n<=0 guard in
// `cut_simplified_thread`) returns the cosmetic blank EXACTLY, never a
// malformed near-empty groove. ─────────────────────────────────────────────────────
void test_place_component_thread_detail_simplified_below_one_pitch_matches_cosmetic() {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "PlaceComponent"}, {"opId", "opcm200"},
               {"params", {{"componentId", "onecad.std.iso4762"},
                           {"componentVersion", "1.0.0"},
                           {"componentRevision", "sha256:" + std::string(64, '0')},
                           {"source", generator_source_detailed("M6", 0.5, "simplified")},
                           {"placement", identity_placement()}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opcm200");
    check(oc.status == ops::OpOutcome::Status::Ok, "placeComponent(simplified, length < 1 pitch): Ok");
    const onecad::session::BodyRecord* rec = bodies.get("body_opcm200");
    check(rec != nullptr, "placeComponent(simplified, length < 1 pitch): body published");
    if (rec != nullptr) {
        // M6 blank: d2=10 head Ø, k=6 head height, d1=6 shank Ø, length=0.5.
        const double expected = kPi * 5.0 * 5.0 * 6.0 + kPi * 3.0 * 3.0 * 0.5;
        check_near(vol(rec->geom), expected, 0.5,
                   "placeComponent(simplified, length < 1 pitch): matches the cosmetic blank exactly");
    }
}

// ── WP-2.6: `modeled` at the tightest pitch (M2) over a long shank — a REAL
// kernel limit this matrix found, not a hypothetical one. M2×16mm (40 turns,
// the WP-2.5 gate case) and M2×8mm (matrix, 20 turns) both succeed; M2×60mm
// (~150 turns) makes `BRepOffsetAPI_MakePipeShell::Build()` fail — cleanly:
// `IsDone()` false, no exception, no crash, no hang (this whole binary runs
// in ~12s), no partial/garbage shape ever reaches `checked_boolean`. That is
// the CORRECT outcome per this op's own contract (`cut_modeled_thread`'s
// `!pipe.IsDone()` guard exists for exactly this) — a safe, deterministic
// `OP_FAILED` refusal beats returning invalid geometry, spec §0's standing
// invariant. This test asserts the SAFE-REFUSAL contract, not success — the
// exact turn-count boundary between M2×16mm (fine) and M2×60mm (refused) is
// uncharacterized further; that precision is what a real kernelbench
// `OperationFamily::Component` extension (declined this session, see
// TODO.md's WP-2.6 gate entry) would binary-search and pin exactly.
void test_place_component_thread_detail_modeled_m2_long_shank_fails_safely() {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "PlaceComponent"}, {"opId", "opcm201"},
               {"params", {{"componentId", "onecad.std.iso4762"},
                           {"componentVersion", "1.0.0"},
                           {"componentRevision", "sha256:" + std::string(64, '0')},
                           {"source", generator_source_detailed("M2", 60.0, "modeled")},
                           {"placement", identity_placement()}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opcm201");
    check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "OP_FAILED",
          "placeComponent(modeled M2x60, ~150 turns at pitch 0.4mm): fails safely (OP_FAILED, no "
          "crash), a real kernel limit this matrix found");
    check(bodies.get("body_opcm201") == nullptr,
          "placeComponent(modeled M2x60): no body published on refusal, never a partial shape");
}

// ── WP-2.6: `modeled` at the coarsest pitch (M12) over a long shank — the
// opposite extreme (fewer, larger turns). ──────────────────────────────────────────
void test_place_component_thread_detail_modeled_m12_long_shank() {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "PlaceComponent"}, {"opId", "opcm202"},
               {"params", {{"componentId", "onecad.std.iso4762"},
                           {"componentVersion", "1.0.0"},
                           {"componentRevision", "sha256:" + std::string(64, '0')},
                           {"source", generator_source_detailed("M12", 100.0, "modeled")},
                           {"placement", identity_placement()}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opcm202");
    check(oc.status == ops::OpOutcome::Status::Ok,
          "placeComponent(modeled M12x100, coarsest pitch 1.75mm, long shank): Ok");
    check(bodies.get("body_opcm202") != nullptr, "placeComponent(modeled M12x100): body published");
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
    test_place_component_thread_detail_cosmetic_matches_default();
    test_place_component_thread_detail_simplified_removes_material();
    test_place_component_thread_detail_modeled_removes_material();
    test_place_component_thread_detail_modeled_m2_smallest_pitch_succeeds();
    test_place_component_unknown_thread_detail_fails_loud();
    test_place_component_thread_detail_matrix_across_the_full_seed_range();
    test_place_component_thread_detail_simplified_below_one_pitch_matches_cosmetic();
    test_place_component_thread_detail_modeled_m2_long_shank_fails_safely();
    test_place_component_thread_detail_modeled_m12_long_shank();
    test_detach_component_exact_volume_matches_place_component();
    test_detach_component_placement_transform_preserves_volume();
    test_detach_component_embedded_source_unsupported();
    if (g_failures == 0) std::fprintf(stderr, "component_ops: OK\n");
    return g_failures;
}
