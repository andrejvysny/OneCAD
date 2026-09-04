// test_chamfer_reference_face.cpp — WP-F: an ASYMMETRIC chamfer must keep its long
// leg on the same PHYSICAL face across an upstream edit, and the machinery that
// makes that true (`params.referenceFaces`, SCHEMA §7.3 + §9 + §7.6).
//
// This file started as the RED-FIRST probe for the defect and now pins the fix:
//   (a) a TYPED pair keeps the 4 mm leg on the same physical face across an
//       ordinal-permuting upstream hole (the invariant this whole WP exists for);
//   (b) a LEGACY record — asymmetric, no pair — HALTS with the op-built §9
//       `legacyReferenceFace` item in EVERY lane, with `post_upstream_edit` false
//       and true alike: the ordinal rule is gone and there is no fallback left;
//   (c) the negative control for (a) — the same record pairing the OTHER adjacent
//       face produces the opposite geometry, so the pair is what decides;
//   (d) a pair naming a NON-ADJACENT face is refused by name, head untouched;
//   (e) an UNCOVERED contour of a partially typed record halts on the slot the
//       created pair will occupy, naming that contour's own seed edge;
//   (f) two pairs disagreeing on ONE tangent chain are refused by name;
//   (g) the distance-angle form (`angleDeg` → `AddDA`) shares the reference face;
//   (h) the shared `adjacent_face_ordinals` helper de-duplicates a cylinder seam;
//   (i) a NON-v1 record (no `tangentClosureVersion`) keys the pairs per EDGE.
//
// `FilletChamferOp.cpp:83-106 reference_face` picks "the adjacent face with the
// smaller snapshot-scoped face ordinal" and hands it to
// `BRepFilletAPI_MakeChamfer::Add(d1, d2, edge, face)`, which measures d1 ON that
// face. A face ordinal is a `TopExp::MapShapes` index over the PREDECESSOR shape,
// so an upstream feature that reorders the face map swaps which adjacent face is
// "smaller" and the two chamfer legs silently MIRROR. Nothing downstream sees it:
// the removed wedge is d1·d2/2·L either way, so the removed volume, the mass and
// the face count are all invariant under the swap (asserted below, so that this
// probe's own signature is not mistaken for one of them). The chamfer FACE's
// bounding box is not invariant, and that is what is measured here.
//
// The model: a 40×20×10 box (XY rect, extruded 10), chamfered on the top-front
// edge — the one shared by the +Z top face (z = 10) and the -Y front wall (y = 0)
// — with radius = 4 (d1) and distance2 = 1 (d2). The chamfer face's (y, z) extent
// says which face carries the 4 mm leg:
//     d1 on the top (+Z) face   -> y ∈ [0, 4], z ∈ [ 9, 10]
//     d1 on the front (-Y) wall -> y ∈ [0, 1], z ∈ [ 6, 10]
// The same chamfer is run three ways: on the plain box, and with an upstream Hole
// in the -X wall (blind, and through-all) inserted BEFORE it. That hole is
// geometrically disjoint from both chamfered faces — it never touches y = 0 or
// z = 10 — so a physically-anchored reference face gives the SAME chamfer face in
// all three. Measured today (HEAD e781272, OCCT 8.0.1): the plain box orders the
// pair front = f:2 < top = f:6 and puts the 4 mm leg on the front wall, while the
// hole reorders it to top = f:4 < front = f:5 and the legs mirror.
//
// No framework: exit code == failure count.
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include <BRepBndLib.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <Bnd_Box.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Shape.hxx>

#include "elementmap/ElementMapPartition.h"
#include "elementmap/Scoring.h"
#include "nlohmann/json.hpp"
#include "ops/ExtrudeOp.h"
#include "ops/FilletChamferOp.h"
#include "ops/HoleOp.h"
#include "ops/OpTypes.h"
#include "session/BodyStore.h"
#include "session/EdgePicks.h"
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
    if (!cond) { std::fprintf(stderr, "FAIL: %s\n", msg.c_str()); ++g_failures; }
}
void check_near(double got, double want, double tol, const std::string& msg) {
    if (std::abs(got - want) > tol) {
        std::fprintf(stderr, "FAIL: %s (got %.9f want %.9f tol %g)\n", msg.c_str(), got, want, tol);
        ++g_failures;
    }
}

// The box: 40 (x) × 20 (y) × 10 (z), corner at the world origin.
constexpr double kW = 40.0, kH = 20.0, kDepth = 10.0;
// The asymmetric legs: d1 (`radius`) rides the reference face, d2 (`distance2`)
// the other. 4:1 is far enough apart that a mirrored pair cannot be read as
// tolerance noise, and both fit within the 20 mm and 10 mm faces they cut.
constexpr double kD1 = 4.0, kD2 = 1.0;
// The upstream hole: Ø6 in the -X wall, centred (0, 10, 5). Its wall spans
// y ∈ [7, 13], z ∈ [2, 8] — disjoint from BOTH chamfered faces, so it changes the
// face map without changing any geometry the chamfer measures against.
constexpr double kHoleD = 6.0, kHoleDepth = 10.0;
// The distance-angle form: `radius` on the reference face, the far leg
// radius·tan(30°) ≈ 2.309 — far enough from 4 that `long_leg_face` reads it.
constexpr double kAngleDeg = 30.0;

// The `XY` sketch plane is NON-STANDARD by design (Sketch.h:59-69): sketch u maps
// to world +Y and sketch v to world -X. Authoring u ∈ [0, kH] and v ∈ [-kW, 0] is
// therefore what puts the extruded box at world x ∈ [0, 40], y ∈ [0, 20],
// z ∈ [0, 10] — the frame every coordinate below is written in.
json rect_sketch() {
    return json{{"sketchId", "sk1"},
                {"plane", {{"kind", "XY"}}},
                {"entities", json::array({
                     json{{"id", "e1"}, {"type", "Line"}, {"p0", {0, 0}}, {"p1", {kH, 0}}},
                     json{{"id", "e2"}, {"type", "Line"}, {"p0", {kH, 0}}, {"p1", {kH, -kW}}},
                     json{{"id", "e3"}, {"type", "Line"}, {"p0", {kH, -kW}}, {"p1", {0, -kW}}},
                     json{{"id", "e4"}, {"type", "Line"}, {"p0", {0, -kW}}, {"p1", {0, 0}}}})},
                {"constraints", json::array()}};
}

