// test_gear_op.cpp — the SCHEMA §7.3 `Gear` OPERATION, in-process via
// `ops::execute_gear` on real OCCT.
//
// `test_gear_tool.cpp` already pins the solid builder, so this file is about
// the OP contract rather than the geometry:
//
//   * NewBody lineage — mints `body_<opId>` (D1), ONE `created` event, and an
//     EMPTY element-map delta. A gear tracks nothing pre-existing, so the
//     partition must come back untouched.
//   * both placement modes — a picked face (axis = the face's INWARD normal,
//     frozen point re-projected and fenced at 1e-3 mm) and an explicit frozen
//     frame.
//   * the recipe-conditional payload and the worker's own re-validation, which
//     is an INDEPENDENT trust boundary: a hand-authored plan like the ones
//     below never passes through the Rust session's validator at all.
//   * the refusal matrix, including the placement fence and the
//     exactly-one-of-face/frame rule.
//
// No framework: exit code == failure count (clamped, see the end of main).
#include <cmath>
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include <BRepPrimAPI_MakeBox.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Shape.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "ops/GearOp.h"
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

constexpr double kPi = 3.14159265358979323846;

int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}

double vol(const TopoDS_Shape& s) { return onecad::session::shape_volume(s); }

TopoDS_Shape box_20_20_25() { return BRepPrimAPI_MakeBox(20.0, 20.0, 25.0).Shape(); }

TopoDS_Shape face_by_center(const TopoDS_Shape& shape, double cx, double cy, double cz) {
    NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    TopoDS_Shape best;
    double best_d2 = -1.0;
    for (int i = 1; i <= faces.Extent(); ++i) {
        const km::ElementDescriptor d = em::ElementMapPartition::describe(faces(i));
        const double dx = d.center.X() - cx, dy = d.center.Y() - cy, dz = d.center.Z() - cz;
        const double d2 = dx * dx + dy * dy + dz * dz;
        if (best_d2 < 0.0 || d2 < best_d2) {
            best_d2 = d2;
            best = faces(i);
        }
    }
    return best;
}

json face_input(const std::string& body_id, const std::string& elem_id, const TopoDS_Shape& face,
                double ax, double ay, double az) {
    return json{{"primary", {{"bodyId", body_id}, {"elementId", elem_id}, {"kind", "face"}}},
                {"intent",
                 {{"kind", "face"},
                  {"descriptor", em::ElementMapPartition::descriptor_to_json(
                                     em::ElementMapPartition::describe(face))}}},
                {"anchor", {{"worldPoint", {ax, ay, az}}}}};
}

struct Ctx {
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;
    ops::OpContext make(BodyStore& bodies, em::ElementMapPartition& part) {
        return ops::OpContext{bodies, &sketches, part, &last_sketch, false, json::object(), &cancel};
    }
};

/// The canonical involute block; `extra` is merged over it.
json involute_block(const json& extra = json::object()) {
    json b = {{"teeth", 20},        {"module", 2.0},   {"height", 5.0},
              {"pressureAngleDeg", 20.0}, {"shift", 0.0},    {"helixAngleDeg", 0.0},
              {"doubleHelix", false},     {"propertiesFromTool", false},
              {"undercut", false},        {"backlash", 0.0}, {"clearance", 0.25},
              {"head", 0.0},              {"sampleCount", 12}};
    b.update(extra);
    return b;
}

/// A Gear op placed on an explicit frozen FRAME at the origin, +Z axis.
json gear_op_frame(const json& block_extra = json::object(),
                   const json& params_extra = json::object()) {
    json params = {{"recipe", "involuteExternal"},
                   {"placement",
                    {{"face", nullptr},
                     {"frame",
                      {{"origin", {0.0, 0.0, 0.0}},
                       {"axis", {0.0, 0.0, 1.0}},
                       {"xDir", {1.0, 0.0, 0.0}}}},
                     {"point", {0.0, 0.0, 0.0}}}},
                   {"involuteExternal", involute_block(block_extra)}};
    params.update(params_extra);
    return json{{"opType", "Gear"},
                {"opId", "opgear"},
                {"inputs", json::array()},
                {"params", params}};
}

