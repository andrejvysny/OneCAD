// test_extrude_draft.cpp — roadmap WP0.6, "Draft applied-or-refused".
//
// The work package's FIRST mandated task is a circular-profile red probe: static
// inspection proves `apply_draft` iterates PLANAR faces only, but it does not
// prove what OCCT reports when zero faces were added. If OCCT returned success
// with unchanged geometry the product would claim a draft angle it never applied
// — the P0 class of defect. That probe is `circular_profile_is_applied_or_refused`
// below, and the rest of the file pins the behaviour it establishes.
//
// Every refusal string in `apply_draft` is pinned here. Before this file existed
// the only draft coverage was one square prism asserting `500 < v < 990`, which
// no refusal path and no closed-form volume could ever fail.
//
// In-process via execute_extrude with real OCCT. No framework: exit code ==
// failure count.
#include <cmath>
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include <Bnd_Box.hxx>
#include <BRepBndLib.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS_Shape.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "ops/ExtrudeOp.h"
#include "ops/OpTypes.h"
#include "session/BodyStore.h"
#include "session/ShapeMetrics.h"
#include "util/Cancel.h"

using nlohmann::json;
namespace ops = onecad::ops;
namespace em = onecad::elementmap;
using onecad::session::BodyStore;

namespace {
int g_failures = 0;

void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}

/// Probe finding — printed so the ANSWER, not just the pass, is on the record.
void note(const std::string& msg) { std::printf("  probe: %s\n", msg.c_str()); }

void check_near(double got, double want, double tol, const std::string& msg) {
    if (std::abs(got - want) > tol) {
        std::fprintf(stderr, "FAIL: %s (got %.6f want %.6f)\n", msg.c_str(), got, want);
        ++g_failures;
    }
}

/// A w×h rectangle on XY. World map (u,v)->(-v,u,0): world x∈[-h,0], y∈[0,w].
json rect_sketch(const std::string& sid, double w, double h) {
    json sk;
    sk["sketchId"] = sid;
    sk["plane"] = json{{"kind", "XY"}};
    sk["entities"] = json::array({
        json{{"id", "e1"}, {"type", "Line"}, {"p0", {0, 0}}, {"p1", {w, 0}}},
        json{{"id", "e2"}, {"type", "Line"}, {"p0", {w, 0}}, {"p1", {w, h}}},
        json{{"id", "e3"}, {"type", "Line"}, {"p0", {w, h}}, {"p1", {0, h}}},
        json{{"id", "e4"}, {"type", "Line"}, {"p0", {0, h}}, {"p1", {0, 0}}},
    });
    sk["constraints"] = json::array();
    return sk;
}

/// A circle on XY — the profile whose extrusion has NO planar side face at all.
json circle_sketch(const std::string& sid, double radius) {
    json sk;
    sk["sketchId"] = sid;
    sk["plane"] = json{{"kind", "XY"}};
    sk["entities"] = json::array({
        json{{"id", "c1"}, {"type", "Circle"}, {"center", {0, 0}}, {"radius", radius}},
    });
    sk["constraints"] = json::array();
    return sk;
}

/// A slot: two straight flanks joined by two semicircular caps (the geometry
/// `test_sketch_arc_endpoints.cpp` builds). Its extrusion has BOTH planar and
/// cylindrical side faces, so the eligible set is non-empty but partial — the
/// case the spec calls "mixed planar/curved". Arc angles are RADIANS.
json slot_sketch(const std::string& sid, double half_length, double radius) {
    json sk;
    sk["sketchId"] = sid;
    sk["plane"] = json{{"kind", "XY"}};
    sk["entities"] = json::array({
        json{{"id", "l1"},
             {"type", "Line"},
             {"p0", {-half_length, radius}},
             {"p1", {half_length, radius}}},
        json{{"id", "l2"},
             {"type", "Line"},
             {"p0", {-half_length, -radius}},
             {"p1", {half_length, -radius}}},
        // cap@+x: start (−π/2) sweeping CCW to (+π/2).
        json{{"id", "a1"},
             {"type", "Arc"},
             {"center", {half_length, 0.0}},
             {"radius", radius},
             {"startAngle", -M_PI / 2},
             {"endAngle", M_PI / 2}},
        // cap@−x: start (+π/2) sweeping CCW to (−π/2 ≡ 3π/2).
        json{{"id", "a2"},
             {"type", "Arc"},
             {"center", {-half_length, 0.0}},
             {"radius", radius},
             {"startAngle", M_PI / 2},
             {"endAngle", -M_PI / 2}},
    });
    sk["constraints"] = json::array();
    return sk;
}

struct Ctx {
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;
    ops::OpContext make(BodyStore& b, em::ElementMapPartition& p) {
        return ops::OpContext{b, &sketches, p, &last_sketch, false, json::object(), &cancel};
    }
};

