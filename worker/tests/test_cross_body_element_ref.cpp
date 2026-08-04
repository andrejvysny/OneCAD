// test_cross_body_element_ref.cpp — VF-M7: an ElementMap partition entry minted on
// body A must NEVER be applied to body B.
//
// ── The hazard ───────────────────────────────────────────────────────────────
// A `TopoKey` ("f:N") is a 1-based ORDINAL in `TopExp::MapShapes(bodyShape, FACE)`.
// It is snapshot-scoped AND body-scoped: "f:3" means nothing outside the body it
// was minted against. `HoleOp::resolve_host_face` and `ShellOp` used to take the
// entry's TopoKey straight from `partition.find(elementId)` and apply it to the
// OP'S TARGET body without ever checking `entry->body_id` — so an op on body B
// carrying an ElementId minted on body A bound *B's Nth face*, chosen by an
// ordinal that describes A. The correct pattern is FaceProjection.cpp's: read
// `entry->body_id` and look up THAT body.
//
// Cross-body ElementIds are reachable in production: ElementIds are globally
// unique and body-INDEPENDENT (Invariant 2), a stale/copied ref survives an edit,
// and `PlanExecutor::resolve_input_refs` deliberately skips re-minting an id it
// already tracks (`if (job.partition.contains(...)) continue;`) — so the foreign
// entry stays foreign and reaches the op executor unchanged.
//
// ── What this asserts (geometry, never a tag) ────────────────────────────────
//   1. `topokey_for_element_in_body` — the new gate — returns the key only for the
//      owning body, "" for a foreign one / an absent id.
//   2. Hole, SILENT WRONG DRILL: a ref whose descriptor cannot bind on B but whose
//      foreign TopoKey names a real planar face of B. BEFORE the fix the op drilled
//      that face and returned Ok. AFTER, the ladder refuses (NeedsRepair STATE) and
//      B's volume is *bit-identical* to its pre-op volume.
//   3. Hole, WRONG SEAT: the realistic pick — a good descriptor+anchor for B's TOP
//      face riding a foreign ElementId. BEFORE the fix the foreign ordinal won and
//      the op died on the point fence against a face the user never named. AFTER,
//      the ladder binds the top face and the drill removes exactly π·r²·t.
//   4. Shell, WRONG WALL: same ref shape. BEFORE the fix the shell opened a SIDE
//      wall (cavity 18×16×26); AFTER it opens the TOP (cavity 16×16×28). Both are
//      exact analytic volumes, so the two outcomes cannot be confused.
//
// Bodies are deliberately DIFFERENT geometry with DIFFERENT face counts (a Ø10×10
// cylinder, 3 faces / a 20×20×30 box, 6 faces): the foreign ordinal is then valid
// on both bodies but means something else on each — the purest form of the hazard.
//
// No framework: exit code == failure count.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "ops/HoleOp.h"
#include "ops/OpTypes.h"
#include "ops/ShellOp.h"
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

// Body B: the op TARGET. 20×20×30 box (6 faces), volume 12000, top face at z=30.
constexpr double kBoxVol = 20.0 * 20.0 * 30.0;
// Body A: the FOREIGN body the stray ElementId belongs to. Ø10×10 cylinder — a
// different face COUNT (3) and no face whose area matches any of B's, so a
// descriptor minted here can never legitimately auto-bind on B.
constexpr double kCylRadius = 5.0;
constexpr double kCylHeight = 10.0;

constexpr const char* kBodyA = "body_a";
constexpr const char* kBodyB = "body_b";
constexpr const char* kForeignElement = "el_on_body_a";

int g_failures = 0;

void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}

void check_rel(double got, double want, double rel, const std::string& msg) {
    const double tol = std::abs(want) * rel;
    if (std::abs(got - want) > tol) {
        std::fprintf(stderr, "FAIL: %s (got %.9f want %.9f, tol %.2g)\n", msg.c_str(), got, want,
                     tol);
        ++g_failures;
    }
}

double vol(const TopoDS_Shape& s) { return onecad::session::shape_volume(s); }

TopoDS_Shape box_b() { return BRepPrimAPI_MakeBox(20.0, 20.0, 30.0).Shape(); }
TopoDS_Shape cylinder_a() { return BRepPrimAPI_MakeCylinder(kCylRadius, kCylHeight).Shape(); }