struct Ctx {
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;
    ops::OpContext make(BodyStore& b, em::ElementMapPartition& p) {
        return ops::OpContext{b, &sketches, p, &last_sketch, false, json::object(), &cancel};
    }
};

// The sub-shape of `kind` whose descriptor centre is nearest (cx,cy,cz) —
// test_wp6_ops.cpp:50 / test_hole_op.cpp:82.
TopoDS_Shape nearest(const TopoDS_Shape& shape, TopAbs_ShapeEnum kind, double cx, double cy,
                     double cz) {
    TopTools_IndexedMapOfShape map;
    TopExp::MapShapes(shape, kind, map);
    TopoDS_Shape best;
    double best_d2 = -1.0;
    for (int i = 1; i <= map.Extent(); ++i) {
        const km::ElementDescriptor d = em::ElementMapPartition::describe(map(i));
        const double dx = d.center.X() - cx, dy = d.center.Y() - cy, dz = d.center.Z() - cz;
        const double d2 = dx * dx + dy * dy + dz * dz;
        if (best_d2 < 0.0 || d2 < best_d2) { best_d2 = d2; best = map(i); }
    }
    return best;
}

// A semantic ref carrying the element's frozen descriptor + a world anchor, the
// shape the wire lowers (`elementId` is unminted here, so both ops take the
// descriptor+anchor ladder — the same path a re-resolved ref takes).
json element_input(const std::string& body_id, const char* elem_id, const char* kind,
                   const TopoDS_Shape& shape, double ax, double ay, double az) {
    return json{{"primary", {{"bodyId", body_id}, {"elementId", elem_id}, {"kind", kind}}},
                {"intent", {{"kind", kind},
                            {"descriptor", em::ElementMapPartition::descriptor_to_json(
                                               em::ElementMapPartition::describe(shape))}}},
                {"anchor", {{"worldPoint", {ax, ay, az}}}}};
}

// The 1-based `TopExp::MapShapes(shape, TopAbs_FACE)` ordinal of the face whose
// centre is nearest (cx,cy,cz) — the same index `reference_face` compares and the
// same one a TopoKey "f:N" is built from.
int face_ordinal(const TopoDS_Shape& shape, double cx, double cy, double cz) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    return faces.FindIndex(nearest(shape, TopAbs_FACE, cx, cy, cz));
}

void print_face_table(const char* label, const TopoDS_Shape& shape) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    for (int i = 1; i <= faces.Extent(); ++i) {
        const km::ElementDescriptor d = em::ElementMapPartition::describe(faces(i));
        std::fprintf(stderr, "  [%s] f:%d centre=(%.3f, %.3f, %.3f) n=(%.3f, %.3f, %.3f) %s\n",
                     label, i, d.center.X(), d.center.Y(), d.center.Z(), d.normal.X(),
                     d.normal.Y(), d.normal.Z(),
                     d.surfaceType == GeomAbs_Plane ? "plane" : "curved");
    }
}

// Which upstream feature (if any) runs between the extrude and the chamfer.
enum class Upstream { None, HoleMinusXBlind, HoleMinusXThrough };

const char* upstream_name(Upstream u) {
    switch (u) {
        case Upstream::None: return "plain box";
        case Upstream::HoleMinusXBlind: return "hole -X wall, blind";
        case Upstream::HoleMinusXThrough: return "hole -X wall, through";
    }
    return "?";
}

// How the chamfer under test names its reference face, and the edit context it
// runs in. `typed` authors the WP-F `referenceFaces` pair; `legacy` (the default)
// leaves the field absent, which is every pre-WP-F document.
// Which adjacent face the pair names. TOP and FRONT are both legal (both bound
// the chamfered edge) and produce OPPOSITE geometry, which is what proves the
// pair — and nothing else — decides. BOTTOM is not adjacent at all: the decoy.
enum class PairFace { Top, Front, Bottom };

struct ChamferSpec {
    bool typed = false;                     // author a `{edgeId, faceId}` pair
    PairFace pair_face = PairFace::Top;     // …naming this face
    bool post_upstream_edit = false;        // §7.2 `stepIndex > editedFrom`
    bool angle = false;                     // `angleDeg` (AddDA) instead of `distance2`
    bool v1 = true;                         // carry `tangentClosureVersion: 1`
};

// id + the world point that locates the face on the 40x20x10 box.
struct PairFaceRef {
    const char* id;
    double x, y, z;
};

PairFaceRef pair_face_ref(PairFace face) {
    switch (face) {
        case PairFace::Front: return {"el_front", kW / 2.0, 0.0, kDepth / 2.0};
        case PairFace::Bottom: return {"el_bottom", kW / 2.0, kH / 2.0, 0.0};
        case PairFace::Top: break;
    }
    return {"el_top", kW / 2.0, kH / 2.0, kDepth};
}

struct Probe {
    bool ok = false;
    bool status_ok = false;  // the chamfer returned Ok (repairs may still be set)
    std::string error_code, error_message;
    json needs_repair = json::array();
    json diagnostics = json::array();
    int top_ordinal = 0;    // the +Z face (z = 10) in the PRE-chamfer shape
    int front_ordinal = 0;  // the -Y wall (y = 0) in the PRE-chamfer shape
    double y_min = 0.0, y_max = 0.0, z_min = 0.0, z_max = 0.0;  // the chamfer face's bbox
    double volume_before = 0.0, volume_after = 0.0;
    std::size_t faces_before = 0, faces_after = 0;
};

