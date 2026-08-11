// test_transform_body.cpp — WP-B W0 `TransformBody` numerics + lineage + the
// element-map placement migration. In-process via the op executor (real OCCT).
//
// What is pinned here:
//   (1) TRANSLATE — exact bbox shift, volume invariant to 1e-9, BodyId preserved,
//       exactly one `modified` event.
//   (2) ROTATE 90° about Z — the bbox of a 20×10×5 box becomes 10×20×5 (a
//       DIFFERENT box, so a dropped rotation cannot pass), volume invariant.
//   (3) T∘R ORDER — the normative order is pinned with a case where the two
//       orders DISAGREE (a non-central pivot); R-then-T and T-then-R land the
//       body in different places and the test names the expected one.
//   (4) COPY — single ⇒ `body_<opId>`, multi ⇒ `body_<opId>:<k>` ordered by the
//       target's index in `targets`; the SOURCES are untouched and emit no event.
//   (5) APPLY_PLACEMENT PROOF — a tracked element rebinds at ladder level 1 with
//       `scoring_call_count() == 0` (no descriptor scoring at all) AND its stored
//       anchor moved by EXACTLY the transform (the latent staleness this closes).
//   (6) IDENTITY — a zero placement is legal and changes nothing (same volume,
//       same bbox, same face/edge/vertex counts, anchors unmoved).
//   (7) GUARDS — unknown body ⇒ REF_UNRESOLVED; empty targets / duplicate target /
//       zero axis with a non-zero angle ⇒ recoverable OP_FAILED.
// No framework: exit code == failure count.
#include <cmath>
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
#include "elementmap/Scoring.h"
#include "nlohmann/json.hpp"
#include "ops/OpTypes.h"
#include "ops/TransformOp.h"
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
void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}
void check_near(double got, double want, double tol, const std::string& msg) {
    if (!(std::abs(got - want) <= tol)) {
        std::fprintf(stderr, "FAIL: %s (got %.12f want %.12f)\n", msg.c_str(), got, want);
        ++g_failures;
    }
}

onecad::session::ShapeMetrics metrics(const TopoDS_Shape& s) {
    return onecad::session::compute_shape_metrics(s);
}

// `compute_shape_metrics` reads a `Bnd_Box`, and OCCT pads every bounding box by a
// fixed 1e-7 GAP on each side. That padding is identical before and after a rigid
// motion, so a bbox DELTA is exact (the translate case asserts at 1e-9) while an
// ABSOLUTE bbox coordinate must allow the gap.
constexpr double kBboxGapTol = 1e-6;

struct Ctx {
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;
    ops::OpContext make(BodyStore& bodies, em::ElementMapPartition& part) {
        return ops::OpContext{bodies, &sketches, part, &last_sketch, false, json::object(), &cancel};
    }
};

// A `TransformBody` op JSON. `targets` in order; translate as a BARE-number array
// (the legacy/hand-authored scalar form — the object form is covered by the Rust
// integration test, and both MUST be accepted).
json transform_op(const std::string& op_id, const std::vector<std::string>& targets, double tx,
                  double ty, double tz, json rotate = json(), bool copy = false) {
    json t = json::array();
    for (const std::string& id : targets) t.push_back(id);
    json params = {{"targets", std::move(t)}, {"translate", {tx, ty, tz}}, {"copy", copy}};
    if (!rotate.is_null()) params["rotate"] = std::move(rotate);
    json inputs = json::array();
    for (const std::string& id : targets) {
        inputs.push_back(
            json{{"primary", {{"bodyId", id}, {"elementId", id}, {"kind", "body"}}}});
    }
    return json{{"opType", "TransformBody"},
                {"opId", op_id},
                {"inputs", std::move(inputs)},
                {"params", std::move(params)}};
}

json rotate_json(double cx, double cy, double cz, double ax, double ay, double az,
                 double angle_deg) {
    // `angleDeg` in the {value} object form — the shape the Rust core normalizes to.
    return json{{"center", {cx, cy, cz}},
                {"axis", {ax, ay, az}},
                {"angleDeg", {{"value", angle_deg}}}};
}