int face_count(const TopoDS_Shape& shape) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    return faces.Extent();
}

// The FACE whose descriptor centre is nearest (cx,cy,cz) — test_hole_op.cpp:76.
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

bool is_planar(const TopoDS_Shape& face) {
    if (face.IsNull() || face.ShapeType() != TopAbs_FACE) return false;
    return BRepAdaptor_Surface(TopoDS::Face(face)).GetType() == GeomAbs_Plane;
}

// The decoy: an A-face whose TopoKey, applied to B, names a real planar face of B
// that is NOT the top face and is NOT parallel to it. Both conditions matter:
//   * planar  — else the mis-bind dies on "not a planar seat" and the wrong-bind
//     never gets far enough to change geometry;
//   * non-parallel to +Z — a bottom-face mis-bind would shell to the SAME volume as
//     the intended top-face shell, making the geometric assertion blind.
// Searched rather than hard-coded so the test does not silently rot if OCCT's
// primitive face ordering changes.
struct Decoy {
    std::string topo_key;   // ordinal minted on A
    TopoDS_Shape a_face;    // the A sub-face that mints it
    TopoDS_Shape b_face;    // what that ordinal names on B — the wrong bind
};

bool find_decoy(const TopoDS_Shape& a, const TopoDS_Shape& b, const TopoDS_Shape& b_top,
                Decoy& out) {
    TopTools_IndexedMapOfShape a_faces;
    TopExp::MapShapes(a, TopAbs_FACE, a_faces);
    const km::ElementDescriptor top = em::ElementMapPartition::describe(b_top);
    for (int i = 1; i <= a_faces.Extent(); ++i) {
        const std::string key = "f:" + std::to_string(i);
        const TopoDS_Shape b_face = em::ElementMapPartition::shape_for_topokey(b, key);
        if (!is_planar(b_face) || b_face.IsSame(b_top)) continue;
        const km::ElementDescriptor d = em::ElementMapPartition::describe(b_face);
        if (std::abs(d.normal.Dot(top.normal)) > 0.5) continue;  // parallel ⇒ blind assertion
        out.topo_key = key;
        out.a_face = a_faces(i);
        out.b_face = b_face;
        return true;
    }
    return false;
}

// A face input ref: identity + (optionally) a frozen descriptor + a world anchor.
// `element_id` is deliberately allowed to name an element of ANOTHER body — that is
// the whole point of this file.
json face_input(const std::string& body_id, const std::string& element_id,
                const TopoDS_Shape& descriptor_face, const gp_Pnt* anchor) {
    json in = {{"primary", {{"bodyId", body_id}, {"elementId", element_id}, {"kind", "face"}}},
               {"intent",
                {{"kind", "face"},
                 {"descriptor", em::ElementMapPartition::descriptor_to_json(
                                    em::ElementMapPartition::describe(descriptor_face))}}}};
    if (anchor != nullptr) {
        in["anchor"] = {{"worldPoint", {anchor->X(), anchor->Y(), anchor->Z()}}};
    }
    return in;
}

json body_input(const std::string& body_id) {
    return json{{"primary", {{"bodyId", body_id}, {"elementId", body_id}, {"kind", "body"}}}};
}

struct Ctx {
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;
    ops::OpContext make(BodyStore& bodies, em::ElementMapPartition& part) {
        return ops::OpContext{bodies, &sketches, part, &last_sketch, false, json::object(), &cancel};
    }
};

// The shared fixture: bodies A + B in one store, `kForeignElement` minted on A's
// decoy face, and the decoy resolved. Every case starts from here.
struct Fixture {
    BodyStore bodies;
    em::ElementMapPartition partition;
    TopoDS_Shape a;
    TopoDS_Shape b;
    TopoDS_Shape b_top;
    gp_Pnt b_top_center{10.0, 10.0, 30.0};
    Decoy decoy;
    bool ok = false;

