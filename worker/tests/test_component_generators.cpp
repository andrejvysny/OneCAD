// test_component_generators.cpp — Component Library WP-A1 + WP-F2: the
// per-family generator dispatch (`ComponentGenerators.h`) and the seed
// catalog's dimension tables (`FastenerTables.h`,
// `MachineElementTables.h`), in-process via `ops::execute_place_component`
// (real OCCT). No framework: exit code == failure count. Mirrors
// `test_component_ops.cpp`'s shape.
//
// EVERY expected dimension below is hardcoded FROM THE SOURCE (the BOLTS
// column, the ISO table, the NEMA frame), never read back out of the table
// headers — a test that asks the table what the table says proves nothing.
// This is spec §6.5's
// "spot-check harness asserting key dimensions per size against a reference
// source", and it is why a typo in a row fails here rather than shipping.
#include <cmath>
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
void check_near(double got, double want, double tol, const std::string& msg) {
    if (std::abs(got - want) > tol) {
        std::fprintf(stderr, "FAIL: %s (got %.4f want %.4f)\n", msg.c_str(), got, want);
        ++g_failures;
    }
}

constexpr double kPi = 3.14159265358979323846;

/// Volume of a regular hexagonal prism given the ACROSS-FLATS width `s` —
/// `(√3/2)·s²·h`. Written out rather than reused from the generator so the two
/// derivations stay independent.
double hex_prism_volume(double s, double h) { return (std::sqrt(3.0) / 2.0) * s * s * h; }

/// Volume of a spherical cap of base radius `a` and height `k`.
double dome_volume(double a, double k) {
    const double r = (a * a + k * k) / (2.0 * k);
    return kPi * k * k * (3.0 * r - k) / 3.0;
}

double cyl_volume(double d, double h) { return kPi * (d / 2.0) * (d / 2.0) * h; }

/// Volume of the stepped bearing ring: the full boundary annulus minus the
/// two face reliefs. The 0.25 / 0.15 fractions are the generator's declared
/// RENDERING constants, re-typed here (not read from it) so this stays an
/// independent derivation; the boundary dimensions come from the ISO 15
/// series, like every other expectation in this file.
double bearing_volume(double bore, double od, double width) {
    const double ri = bore / 2.0;
    const double ro = od / 2.0;
    const double t = (ro - ri) * 0.25;
    const double g = width * 0.15;
    return kPi * (ro * ro - ri * ri) * width -
           2.0 * kPi * ((ro - t) * (ro - t) - (ri + t) * (ri + t)) * g;
}