// The FACE whose descriptor centre is nearest (cx,cy,cz).
TopoDS_Shape face_by_center(const TopoDS_Shape& shape, double cx, double cy, double cz) {
    TopTools_IndexedMapOfShape faces;
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

// ── (1) TRANSLATE — exact bbox shift, volume invariant, id preserved ──────────
void test_translate_exact() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 10.0, 5.0).Shape();  // vol 1000
    BodyStore bodies;
    bodies.create("body_1", "op0", box);
    em::ElementMapPartition part;
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);

    const json op = transform_op("opt", {"body_1"}, 15.0, -3.0, 2.5);
    const ops::OpOutcome oc = ops::execute_transform_body(ctx, op, "opt");
    check(oc.status == ops::OpOutcome::Status::Ok, "translate: Ok");
    check(oc.needs_repair.empty(), "translate: no NeedsRepair");
    check(oc.body_events.size() == 1 && oc.body_events[0].kind == "modified" &&
              oc.body_events[0].body_id == "body_1",
          "translate: MODIFY lineage — one `modified` event, BodyId preserved");
    check(bodies.size() == 1, "translate: no new body minted");

    const auto before = metrics(box);
    const auto m = metrics(bodies.get("body_1")->geom);
    check_near(m.volume, 1000.0, 1e-9, "translate: volume invariant to 1e-9");
    // EXACT shift: the Bnd_Box gap is the same on both boxes, so it cancels.
    const double d[3] = {15.0, -3.0, 2.5};
    for (int i = 0; i < 3; ++i) {
        check_near(m.bbox_min[i] - before.bbox_min[i], d[i], 1e-9,
                   "translate: bbox min shifted by EXACTLY the translation");
        check_near(m.bbox_max[i] - before.bbox_max[i], d[i], 1e-9,
                   "translate: bbox max shifted by EXACTLY the translation");
    }
}

// ── (2) ROTATE 90° about Z ────────────────────────────────────────────────────
void test_rotate_ninety_about_z() {
    // Box 20×10×5 at the origin; rotate +90° about Z through the ORIGIN.
    // (x,y) → (−y, x): x∈[0,20], y∈[0,10] ⇒ x'∈[−10,0], y'∈[0,20].
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 10.0, 5.0).Shape();
    BodyStore bodies;
    bodies.create("body_1", "op0", box);
    em::ElementMapPartition part;
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);

    const json op =
        transform_op("opr", {"body_1"}, 0.0, 0.0, 0.0, rotate_json(0, 0, 0, 0, 0, 1, 90.0));
    const ops::OpOutcome oc = ops::execute_transform_body(ctx, op, "opr");
    check(oc.status == ops::OpOutcome::Status::Ok, "rotate: Ok");

    const auto m = metrics(bodies.get("body_1")->geom);
    check_near(m.volume, 1000.0, 1e-9, "rotate: volume invariant to 1e-9");
    check_near(m.bbox_min[0], -10.0, kBboxGapTol, "rotate: xmin −10");
    check_near(m.bbox_max[0], 0.0, kBboxGapTol, "rotate: xmax 0");
    check_near(m.bbox_min[1], 0.0, kBboxGapTol, "rotate: ymin 0");
    check_near(m.bbox_max[1], 20.0, kBboxGapTol, "rotate: ymax 20");
    check_near(m.bbox_min[2], 0.0, kBboxGapTol, "rotate: z untouched (min)");
    check_near(m.bbox_max[2], 5.0, kBboxGapTol, "rotate: z untouched (max)");
    check(m.face_count == 6 && m.edge_count == 12 && m.vertex_count == 8,
          "rotate: topology unchanged (a rigid motion adds/removes nothing)");
}