// extrude -> [upstream hole] -> asymmetric chamfer of the top-front edge.
// Every measurement is taken on the EXACT BRep; nothing is meshed.
Probe run_probe(Upstream upstream, const ChamferSpec& spec = {}) {
    Probe p;
    BodyStore bodies;
    em::ElementMapPartition part;
    Ctx c;
    c.sketches.push_back({"sk1", rect_sketch()});
    c.last_sketch = "sk1";
    const std::string body = "body_ope";
    const std::string tag = std::string("[") + upstream_name(upstream) + "] ";

    {
        ops::OpContext ctx = c.make(bodies, part);
        json op = {{"opType", "Extrude"}, {"opId", "ope"},
                   {"params", {{"sketchId", "sk1"}, {"distance", kDepth},
                               {"extrudeMode", "Blind"}, {"booleanMode", "NewBody"}}}};
        const ops::OpOutcome oc = ops::execute_extrude(ctx, op, "ope");
        check(oc.status == ops::OpOutcome::Status::Ok && bodies.contains(body),
              tag + "extrude: Ok (" + oc.error_code + " " + oc.error_message + ")");
        if (!bodies.contains(body)) return p;
        check_near(onecad::session::shape_volume(bodies.get(body)->geom), kW * kH * kDepth, 1e-6,
                   tag + "extrude: the box is 40x20x10");
    }

    if (upstream != Upstream::None) {
        const double fx = 0.0, fy = kH / 2.0, fz = kDepth / 2.0;  // the -X wall's centre
        const json depth =
            upstream == Upstream::HoleMinusXThrough ? json(nullptr) : json(kHoleDepth);
        const TopoDS_Shape seat = nearest(bodies.get(body)->geom, TopAbs_FACE, fx, fy, fz);
        ops::OpContext ctx = c.make(bodies, part);
        json op = {{"opType", "Hole"}, {"opId", "ophl"},
                   {"inputs", json::array({
                        json{{"primary", {{"bodyId", body}, {"elementId", body}, {"kind", "body"}}}},
                        element_input(body, "el_seat", "face", seat, fx, fy, fz)})},
                   {"params", {{"targetBodyId", body}, {"point", {fx, fy, fz}},
                               {"holeType", "simple"}, {"diameter", kHoleD}, {"depth", depth}}}};
        const ops::OpOutcome oc = ops::execute_hole(ctx, op, "ophl");
        check(oc.status == ops::OpOutcome::Status::Ok && oc.needs_repair.empty(),
              tag + "hole: Ok, no NeedsRepair (" + oc.error_code + " " + oc.error_message +
                  " repairs=" + json(oc.needs_repair).dump() + ")");
        // A NeedsRepair hole leaves the body UNTOUCHED, which would make the edited
        // run a copy of the plain one and the invariant below pass vacuously. Stop.
        if (oc.status != ops::OpOutcome::Status::Ok || !oc.needs_repair.empty()) return p;
    }

    // The PREDECESSOR shape — the one whose face map `reference_face` reads.
    const TopoDS_Shape pre = bodies.get(body)->geom;
    p.top_ordinal = face_ordinal(pre, kW / 2.0, kH / 2.0, kDepth);
    p.front_ordinal = face_ordinal(pre, kW / 2.0, 0.0, kDepth / 2.0);
    p.volume_before = onecad::session::shape_volume(pre);
    p.faces_before = onecad::session::compute_shape_metrics(pre).face_count;
    print_face_table(upstream_name(upstream), pre);

    const TopoDS_Shape edge = nearest(pre, TopAbs_EDGE, kW / 2.0, 0.0, kDepth);
    {
        // The face refs ride `inputs[N + i]` with N = `edgeIds.length` = 1, so the
        // pair's slot is `opc.input1` (SCHEMA §7.3, slot order normative).
        json inputs = json::array(
            {element_input(body, "el_edge", "edge", edge, kW / 2.0, 0.0, kDepth)});
        json params = {{"mode", "Chamfer"}, {"radius", kD1},
                       {"edgeIds", json::array({"el_edge"})}, {"chainTangentEdges", true}};
        if (spec.v1) params["tangentClosureVersion"] = 1;
        if (spec.angle) {
            params["angleDeg"] = kAngleDeg;
        } else {
            params["distance2"] = kD2;
        }
        if (spec.typed) {
            const PairFaceRef ref = pair_face_ref(spec.pair_face);
            const TopoDS_Shape face = nearest(pre, TopAbs_FACE, ref.x, ref.y, ref.z);
            inputs.push_back(
                element_input(body, ref.id, "face", face, ref.x, ref.y, ref.z));
            params["referenceFaces"] =
                json::array({json{{"edgeId", "el_edge"}, {"faceId", ref.id}}});
        }
        json op = {{"opType", "Chamfer"}, {"opId", "opc"}, {"inputs", inputs},
                   {"params", params}};
        ops::OpContext ctx = c.make(bodies, part);
        ctx.post_upstream_edit = spec.post_upstream_edit;
        const ops::OpOutcome oc = ops::execute_chamfer(ctx, op, "opc");
        p.status_ok = oc.status == ops::OpOutcome::Status::Ok;
        p.error_code = oc.error_code;
        p.error_message = oc.error_message;
        for (const json& item : oc.needs_repair) p.needs_repair.push_back(item);
        for (const json& item : oc.diagnostics) p.diagnostics.push_back(item);
        // A refused or halted chamfer leaves the body UNTOUCHED (the executor
        // reverts the step); the caller asserts on the outcome fields instead.
        if (!p.status_ok || !oc.needs_repair.empty()) return p;
    }

    const TopoDS_Shape post = bodies.get(body)->geom;
    p.volume_after = onecad::session::shape_volume(post);
    p.faces_after = onecad::session::compute_shape_metrics(post).face_count;

    // The chamfer face is the only PLANAR face tilted in BOTH y and z: every box
    // face is axis-aligned and the hole's wall is a cylinder. Asserting it is
    // UNIQUE is what makes the bbox below unambiguous.
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(post, TopAbs_FACE, faces);
    int found = 0;
    for (int i = 1; i <= faces.Extent(); ++i) {
        const km::ElementDescriptor d = em::ElementMapPartition::describe(faces(i));
        if (d.surfaceType != GeomAbs_Plane) continue;
        if (std::abs(d.normal.Y()) < 0.05 || std::abs(d.normal.Z()) < 0.05) continue;
        Bnd_Box bb;
        BRepBndLib::Add(faces(i), bb);
        double xn = 0, yn = 0, zn = 0, xx = 0, yx = 0, zx = 0;
        bb.Get(xn, yn, zn, xx, yx, zx);
        p.y_min = yn; p.y_max = yx; p.z_min = zn; p.z_max = zx;
        ++found;
    }
    check(found == 1,
          tag + "exactly one tilted planar face (the chamfer), got " + std::to_string(found));
    p.ok = found == 1;
    return p;
}