/// Volume of the stepper motor: body block + pilot boss + the shaft's run
/// BEYOND the boss (the rest of it is inside the boss already) − four blind
/// mounting holes.
double motor_volume(double frame, double body_length, double pilot_d, double pilot_h,
                    double shaft_d, double shaft_l, double hole_d, double hole_depth) {
    return frame * frame * body_length + cyl_volume(pilot_d, pilot_h) +
           cyl_volume(shaft_d, shaft_l - pilot_h) - 4.0 * cyl_volume(hole_d, hole_depth);
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
/// is the published body's volume when it published one, and `metrics_out`
/// (optional) its full metrics — the bounding box is how a face RADIUS gets
/// pinned, which volume alone cannot do.
ops::OpOutcome place(const std::string& op_id, const json& source, double& volume_out,
                     onecad::session::ShapeMetrics* metrics_out = nullptr) {
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
    if (metrics_out != nullptr && rec != nullptr) {
        *metrics_out = onecad::session::compute_shape_metrics(rec->geom);
    }
    return oc;
}

// ── ISO 7380-1 button head M6×20: dome (dk 10.5, k 3.3) + shank (Ø6 × 20). ──
void test_iso7380_button_head_exact_volume() {
    double v = 0.0;
    const ops::OpOutcome oc = place("g7380", sized_source("iso7380", "M6", 20.0), v);
    check(oc.status == ops::OpOutcome::Status::Ok, "iso7380 M6x20: Ok");
    check_near(v, dome_volume(10.5 / 2.0, 3.3) + cyl_volume(6.0, 20.0), 0.5,
               "iso7380 M6x20: dome + shank volume");
}

// ── ISO 4017 fully-threaded hex screw M8×30: hex head (s 13, k 5.3) + shank. ──
void test_iso4017_hex_screw_exact_volume() {
    double v = 0.0;
    const ops::OpOutcome oc = place("g4017", sized_source("iso4017", "M8", 30.0), v);
    check(oc.status == ops::OpOutcome::Status::Ok, "iso4017 M8x30: Ok");
    check_near(v, hex_prism_volume(13.0, 5.3) + cyl_volume(8.0, 30.0), 0.5,
               "iso4017 M8x30: hex head + shank volume");
}

// ── ISO 4014 shares 4017's head/shank geometry at the same size — the two
// standards differ ONLY in the threaded run, which `cosmetic` does not render.
// A difference here would mean the tables disagree about `k`/`s`/`d1`. ───────
void test_iso4014_cosmetic_matches_iso4017() {
    double v14 = 0.0, v17 = 0.0;
    place("g4014c", sized_source("iso4014", "M8", 30.0), v14);
    place("g4017c", sized_source("iso4017", "M8", 30.0), v17);
    check_near(v14, v17, 1e-6, "iso4014 == iso4017 at cosmetic detail (same head/shank columns)");
}

// ── The partial/full thread split is REAL: at length 40 > b1 18, ISO 4014
// threads only 18 mm from the tip while ISO 4017 threads all 40, so 4014 keeps
// strictly more material. Both stay below the unthreaded blank. ──────────────
void test_iso4014_threads_less_than_iso4017() {
    double cosmetic = 0.0, v14 = 0.0, v17 = 0.0;
    place("g4014b", sized_source("iso4014", "M6", 40.0), cosmetic);
    place("g4014s", sized_source("iso4014", "M6", 40.0, "simplified"), v14);
    place("g4017s", sized_source("iso4017", "M6", 40.0, "simplified"), v17);
    check(v14 < cosmetic, "iso4014 simplified removes material");
    check(v17 < v14, "iso4017 (fully threaded) removes MORE than iso4014 (b1 = 18 of 40 mm)");
}

// ── ISO 4032 hex nut M6: hex prism (s 10, m 5.2) minus a Ø6 through bore. ────
void test_iso4032_hex_nut_exact_volume() {
    double v = 0.0;
    const ops::OpOutcome oc = place("g4032", sized_source("iso4032", "M6", 20.0), v);
    check(oc.status == ops::OpOutcome::Status::Ok, "iso4032 M6: Ok");
    check_near(v, hex_prism_volume(10.0, 5.2) - cyl_volume(6.0, 5.2), 0.5,
               "iso4032 M6: hex prism minus bore");
}

// ── ISO 7089 plain washer M6: annulus 6.4 / 12 × 1.6. ───────────────────────
void test_iso7089_washer_exact_volume() {
    double v = 0.0;
    const ops::OpOutcome oc = place("g7089", sized_source("iso7089", "M6", 20.0), v);
    check(oc.status == ops::OpOutcome::Status::Ok, "iso7089 M6: Ok");
    check_near(v, cyl_volume(12.0, 1.6) - cyl_volume(6.4, 1.6), 0.2,
               "iso7089 M6: annulus volume");
}

// ── ISO 7093-1 large-series washer M6: annulus 6.4 / 18 × 1.6 — the large
// series' whole point is the bigger outer Ø at the same bore. ───────────────
void test_iso7093_washer_exact_volume() {
    double v = 0.0;
    const ops::OpOutcome oc = place("g7093", sized_source("iso7093", "M6", 20.0), v);
    check(oc.status == ops::OpOutcome::Status::Ok, "iso7093 M6: Ok");
    check_near(v, cyl_volume(18.0, 1.6) - cyl_volume(6.4, 1.6), 0.3,
               "iso7093 M6: large-series annulus volume");
}

// ── ISO 7089's M10 bore is 10.5, NOT the 10.0 the BOLTS source lists (a slip
// there — a 10.0 bore would not pass an M10 bolt). Pinned so the correction
// cannot be silently reverted by a future re-import. ────────────────────────
void test_iso7089_m10_bore_is_the_corrected_value() {
    double v = 0.0;
    place("g7089m10", sized_source("iso7089", "M10", 20.0), v);
    check_near(v, cyl_volume(20.0, 2.0) - cyl_volume(10.5, 2.0), 0.2,
               "iso7089 M10: bore is 10.5 (ISO 7089 / DIN 125 A), not BOLTS' 10.0");
}

// ── ISO 15 deep-groove bearing 608: the boundary dimensions 8 × 22 × 7 are
// pinned twice over — the bounding box fixes the OD face radius at 11.0 and
// the width at 7.0, and the volume fixes the bore, which no bbox can see. ───
void test_iso15_608_exact_dimensions_and_volume() {
    double v = 0.0;
    onecad::session::ShapeMetrics m;
    const ops::OpOutcome oc = place("g15a", param_source("iso15", {{"code", "608"}}), v, &m);
    check(oc.status == ops::OpOutcome::Status::Ok, "iso15 608: Ok");
    check_near(m.bbox_max[0], 11.0, 1e-4, "iso15 608: OD face radius is 11.0 (D = 22)");
    check_near(m.bbox_min[0], -11.0, 1e-4, "iso15 608: OD face radius is 11.0 the other way");
    check_near(m.bbox_min[2], 0.0, 1e-4, "iso15 608: seats at z = 0");
    check_near(m.bbox_max[2], 7.0, 1e-4, "iso15 608: width is 7.0 (B)");
    check_near(v, bearing_volume(8.0, 22.0, 7.0), 0.5, "iso15 608: stepped-ring volume");
}

// ── 6802 is the thin-section extreme of the seeded series (15 × 24 × 5): a
// 4.5 mm radial wall, where a race/relief fraction that overshot would fold
// the profile on itself. ───────────────────────────────────────────────────
void test_iso15_6802_thin_section_exact_volume() {
    double v = 0.0;
    onecad::session::ShapeMetrics m;
    const ops::OpOutcome oc = place("g15b", param_source("iso15", {{"code", "6802"}}), v, &m);
    check(oc.status == ops::OpOutcome::Status::Ok, "iso15 6802: Ok");
    check_near(m.bbox_max[0], 12.0, 1e-4, "iso15 6802: OD face radius is 12.0 (D = 24)");
    check_near(m.bbox_max[2], 5.0, 1e-4, "iso15 6802: width is 5.0 (B)");
    check_near(v, bearing_volume(15.0, 24.0, 5.0), 0.5, "iso15 6802: stepped-ring volume");
}

// ── An instance that names no code takes the family's DOCUMENTED default
// (608, the same one the seed package declares) — the library's own preview
// lane sends no params at all. An absent param defaulting is not the
// substitution invariant 4 forbids; an unknown one being swapped would be. ──
void test_iso15_defaults_to_608_when_no_code_is_sent() {
    double defaulted = 0.0, explicit_608 = 0.0;
    check(place("g15d", param_source("iso15", json::object()), defaulted).status ==
              ops::OpOutcome::Status::Ok,
          "iso15 with no params: Ok");
    place("g15e", param_source("iso15", {{"code", "608"}}), explicit_608);
    check_near(defaulted, explicit_608, 1e-9, "iso15 default code is 608");
}

// ── A bearing code the seeded series does not carry is refused with the
// codes that exist — 6203 is a real bearing, which is exactly why guessing
// the nearest seeded one (6202, 5 mm smaller in OD) would be a silent wrong
// part. ────────────────────────────────────────────────────────────────────
void test_unknown_bearing_code_fails_loud() {
    double v = 0.0;
    const ops::OpOutcome oc = place("g15bad", param_source("iso15", {{"code", "6203"}}), v);
    check(oc.status == ops::OpOutcome::Status::Failed, "iso15 6203: Failed");
    check(oc.error_code == "OP_FAILED", "iso15 6203: OP_FAILED");
    check(oc.error_message.find("6203") != std::string::npos,
          "iso15 6203: names the requested code");
    check(oc.error_message.find("6202") != std::string::npos,
          "iso15 6203: lists the known codes");
}

// ── Every seeded bearing code builds a publishable solid. ───────────────────
void test_every_bearing_code_builds() {
    int n = 0;
    for (const char* code : {"625", "608", "6000", "6001", "6200", "6201", "6202", "6802"}) {
        double v = 0.0;
        const ops::OpOutcome oc =
            place("g15all" + std::to_string(n++), param_source("iso15", {{"code", code}}), v);
        check(oc.status == ops::OpOutcome::Status::Ok, std::string("iso15 ") + code + ": builds");
        check(v > 0.0, std::string("iso15 ") + code + ": positive volume");
    }
}

// ── NEMA 17 at a 40 mm body: 42.3 mm frame, Ø22 × 2 pilot boss, Ø5 × 24
// shaft, four Ø3 blind holes 4.5 deep on the 31 mm square. The bbox pins the
// frame size and the shaft's reach through the faceplate (z < 0). ──────────
void test_nema17_exact_dimensions_and_volume() {
    double v = 0.0;
    onecad::session::ShapeMetrics m;
    const ops::OpOutcome oc =
        place("gn17", param_source("nema17", {{"length", 40.0}}), v, &m);
    check(oc.status == ops::OpOutcome::Status::Ok, "nema17 L40: Ok");
    check_near(m.bbox_max[0], 42.3 / 2.0, 1e-4, "nema17: frame is 42.3 mm square");
    check_near(m.bbox_min[1], -42.3 / 2.0, 1e-4, "nema17: frame is 42.3 mm square in Y too");
    check_near(m.bbox_min[2], -24.0, 1e-4, "nema17: shaft reaches 24 mm past the faceplate");
    check_near(m.bbox_max[2], 40.0, 1e-4, "nema17: the body IS the free length");
    check_near(v, motor_volume(42.3, 40.0, 22.0, 2.0, 5.0, 24.0, 3.0, 4.5), 0.5,
               "nema17 L40: block + boss + shaft − 4 mounting holes");
}

// ── NEMA 23 at a 50 mm body: 56.4 mm frame, Ø38.1 × 1.6 pilot, Ø6.35 × 21
// shaft, four Ø5.1 CLEARANCE holes 6.0 deep on the 47.14 mm square. ────────
void test_nema23_exact_dimensions_and_volume() {
    double v = 0.0;
    onecad::session::ShapeMetrics m;
    const ops::OpOutcome oc =
        place("gn23", param_source("nema23", {{"length", 50.0}}), v, &m);
    check(oc.status == ops::OpOutcome::Status::Ok, "nema23 L50: Ok");
    check_near(m.bbox_max[0], 56.4 / 2.0, 1e-4, "nema23: frame is 56.4 mm square");
    check_near(m.bbox_min[2], -21.0, 1e-4, "nema23: shaft reaches 21 mm past the faceplate");
    check_near(m.bbox_max[2], 50.0, 1e-4, "nema23: the body IS the free length");
    check_near(v, motor_volume(56.4, 50.0, 38.1, 1.6, 6.35, 21.0, 5.1, 6.0), 0.5,
               "nema23 L50: block + boss + shaft − 4 mounting holes");
}

// ── A motor that names no length takes the FRAME's default (40 mm), not the
// screw-shaped 20 mm the op layer defaults `length` to — the seed package
// declares the same 40, so the two cannot drift. ───────────────────────────
void test_nema_default_body_length_is_the_frames_own() {
    double defaulted = 0.0, explicit_40 = 0.0;
    check(place("gn17d", param_source("nema17", json::object()), defaulted).status ==
              ops::OpOutcome::Status::Ok,
          "nema17 with no params: Ok");
    place("gn17e", param_source("nema17", {{"length", 40.0}}), explicit_40);
    check_near(defaulted, explicit_40, 1e-9, "nema17 default body length is 40 mm");
}

// ── A body length below the frame's minimum is refused, never clamped: a
// 10 mm NEMA 17 is not a smaller motor, it is not a motor. ─────────────────
void test_nema_body_length_below_the_minimum_fails_loud() {
    double v = 0.0;
    const ops::OpOutcome oc = place("gn17bad", param_source("nema17", {{"length", 10.0}}), v);
    check(oc.status == ops::OpOutcome::Status::Failed, "nema17 L10: Failed");
    check(oc.error_message.find("minimum") != std::string::npos,
          "nema17 L10: says it is below the minimum");
}

// ── An unknown generatorId fails LOUD and names the registered ids. Before
// WP-A1 this silently built an ISO 4762 socket cap — the exact
// silent-substitution failure spec §0 invariant 4 forbids. ──────────────────
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

// ── A size the family's standard does not define fails loud too — ISO 7380-1
// starts at M3, so M2 is an error, never the nearest size. ──────────────────
void test_unknown_thread_in_a_new_family_fails_loud() {
    double v = 0.0;
    const ops::OpOutcome oc = place("g7380bad", sized_source("iso7380", "M2", 10.0), v);
    check(oc.status == ops::OpOutcome::Status::Failed, "iso7380 M2: Failed (not defined by ISO 7380-1)");
    check(oc.error_message.find("M2") != std::string::npos, "iso7380 M2: names the requested size");
    check(oc.error_message.find("M3") != std::string::npos, "iso7380 M2: lists the known sizes");
}

// ── Every seeded size of every family builds a publishable single solid at
// cosmetic detail. This is the table-extremes gate in its cheapest form: a row
// that exists but cannot be built is a broken row. ─────────────────────────
void test_every_seeded_size_builds() {
    struct Family {
        const char* id;
        std::vector<const char*> threads;
    };
    const std::vector<Family> families = {
        {"iso4762", {"M2", "M2.5", "M3", "M4", "M5", "M6", "M8", "M10", "M12"}},
        {"iso7380", {"M3", "M4", "M5", "M6", "M8", "M10", "M12"}},
        {"iso4014", {"M3", "M4", "M5", "M6", "M8", "M10", "M12"}},
        {"iso4017", {"M2", "M2.5", "M3", "M4", "M5", "M6", "M8", "M10", "M12"}},
        {"iso4032", {"M2", "M2.5", "M3", "M4", "M5", "M6", "M8", "M10", "M12"}},
        {"iso7089", {"M2", "M2.5", "M3", "M4", "M5", "M6", "M8", "M10", "M12"}},
        {"iso7093", {"M3", "M4", "M5", "M6", "M8", "M10", "M12"}},
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

// ── Table extremes × thread detail for the three NEW threaded families
// (spec §10's "robustness cases from the table extremes"). WP-2.6 established
// this mechanism for ISO 4762 after finding kernelbench's case-v2 schema is
// architecturally fillet-only; this is the same mechanism, applied to the
// families WP-A1 seeds. Length is 4×d1 per size — a proportional synthetic
// value, WP-2.6's own convention, chosen so even the finest pitch clears
// several grooves/turns. ───────────────────────────────────────────────────
void test_thread_detail_extremes_for_the_new_families() {
    struct Case {
        const char* id;
        const char* thread;
        double d1;
    };
    // Smallest and largest declared size per family, plus a midpoint.
    const std::vector<Case> cases = {
        {"iso7380", "M3", 3.0},  {"iso7380", "M6", 6.0},  {"iso7380", "M12", 12.0},
        {"iso4014", "M3", 3.0},  {"iso4014", "M6", 6.0},  {"iso4014", "M12", 12.0},
        {"iso4017", "M2", 2.0},  {"iso4017", "M6", 6.0},  {"iso4017", "M12", 12.0},
    };
    int n = 0;
    for (const Case& c : cases) {
        for (const char* detail : {"cosmetic", "simplified", "modeled"}) {
            double v = 0.0;
            const std::string op_id = "gx" + std::to_string(n++);
            const ops::OpOutcome oc =
                place(op_id, sized_source(c.id, c.thread, 4.0 * c.d1, detail), v);
            const std::string what = std::string(c.id) + " " + c.thread + " " + detail;
            check(oc.status == ops::OpOutcome::Status::Ok, what + ": Ok");
            check(std::isfinite(v) && v > 0.0, what + ": finite positive volume");
        }
    }
}

// ── The registered-id list is the seed catalog, and the unknown-id message
// quotes it — a new family that forgets to register would leave this stale. ──
void test_known_generator_ids_covers_the_seed_catalog() {
    const std::string ids = ops::known_generator_ids();
    for (const char* id : {"iso15", "iso4014", "iso4017", "iso4032", "iso4762", "iso7089",
                           "iso7093", "iso7380", "nema17", "nema23"}) {
        check(ids.find(id) != std::string::npos,
              std::string("known_generator_ids lists ") + id);
    }
}

}  // namespace

int main() {
    test_iso7380_button_head_exact_volume();
    test_iso4017_hex_screw_exact_volume();
    test_iso4014_cosmetic_matches_iso4017();
    test_iso4014_threads_less_than_iso4017();
    test_iso4032_hex_nut_exact_volume();
    test_iso7089_washer_exact_volume();
    test_iso7093_washer_exact_volume();
    test_iso7089_m10_bore_is_the_corrected_value();
    test_unknown_generator_id_fails_loud();
    test_unknown_thread_in_a_new_family_fails_loud();
    test_every_seeded_size_builds();
    test_thread_detail_extremes_for_the_new_families();
    test_iso15_608_exact_dimensions_and_volume();
    test_iso15_6802_thin_section_exact_volume();
    test_iso15_defaults_to_608_when_no_code_is_sent();
    test_unknown_bearing_code_fails_loud();
    test_every_bearing_code_builds();
    test_nema17_exact_dimensions_and_volume();
    test_nema23_exact_dimensions_and_volume();
    test_nema_default_body_length_is_the_frames_own();
    test_nema_body_length_below_the_minimum_fails_loud();
    test_known_generator_ids_covers_the_seed_catalog();
    if (g_failures == 0) std::fprintf(stderr, "test_component_generators: all checks passed\n");
    return g_failures;
}