    Fixture() {
        a = cylinder_a();
        b = box_b();
        bodies.create(kBodyA, "op_a", a);
        bodies.create(kBodyB, "op_b", b);
        b_top = face_by_center(b, 10.0, 10.0, 30.0);
        if (!find_decoy(a, b, b_top, decoy)) return;
        const em::DeltaEntry e = partition.mint(kBodyA, kForeignElement, km::ElementKind::Face,
                                                decoy.a_face, a, json::object());
        ok = (e.topo_key == decoy.topo_key && e.body_id == kBodyA);
    }
};

// ── 1. The gate itself ───────────────────────────────────────────────────────
void test_topokey_gate() {
    Fixture f;
    check(f.ok, "gate: fixture minted the foreign element on body A");
    if (!f.ok) return;

    check(em::ElementMapPartition::topokey_for_element_in_body(f.partition, kForeignElement,
                                                               kBodyA) == f.decoy.topo_key,
          "gate: the owning body gets the key");
    check(em::ElementMapPartition::topokey_for_element_in_body(f.partition, kForeignElement,
                                                               kBodyB)
              .empty(),
          "gate: a FOREIGN body gets \"\" (never the ordinal)");
    check(em::ElementMapPartition::topokey_for_element_in_body(f.partition, "el_absent", kBodyA)
              .empty(),
          "gate: an absent element gets \"\"");
    check(em::ElementMapPartition::topokey_for_element_in_body(f.partition, "", kBodyA).empty(),
          "gate: an empty element id gets \"\"");
    check(em::ElementMapPartition::topokey_for_element_in_body(f.partition, kForeignElement, "")
              .empty(),
          "gate: an empty body id gets \"\"");

    // The hazard is real only if the foreign ordinal DOES name a face of B.
    check(f.partition.find(kForeignElement) != nullptr, "gate: entry exists");
    check(!em::ElementMapPartition::shape_for_topokey(f.b, f.decoy.topo_key).IsNull(),
          "hazard precondition: the foreign ordinal names a real sub-shape of B");
    check(!f.decoy.b_face.IsSame(f.b_top),
          "hazard precondition: that sub-shape is NOT the face the user named");
    check(face_count(f.a) != face_count(f.b),
          "hazard precondition: A and B have different face counts");
}

// ── 2. Hole — the SILENT wrong drill ─────────────────────────────────────────
// The ref's own descriptor (a cylinder face) cannot bind anywhere on a box, so the
// ONLY thing that could produce geometry here is the foreign ordinal. The point sits
// on the decoy face, so the seat/plane/boundary fences all pass and the pre-fix drill
// succeeds silently. Post-fix the ladder refuses and B is untouched.
void test_hole_silent_wrong_drill() {
    Fixture f;
    check(f.ok, "hole/silent: fixture");
    if (!f.ok) return;

    const km::ElementDescriptor decoy_d = em::ElementMapPartition::describe(f.decoy.b_face);
    const gp_Pnt on_decoy = decoy_d.center;
    const json op = {
        {"opType", "Hole"},
        {"opId", "op_hole_silent"},
        {"inputs", json::array({body_input(kBodyB),
                                // Descriptor from A's own face ⇒ unbindable on B.
                                face_input(kBodyB, kForeignElement, f.decoy.a_face, nullptr)})},
        {"params",
         {{"targetBodyId", kBodyB},
          {"point", {on_decoy.X(), on_decoy.Y(), on_decoy.Z()}},
          {"holeType", "simple"},
          {"diameter", 6.0},
          {"depth", nullptr}}}};

    Ctx c;
    ops::OpContext ctx = c.make(f.bodies, f.partition);
    const ops::OpOutcome oc = ops::execute_hole(ctx, op, "op_hole_silent");

    std::fprintf(stderr,
                 "observed[hole/silent]: status=%d needsRepair=%zu bodyVolume=%.9f "
                 "(pristine=%.9f)\n",
                 static_cast<int>(oc.status), oc.needs_repair.size(),
                 vol(f.bodies.get(kBodyB)->geom), kBoxVol);

    check(oc.status == ops::OpOutcome::Status::Ok, "hole/silent: recoverable STATE, not an Err");
    check(!oc.needs_repair.empty(),
          "hole/silent: a ref that binds nowhere on B is NeedsRepair, not a drill");
    check(oc.body_events.empty(), "hole/silent: nothing published");
    // The load-bearing assertion: B is bit-identical to its pristine self. Before the
    // fix this was 12000 − π·3²·(decoy depth).
    check_rel(vol(f.bodies.get(kBodyB)->geom), kBoxVol, 1e-12,
              "hole/silent: B is UNTOUCHED (the foreign ordinal drilled nothing)");
}