// ── (3) T∘R ORDER — a case where R-then-T ≠ T-then-R ──────────────────────────
void test_translate_after_rotate_order() {
    // Unit box at the origin; rotate +90° about Z through the ORIGIN (a NON-central
    // pivot for this body), then translate +10 in X.
    //   NORMATIVE  X' = T(R(X)):  x∈[0,1] → x'∈[−1,0] → +10 ⇒ x'∈[9,10], y'∈[0,1].
    //   The other order T-then-R would give: x∈[0,1] +10 ⇒ [10,11], then rotate
    //   about the origin ⇒ x'∈[−1,0], y'∈[10,11] — a completely different place.
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(1.0, 1.0, 1.0).Shape();
    BodyStore bodies;
    bodies.create("body_1", "op0", box);
    em::ElementMapPartition part;
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);

    const json op =
        transform_op("opo", {"body_1"}, 10.0, 0.0, 0.0, rotate_json(0, 0, 0, 0, 0, 1, 90.0));
    const ops::OpOutcome oc = ops::execute_transform_body(ctx, op, "opo");
    check(oc.status == ops::OpOutcome::Status::Ok, "order: Ok");

    const auto m = metrics(bodies.get("body_1")->geom);
    check_near(m.bbox_min[0], 9.0, kBboxGapTol, "order: T∘R ⇒ xmin 9 (T-then-R would be −1)");
    check_near(m.bbox_max[0], 10.0, kBboxGapTol, "order: T∘R ⇒ xmax 10");
    check_near(m.bbox_min[1], 0.0, kBboxGapTol, "order: T∘R ⇒ ymin 0 (T-then-R would be 10)");
    check_near(m.bbox_max[1], 1.0, kBboxGapTol, "order: T∘R ⇒ ymax 1");
}

// ── (4) COPY — single + multi ordinals; sources untouched ─────────────────────
void test_copy_single_and_multi() {
    {
        const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
        BodyStore bodies;
        bodies.create("body_1", "op0", box);
        em::ElementMapPartition part;
        Ctx c;
        ops::OpContext ctx = c.make(bodies, part);

        const json op = transform_op("opc1", {"body_1"}, 50.0, 0.0, 0.0, json(), /*copy=*/true);
        const ops::OpOutcome oc = ops::execute_transform_body(ctx, op, "opc1");
        check(oc.status == ops::OpOutcome::Status::Ok, "copy1: Ok");
        check(oc.body_events.size() == 1 && oc.body_events[0].kind == "created" &&
                  oc.body_events[0].body_id == "body_opc1",
              "copy1: ONE target ⇒ plain `body_<opId>`, `created`");
        check(bodies.size() == 2, "copy1: source + copy");
        check_near(metrics(bodies.get("body_1")->geom).bbox_min[0], 0.0, kBboxGapTol,
                   "copy1: SOURCE untouched");
        check_near(metrics(bodies.get("body_opc1")->geom).bbox_min[0], 50.0, kBboxGapTol,
                   "copy1: the copy moved");
        check_near(metrics(bodies.get("body_opc1")->geom).volume, 1000.0, 1e-9,
                   "copy1: volume invariant");
    }
    {
        const TopoDS_Shape a = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
        const TopoDS_Shape b = BRepPrimAPI_MakeBox(gp_Pnt(100, 0, 0), 2.0, 2.0, 2.0).Shape();
        BodyStore bodies;
        bodies.create("body_A", "op0", a);
        bodies.create("body_B", "op0", b);
        em::ElementMapPartition part;
        Ctx c;
        ops::OpContext ctx = c.make(bodies, part);

        // Ordinals follow the `targets` ORDER, not the BodyStore's sorted order —
        // pass B first so a sorted-order implementation would fail here.
        const json op =
            transform_op("opc2", {"body_B", "body_A"}, 0.0, 7.0, 0.0, json(), /*copy=*/true);
        const ops::OpOutcome oc = ops::execute_transform_body(ctx, op, "opc2");
        check(oc.status == ops::OpOutcome::Status::Ok, "copyN: Ok");
        check(oc.body_events.size() == 2, "copyN: two created events");
        check(oc.body_events.size() == 2 && oc.body_events[0].body_id == "body_opc2:0" &&
                  oc.body_events[1].body_id == "body_opc2:1",
              "copyN: N>1 ⇒ `body_<opId>:<k>`, k = index in targets");
        check(bodies.size() == 4, "copyN: 2 sources + 2 copies");
        check_near(metrics(bodies.get("body_opc2:0")->geom).volume, 8.0, 1e-9,
                   "copyN: :0 is the copy of targets[0] == body_B (2³ = 8)");
        check_near(metrics(bodies.get("body_opc2:1")->geom).volume, 1000.0, 1e-9,
                   "copyN: :1 is the copy of targets[1] == body_A (10³ = 1000)");
        check_near(metrics(bodies.get("body_B")->geom).bbox_min[1], 0.0, kBboxGapTol,
                   "copyN: source B untouched");
        check_near(metrics(bodies.get("body_A")->geom).bbox_min[1], 0.0, kBboxGapTol,
                   "copyN: source A untouched");
    }
}