/// A Gear op seated on the box's TOP face at (10,10,25).
json gear_op_face(const TopoDS_Shape& host, const json& block_extra = json::object(),
                  const json& point = json::array({10.0, 10.0, 25.0})) {
    const TopoDS_Shape top = face_by_center(host, 10, 10, 25);
    json params = {{"recipe", "involuteExternal"},
                   {"placement",
                    {{"face", face_input("body_1", "el_top", top, 10, 10, 25)},
                     {"frame", nullptr},
                     {"point", point}}},
                   {"involuteExternal", involute_block(block_extra)}};
    return json{{"opType", "Gear"},
                {"opId", "opgear"},
                {"inputs", json::array({face_input("body_1", "el_top", top, 10, 10, 25)})},
                {"params", params}};
}

/// Run a gear op against a fresh store holding the box as `body_1`.
struct RunResult {
    ops::OpOutcome outcome;
    BodyStore bodies;
    em::ElementMapPartition part;
};

void run_expect_ok(const json& op, const char* name,
                   const std::function<void(const ops::OpOutcome&, BodyStore&,
                                            em::ElementMapPartition&)>& then) {
    BodyStore bodies;
    em::ElementMapPartition part;
    bodies.create("body_1", "op0", box_20_20_25());
    Ctx c;
    auto ctx = c.make(bodies, part);
    const ops::OpOutcome out = ops::execute_gear(ctx, op, "opgear");
    if (!out.error_code.empty()) {
        std::fprintf(stderr, "FAIL: %s refused unexpectedly: %s (%s)\n", name,
                     out.error_message.c_str(), out.error_code.c_str());
        ++g_failures;
        return;
    }
    if (!out.needs_repair.empty()) {
        std::fprintf(stderr, "FAIL: %s returned NeedsRepair unexpectedly\n", name);
        ++g_failures;
        return;
    }
    then(out, bodies, part);
}

void run_expect_refusal(const json& op, const char* name, const char* want_code = nullptr) {
    BodyStore bodies;
    em::ElementMapPartition part;
    bodies.create("body_1", "op0", box_20_20_25());
    Ctx c;
    auto ctx = c.make(bodies, part);
    const ops::OpOutcome out = ops::execute_gear(ctx, op, "opgear");
    if (out.error_code.empty()) {
        std::fprintf(stderr, "FAIL: %s was expected to refuse but succeeded\n", name);
        ++g_failures;
        return;
    }
    if (out.error_message.empty()) {
        std::fprintf(stderr, "FAIL: %s refused with no message\n", name);
        ++g_failures;
    }
    if (want_code && out.error_code != want_code) {
        std::fprintf(stderr, "FAIL: %s refused with %s, expected %s\n", name,
                     out.error_code.c_str(), want_code);
        ++g_failures;
    }
    // A refusal must not mint anything.
    if (!out.body_events.empty() || !out.body_ids.empty()) {
        std::fprintf(stderr, "FAIL: %s refused but still reported body lifecycle\n", name);
        ++g_failures;
    }
    if (bodies.get("body_opgear") != nullptr) {
        std::fprintf(stderr, "FAIL: %s refused but still created the body\n", name);
        ++g_failures;
    }
}

}  // namespace