// ── 3. Hole — the realistic pick: a good ref for B's TOP face, foreign ElementId ─
void test_hole_wrong_seat() {
    Fixture f;
    check(f.ok, "hole/seat: fixture");
    if (!f.ok) return;

    const json op = {
        {"opType", "Hole"},
        {"opId", "op_hole_seat"},
        {"inputs", json::array({body_input(kBodyB),
                                face_input(kBodyB, kForeignElement, f.b_top, &f.b_top_center)})},
        {"params",
         {{"targetBodyId", kBodyB},
          {"point", {f.b_top_center.X(), f.b_top_center.Y(), f.b_top_center.Z()}},
          {"holeType", "simple"},
          {"diameter", 6.0},
          {"depth", nullptr}}}};

    Ctx c;
    ops::OpContext ctx = c.make(f.bodies, f.partition);
    const ops::OpOutcome oc = ops::execute_hole(ctx, op, "op_hole_seat");

    std::fprintf(stderr, "observed[hole/seat]: status=%d code=%s msg=%s volume=%.9f\n",
                 static_cast<int>(oc.status), oc.error_code.c_str(), oc.error_message.c_str(),
                 vol(f.bodies.get(kBodyB)->geom));

    check(oc.status == ops::OpOutcome::Status::Ok, "hole/seat: Ok (the ladder binds the top face)");
    check(oc.needs_repair.empty(), "hole/seat: the descriptor+anchor auto-binds B's top face");
    // Drilled from z=30 straight down through 30 mm of box.
    check_rel(vol(f.bodies.get(kBodyB)->geom), kBoxVol - kPi * 3.0 * 3.0 * 30.0, 1e-6,
              "hole/seat: 12000 − π·3²·30 (drilled through the TOP, not a side wall)");
}

// ── 4. Shell — the wrong wall ────────────────────────────────────────────────
void test_shell_wrong_wall() {
    Fixture f;
    check(f.ok, "shell: fixture");
    if (!f.ok) return;

    const json op = {
        {"opType", "Shell"},
        {"opId", "op_shell"},
        {"inputs", json::array({body_input(kBodyB),
                                face_input(kBodyB, kForeignElement, f.b_top, &f.b_top_center)})},
        {"params", {{"targetBodyId", kBodyB}, {"thickness", 2.0}}}};

    Ctx c;
    ops::OpContext ctx = c.make(f.bodies, f.partition);
    const ops::OpOutcome oc = ops::execute_shell(ctx, op, "op_shell");

    const double got = vol(f.bodies.get(kBodyB)->geom);
    // Open TOP  ⇒ cavity 16×16×28 = 7168 ⇒ 4832.
    // Open SIDE ⇒ cavity 18×16×26 = 7488 ⇒ 4512 (what the foreign ordinal produced).
    const double want_top = kBoxVol - 16.0 * 16.0 * 28.0;
    const double want_side = kBoxVol - 18.0 * 16.0 * 26.0;
    std::fprintf(stderr, "observed[shell]: status=%d needsRepair=%zu volume=%.9f (top=%.3f side=%.3f)\n",
                 static_cast<int>(oc.status), oc.needs_repair.size(), got, want_top, want_side);

    check(oc.status == ops::OpOutcome::Status::Ok, "shell: Ok");
    check(oc.needs_repair.empty(), "shell: the descriptor+anchor auto-binds B's top face");
    check_rel(got, want_top, 1e-6, "shell: cavity 16×16×28 — the TOP wall was opened");
    check(std::abs(got - want_side) > 1.0,
          "shell: NOT the side-wall cavity the foreign ordinal selected");
}

}  // namespace

int main() {
    test_topokey_gate();
    test_hole_silent_wrong_drill();
    test_hole_wrong_seat();
    test_shell_wrong_wall();
    if (g_failures == 0) std::fprintf(stderr, "test_cross_body_element_ref: all checks passed\n");
    return g_failures;
}