// Which PHYSICAL face carries the 4 mm leg, read off the chamfer face's extent.
std::string long_leg_face(const Probe& p) {
    const double dy = p.y_max - p.y_min, dz = p.z_max - p.z_min;
    if (std::abs(dy - dz) < 0.5) return "ambiguous";
    return dy > dz ? "top(+Z)" : "front(-Y)";
}

void report(const Probe& p, Upstream u) {
    std::fprintf(stderr,
                 "chamfer_ref: %-22s top=f:%d front=f:%d (smaller ordinal: %s)  chamfer face "
                 "y=[%.6f, %.6f] z=[%.6f, %.6f]  4mm leg on %s  vol %.6f -> %.6f  faces %zu -> %zu\n",
                 upstream_name(u), p.top_ordinal, p.front_ordinal,
                 p.top_ordinal < p.front_ordinal ? "top(+Z)" : "front(-Y)", p.y_min, p.y_max,
                 p.z_min, p.z_max, long_leg_face(p).c_str(), p.volume_before, p.volume_after,
                 p.faces_before, p.faces_after);
}

// The chamfer face is a 4 mm × 1 mm patch anchored on the ORIGINAL edge corner
// (y = 0 and z = 10 both stay on it), whichever way round the legs landed. This
// holds in every run and pins that the asymmetric params really reached the
// kernel — an equal-leg read of them would give a 4×4 or 1×1 patch.
void expect_asymmetric_patch(const Probe& p, Upstream u) {
    if (!p.ok) return;
    const std::string tag = std::string("[") + upstream_name(u) + "] ";
    const double dy = p.y_max - p.y_min, dz = p.z_max - p.z_min;
    check_near(p.y_min, 0.0, 1e-5, tag + "chamfer face still meets the front wall at y = 0");
    check_near(p.z_max, kDepth, 1e-5, tag + "chamfer face still meets the top face at z = 10");
    check_near(std::max(dy, dz), kD1, 1e-5, tag + "chamfer face long side == radius (4 mm)");
    check_near(std::min(dy, dz), kD2, 1e-5, tag + "chamfer face short side == distance2 (1 mm)");
}

// THE PREMISE: the upstream hole must actually REORDER the edge's two adjacent
// faces, otherwise this probe is not exercising the ordinal rule at all. It is a
// property of OCCT's boolean, not of the reference-face rule, so it holds before
// and after any fix.
void expect_reordered(const Probe& base, const Probe& edited, Upstream u) {
    if (!(base.ok && edited.ok)) return;
    const bool base_top_first = base.top_ordinal < base.front_ordinal;
    const bool edited_top_first = edited.top_ordinal < edited.front_ordinal;
    check(base_top_first != edited_top_first,
          std::string("premise [") + upstream_name(u) +
              "]: the upstream hole reorders the edge's adjacent faces (plain top=f:" +
              std::to_string(base.top_ordinal) + " front=f:" + std::to_string(base.front_ordinal) +
              ", edited top=f:" + std::to_string(edited.top_ordinal) +
              " front=f:" + std::to_string(edited.front_ordinal) + ")");
}

// Nothing else in the pipeline can see the swap: the removed wedge is d1·d2/2·L
// whichever face carries d1, so the chamfer's volume delta and its +1 face are
// identical across the runs. This is why the defect is silent, and it is exactly
// why the bbox — not the volume — is the assertion below.
void expect_volume_is_blind(const Probe& base, const Probe& edited, Upstream u) {
    if (!(base.ok && edited.ok)) return;
    const std::string tag = std::string("[") + upstream_name(u) + "] ";
    const double wedge = 0.5 * kD1 * kD2 * kW;
    check_near(base.volume_before - base.volume_after, wedge, 1e-6,
               "[plain box] chamfer removes exactly d1·d2/2·L = 80 mm^3");
    check_near(edited.volume_before - edited.volume_after, wedge, 1e-6,
               tag + "chamfer removes the SAME d1·d2/2·L = 80 mm^3 (volume cannot see the swap)");
    check(edited.faces_after - edited.faces_before == base.faces_after - base.faces_before,
          tag + "chamfer adds the same one face (the face count cannot see the swap either)");
}

// THE INVARIANT (WP-F): the 4 mm leg belongs to a PHYSICAL face, so an upstream
// feature that touches neither chamfered face must not move it. Asserted as the
// chamfer face's own bounding box, which is the only signature that can tell the
// two orientations apart.
void expect_same_physical_face(const Probe& base, const Probe& edited, Upstream u) {
    if (!(base.ok && edited.ok)) return;
    const std::string tag = std::string("[") + upstream_name(u) + "] ";
    check(long_leg_face(edited) == long_leg_face(base),
          tag + "the 4 mm leg stays on the same PHYSICAL face across the upstream edit (plain: " +
              long_leg_face(base) + ", edited: " + long_leg_face(edited) + ")");
    check_near(edited.y_min, base.y_min, 1e-6, tag + "chamfer face y_min unchanged");
    check_near(edited.y_max, base.y_max, 1e-6, tag + "chamfer face y_max unchanged");
    check_near(edited.z_min, base.z_min, 1e-6, tag + "chamfer face z_min unchanged");
    check_near(edited.z_max, base.z_max, 1e-6, tag + "chamfer face z_max unchanged");
}

void expect_chamfer_ok(const Probe& p, const std::string& tag) {
    check(p.status_ok && p.needs_repair.empty() && p.ok,
          tag + "chamfer: Ok, no NeedsRepair (" + p.error_code + " " + p.error_message +
              " repairs=" + p.needs_repair.dump() + ")");
}

// ── shared model builder for the cases that need their own topology ──────────

