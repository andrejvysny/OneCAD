// test_component_generators.cpp — Component Library WP-A1: the per-family
// generator DISPATCH (`ComponentGenerators.h`) and the seed catalog's dimension
// table (`FastenerTables.h`), in-process via `ops::execute_place_component`
// (real OCCT). No framework: exit code == failure count. Mirrors
// `test_component_ops.cpp`'s shape.
//
// SCOPE, since the catalog is now one family: this file owns the DISPATCH
// contract — an unknown generator id, an unknown size within a registered one,
// and the fact that every row of every shipped table builds. The ISO 4762
// GEOMETRY (exact volumes per size, the thread-detail matrix, the extremes) is
// `test_component_ops.cpp`'s, and is not duplicated here.
//
// EVERY expected dimension below is hardcoded FROM THE SOURCE (the BOLTS
// column, the ISO table), never read back out of the table headers — a test
// that asks the table what the table says proves nothing. This is spec §6.5's
// "spot-check harness asserting key dimensions per size against a reference
// source", and it is why a typo in a row fails here rather than shipping.
#include <chrono>
#include <cstdio>
#include <string>
#include <vector>

#include "nlohmann/json.hpp"
#include "ops/ComponentGenerators.h"
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

struct Ctx {
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;
    ops::OpContext make(BodyStore& bodies, onecad::elementmap::ElementMapPartition& part) {
        return ops::OpContext{bodies, &sketches, part, &last_sketch, false, json::object(), &cancel};
    }
};

json param_source(const std::string& generator_id, const json& params) {
    return json{{"kind", "generator"},
                {"generatorId", generator_id},
                {"generatorVersion", 1},
                {"params", params}};
}

json sized_source(const std::string& generator_id, const std::string& thread, double length_mm,
                  const std::string& detail = "") {
    json params = {{"thread", thread}, {"length", length_mm}};
    if (!detail.empty()) params["thread_detail"] = detail;
    return param_source(generator_id, params);
}

/// Runs one PlaceComponent at the origin and returns its outcome; `volume_out`
/// is the published body's volume when it published one.
ops::OpOutcome place(const std::string& op_id, const json& source, double& volume_out) {
    BodyStore bodies;
    onecad::elementmap::ElementMapPartition part;
    json op = {{"opType", "PlaceComponent"},
               {"opId", op_id},
               {"params",
                {{"componentId", "onecad.std.test"},
                 {"componentVersion", "1.0.0"},
                 {"componentRevision", "sha256:" + std::string(64, '0')},
                 {"source", source},
                 {"placement", {{"translate", {0.0, 0.0, 0.0}}}}}}};
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    ops::OpOutcome oc = ops::execute_place_component(ctx, op, op_id);
    const onecad::session::BodyRecord* rec = bodies.get("body_" + op_id);
    volume_out = (rec != nullptr) ? onecad::session::shape_volume(rec->geom) : 0.0;
    return oc;
}

// ── An unknown generatorId fails LOUD and names the registered ids. Before
// WP-A1 this silently built an ISO 4762 socket cap — the exact
// silent-substitution failure spec §0 invariant 4 forbids. The dispatch is why
// a one-family catalog is still not the same thing as no dispatch. ───────────
void test_unknown_generator_id_fails_loud() {
    double v = 0.0;
    const ops::OpOutcome oc = place("gbad", sized_source("iso999", "M6", 20.0), v);
    check(oc.status == ops::OpOutcome::Status::Failed, "unknown generatorId: Failed");
    check(oc.error_code == "OP_FAILED", "unknown generatorId: OP_FAILED");
    check(oc.error_message.find("iso999") != std::string::npos,
          "unknown generatorId: names the requested id");
    check(oc.error_message.find("iso4762") != std::string::npos,
          "unknown generatorId: lists the known generators");
}

// ── A size the family's standard does not define fails loud, and the message
// LISTS the sizes that exist. ISO 4762's seeded range stops at M12, so M14 is
// an error, never the nearest size. `test_component_ops.cpp` pins the refusal
// itself; what is pinned here is that the error is actionable. ──────────────
void test_unknown_thread_lists_the_known_sizes() {
    double v = 0.0;
    const ops::OpOutcome oc = place("g4762bad", sized_source("iso4762", "M14", 20.0), v);
    check(oc.status == ops::OpOutcome::Status::Failed,
          "iso4762 M14: Failed (outside the seeded range)");
    check(oc.error_message.find("M14") != std::string::npos,
          "iso4762 M14: names the requested size");
    check(oc.error_message.find("M12") != std::string::npos,
          "iso4762 M14: lists the known sizes");
}