// ── (5) APPLY_PLACEMENT — level-1 rebind with ZERO scoring + anchors migrated ──
void test_tracked_element_follows_the_body() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 10.0, 5.0).Shape();
    BodyStore bodies;
    bodies.create("body_1", "op0", box);
    em::ElementMapPartition part;

    // Track the +Z cap face WITH the anchor evidence a real pick carries: a world
    // point, a local frame (origin + axes), and the two placement-INVARIANT fields.
    const TopoDS_Shape top = face_by_center(box, 10, 5, 5);
    part.mint("body_1", "el_top", km::ElementKind::Face, top, box,
              json{{"worldPoint", {10.0, 5.0, 5.0}},
                   {"surfaceUv", {0.25, 0.75}},
                   {"adjacencyHint", "d41d8cd98f00b204"},
                   {"localFrame",
                    {{"origin", {10.0, 5.0, 5.0}},
                     {"x", {1.0, 0.0, 0.0}},
                     {"y", {0.0, 1.0, 0.0}},
                     {"z", {0.0, 0.0, 1.0}}}}});
    const std::string key_before = part.find("el_top")->topo_key;

    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    // Rotate +90° about Z through the origin, then translate +100 in X.
    //   anchor point (10,5,5) → rotate ⇒ (−5,10,5) → translate ⇒ (95,10,5)
    //   frame x (1,0,0) → (0,1,0);  y (0,1,0) → (−1,0,0);  z unchanged.
    const json op =
        transform_op("opa", {"body_1"}, 100.0, 0.0, 0.0, rotate_json(0, 0, 0, 0, 0, 1, 90.0));

    em::reset_scoring_call_count();
    const ops::OpOutcome oc = ops::execute_transform_body(ctx, op, "opa");

    check(oc.status == ops::OpOutcome::Status::Ok, "placement: Ok");
    check(oc.needs_repair.empty(), "placement: no NeedsRepair (a rigid motion loses nothing)");
    check(em::scoring_call_count() == 0,
          "placement: descriptor scorer NEVER consulted (unique history image ⇒ level 1)");
    check(part.contains("el_top"), "placement: the tracked face is still bound");

    const em::PartitionEntry* e = part.find("el_top");
    if (e == nullptr) {
        std::fprintf(stderr, "FAIL: placement: entry vanished\n");
        ++g_failures;
        return;
    }
    check(!e->topo_key.empty() && e->topo_key[0] == 'f',
          "placement: rebound to a FACE topoKey in the new snapshot");
    check(e->topo_key == key_before,
          "placement: a rigid motion does not renumber the face ordinals");
    // The descriptor is recomputed from the MOVED sub-shape by apply_history.
    check_near(e->descriptor.center.X(), 95.0, 1e-6, "placement: descriptor centre x moved");
    check_near(e->descriptor.center.Y(), 10.0, 1e-6, "placement: descriptor centre y moved");
    check_near(e->descriptor.center.Z(), 5.0, 1e-6, "placement: descriptor centre z moved");

    // THE PROOF: the stored anchor moved by EXACTLY the transform (this is what
    // `apply_placement` adds — `apply_history` leaves the anchor alone).
    const json& a = e->anchor;
    check(a.is_object() && a.contains("worldPoint"), "placement: anchor still present");
    check_near(a["worldPoint"][0].get<double>(), 95.0, 1e-9, "placement: anchor x = 95");
    check_near(a["worldPoint"][1].get<double>(), 10.0, 1e-9, "placement: anchor y = 10");
    check_near(a["worldPoint"][2].get<double>(), 5.0, 1e-9, "placement: anchor z = 5");
    check_near(a["localFrame"]["origin"][0].get<double>(), 95.0, 1e-9,
               "placement: localFrame origin is a POINT (full transform)");
    check_near(a["localFrame"]["x"][0].get<double>(), 0.0, 1e-9,
               "placement: localFrame x is a DIRECTION — rotation only, no +100");
    check_near(a["localFrame"]["x"][1].get<double>(), 1.0, 1e-9,
               "placement: localFrame x rotated (1,0,0) → (0,1,0)");
    check_near(a["localFrame"]["y"][0].get<double>(), -1.0, 1e-9,
               "placement: localFrame y rotated (0,1,0) → (−1,0,0)");
    check_near(a["localFrame"]["z"][2].get<double>(), 1.0, 1e-9,
               "placement: localFrame z (rotation axis) unchanged");
    // Placement-invariant evidence is deliberately NOT rewritten.
    check_near(a["surfaceUv"][0].get<double>(), 0.25, 1e-12,
               "placement: surfaceUv is parametric ⇒ untouched");
    check(a.value("adjacencyHint", "") == "d41d8cd98f00b204",
          "placement: adjacencyHint is a topology hash ⇒ untouched");
}