// The 40×20×10 box on its own store. `false` ⇒ the extrude failed; the caller
// stops rather than asserting against an empty store.
bool build_box(BodyStore& bodies, em::ElementMapPartition& part, Ctx& c) {
    c.sketches.push_back({"sk1", rect_sketch()});
    c.last_sketch = "sk1";
    ops::OpContext ctx = c.make(bodies, part);
    json op = {{"opType", "Extrude"}, {"opId", "ope"},
               {"params", {{"sketchId", "sk1"}, {"distance", kDepth},
                           {"extrudeMode", "Blind"}, {"booleanMode", "NewBody"}}}};
    const ops::OpOutcome oc = ops::execute_extrude(ctx, op, "ope");
    return oc.status == ops::OpOutcome::Status::Ok && bodies.contains("body_ope");
}

Probe outcome_probe(const ops::OpOutcome& oc) {
    Probe p;
    p.status_ok = oc.status == ops::OpOutcome::Status::Ok;
    p.error_code = oc.error_code;
    p.error_message = oc.error_message;
    for (const json& item : oc.needs_repair) p.needs_repair.push_back(item);
    for (const json& item : oc.diagnostics) p.diagnostics.push_back(item);
    return p;
}

// ── the op-built §9 `legacyReferenceFace` item, field for field (SCHEMA §9) ──
void expect_legacy_item(const Probe& p, const std::string& tag, const char* ref_id,
                        const char* seed_edge_id,
                        const std::vector<std::string>& candidate_keys) {
    std::fprintf(stderr, "%s needsRepair = %s\n", tag.c_str(), p.needs_repair.dump().c_str());
    // A halt is STATE, not an error: the step's status is Ok and the body is
    // untouched. Reporting it as a failure would make a repairable record look
    // like a broken document.
    check(p.status_ok,
          tag + "the halt is STATE, not an error (code '" + p.error_code + "')");
    if (p.needs_repair.size() != 1) {
        check(false, tag + "exactly ONE §9 item, got " + p.needs_repair.dump());
        return;
    }
    const json& item = p.needs_repair[0];
    check(item.value("refId", "") == ref_id,
          tag + "refId == " + ref_id + " (got '" + item.value("refId", "") + "')");
    check(item.value("reason", "") == "legacyReferenceFace",
          tag + "reason == legacyReferenceFace (got '" + item.value("reason", "") + "')");
    check(item.value("ladderFailed", "") == "descriptor", tag + "ladderFailed == descriptor");
    check(item.value("elementId", "?") == "",
          tag + "elementId is EMPTY — the slot does not exist yet, so the repair is a CREATE");
    check(item.value("seedEdgeId", "") == seed_edge_id,
          tag + "seedEdgeId == " + seed_edge_id + " (got '" + item.value("seedEdgeId", "") +
              "')");
    check(item.value("scoringVersion", 0) == onecad::elementmap::kResolverVersion,
          tag + "scoringVersion == kResolverVersion");
    check(item.contains("anchor") && item["anchor"].contains("worldPoint"),
          tag + "the item carries the seed edge's anchor");
    const json& candidates = item["candidates"];
    if (candidates.size() != candidate_keys.size()) {
        check(false, tag + "candidate count == " + std::to_string(candidate_keys.size()) +
                         ", got " + candidates.dump());
        return;
    }
    for (std::size_t i = 0; i < candidate_keys.size(); ++i) {
        check(candidates[i].value("topoKey", "") == candidate_keys[i],
              tag + "candidate " + std::to_string(i) + " == " + candidate_keys[i] + " (got '" +
                  candidates[i].value("topoKey", "") + "')");
        // A DELIBERATE tie: the ordinal rule is exactly what must stop deciding,
        // so neither candidate may carry an auto-bindable score.
        check_near(candidates[i].value("score", -1.0), 0.5, 1e-12,
                   tag + "candidate " + std::to_string(i) + " scores the deliberate tie 0.5");
        check_near(candidates[i].value("margin", -1.0), 0.0, 1e-12,
                   tag + "candidate " + std::to_string(i) + " margin 0 (the user MUST choose)");
        check(!candidates[i].value("summary", std::string()).empty(),
              tag + "candidate " + std::to_string(i) + " carries the ladder's own summary");
    }
}

// ── a by-name refusal: top-level OP_FAILED + exactly ONE §7.2 diagnostic ─────
void expect_refusal(const Probe& p, const std::string& tag, const char* code,
                    const char* edge_id, const char* face_id) {
    std::fprintf(stderr, "%s diagnostics = %s (%s: %s)\n", tag.c_str(),
                 p.diagnostics.dump().c_str(), p.error_code.c_str(), p.error_message.c_str());
    check(!p.status_ok && p.error_code == "OP_FAILED",
          tag + "refused as a top-level OP_FAILED (got '" + p.error_code + "')");
    check(p.needs_repair.empty(), tag + "a refusal is not a repair prompt");
    if (p.diagnostics.size() != 1) {
        check(false, tag + "exactly ONE diagnostic, got " + p.diagnostics.dump());
        return;
    }
    const json& d = p.diagnostics[0];
    check(d.value("severity", "") == "error", tag + "severity == error");
    check(d.value("code", "") == code,
          tag + "code == " + code + " (got '" + d.value("code", "") + "')");
    check(d.value("stage", "") == "resolve", tag + "stage == resolve");
    check(!d.value("message", std::string()).empty(), tag + "the diagnostic names the failure");
    const json evidence = d.value("evidence", json::object());
    const json chamfer = evidence.value("chamfer", json::object());
    check(chamfer.value("edge", "") == edge_id,
          tag + "evidence.chamfer.edge == " + edge_id + " (got '" + chamfer.value("edge", "") +
              "')");
    check(chamfer.value("face", "") == face_id,
          tag + "evidence.chamfer.face == " + face_id + " (got '" + chamfer.value("face", "") +
              "')");
}