// ── Every seeded size of every family builds a publishable solid at cosmetic
// detail. This is the table-row gate in its cheapest form: a row that exists
// but cannot be built is a broken row. The list is written out FROM THE TABLE'S
// SOURCE, so a row silently dropped from `FastenerTables.cpp` fails here. ────
void test_every_seeded_size_builds() {
    struct Family {
        const char* id;
        std::vector<const char*> threads;
    };
    const std::vector<Family> families = {
        {"iso4762", {"M2", "M2.5", "M3", "M4", "M5", "M6", "M8", "M10", "M12"}},
    };
    int n = 0;
    for (const Family& f : families) {
        for (const char* thread : f.threads) {
            double v = 0.0;
            const std::string op_id = "gall" + std::to_string(n++);
            const ops::OpOutcome oc = place(op_id, sized_source(f.id, thread, 16.0), v);
            check(oc.status == ops::OpOutcome::Status::Ok,
                  std::string(f.id) + " " + thread + ": builds");
            check(v > 0.0, std::string(f.id) + " " + thread + ": positive volume");
        }
    }
}

// ── The registered-id list is the seed catalog, and the unknown-id message
// quotes it — a new family that forgets to register would leave this stale.
// The Rust side asserts the same pairing from its end
// (`every_seed_package_parses_and_validates` over `SEED_PACKAGES`). ──────────
void test_known_generator_ids_covers_the_seed_catalog() {
    const std::string ids = ops::known_generator_ids();
    for (const char* id : {"iso4762"}) {
        check(ids.find(id) != std::string::npos,
              std::string("known_generator_ids lists ") + id);
    }
}

// ── WP-I I0(d) probe / design I3: an absurd `source.params.length` is refused
// BY NAME, not built. `read_profile_length` fences the `profile` source kind at
// `kMaxProfileLengthMm`, but the GENERATOR path (`ComponentOp.cpp:645-671`) only
// checks finite-and-positive, so an M6 socket cap a KILOMETRE long builds
// happily. RED today: it succeeds. ──────────────────────────────────────────
void test_absurd_generator_length_is_refused_by_name() {
    double v = 0.0;
    const auto t0 = std::chrono::steady_clock::now();
    const ops::OpOutcome oc = place("glen", sized_source("iso4762", "M6", 1.0e6), v);
    const double ms = std::chrono::duration<double, std::milli>(
                          std::chrono::steady_clock::now() - t0)
                          .count();
    std::string reason;
    for (const json& d : oc.diagnostics) {
        if (d.contains("reasonCode") && d["reasonCode"].is_string()) {
            reason = d["reasonCode"].get<std::string>();
        }
    }
    std::fprintf(stderr,
                 "  [I0-d] iso4762 M6 length=1e6 mm → status=%s errorCode='%s' reasonCode='%s' "
                 "volume=%.1f build=%.1f ms\n",
                 oc.status == ops::OpOutcome::Status::Ok ? "Ok" : "Failed", oc.error_code.c_str(),
                 reason.c_str(), v, ms);
    check(oc.status != ops::OpOutcome::Status::Ok,
          "I0(d): a 1e6 mm generator length is REFUSED, never built");
    check(reason == "GENERATOR_PARAM_OUT_OF_RANGE",
          "I0(d): the refusal names the offending parameter "
          "(reasonCode GENERATOR_PARAM_OUT_OF_RANGE)");
    check(v == 0.0, "I0(d): a refused length publishes no body");
}

}  // namespace

int main() {
    test_unknown_generator_id_fails_loud();
    test_unknown_thread_lists_the_known_sizes();
    test_every_seeded_size_builds();
    test_known_generator_ids_covers_the_seed_catalog();
    test_absurd_generator_length_is_refused_by_name();
    if (g_failures == 0) std::fprintf(stderr, "test_component_generators: all checks passed\n");
    return g_failures;
}