// ── (6) IDENTITY — legal, and a genuine no-op ─────────────────────────────────
void test_identity_is_a_no_op() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 10.0, 5.0).Shape();
    BodyStore bodies;
    bodies.create("body_1", "op0", box);
    em::ElementMapPartition part;
    const TopoDS_Shape top = face_by_center(box, 10, 5, 5);
    part.mint("body_1", "el_top", km::ElementKind::Face, top, box,
              json{{"worldPoint", {10.0, 5.0, 5.0}}});
    const auto before = metrics(box);

    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const json op =
        transform_op("opi", {"body_1"}, 0.0, 0.0, 0.0, rotate_json(0, 0, 0, 0, 0, 1, 0.0));
    const ops::OpOutcome oc = ops::execute_transform_body(ctx, op, "opi");

    check(oc.status == ops::OpOutcome::Status::Ok, "identity: LEGAL (never a failure)");
    check(oc.needs_repair.empty(), "identity: no NeedsRepair");
    const auto after = metrics(bodies.get("body_1")->geom);
    check_near(after.volume, before.volume, 1e-12, "identity: same volume");
    for (int i = 0; i < 3; ++i) {
        check_near(after.bbox_min[i], before.bbox_min[i], 1e-12, "identity: same bbox min");
        check_near(after.bbox_max[i], before.bbox_max[i], 1e-12, "identity: same bbox max");
    }
    check(after.face_count == before.face_count && after.edge_count == before.edge_count &&
              after.vertex_count == before.vertex_count,
          "identity: same signature inputs (counts)");
    const em::PartitionEntry* e = part.find("el_top");
    check(e != nullptr, "identity: the tracked element survives");
    if (e != nullptr) {
        check_near(e->anchor["worldPoint"][0].get<double>(), 10.0, 1e-12,
                   "identity: the anchor did not move");
    }
}

