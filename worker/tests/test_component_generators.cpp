// test_component_generators.cpp — Component Library WP-A1: the per-family
// generator DISPATCH (`ComponentGenerators.h`) and the seed catalog's dimension
// table (`FastenerTables.h`), in-process via `ops::execute_place_component`
// (real OCCT). No framework: exit code == failure count. Mirrors
// `test_component_ops.cpp`'s shape.
//
// SCOPE: this file owns the DISPATCH contract — an unknown generator id, an
// unknown size within a registered one, the fact that every row of every shipped
// table builds — and the exact-volume/bbox oracles of ISO 4032 and ISO 7089. The ISO 4762
// GEOMETRY (exact volumes per size, the thread-detail matrix, the extremes) is
// `test_component_ops.cpp`'s, and is not duplicated here.
//
// EVERY expected dimension below is hardcoded FROM THE SOURCE (the BOLTS
// column, the ISO table), never read back out of the table headers — a test
// that asks the table what the table says proves nothing. This is spec §6.5's
// "spot-check harness asserting key dimensions per size against a reference
// source", and it is why a typo in a row fails here rather than shipping.
#include <chrono>
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <Bnd_Box.hxx>
#include <BRepBndLib.hxx>

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
        {"iso4032", {"M2", "M2.5", "M3", "M4", "M5", "M6", "M8", "M10", "M12"}},
        {"iso4762", {"M2", "M2.5", "M3", "M4", "M5", "M6", "M8", "M10", "M12"}},
        {"iso7089", {"M2", "M2.5", "M3", "M4", "M5", "M6", "M8", "M10", "M12"}},
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

// ── ISO 4032 / ISO 7089: exact-volume and bbox oracles. EVERY number below is
// typed from the BOLTS columns (nut.blt `hexagonnut1` d1/s/m_max; washer.blt
// `plainwasher1` d1/d2/s, M10 inner Ø 10.5 per ISO 7089), never read out of
// `FastenerTables`. ──────────────────────────────────────────────────────────
constexpr double kPi = 3.14159265358979323846;

struct PlacedBox {
    double volume = 0.0;
    double xmin = 0, xmax = 0, ymin = 0, ymax = 0, zmin = 0, zmax = 0;
    bool ok = false;
};

PlacedBox place_box(const std::string& op_id, const json& source) {
    PlacedBox out;
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
    const ops::OpOutcome oc = ops::execute_place_component(ctx, op, op_id);
    const onecad::session::BodyRecord* rec = bodies.get("body_" + op_id);
    if (oc.status != ops::OpOutcome::Status::Ok || rec == nullptr) return out;
    out.volume = onecad::session::shape_volume(rec->geom);
    Bnd_Box box;
    BRepBndLib::AddOptimal(rec->geom, box, /*useTriangulation=*/false, /*useShapeTolerance=*/false);
    box.Get(out.xmin, out.ymin, out.zmin, out.xmax, out.ymax, out.zmax);
    out.ok = true;
    return out;
}

json thread_source(const std::string& generator_id, const std::string& thread) {
    return param_source(generator_id, json{{"thread", thread}});
}

bool near_rel(double got, double want, double rel) {
    return std::fabs(got - want) <= rel * std::fabs(want);
}

// Nut: hexagon area (3√3/2)·R² with R = s/√3, times m, minus the π(d/2)²·m bore.
// The real nut's 30° corner chamfer is deliberately NOT modeled (documented in
// ComponentGenerators.cpp), so it is not in the oracle either.
void check_nut(const char* thread, double d, double s, double m) {
    const std::string label = std::string("iso4032 ") + thread;
    const PlacedBox b = place_box(std::string("n") + thread, thread_source("iso4032", thread));
    check(b.ok, label + ": builds");
    if (!b.ok) return;
    const double r = s / std::sqrt(3.0);
    const double want = (1.5 * std::sqrt(3.0) * r * r) * m - kPi * (d / 2.0) * (d / 2.0) * m;
    check(near_rel(b.volume, want, 1e-9), label + ": exact volume (got " +
                                              std::to_string(b.volume) + " want " +
                                              std::to_string(want) + ")");
    // Hexagon with vertices on +X: x spans ±R, y spans ±s/2.
    check(near_rel(b.xmax, r, 1e-6) && near_rel(b.xmin, -r, 1e-6), label + ": bbox x = ±s/√3");
    check(near_rel(b.ymax, s / 2.0, 1e-6) && near_rel(b.ymin, -s / 2.0, 1e-6),
          label + ": bbox y = ±s/2");
    check(std::fabs(b.zmin) < 1e-9 && near_rel(b.zmax, m, 1e-6),
          label + ": occupies z in [0, m]");
}