// ── (e) an UNCOVERED contour of a partially typed record ─────────────────────
//
// The top-front edge (centre (20, 0, 10)) and the top ∩ -X-wall edge (centre
// (0, 10, 10)) meet at the (0, 0, 10) corner at 90°, so they are two CONTOURS.
// One pair covers the first; the second is NOT a refusal — it halts on the slot
// its own pair will occupy (N = 2 edge refs + 1 existing pair ⇒ `opc.input3`)
// naming ITS OWN seed edge, so the repair converges one pick at a time.
void test_uncovered_contour() {
    BodyStore bodies;
    em::ElementMapPartition part;
    Ctx c;
    if (!build_box(bodies, part, c)) {
        check(false, "[uncovered] the box extruded");
        return;
    }
    const std::string body = "body_ope";
    const TopoDS_Shape pre = bodies.get(body)->geom;
    const TopoDS_Shape e1 = nearest(pre, TopAbs_EDGE, kW / 2.0, 0.0, kDepth);
    const TopoDS_Shape e2 = nearest(pre, TopAbs_EDGE, 0.0, kH / 2.0, kDepth);
    const TopoDS_Shape top = nearest(pre, TopAbs_FACE, kW / 2.0, kH / 2.0, kDepth);
    json op = {
        {"opType", "Chamfer"}, {"opId", "opc"},
        {"inputs", json::array({
             element_input(body, "el_edge", "edge", e1, kW / 2.0, 0.0, kDepth),
             element_input(body, "el_edge2", "edge", e2, 0.0, kH / 2.0, kDepth),
             element_input(body, "el_top", "face", top, kW / 2.0, kH / 2.0, kDepth)})},
        {"params", {{"mode", "Chamfer"}, {"radius", kD1}, {"distance2", kD2},
                    {"edgeIds", json::array({"el_edge", "el_edge2"})},
                    {"chainTangentEdges", true}, {"tangentClosureVersion", 1},
                    {"referenceFaces",
                     json::array({json{{"edgeId", "el_edge"}, {"faceId", "el_top"}}})}}}};
    ops::OpContext ctx = c.make(bodies, part);
    const Probe p = outcome_probe(ops::execute_chamfer(ctx, op, "opc"));
    // The -X wall is f:1 and the top f:6 on the plain box (printed by the probe
    // runs above), so the uncovered contour's seed offers exactly those two.
    expect_legacy_item(p, "[uncovered contour] ", "opc.input3", "el_edge2", {"f:1", "f:6"});
    check_near(onecad::session::shape_volume(bodies.get(body)->geom), kW * kH * kDepth, 1e-6,
               "[uncovered contour] the halt left the body untouched");
}

// ── (f) two pairs disagreeing on ONE tangent chain ───────────────────────────
//
// A 5 mm fillet on the (0, 0) vertical edge turns the box's top outline into a
// line–arc–line TANGENT chain, so those three top edges are ONE contour with ONE
// reference face. Pairing two of them with DIFFERENT faces is a record defect:
// resolving it by precedence would silently drop one of the user's two answers.
void test_reference_face_conflict() {
    BodyStore bodies;
    em::ElementMapPartition part;
    Ctx c;
    if (!build_box(bodies, part, c)) {
        check(false, "[conflict] the box extruded");
        return;
    }
    const std::string body = "body_ope";
    {
        const TopoDS_Shape corner =
            nearest(bodies.get(body)->geom, TopAbs_EDGE, 0.0, 0.0, kDepth / 2.0);
        ops::OpContext ctx = c.make(bodies, part);
        json op = {{"opType", "Fillet"}, {"opId", "opf"},
                   {"inputs", json::array({element_input(body, "el_corner", "edge", corner, 0.0,
                                                         0.0, kDepth / 2.0)})},
                   {"params", {{"mode", "Fillet"}, {"radius", 5.0}}}};
        const ops::OpOutcome oc = ops::execute_fillet(ctx, op, "opf");
        check(oc.status == ops::OpOutcome::Status::Ok && oc.needs_repair.empty(),
              std::string("[conflict] the corner fillet built (") + oc.error_code + " " +
                  oc.error_message + ")");
        if (oc.status != ops::OpOutcome::Status::Ok || !oc.needs_repair.empty()) return;
    }
    const TopoDS_Shape pre = bodies.get(body)->geom;
    // The three tangent top edges after the 5 mm fillet, and the two DIFFERENT
    // faces the two straight ones are paired with.
    const TopoDS_Shape front_top = nearest(pre, TopAbs_EDGE, (5.0 + kW) / 2.0, 0.0, kDepth);
    const TopoDS_Shape arc_top =
        nearest(pre, TopAbs_EDGE, 5.0 - 5.0 * M_SQRT1_2, 5.0 - 5.0 * M_SQRT1_2, kDepth);
    const TopoDS_Shape side_top = nearest(pre, TopAbs_EDGE, 0.0, (5.0 + kH) / 2.0, kDepth);
    const TopoDS_Shape top = nearest(pre, TopAbs_FACE, kW / 2.0, kH / 2.0, kDepth);
    const TopoDS_Shape side = nearest(pre, TopAbs_FACE, 0.0, (5.0 + kH) / 2.0, kDepth / 2.0);
    json op = {
        {"opType", "Chamfer"}, {"opId", "opc"},
        {"inputs", json::array({
             element_input(body, "el_front", "edge", front_top, (5.0 + kW) / 2.0, 0.0, kDepth),
             element_input(body, "el_arc", "edge", arc_top, 5.0 - 5.0 * M_SQRT1_2,
                           5.0 - 5.0 * M_SQRT1_2, kDepth),
             element_input(body, "el_side", "edge", side_top, 0.0, (5.0 + kH) / 2.0, kDepth),
             element_input(body, "el_top", "face", top, kW / 2.0, kH / 2.0, kDepth),
             element_input(body, "el_wall", "face", side, 0.0, (5.0 + kH) / 2.0, kDepth / 2.0)})},
        {"params", {{"mode", "Chamfer"}, {"radius", 1.0}, {"distance2", 0.5},
                    {"edgeIds", json::array({"el_front", "el_arc", "el_side"})},
                    {"chainTangentEdges", true}, {"tangentClosureVersion", 1},
                    {"referenceFaces",
                     json::array({json{{"edgeId", "el_front"}, {"faceId", "el_top"}},
                                  json{{"edgeId", "el_side"}, {"faceId", "el_wall"}}})}}}};
    ops::OpContext ctx = c.make(bodies, part);
    const Probe p = outcome_probe(ops::execute_chamfer(ctx, op, "opc"));
    // The evidence names the SECOND pair — the one that disagreed.
    expect_refusal(p, "[conflict] ", "CHAMFER_REFERENCE_FACE_CONFLICT", "el_side", "el_wall");
}