struct Run {
    ops::OpOutcome outcome;
    bool created = false;
    double volume = 0.0;
    double height = 0.0;
    double width_x = 0.0;
    int faces = 0;
};

/// Extrudes `sketch` with the given draft and reports what came back.
Run extrude(const json& sketch, double distance, double draft_deg) {
    BodyStore bodies;
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", sketch});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json op = {{"opType", "Extrude"},
               {"opId", "ope"},
               {"params",
                {{"sketchId", "sk1"},
                 {"distance", distance},
                 {"draftAngleDeg", draft_deg},
                 {"extrudeMode", "Blind"},
                 {"booleanMode", "NewBody"}}}};
    Run run;
    run.outcome = ops::execute_extrude(ctx, op, "ope");
    run.created = bodies.contains("body_ope");
    if (run.created) {
        const TopoDS_Shape& shape = bodies.get("body_ope")->geom;
        run.volume = onecad::session::shape_volume(shape);
        Bnd_Box box;
        BRepBndLib::Add(shape, box);
        if (!box.IsVoid()) {
            double xmin, ymin, zmin, xmax, ymax, zmax;
            box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
            run.height = zmax - zmin;
            run.width_x = xmax - xmin;
        }
        for (TopExp_Explorer exp(shape, TopAbs_FACE); exp.More(); exp.Next()) ++run.faces;
    }
    return run;
}

// ── The mandated red probe ──────────────────────────────────────────────────

/// A circular profile extrudes to a cylinder whose only side face is CURVED, so
/// `apply_draft` finds zero eligible faces. The requirement is binary: either a
/// proven taper, or a NAMED refusal. What must never happen is the third
/// outcome — success with geometry identical to the undrafted prism, which would
/// report a draft angle that was never applied.
void circular_profile_is_applied_or_refused() {
    const Run drafted = extrude(circle_sketch("sk1", 5.0), 10.0, 10.0);
    const Run straight = extrude(circle_sketch("sk1", 5.0), 10.0, 0.0);

    check(straight.created, "circular probe: the undrafted control builds");
    const bool refused = drafted.outcome.status == ops::OpOutcome::Status::Failed;
    if (refused) {
        note("circular profile REFUSES: " + drafted.outcome.error_message);
        check(!drafted.created, "circular probe: a refusal publishes no body");
        check(drafted.outcome.error_code == "OP_FAILED",
              "circular probe: refusal carries a stable code, got '" + drafted.outcome.error_code + "'");
        check(drafted.outcome.error_message == "Extrude draft refused: no eligible planar side faces",
              "circular probe: refusal is NAMED, got '" + drafted.outcome.error_message + "'");
        return;
    }
    // Not refused ⇒ the taper must be real. Silent success at the undrafted
    // volume is the P0 defect this probe exists to catch.
    note("circular profile APPLIES a taper: volume " + std::to_string(drafted.volume) +
         " vs undrafted " + std::to_string(straight.volume));
    check(drafted.created, "circular probe: a non-refusal must publish a body");
    check(std::abs(drafted.volume - straight.volume) > 1e-6,
          "circular probe: SILENT NO-OP — draft reported success with the undrafted volume");
}

// ── Applied: closed form, both signs ────────────────────────────────────────

/// Frustum of a square pyramid: V = h/3 · (A₁ + A₂ + √(A₁A₂)).
double frustum_volume(double base_side, double top_side, double height) {
    const double a1 = base_side * base_side;
    const double a2 = top_side * top_side;
    return height / 3.0 * (a1 + a2 + std::sqrt(a1 * a2));
}

/// The neutral plane is the SKETCH plane, so the base keeps its 10×10 footprint
/// and the top shrinks by `h·tan(angle)` per side. Asserting the closed form
/// (not a range) is what makes a wrong angle, a wrong neutral plane or a
/// one-sided taper detectable.
void square_prism_taper_matches_the_closed_form() {
    const double h = 10.0;
    const double base = 10.0;
    const double inset = h * std::tan(10.0 * M_PI / 180.0);
    const Run positive = extrude(rect_sketch("sk1", base, base), h, 10.0);
    check(positive.outcome.status == ops::OpOutcome::Status::Ok, "draft +10°: Ok");
    check(positive.created, "draft +10°: body created");
    if (positive.created) {
        check_near(positive.volume, frustum_volume(base, base - 2.0 * inset, h), 0.5,
                   "draft +10°: volume equals the closed-form frustum");
        check_near(positive.height, h, 1e-6, "draft +10°: height is unchanged by the taper");
        // Direction: the drafted solid is NARROWER than the prism at its widest,
        // never wider — a sign flip would show up here and nowhere in a volume range.
        check_near(positive.width_x, base, 1e-6,
                   "draft +10°: the neutral (sketch) plane keeps the base footprint");
    }

    // The opposite sign must taper the other way, and by the same amount.
    const Run negative = extrude(rect_sketch("sk1", base, base), h, -10.0);
    check(negative.outcome.status == ops::OpOutcome::Status::Ok, "draft -10°: Ok");
    if (negative.created && positive.created) {
        check_near(negative.volume, frustum_volume(base, base + 2.0 * inset, h), 0.5,
                   "draft -10°: volume equals the outward closed-form frustum");
        check(negative.volume > positive.volume,
              "draft -10°: the sign genuinely reverses the taper");
    }
}