void check_washer(const char* thread, double d1, double d2, double h) {
    const std::string label = std::string("iso7089 ") + thread;
    const PlacedBox b = place_box(std::string("w") + thread, thread_source("iso7089", thread));
    check(b.ok, label + ": builds");
    if (!b.ok) return;
    const double want = kPi * ((d2 / 2.0) * (d2 / 2.0) - (d1 / 2.0) * (d1 / 2.0)) * h;
    check(near_rel(b.volume, want, 1e-9), label + ": exact volume (got " +
                                              std::to_string(b.volume) + " want " +
                                              std::to_string(want) + ")");
    check(near_rel(b.xmax, d2 / 2.0, 1e-6) && near_rel(b.xmin, -d2 / 2.0, 1e-6) &&
              near_rel(b.ymax, d2 / 2.0, 1e-6) && near_rel(b.ymin, -d2 / 2.0, 1e-6),
          label + ": bbox xy = ±d2/2");
    check(std::fabs(b.zmin) < 1e-9 && near_rel(b.zmax, h, 1e-6),
          label + ": occupies z in [0, h]");
}

void test_nut_exact_volumes() {
    check_nut("M2", 2.0, 4.0, 1.6);
    check_nut("M6", 6.0, 10.0, 5.2);
    check_nut("M8", 8.0, 13.0, 6.8);
    check_nut("M10", 10.0, 16.0, 8.4);
    check_nut("M12", 12.0, 18.0, 10.8);
}

void test_washer_exact_volumes() {
    check_washer("M2", 2.2, 5.0, 0.3);
    check_washer("M6", 6.4, 12.0, 1.6);
    check_washer("M8", 8.4, 16.0, 1.6);
    // The documented correction: BOLTS says 10.0, ISO 7089 / DIN 125 A say 10.5.
    check_washer("M10", 10.5, 20.0, 2.0);
    check_washer("M12", 13.0, 24.0, 2.5);
    const PlacedBox m10 = place_box("w10bore", thread_source("iso7089", "M10"));
    // Volume the BOLTS 10.0 bore would give: π(10² − 5²)·2.
    const double with_slip = kPi * (10.0 * 10.0 - 5.0 * 5.0) * 2.0;
    check(!near_rel(m10.volume, with_slip, 1e-6), "iso7089 M10: bore is 10.5, not BOLTS' 10.0");
}

// A thread the table lacks, and a pinned version the worker does not implement,
// are refused by name — never substituted.
void test_new_families_refuse_unknown_thread_and_version() {
    for (const char* id : {"iso4032", "iso7089"}) {
        double v = 0.0;
        const ops::OpOutcome oc = place(std::string("u") + id, thread_source(id, "M14"), v);
        check(oc.status == ops::OpOutcome::Status::Failed, std::string(id) + " M14: Failed");
        check(oc.error_message.find("M14") != std::string::npos &&
                  oc.error_message.find("M12") != std::string::npos,
              std::string(id) + " M14: names the size and lists known sizes");

        json src = thread_source(id, "M6");
        src["generatorVersion"] = 2;
        const ops::OpOutcome v2 = place(std::string("v2") + id, src, v);
        check(v2.status == ops::OpOutcome::Status::Failed && v2.error_code == "UNSUPPORTED",
              std::string(id) + " version 2: UNSUPPORTED");
        check(v2.error_message.find(id) != std::string::npos &&
                  v2.error_message.find("supported version: 1") != std::string::npos,
              std::string(id) + " version 2: names generator and supported version");
    }
}

// `length` and `thread_detail` are accepted and ignored by nut and washer.
void test_new_families_ignore_length_and_thread_detail() {
    for (const char* id : {"iso4032", "iso7089"}) {
        double a = 0.0, b = 0.0;
        place(std::string("ia") + id, thread_source(id, "M6"), a);
        place(std::string("ib") + id, sized_source(id, "M6", 137.0, "modeled"), b);
        check(a > 0.0 && near_rel(a, b, 1e-12), std::string(id) + ": length/thread_detail ignored");
    }
}

// ── The registered-id list is the seed catalog, and the unknown-id message
// quotes it — a new family that forgets to register would leave this stale.
// The Rust side asserts the same pairing from its end
// (`every_seed_package_parses_and_validates` over `SEED_PACKAGES`). ──────────
void test_known_generator_ids_covers_the_seed_catalog() {
    const std::string ids = ops::known_generator_ids();
    for (const char* id : {"iso4032", "iso4762", "iso7089"}) {
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
    test_nut_exact_volumes();
    test_washer_exact_volumes();
    test_new_families_refuse_unknown_thread_and_version();
    test_new_families_ignore_length_and_thread_detail();
    test_known_generator_ids_covers_the_seed_catalog();
    test_absurd_generator_length_is_refused_by_name();
    if (g_failures == 0) std::fprintf(stderr, "test_component_generators: all checks passed\n");
    return g_failures;
}