// ── (h) the shared `adjacent_face_ordinals` helper (SCHEMA §7.6) ─────────────
void test_adjacent_face_ordinals() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    const std::vector<int> box_faces = onecad::session::adjacent_face_ordinals(
        box, TopoDS::Edge(nearest(box, TopAbs_EDGE, 0.0, 0.0, 5.0)));
    check(box_faces.size() == 2,
          "adjacency: a manifold box edge lists TWO faces, got " +
              std::to_string(box_faces.size()));
    if (box_faces.size() == 2) {
        check(box_faces[0] < box_faces[1], "adjacency: face ordinals ASCENDING");
    }

    // A full cylinder's SEAM edge is bounded on BOTH sides by the one lateral
    // face. `TopTools_IndexedMapOfShape::FindIndex` identifies `IsSame` faces, so
    // the raw ancestor list has two entries and the published list has ONE.
    const TopoDS_Shape cylinder = BRepPrimAPI_MakeCylinder(5.0, 10.0).Shape();
    TopTools_IndexedMapOfShape edges;
    TopExp::MapShapes(cylinder, TopAbs_EDGE, edges);
    TopTools_IndexedDataMapOfShapeListOfShape ancestors;
    TopExp::MapShapesAndAncestors(cylinder, TopAbs_EDGE, TopAbs_FACE, ancestors);
    int seams = 0, circles = 0;
    for (int i = 1; i <= edges.Extent(); ++i) {
        const std::vector<int> adjacent =
            onecad::session::adjacent_face_ordinals(cylinder, TopoDS::Edge(edges(i)));
        const int raw = ancestors(ancestors.FindIndex(edges(i))).Extent();
        std::fprintf(stderr, "adjacency: cylinder e:%d raw=%d deduped=%zu\n", i, raw,
                     adjacent.size());
        if (adjacent.size() == 1) {
            ++seams;
            check(raw == 2,
                  "adjacency: the seam's RAW ancestor list really did hold two entries");
        } else if (adjacent.size() == 2) {
            ++circles;
            check(adjacent[0] < adjacent[1], "adjacency: cylinder cap edge ASCENDING");
        }
    }
    check(seams == 1, "adjacency: exactly ONE seam edge yields a single entry, got " +
                          std::to_string(seams));
    check(circles == 2, "adjacency: both cap circles yield two entries, got " +
                            std::to_string(circles));
}

}  // namespace