// ── Mixed and near-limit ────────────────────────────────────────────────────

/// A slot profile — two straight flanks and two semicircular caps.
///
/// FINDING, recorded rather than asserted: at the BRep level this is NOT yet a
/// mixed profile. The probe reports 28 faces on the undrafted solid (2 flanks +
/// 24 cap segments + top + bottom) and a volume 0.19% under the analytic
/// `(400 + 25π) · 10`, which is exactly a 24-gon inscribed in the cap circles. So
/// an `Arc` entity still reaches the BRep as a polyline while a full `Circle`
/// stays analytic (see the circular probe above, which finds a genuine cylinder).
/// Every side face here is therefore planar and eligible, which is why the draft
/// applies rather than partially applying.
///
/// The assertion stands either way — applied or NAMED-refused, never a silent
/// no-op — and becomes a true mixed-profile probe for free once the exact
/// analytic path covers plain arc profiles (roadmap Phase 2 residual).
void mixed_planar_and_curved_profile_is_applied_or_refused() {
    const Run drafted = extrude(slot_sketch("sk1", 20.0, 5.0), 10.0, 10.0);
    const Run straight = extrude(slot_sketch("sk1", 20.0, 5.0), 10.0, 0.0);
    check(straight.created, "mixed profile: the undrafted control builds");
    if (drafted.outcome.status == ops::OpOutcome::Status::Failed) {
        note("mixed profile REFUSES: " + drafted.outcome.error_message);
        check(!drafted.created, "mixed profile: a refusal publishes no body");
        check(drafted.outcome.error_message.rfind("Extrude draft refused:", 0) == 0,
              "mixed profile: refusal is NAMED, got '" + drafted.outcome.error_message + "'");
        return;
    }
    note("mixed profile APPLIES a taper: volume " + std::to_string(drafted.volume) +
         " vs undrafted " + std::to_string(straight.volume) + " over " +
         std::to_string(straight.faces) + " undrafted faces");
    check(drafted.created, "mixed profile: a non-refusal must publish a body");
    check(std::abs(drafted.volume - straight.volume) > 1e-6,
          "mixed profile: SILENT NO-OP — success at the undrafted volume");
}

/// Near the limit the taper self-intersects long before the top face closes. The
/// requirement is a safe refusal or a valid solid — never a crash, and never a
/// published invalid shape.
void near_limit_angle_refuses_safely() {
    for (const double angle : {89.0, -89.0}) {
        const Run run = extrude(rect_sketch("sk1", 10, 10), 10.0, angle);
        const std::string label = "draft " + std::to_string(static_cast<int>(angle)) + "°";
        if (run.outcome.status == ops::OpOutcome::Status::Failed) {
            check(!run.created, label + ": a refusal publishes no body");
            check(!run.outcome.error_message.empty(), label + ": refusal carries a message");
        } else {
            check(run.created, label + ": a non-refusal must publish a body");
            check(run.volume > 0.0, label + ": a published solid has positive volume");
        }
    }
}

/// Below the epsilon the draft is a no-op BY CONTRACT — and must therefore stay a
/// clean straight prism rather than reaching the refusal paths at all.
void a_sub_epsilon_angle_is_a_clean_no_op() {
    const Run run = extrude(rect_sketch("sk1", 10, 10), 10.0, 1e-9);
    check(run.outcome.status == ops::OpOutcome::Status::Ok, "sub-epsilon draft: Ok");
    check(run.created, "sub-epsilon draft: body created");
    if (run.created) check_near(run.volume, 1000.0, 1e-6, "sub-epsilon draft: straight prism");
}

/// The same request twice yields the same solid: a taper that drifted between
/// runs would make every volume assertion above unfalsifiable.
void draft_is_deterministic() {
    const Run first = extrude(rect_sketch("sk1", 10, 10), 10.0, 10.0);
    const Run second = extrude(rect_sketch("sk1", 10, 10), 10.0, 10.0);
    check(first.created && second.created, "determinism: both runs build");
    if (first.created && second.created) {
        check_near(second.volume, first.volume, 1e-9, "determinism: identical volume");
    }
}

}  // namespace

int main() {
    circular_profile_is_applied_or_refused();
    square_prism_taper_matches_the_closed_form();
    mixed_planar_and_curved_profile_is_applied_or_refused();
    near_limit_angle_refuses_safely();
    a_sub_epsilon_angle_is_a_clean_no_op();
    draft_is_deterministic();
    if (g_failures == 0) std::printf("extrude draft semantics passed\n");
    return g_failures;
}