// ── (7) GUARDS — every bad placement is RECOVERABLE, never a protocol error ────
void test_guards() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    BodyStore bodies;
    bodies.create("body_1", "op0", box);
    em::ElementMapPartition part;
    Ctx c;

    {
        ops::OpContext ctx = c.make(bodies, part);
        const json op = transform_op("opg", {"body_missing"}, 1.0, 0.0, 0.0);
        const ops::OpOutcome oc = ops::execute_transform_body(ctx, op, "opg");
        check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "REF_UNRESOLVED",
              "guard: unknown target ⇒ REF_UNRESOLVED");
    }
    {
        ops::OpContext ctx = c.make(bodies, part);
        json op = transform_op("opg", {"body_1"}, 1.0, 0.0, 0.0);
        op["params"]["targets"] = json::array();
        const ops::OpOutcome oc = ops::execute_transform_body(ctx, op, "opg");
        check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "OP_FAILED",
              "guard: empty targets ⇒ OP_FAILED");
    }
    {
        ops::OpContext ctx = c.make(bodies, part);
        const json op = transform_op("opg", {"body_1", "body_1"}, 1.0, 0.0, 0.0);
        const ops::OpOutcome oc = ops::execute_transform_body(ctx, op, "opg");
        check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "OP_FAILED",
              "guard: duplicate target ⇒ OP_FAILED");
    }
    {
        ops::OpContext ctx = c.make(bodies, part);
        const json op =
            transform_op("opg", {"body_1"}, 0.0, 0.0, 0.0, rotate_json(0, 0, 0, 0, 0, 0, 45.0));
        const ops::OpOutcome oc = ops::execute_transform_body(ctx, op, "opg");
        check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "OP_FAILED",
              "guard: zero axis with angleDeg != 0 ⇒ OP_FAILED");
    }
    {
        ops::OpContext ctx = c.make(bodies, part);
        json op = transform_op("opg", {"body_1"}, 1.0, 0.0, 0.0);
        op["params"]["translate"] = json::array({1.0, 0.0});
        const ops::OpOutcome oc = ops::execute_transform_body(ctx, op, "opg");
        check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "OP_FAILED",
              "guard: short translate vector ⇒ OP_FAILED");
    }
    {
        ops::OpContext ctx = c.make(bodies, part);
        json op = transform_op("opg", {"body_1"}, 1.0, 0.0, 0.0);
        op["params"]["copy"] = "false";
        const ops::OpOutcome oc = ops::execute_transform_body(ctx, op, "opg");
        check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "OP_FAILED",
              "guard: non-boolean copy ⇒ OP_FAILED");
    }
    {
        ops::OpContext ctx = c.make(bodies, part);
        json op = transform_op("opg", {"body_1"}, 1.0, 0.0, 0.0);
        json targets = json::array();
        for (int i = 0; i < 129; ++i) targets.push_back("body_" + std::to_string(i));
        op["params"]["targets"] = std::move(targets);
        const ops::OpOutcome oc = ops::execute_transform_body(ctx, op, "opg");
        check(oc.status == ops::OpOutcome::Status::Failed && oc.error_code == "OP_FAILED",
              "guard: oversized targets array ⇒ OP_FAILED");
    }
    // A multi-target placement whose SECOND target is missing must leave the FIRST
    // one untouched (resolve-all-before-mutate).
    {
        ops::OpContext ctx = c.make(bodies, part);
        const json op = transform_op("opg", {"body_1", "body_missing"}, 5.0, 0.0, 0.0);
        const ops::OpOutcome oc = ops::execute_transform_body(ctx, op, "opg");
        check(oc.status == ops::OpOutcome::Status::Failed,
              "guard: partial multi-target placement refuses");
        check_near(metrics(bodies.get("body_1")->geom).bbox_min[0], 0.0, kBboxGapTol,
                   "guard: the resolvable target was NOT half-applied");
    }
    check(bodies.size() == 1, "guard: no body minted by any failing placement");
}

}  // namespace

int main() {
    test_translate_exact();
    test_rotate_ninety_about_z();
    test_translate_after_rotate_order();
    test_copy_single_and_multi();
    test_tracked_element_follows_the_body();
    test_identity_is_a_no_op();
    test_guards();
    if (g_failures == 0) std::fprintf(stderr, "test_transform_body: all checks PASS\n");
    return g_failures;
}