int main() {
    try {
        // ── (a) THE INVARIANT: a TYPED pair anchors the 4 mm leg to a PHYSICAL
        //     face, so an upstream feature that touches neither chamfered face
        //     cannot move it. This is what the whole work package is for. ──
        const ChamferSpec typed{.typed = true};
        const Probe t_plain = run_probe(Upstream::None, typed);
        const Probe t_blind = run_probe(Upstream::HoleMinusXBlind, typed);
        const Probe t_through = run_probe(Upstream::HoleMinusXThrough, typed);
        std::fprintf(stderr, "-- (a) typed `referenceFaces` pair naming the TOP face --\n");
        report(t_plain, Upstream::None);
        report(t_blind, Upstream::HoleMinusXBlind);
        report(t_through, Upstream::HoleMinusXThrough);
        expect_chamfer_ok(t_plain, "[typed, plain box] ");
        expect_chamfer_ok(t_blind, "[typed, hole -X wall, blind] ");
        expect_chamfer_ok(t_through, "[typed, hole -X wall, through] ");
        expect_asymmetric_patch(t_plain, Upstream::None);
        expect_asymmetric_patch(t_blind, Upstream::HoleMinusXBlind);
        expect_asymmetric_patch(t_through, Upstream::HoleMinusXThrough);
        // The premise still holds under the fix — the hole reorders the pair; the
        // pair simply stops caring.
        expect_reordered(t_plain, t_blind, Upstream::HoleMinusXBlind);
        expect_reordered(t_plain, t_through, Upstream::HoleMinusXThrough);
        expect_volume_is_blind(t_plain, t_blind, Upstream::HoleMinusXBlind);
        expect_volume_is_blind(t_plain, t_through, Upstream::HoleMinusXThrough);
        expect_same_physical_face(t_plain, t_blind, Upstream::HoleMinusXBlind);
        expect_same_physical_face(t_plain, t_through, Upstream::HoleMinusXThrough);
        // …and the pair really DECIDED it: on the plain box the ordinal rule picks
        // the front wall, so a pair naming the top is a visible, opposite choice.
        // Without this the invariant above could pass on a coincidence.
        check(long_leg_face(t_plain) == "top(+Z)",
              "[typed, plain box] the 4 mm leg is on the PAIRED face (top), not the "
              "smaller-ordinal one (got " + long_leg_face(t_plain) + ")");

        // ── (b)+(c) A LEGACY record — asymmetric, no pair — HALTS, in EVERY
        //     lane. The ordinal rule is gone, so there is nothing left to fall
        //     back to. `post_upstream_edit` is deliberately NOT consulted: it is a
        //     per-PLAN claim that every `ToEnd(0)` lane sets and that `RevertToEnd`
        //     (undo/redo) and `RegenToStep` previews omit, so gating the halt on it
        //     would fire on open and vanish on the very next redo. Both values must
        //     produce the SAME item. ──
        const Probe halted_replay = run_probe(Upstream::None);
        const Probe halted_post_edit = run_probe(Upstream::None, {.post_upstream_edit = true});
        std::fprintf(stderr, "-- (b)+(c) LEGACY record with no pair: halts in every lane --\n");
        // The plain box orders the edge's adjacent pair front f:2 < top f:6.
        expect_legacy_item(halted_replay, "[legacy, no edit context] ", "opc.input1", "el_edge",
                           {"f:2", "f:6"});
        expect_legacy_item(halted_post_edit, "[legacy, post-edit] ", "opc.input1", "el_edge",
                           {"f:2", "f:6"});
        check(halted_replay.needs_repair == halted_post_edit.needs_repair,
              "[legacy] the halt is IDENTICAL with and without an edit context — a guard "
              "that changed shape between an open and a redo would protect nothing");
        // The same record downstream of the ordinal-permuting hole halts too, so
        // there is no lane in which the old mirrored geometry can still be built.
        expect_legacy_item(run_probe(Upstream::HoleMinusXBlind), "[legacy, after the hole] ",
                           "opc.input1", "el_edge", {"f:4", "f:5"});

        // ── (b2) THE NEGATIVE CONTROL for (a): the same record differing ONLY in
        //     which adjacent face the pair names produces the OPPOSITE geometry,
        //     and keeps it across the hole. Without this, (a) could pass on a
        //     coincidence of the box's face order. ──
        const ChamferSpec typed_front{.typed = true, .pair_face = PairFace::Front};
        const Probe f_plain = run_probe(Upstream::None, typed_front);
        const Probe f_blind = run_probe(Upstream::HoleMinusXBlind, typed_front);
        std::fprintf(stderr, "-- (b2) the SAME record pairing the FRONT wall instead --\n");
        report(f_plain, Upstream::None);
        report(f_blind, Upstream::HoleMinusXBlind);
        expect_chamfer_ok(f_plain, "[pair=front, plain box] ");
        expect_chamfer_ok(f_blind, "[pair=front, hole -X wall, blind] ");
        expect_asymmetric_patch(f_plain, Upstream::None);
        expect_same_physical_face(f_plain, f_blind, Upstream::HoleMinusXBlind);
        check(long_leg_face(f_plain) == "front(-Y)",
              "[pair=front, plain box] the 4 mm leg follows the PAIR onto the front wall (got " +
                  long_leg_face(f_plain) + ")");
        check(long_leg_face(f_plain) != long_leg_face(t_plain),
              "[plain box] two records differing ONLY in the pair produce OPPOSITE geometry — "
              "the pair, and nothing else, decides");

        // ── (d) a pair naming a face that is NOT adjacent to its edge ──
        const Probe not_adjacent =
            run_probe(Upstream::None, {.typed = true, .pair_face = PairFace::Bottom});
        expect_refusal(not_adjacent, "[not adjacent] ", "CHAMFER_REFERENCE_FACE_NOT_ADJACENT",
                       "el_edge", "el_bottom");

        // ── (e) an UNCOVERED contour of a partially typed record ──
        test_uncovered_contour();

        // ── (f) two pairs disagreeing on ONE tangent chain ──
        test_reference_face_conflict();

        // ── (g) the distance-angle form (`AddDA`) shares the reference face ──
        const ChamferSpec typed_angle{.typed = true, .angle = true};
        const Probe a_plain = run_probe(Upstream::None, typed_angle);
        const Probe a_blind = run_probe(Upstream::HoleMinusXBlind, typed_angle);
        std::fprintf(stderr, "-- (g) angleDeg %.1f with a typed pair --\n", kAngleDeg);
        report(a_plain, Upstream::None);
        report(a_blind, Upstream::HoleMinusXBlind);
        expect_chamfer_ok(a_plain, "[angle, plain box] ");
        expect_chamfer_ok(a_blind, "[angle, hole -X wall, blind] ");
        expect_same_physical_face(a_plain, a_blind, Upstream::HoleMinusXBlind);
        check(long_leg_face(a_plain) == "top(+Z)",
              "[angle, plain box] AddDA measures `radius` on the PAIRED face (got " +
                  long_leg_face(a_plain) + ")");
        // `radius` on the reference face, the far leg radius·tan(angle): proof the
        // angle really reached `AddDA` rather than being read as a second distance.
        if (a_plain.ok) {
            check_near(a_plain.y_max - a_plain.y_min, kD1, 1e-5,
                       "[angle, plain box] the leg ON the reference face == radius");
            check_near(a_plain.z_max - a_plain.z_min, kD1 * std::tan(kAngleDeg * M_PI / 180.0),
                       1e-5, "[angle, plain box] the far leg == radius*tan(angleDeg)");
        }

        // ── (h) the shared adjacency helper ──
        test_adjacent_face_ordinals();

        // ── (i) NON-v1 records (no `tangentClosureVersion`) key the pairs per
        //     EDGE and execute seed-only; the same two rules apply. ──
        const ChamferSpec non_v1{.typed = true, .v1 = false};
        const Probe n_plain = run_probe(Upstream::None, non_v1);
        const Probe n_blind = run_probe(Upstream::HoleMinusXBlind, non_v1);
        std::fprintf(stderr, "-- (i) NON-v1 record (no tangentClosureVersion) --\n");
        report(n_plain, Upstream::None);
        report(n_blind, Upstream::HoleMinusXBlind);
        expect_chamfer_ok(n_plain, "[non-v1 typed, plain box] ");
        expect_chamfer_ok(n_blind, "[non-v1 typed, hole -X wall, blind] ");
        expect_same_physical_face(n_plain, n_blind, Upstream::HoleMinusXBlind);
        check(long_leg_face(n_plain) == "top(+Z)",
              "[non-v1 typed, plain box] the 4 mm leg is on the PAIRED face (got " +
                  long_leg_face(n_plain) + ")");
        const Probe n_halted =
            run_probe(Upstream::None, {.post_upstream_edit = true, .v1 = false});
        expect_legacy_item(n_halted, "[non-v1 legacy, post-edit] ", "opc.input1", "el_edge",
                           {"f:2", "f:6"});
    } catch (const std::exception& e) {
        std::fprintf(stderr, "FAIL: exception %s\n", e.what());
        ++g_failures;
    }
    if (g_failures == 0) std::fprintf(stderr, "test_chamfer_reference_face: all checks passed\n");
    return g_failures == 0 ? 0 : 1;
}