int main() {
    // --- Frame placement: the D1 mint contract. ---
    run_expect_ok(gear_op_frame(), "frame placement",
                  [](const ops::OpOutcome& out, BodyStore& bodies, em::ElementMapPartition& part) {
                      check(out.body_ids.size() == 1, "exactly one body id");
                      check(!out.body_ids.empty() && out.body_ids[0] == "body_opgear",
                            "D1: minted id is body_<opId>");
                      check(out.body_events.size() == 1, "exactly one body event");
                      if (out.body_events.size() == 1) {
                          check(out.body_events[0].kind == "created", "the event is `created`");
                          check(out.body_events[0].body_id == "body_opgear", "event names the mint");
                      }
                      // EMPTY delta: a gear tracks nothing pre-existing, so
                      // apply_history is not involved and the partition must be
                      // untouched.
                      check(out.delta.empty(), "element-map delta is EMPTY");
                      check(out.needs_repair.empty(), "no needsRepair");

                      const auto* rec = bodies.get("body_opgear");
                      check(rec != nullptr, "the body was created in the store");
                      if (rec) {
                          check(vol(rec->geom) > 0.0, "minted body has positive volume");
                          // m=2, z=20 => pitch diameter 40; the body must sit
                          // inside the addendum cylinder (da = 44).
                          check(vol(rec->geom) < kPi * 22.0 * 22.0 * 5.0,
                                "volume within the addendum cylinder");
                      }
                      // The host body is untouched — a gear modifies nothing.
                      const auto* host = bodies.get("body_1");
                      check(host != nullptr && std::abs(vol(host->geom) - 10000.0) < 1e-6,
                            "the host body is untouched");
                      (void)part;
                  });

    // --- Face placement: axis is the INWARD normal, so the gear grows DOWN
    //     from the box's top face into the material, not up into the void. ---
    {
        const TopoDS_Shape host = box_20_20_25();
        run_expect_ok(gear_op_face(host), "face placement",
                      [](const ops::OpOutcome& out, BodyStore& bodies,
                         em::ElementMapPartition& part) {
                          check(!out.body_ids.empty() && out.body_ids[0] == "body_opgear",
                                "face placement mints body_<opId> too");
                          const auto* rec = bodies.get("body_opgear");
                          check(rec != nullptr, "body created");
                          if (rec) {
                              Bnd_Box bb;
                              BRepBndLib::Add(rec->geom, bb);
                              double xmin, ymin, zmin, xmax, ymax, zmax;
                              bb.Get(xmin, ymin, zmin, xmax, ymax, zmax);
                              // Seated at z=25 growing inward (−Z): the body must
                              // occupy [20, 25], never [25, 30].
                              check(zmax <= 25.0 + 1e-6,
                                    "gear does not grow OUT of the seat face");
                              check(zmin >= 20.0 - 1e-6, "gear spans exactly its height inward");
                          }
                          (void)part;
                      });
    }

    // --- Determinism: two runs of the same op agree bit-for-bit on volume. ---
    {
        BodyStore b1, b2;
        em::ElementMapPartition p1, p2;
        b1.create("body_1", "op0", box_20_20_25());
        b2.create("body_1", "op0", box_20_20_25());
        Ctx c1, c2;
        auto x1 = c1.make(b1, p1);
        auto x2 = c2.make(b2, p2);
        const auto o1 = ops::execute_gear(x1, gear_op_frame(), "opgear");
        const auto o2 = ops::execute_gear(x2, gear_op_frame(), "opgear");
        check(o1.error_code.empty() && o2.error_code.empty(), "both determinism runs succeeded");
        const auto* r1 = b1.get("body_opgear");
        const auto* r2 = b2.get("body_opgear");
        check(r1 && r2 && vol(r1->geom) == vol(r2->geom), "double-run volume is bit-identical");
    }

    // --- Bores travel through the op layer. ---
    run_expect_ok(gear_op_frame(json{{"axleHole", true}, {"axleHoleDiameter", 10.0}}),
                  "frame placement + axle bore",
                  [](const ops::OpOutcome& out, BodyStore& bodies, em::ElementMapPartition&) {
                      const auto* plain = bodies.get("body_opgear");
                      check(plain != nullptr, "bored body created");
                      check(out.delta.empty(), "bored gear still has an EMPTY delta");
                  });

    // --- Refusal matrix -----------------------------------------------------
    run_expect_refusal(gear_op_frame(json{{"teeth", 2}}), "teeth below the minimum");
    run_expect_refusal(gear_op_frame(json{{"module", 0.0}}), "zero module");
    run_expect_refusal(gear_op_frame(json{{"height", 0.0}}), "zero height");
    run_expect_refusal(gear_op_frame(json{{"pressureAngleDeg", 0.0}}), "zero pressure angle");
    run_expect_refusal(gear_op_frame(json{{"sampleCount", 1}}), "sampleCount below 2");
    run_expect_refusal(gear_op_frame(json{{"axleHole", true}, {"axleHoleDiameter", 0.0}}),
                       "axle bore with no diameter");
    run_expect_refusal(gear_op_frame(json{{"axleHole", true}, {"axleHoleDiameter", 500.0}}),
                       "axle bore that swallows the teeth");

    // Helical is UNSUPPORTED in this version and must be refused BY NAME, not
    // silently flattened to a spur gear.
    run_expect_refusal(gear_op_frame(json{{"helixAngleDeg", 15.0}}), "helical gear");
    run_expect_refusal(gear_op_frame(json{{"doubleHelix", true}}), "herringbone gear");

    // Unknown recipe ⇒ UNSUPPORTED, distinct from a bad parameter.
    {
        json op = gear_op_frame();
        op["params"]["recipe"] = "cycloid";
        run_expect_refusal(op, "unknown recipe", "UNSUPPORTED");
    }

    // Placement must be exactly one of face/frame.
    {
        const TopoDS_Shape host = box_20_20_25();
        json both = gear_op_face(host);
        // The frame's origin deliberately MATCHES the placement point here, so
        // this case isolates the both-are-set guard. An earlier revision used a
        // mismatched origin and passed for the wrong reason: with the guard
        // removed the op still refused, just via the origin/point check. A
        // refusal test that does not pin WHICH rule fired is not a test of that
        // rule.
        both["params"]["placement"]["frame"] = json{{"origin", {10.0, 10.0, 25.0}},
                                                    {"axis", {0.0, 0.0, 1.0}},
                                                    {"xDir", {1.0, 0.0, 0.0}}};
        run_expect_refusal(both, "both face AND frame");

        json neither = gear_op_frame();
        neither["params"]["placement"]["frame"] = nullptr;
        run_expect_refusal(neither, "neither face nor frame");
    }

    // The 1e-3 mm placement fence: a point well off the seat plane is refused
    // rather than silently snapped.
    {
        const TopoDS_Shape host = box_20_20_25();
        run_expect_refusal(gear_op_face(host, json::object(), json::array({10.0, 10.0, 30.0})),
                           "point 5 mm off the seat plane");
        // ...and a point on the plane but OUTSIDE the face boundary.
        run_expect_refusal(gear_op_face(host, json::object(), json::array({500.0, 500.0, 25.0})),
                           "point outside the face boundary");
    }

    // A frame whose xDir is parallel to its axis cannot define a phase.
    {
        json op = gear_op_frame();
        op["params"]["placement"]["frame"]["xDir"] = json::array({0.0, 0.0, 1.0});
        run_expect_refusal(op, "xDir parallel to the axis");
    }
    {
        json op = gear_op_frame();
        op["params"]["placement"]["frame"]["axis"] = json::array({0.0, 0.0, 0.0});
        run_expect_refusal(op, "degenerate frame axis");
    }
    {
        // point must equal the frame origin — two sources of truth for one
        // position would drift apart on a later edit.
        json op = gear_op_frame();
        op["params"]["placement"]["point"] = json::array({1.0, 0.0, 0.0});
        run_expect_refusal(op, "point disagrees with the frame origin");
    }
    {
        // A missing recipe block for the named recipe.
        json op = gear_op_frame();
        op["params"].erase("involuteExternal");
        run_expect_refusal(op, "missing recipe block");
    }

    // --- `gear_body_op_ids` / `is_generated_gear_body`: the referenceability
    //     guard's source of truth (see GearOp.h). ---
    {
        const json plan = {{"ops", json::array({gear_op_frame(),
                                                json{{"opType", "Hole"}, {"opId", "ophole"}}})}};
        const auto ids = ops::gear_body_op_ids(plan);
        check(ids.size() == 1 && ids.count("opgear") == 1, "gear op ids found from the plan");
        check(ops::is_generated_gear_body(plan, "body_opgear"),
              "the gear's minted body is recognised as generated");
        check(!ops::is_generated_gear_body(plan, "body_ophole"),
              "a non-gear body is not recognised as generated");
        check(!ops::is_generated_gear_body(plan, "body_1"), "an unrelated body is not generated");
        check(!ops::is_generated_gear_body(plan, "opgear"),
              "a bare op id is not a body id");
    }

    // --- KNOWN COVERAGE GAP, stated rather than papered over ---------------
    // Nothing above distinguishes the publication TIER: swapping this op's
    // `TierB` for `TierA` leaves every assertion here green. TierB's marginal
    // check over TierA is self-interference, and no parameter set tried so far
    // (extreme backlash, large negative shift, oversized head/clearance)
    // produces a gear that is BRep-valid yet self-intersecting — the builder's
    // own wire/face construction refuses those earlier, which is arguably why
    // the gap exists. The tier is therefore a documented decision (SCHEMA §7.3)
    // held by review, not by this file. A parameter set that reaches
    // publication self-intersecting would be a genuinely valuable addition
    // here; it should be added the moment one is found.

    if (g_failures == 0) std::fprintf(stderr, "OK: gear_op\n");
    // Exit status is 8-bit on POSIX; clamp so a failing run can never report
    // success by landing on a multiple of 256.
    return (g_failures == 0) ? 0 : (g_failures > 100 ? 100 : g_failures);
}
