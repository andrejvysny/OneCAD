// test_offsetface.cpp — OFFSET-FACE W1: the `op.offsetFace` executor (SCHEMA §7.3)
// and the read-only `PrepareOffsetFace` authoring handshake (SCHEMA §7.6).
//
// In-process, real OCCT, driven through `ops::execute_offset_face` /
// `session::handle_prepare_offset_face` — the same entry points PlanExecutor and the
// verb dispatcher use. Numbers are EXACT analytic volumes/radii wherever the geometry
// is analytic; the Phase-0 spike (test_offsetface_spike.cpp) is the characterization
// record this suite builds the CONTRACT on top of.
//
// No framework: exit code == failure count.
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <string>
#include <utility>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepGProp.hxx>
#include <BRepTopAdaptor_FClass2d.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeSphere.hxx>
#include <BRepPrimAPI_MakeWedge.hxx>
#include <BRep_Builder.hxx>
#include <GProp_GProps.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax2.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>
#include <gp_Pnt2d.hxx>
#include <gp_Vec.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "ops/OffsetFaceOp.h"
#include "ops/OpTypes.h"
#include "protocol/Envelope.h"
#include "session/BodyStore.h"
#include "session/HistoryHash.h"
#include "session/PrepareOffsetFace.h"
#include "session/ScratchJob.h"
#include "session/Session.h"
#include "util/Cancel.h"
#include "util/Log.h"

using nlohmann::json;
using onecad::protocol::Envelope;
using onecad::session::BodyStore;
using onecad::session::Session;
namespace ops = onecad::ops;
namespace of = onecad::ops::offsetface;
namespace em = onecad::elementmap;
namespace km = onecad::kernel::elementmap;

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
        std::fprintf(stderr, "FAIL: %s (got %.6f want %.6f)\n", msg.c_str(), got, want);
        ++g_failures;
    }
}

double volume_of(const TopoDS_Shape& s) {
    GProp_GProps props;
    BRepGProp::VolumeProperties(s, props);
    return props.Mass();
}

int face_count(const TopoDS_Shape& s) {
    TopTools_IndexedMapOfShape m;
    TopExp::MapShapes(s, TopAbs_FACE, m);
    return m.Extent();
}

// The 1-based face ordinal whose surface barycenter is nearest (cx,cy,cz).
int face_ordinal_near(const TopoDS_Shape& shape, double cx, double cy, double cz) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    int best = 0;
    double best_d2 = -1.0;
    for (int i = 1; i <= faces.Extent(); ++i) {
        GProp_GProps props;
        BRepGProp::SurfaceProperties(faces(i), props);
        const gp_Pnt c = props.CentreOfMass();
        const double dx = c.X() - cx, dy = c.Y() - cy, dz = c.Z() - cz;
        const double d2 = dx * dx + dy * dy + dz * dz;
        if (best_d2 < 0.0 || d2 < best_d2) {
            best_d2 = d2;
            best = i;
        }
    }
    return best;
}

std::string key_near(const TopoDS_Shape& shape, double cx, double cy, double cz) {
    return of::face_topokey(face_ordinal_near(shape, cx, cy, cz));
}

// Ordinals of every cylindrical face, ascending.
std::vector<int> cylindrical_ordinals(const TopoDS_Shape& shape) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    std::vector<int> out;
    for (int i = 1; i <= faces.Extent(); ++i) {
        if (of::surface_kind(TopoDS::Face(faces(i))) == of::SurfaceKind::Cylinder) out.push_back(i);
    }
    return out;
}

struct Ctx {
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;
    ops::OpContext make(BodyStore& b, em::ElementMapPartition& p) {
        return ops::OpContext{b, &sketches, p, &last_sketch, false, json::object(), &cancel};
    }
};

ops::OpOutcome run_offset(BodyStore& bodies, em::ElementMapPartition& part, json params,
                          const std::string& op_id = "op_off", json inputs = json()) {
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    json op = {{"opType", "OffsetFace"}, {"opId", op_id}, {"params", std::move(params)}};
    if (!inputs.is_null()) op["inputs"] = std::move(inputs);
    return ops::execute_offset_face(ctx, op, op_id);
}

// `faceIds` as snapshot-scoped TopoKeys — the pre-promotion addressing form SCHEMA
// §7.3 allows, which keeps the numeric tests free of id-minting ceremony. The typed
// `inputs[]` ladder rung gets its own dedicated test below.
json offset_params(const std::string& body_id, std::vector<std::string> keys, double distance,
                   const char* type = "Offset", bool chain = true,
                   const std::string& opposite = {}) {
    json ids = json::array();
    for (const std::string& k : keys) ids.push_back(k);
    json primary = json::array();
    if (!keys.empty()) primary.push_back(keys.front());
    json p = {{"targetBodyId", body_id},
              {"faceIds", std::move(ids)},
              {"primaryFaceIds", std::move(primary)},
              {"resultPolicyVersion", 2},
              {"distance", distance},
              {"distanceType", type},
              {"chainTangentFaces", chain}};
    if (!opposite.empty()) p["oppositeFaceId"] = opposite;
    return p;
}

bool failed_with(const ops::OpOutcome& o, const std::string& needle) {
    return o.status == ops::OpOutcome::Status::Failed &&
           o.error_message.find(needle) != std::string::npos;
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. Executor numerics
// ═══════════════════════════════════════════════════════════════════════════

// Box 10³, top face ±2 — exact volumes, face count preserved, single `modified`
// body event on the TARGET (never NewBody, never a fan-out).
void test_box_top_plus_minus() {
    for (const auto& [d, want] : std::vector<std::pair<double, double>>{{2.0, 1200.0},
                                                                       {-2.0, 800.0}}) {
        const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
        BodyStore bodies;
        bodies.create("body_1", "op_seed", box);
        em::ElementMapPartition part;
        const std::string top = key_near(box, 5, 5, 10);
        const ops::OpOutcome o =
            run_offset(bodies, part, offset_params("body_1", {top}, d));
        const std::string label = "box top " + std::to_string(d);
        check(o.status == ops::OpOutcome::Status::Ok, label + ": Ok (" + o.error_message + ")");
        check(o.needs_repair.empty(), label + ": no NeedsRepair");
        check(o.body_events.size() == 1 && o.body_events[0].kind == "modified" &&
                  o.body_events[0].body_id == "body_1",
              label + ": single modified event on the target");
        check(o.body_events.size() == 1 && o.body_events[0].rank_key.has_value(),
              label + ": rankKey published");
        if (bodies.contains("body_1")) {
            const TopoDS_Shape r = bodies.get("body_1")->geom;
            check_near(volume_of(r), want, 1e-6, label + ": exact volume");
            check(face_count(r) == 6, label + ": still 6 faces");
        }
    }
}

// Frustum top +2 must PRESERVE the taper (716.16), not push/pull a prism (725.33).
// 7-arg MakeWedge tapers along Y in both x and z (spike test 3).
void test_frustum_taper_preserved() {
    const TopoDS_Shape frustum = BRepPrimAPI_MakeWedge(10.0, 10.0, 10.0, 2.0, 2.0, 8.0, 8.0).Shape();
    check_near(volume_of(frustum), 653.3333, 1e-3, "frustum: base volume");
    BodyStore bodies;
    bodies.create("body_1", "op_seed", frustum);
    em::ElementMapPartition part;
    // The tapered top is the face at max Y (barycenter y=10, x=z=5).
    const std::string top = key_near(frustum, 5, 10, 5);
    const ops::OpOutcome o = run_offset(bodies, part, offset_params("body_1", {top}, 2.0));
    check(o.status == ops::OpOutcome::Status::Ok, "frustum+2: Ok (" + o.error_message + ")");
    if (bodies.contains("body_1")) {
        check_near(volume_of(bodies.get("body_1")->geom), 716.16, 0.5,
                   "frustum+2: taper preserved (716.16, not 725.33)");
        check(face_count(bodies.get("body_1")->geom) == face_count(frustum),
              "frustum+2: face count preserved");
    }
}

// Full cylinder lateral face: Offset +2 and Radius 12 must agree, both giving r=12.
void test_cylinder_radius_grow() {
    for (const char* type : {"Offset", "Radius"}) {
        const TopoDS_Shape cyl = BRepPrimAPI_MakeCylinder(10.0, 20.0).Shape();
        const std::vector<int> lats = cylindrical_ordinals(cyl);
        check(lats.size() == 1, "cyl: one lateral face");
        if (lats.empty()) return;
        BodyStore bodies;
        bodies.create("body_1", "op_seed", cyl);
        em::ElementMapPartition part;
        const double value = (std::string(type) == "Offset") ? 2.0 : 12.0;
        const ops::OpOutcome o = run_offset(
            bodies, part, offset_params("body_1", {of::face_topokey(lats[0])}, value, type));
        const std::string label = std::string("cyl ") + type;
        check(o.status == ops::OpOutcome::Status::Ok, label + ": Ok (" + o.error_message + ")");
        if (bodies.contains("body_1")) {
            const TopoDS_Shape r = bodies.get("body_1")->geom;
            check_near(volume_of(r), M_PI * 144.0 * 20.0, 1e-3, label + ": volume at r=12");
            const std::vector<int> out_lats = cylindrical_ordinals(r);
            check(out_lats.size() == 1, label + ": one lateral face out");
            if (!out_lats.empty()) {
                TopTools_IndexedMapOfShape faces;
                TopExp::MapShapes(r, TopAbs_FACE, faces);
                BRepAdaptor_Surface surf(TopoDS::Face(faces(out_lats[0])));
                check_near(surf.Cylinder().Radius(), 12.0, 1e-9, label + ": exact radius 12");
            }
        }
    }
}

// σ-CONVENTION regression (spike test 5): growing a HOLE r5 → r6 needs the kernel
// offset d = −1. A naive `d = target − R` would send d = +1 and SHRINK the hole to
// r=4. Both absolute types must land on the same geometry.
void test_hole_sigma_convention() {
    const TopoDS_Shape slab = BRepPrimAPI_MakeBox(gp_Pnt(-10, -10, 0), 20.0, 20.0, 10.0).Shape();
    const TopoDS_Shape drill =
        BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0, 0, -1), gp_Dir(0, 0, 1)), 5.0, 12.0).Shape();
    const TopoDS_Shape holed = BRepAlgoAPI_Cut(slab, drill).Shape();
    const double want_r6 = 4000.0 - M_PI * 36.0 * 10.0;
    const double naive_r4 = 4000.0 - M_PI * 16.0 * 10.0;

    for (const auto& [type, value] :
         std::vector<std::pair<const char*, double>>{{"Radius", 6.0}, {"Diameter", 12.0}}) {
        const std::vector<int> lats = cylindrical_ordinals(holed);
        check(lats.size() == 1, "hole: one cylindrical face");
        if (lats.empty()) return;
        BodyStore bodies;
        bodies.create("body_1", "op_seed", holed);
        em::ElementMapPartition part;
        const ops::OpOutcome o = run_offset(
            bodies, part, offset_params("body_1", {of::face_topokey(lats[0])}, value, type));
        const std::string label = std::string("hole ") + type;
        check(o.status == ops::OpOutcome::Status::Ok, label + ": Ok (" + o.error_message + ")");
        if (bodies.contains("body_1")) {
            const double v = volume_of(bodies.get("body_1")->geom);
            check_near(v, want_r6, 1e-3, label + ": hole GREW to r=6");
            check(std::abs(v - naive_r4) > 1.0, label + ": not the naive-sign r=4 result");
        }
    }
}

// Split cylindrical wall (two G1-tangent half-lateral faces, spike test 6).
//   (a) both faces in the stored operative set ⇒ the chained r=12 result;
//   (b) only ONE stored ⇒ the closure exceeds the set. The kernel would silently
//       move the neighbour anyway, so the op must refuse LOUDLY.
void test_split_cylinder_chain() {
    const gp_Ax2 up(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1), gp_Dir(1, 0, 0));
    const gp_Ax2 down(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1), gp_Dir(-1, 0, 0));
    const TopoDS_Shape fused =
        BRepAlgoAPI_Fuse(BRepPrimAPI_MakeCylinder(up, 10.0, 20.0, M_PI).Shape(),
                         BRepPrimAPI_MakeCylinder(down, 10.0, 20.0, M_PI).Shape())
            .Shape();
    const std::vector<int> lats = cylindrical_ordinals(fused);
    check(lats.size() == 2, "splitcyl: two tangent lateral faces");
    if (lats.size() != 2) return;

    // The G1 closure of ONE half must contain BOTH halves — this is what the guard
    // is guarding, asserted directly on the shared helper.
    const of::ClosureResult closure = of::tangent_closure(fused, {lats[0]});
    check(closure.ok && closure.ordinals.size() == 2, "splitcyl: closure of one half is both");

    {  // (a) both stored
        BodyStore bodies;
        bodies.create("body_1", "op_seed", fused);
        em::ElementMapPartition part;
        const ops::OpOutcome o = run_offset(
            bodies, part,
            offset_params("body_1", {of::face_topokey(lats[0]), of::face_topokey(lats[1])}, 12.0,
                          "Radius"));
        check(o.status == ops::OpOutcome::Status::Ok, "splitcyl both: Ok (" + o.error_message + ")");
        if (bodies.contains("body_1")) {
            check_near(volume_of(bodies.get("body_1")->geom), M_PI * 144.0 * 20.0, 1e-3,
                       "splitcyl both: r=12 volume");
        }
    }
    {  // (b) one stored ⇒ chain exceeds the operative set
        BodyStore bodies;
        bodies.create("body_1", "op_seed", fused);
        em::ElementMapPartition part;
        const TopoDS_Shape before = fused;
        const ops::OpOutcome o = run_offset(
            bodies, part, offset_params("body_1", {of::face_topokey(lats[0])}, 12.0, "Radius"));
        check(failed_with(o, "tangent chain exceeds stored operative set"),
              "splitcyl one: refused (" + o.error_code + " " + o.error_message + ")");
        check(o.error_message.find(of::face_topokey(lats[1])) != std::string::npos,
              "splitcyl one: names the missing face TopoKey");
        check(bodies.contains("body_1") &&
                  std::abs(volume_of(bodies.get("body_1")->geom) - volume_of(before)) < 1e-9,
              "splitcyl one: body untouched (never publish a refused offset)");
    }
}

// A 0.75° DRAFT pair must NOT chain: the two planar faces meet at a 179.25° dihedral,
// far outside the 1e-4 rad G1 tolerance. A 0.5–1° tolerance would swallow it and
// silently widen the operative set — this is the concrete counterexample that pins
// `kTangentAngleTol`.
void test_draft_pair_does_not_chain() {
    // Prism over a CCW polygon whose top edge kinks by exactly 0.75° at x=10.
    const double kink = 10.0 * std::tan(0.75 * M_PI / 180.0);  // ≈ 0.130896
    BRepBuilderAPI_MakePolygon poly;
    poly.Add(gp_Pnt(0.0, 0.0, 0.0));
    poly.Add(gp_Pnt(0.0, -10.0, 0.0));
    poly.Add(gp_Pnt(20.0, -10.0, 0.0));
    poly.Add(gp_Pnt(20.0, kink, 0.0));
    poly.Add(gp_Pnt(10.0, 0.0, 0.0));
    poly.Close();
    const TopoDS_Shape base = BRepBuilderAPI_MakeFace(poly.Wire()).Shape();
    const TopoDS_Shape body = BRepPrimAPI_MakePrism(base, gp_Vec(0, 0, 5)).Shape();
    check(!body.IsNull() && volume_of(body) > 0.0, "draft: prism built");

    // The flat face is the wall over the x∈[0,10] segment at y=0 (barycenter (5,0,2.5)).
    const int flat = face_ordinal_near(body, 5.0, 0.0, 2.5);
    const int drafted = face_ordinal_near(body, 15.0, kink * 0.5, 2.5);
    check(flat != drafted, "draft: the two walls are distinct faces");

    const of::ClosureResult closure = of::tangent_closure(body, {flat});
    check(closure.ok && closure.ordinals.size() == 1,
          "draft: 0.75° neighbour is NOT chained (closure stays one face)");

    // The drafted neighbour's plane must be untouched by the offset.
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(body, TopAbs_FACE, faces);
    const of::FaceSample before = of::sample_face(TopoDS::Face(faces(drafted)));
    check(before.ok, "draft: neighbour samplable");

    BodyStore bodies;
    bodies.create("body_1", "op_seed", body);
    em::ElementMapPartition part;
    // Small offset: two near-parallel planes only re-intersect close by, so 0.05 keeps
    // the corner inside the neighbouring wall (a large offset would be a topology change,
    // not a draft regression).
    const ops::OpOutcome o =
        run_offset(bodies, part, offset_params("body_1", {of::face_topokey(flat)}, 0.05));
    check(o.status == ops::OpOutcome::Status::Ok, "draft: offset Ok (" + o.error_message + ")");
    if (bodies.contains("body_1")) {
        const TopoDS_Shape r = bodies.get("body_1")->geom;
        TopTools_IndexedMapOfShape rfaces;
        TopExp::MapShapes(r, TopAbs_FACE, rfaces);
        bool found = false;
        for (int i = 1; i <= rfaces.Extent(); ++i) {
            const TopoDS_Face f = TopoDS::Face(rfaces(i));
            if (of::surface_kind(f) != of::SurfaceKind::Plane) continue;
            const of::FaceSample s = of::sample_face(f);
            if (!s.ok) continue;
            if (gp_Vec(s.normal).Dot(gp_Vec(before.normal)) < 1.0 - 1e-9) continue;
            if (std::abs(gp_Vec(before.point, s.point).Dot(gp_Vec(before.normal))) < 1e-9) {
                found = true;
                break;
            }
        }
        check(found, "draft: the neighbour plane is UNCHANGED after the offset");
    }
}

// `Total` re-derives `d = distance − t` from the PERSISTED opposite face each regen.
// Box 20×20×10, target total 14 ⇒ t=10 ⇒ d=+4 ⇒ the top lands at z=14.
void test_total_on_box() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 20.0, 10.0).Shape();
    const std::string top = key_near(box, 10, 10, 10);
    const std::string bottom = key_near(box, 10, 10, 0);
    check(top != bottom, "total: top and bottom are distinct faces");

    {
        BodyStore bodies;
        bodies.create("body_1", "op_seed", box);
        em::ElementMapPartition part;
        const ops::OpOutcome o = run_offset(
            bodies, part, offset_params("body_1", {top}, 14.0, "Total", /*chain=*/false, bottom));
        check(o.status == ops::OpOutcome::Status::Ok, "total 14: Ok (" + o.error_message + ")");
        if (bodies.contains("body_1")) {
            check_near(volume_of(bodies.get("body_1")->geom), 20.0 * 20.0 * 14.0, 1e-6,
                       "total 14: exact volume (top moved +4)");
        }
    }
    {  // Total with the tangent chain ON is a contradiction — refuse, never clamp.
        BodyStore bodies;
        bodies.create("body_1", "op_seed", box);
        em::ElementMapPartition part;
        const ops::OpOutcome o = run_offset(
            bodies, part, offset_params("body_1", {top}, 14.0, "Total", /*chain=*/true, bottom));
        check(failed_with(o, "chainTangentFaces:false"), "total+chain: refused");
    }
    {  // Missing oppositeFaceId is a malformed record, not a re-discovery trigger.
        BodyStore bodies;
        bodies.create("body_1", "op_seed", box);
        em::ElementMapPartition part;
        const ops::OpOutcome o =
            run_offset(bodies, part, offset_params("body_1", {top}, 14.0, "Total", false));
        check(failed_with(o, "oppositeFaceId"), "total without opposite: refused");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. Traps the postconditions exist for (spike test 8)
// ═══════════════════════════════════════════════════════════════════════════

void test_traps() {
    {  // (a) R + σd ≤ tol — OCCT returns a "valid" inside-out cylinder; preflight is ours.
        const TopoDS_Shape cyl = BRepPrimAPI_MakeCylinder(10.0, 20.0).Shape();
        const std::vector<int> lats = cylindrical_ordinals(cyl);
        BodyStore bodies;
        bodies.create("body_1", "op_seed", cyl);
        em::ElementMapPartition part;
        const ops::OpOutcome o = run_offset(
            bodies, part, offset_params("body_1", {of::face_topokey(lats.at(0))}, 0.0, "Radius"));
        check(failed_with(o, "would drive the cylinder radius"),
              "trap R→0: preflight refusal (" + o.error_message + ")");
        check(std::abs(volume_of(bodies.get("body_1")->geom) - volume_of(cyl)) < 1e-9,
              "trap R→0: body untouched");
    }
    {  // (b) collapse: the top driven onto the bottom passes IsDone AND BRepCheck.
        const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
        BodyStore bodies;
        bodies.create("body_1", "op_seed", box);
        em::ElementMapPartition part;
        const ops::OpOutcome o =
            run_offset(bodies, part, offset_params("body_1", {key_near(box, 5, 5, 10)}, -10.0));
        std::fprintf(stderr, "INFO: trap collapse: status=%d code=%s msg=%s\n",
                     static_cast<int>(o.status), o.error_code.c_str(), o.error_message.c_str());
        check(o.status != ops::OpOutcome::Status::Ok, "trap collapse: refused");
        check_near(volume_of(bodies.get("body_1")->geom), 1000.0, 1e-9,
                   "trap collapse: body untouched");
    }
    {  // (c) FAN-OUT: raising a U-body's floor past its web would split it in two.
        //     V1 refuses ANY multi-solid outcome; whichever way the kernel reports it
        //     (>1 solid, or an outright build failure), nothing may be published.
        const TopoDS_Shape base = BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 30.0, 10.0, 20.0).Shape();
        const TopoDS_Shape slot = BRepPrimAPI_MakeBox(gp_Pnt(10, -1, 5), 10.0, 12.0, 20.0).Shape();
        const TopoDS_Shape u = BRepAlgoAPI_Cut(base, slot).Shape();
        check_near(volume_of(u), 30.0 * 10.0 * 5.0 + 2.0 * 10.0 * 10.0 * 15.0, 1e-6,
                   "fanout: U-body base volume");
        BodyStore bodies;
        bodies.create("body_1", "op_seed", u);
        em::ElementMapPartition part;
        // Bottom face z=0, outward −Z: d=−6 lifts it to z=6, above the z=5 web.
        const ops::OpOutcome o =
            run_offset(bodies, part, offset_params("body_1", {key_near(u, 15, 5, 0)}, -6.0));
        std::fprintf(stderr, "INFO: trap fanout: status=%d code=%s msg=%s\n",
                     static_cast<int>(o.status), o.error_code.c_str(), o.error_message.c_str());
        check(o.status != ops::OpOutcome::Status::Ok, "trap fanout: refused");
        check_near(volume_of(bodies.get("body_1")->geom), volume_of(u), 1e-6,
                   "trap fanout: body untouched");
    }
    {  // (d) a TopoKey that names no face of the target body. OCCT would SILENTLY
        //     ignore a foreign face and hand back the unchanged shape as a success
        //     (spike trap c), so nothing may be published on this path either.
        const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
        BodyStore bodies;
        bodies.create("body_1", "op_seed", box);
        em::ElementMapPartition part;
        const ops::OpOutcome o = run_offset(bodies, part, offset_params("body_1", {"f:99"}, 2.0));
        check(o.body_events.empty(), "trap foreign: nothing published");
        check(o.status != ops::OpOutcome::Status::Ok || !o.needs_repair.empty(),
              "trap foreign: refused or NeedsRepair, never a silent success");
        check_near(volume_of(bodies.get("body_1")->geom), 1000.0, 1e-9,
                   "trap foreign: body untouched");
    }
    {  // (d2) MULTI-SOLID target: `OffsetFace` is a single-body modify, never a
        //      fan-out, so a body that already carries two solids is refused before
        //      the kernel is touched. (The >1-solid OUTPUT gate is the same
        //      `ranked_solids` count; see the U-body case above for the outcome the
        //      kernel actually produces when an offset would split a body.)
        BRep_Builder builder;
        TopoDS_Compound pair;
        builder.MakeCompound(pair);
        builder.Add(pair, BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape());
        builder.Add(pair, BRepPrimAPI_MakeBox(gp_Pnt(50, 0, 0), 10.0, 10.0, 10.0).Shape());
        BodyStore bodies;
        bodies.create("body_1", "op_seed", pair);
        em::ElementMapPartition part;
        const ops::OpOutcome o =
            run_offset(bodies, part, offset_params("body_1", {key_near(pair, 5, 5, 10)}, 2.0));
        check(failed_with(o, "single-solid target body"),
              "multi-solid target: refused (" + o.error_message + ")");
        check(o.body_events.empty(), "multi-solid target: nothing published");
    }
    {  // (e) unknown distanceType — a malformed record, never a silent Offset.
        const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
        BodyStore bodies;
        bodies.create("body_1", "op_seed", box);
        em::ElementMapPartition part;
        const ops::OpOutcome o = run_offset(
            bodies, part, offset_params("body_1", {key_near(box, 5, 5, 10)}, 2.0, "Bogus"));
        check(failed_with(o, "unknown distanceType"), "unknown distanceType: refused");
    }
    {  // (f) missing targetBodyId (MANDATORY per SCHEMA §7.3).
        const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
        BodyStore bodies;
        bodies.create("body_1", "op_seed", box);
        em::ElementMapPartition part;
        json p = offset_params("body_1", {key_near(box, 5, 5, 10)}, 2.0);
        p.erase("targetBodyId");
        const ops::OpOutcome o = run_offset(bodies, part, p);
        check(failed_with(o, "targetBodyId"), "missing target: refused");
    }
}

// Identity and sub-resolution edits are REFUSED. Construction tolerance may make
// a requested change unbuildable; it must never turn the feature into unchanged
// geometry with a successful `modified` event.
void test_identity_noop() {
    for (const double distance : {0.0, 5.0e-5}) {
        const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
        BodyStore bodies;
        bodies.create("body_1", "op_seed", box);
        em::ElementMapPartition part;
        const ops::OpOutcome o = run_offset(
            bodies, part,
            offset_params("body_1", {key_near(box, 5, 5, 10)}, distance));
        check(o.status == ops::OpOutcome::Status::Failed &&
                  o.error_message.find("below the supported minimum") != std::string::npos,
              "identity/sub-resolution Offset: named refusal");
        check(o.body_events.empty(), "identity/sub-resolution Offset: no lifecycle event");
        check(bodies.contains("body_1") && bodies.get("body_1")->geom.IsSame(box),
              "identity/sub-resolution Offset: predecessor stays untouched");
    }
    {  // Radius equal to the current radius is the absolute-type identity.
        const TopoDS_Shape cyl = BRepPrimAPI_MakeCylinder(10.0, 20.0).Shape();
        const std::vector<int> lats = cylindrical_ordinals(cyl);
        BodyStore bodies;
        bodies.create("body_1", "op_seed", cyl);
        em::ElementMapPartition part;
        const ops::OpOutcome o = run_offset(
            bodies, part,
            offset_params("body_1", {of::face_topokey(lats.at(0))}, 10.0,
                          "Radius"));
        check(o.status == ops::OpOutcome::Status::Failed && o.body_events.empty(),
              "identity(Radius==R): refused without publication");
        check(bodies.contains("body_1") && bodies.get("body_1")->geom.IsSame(cyl),
              "identity(Radius==R): predecessor stays untouched");
    }
}

void test_trimmed_face_sampling() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    const TopoDS_Shape bore = BRepPrimAPI_MakeCylinder(
        gp_Ax2(gp_Pnt(5.0, 5.0, -1.0), gp_Dir(0.0, 0.0, 1.0)), 2.0, 12.0).Shape();
    BRepAlgoAPI_Cut cut(box, bore);
    cut.Build();
    check(cut.IsDone() && !cut.Shape().IsNull(), "trimmed sample: holed box built");

    TopoDS_Face top;
    for (TopExp_Explorer it(cut.Shape(), TopAbs_FACE); it.More(); it.Next()) {
        const TopoDS_Face face = TopoDS::Face(it.Current());
        const of::PlaneInfo plane = of::plane_info(face);
        if (plane.ok && std::abs(plane.location.Z() - 10.0) < 1.0e-7) {
            top = face;
            break;
        }
    }
    check(!top.IsNull(), "trimmed sample: top face with central hole found");
    if (top.IsNull()) return;
    const of::FaceSample sample = of::sample_face(top);
    check(sample.ok, "trimmed sample: an interior point is found away from the UV-center hole");
    if (sample.ok) {
        BRepTopAdaptor_FClass2d classifier(top, 1.0e-9);
        check(classifier.Perform(gp_Pnt2d(sample.u, sample.v), Standard_False) == TopAbs_IN,
              "trimmed sample: returned UV is provably inside the trimmed domain");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. Element-map history — the offset IMAGE adapter (spike test 7)
// ═══════════════════════════════════════════════════════════════════════════

// Every previously tracked face must RELABEL through `apply_history`, not strand:
// the public Generated()/Modified() lists are face-empty, so this only works because
// `OffsetImageHistory` reads `OffsetFacesFromShapes()`.
void test_history_relabels_tracked_faces() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    BodyStore bodies;
    bodies.create("body_1", "op_seed", box);
    em::ElementMapPartition part;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(box, TopAbs_FACE, faces);
    for (int i = 1; i <= faces.Extent(); ++i) {
        part.mint("body_1", "el_f" + std::to_string(i), km::ElementKind::Face, faces(i), box);
    }
    check(part.entries_for_body("body_1").size() == 6, "history: 6 faces tracked before");

    const ops::OpOutcome o =
        run_offset(bodies, part, offset_params("body_1", {key_near(box, 5, 5, 10)}, 2.0));
    check(o.status == ops::OpOutcome::Status::Ok, "history: Ok (" + o.error_message + ")");
    check(o.needs_repair.empty(), "history: nothing stranded (no NeedsRepair)");
    check(o.delta.removed.empty(), "history: nothing dropped");
    check(part.entries_for_body("body_1").size() == 6, "history: all 6 faces still tracked");
    check(!o.delta.relabeled.empty(), "history: rebinding recorded in the delta");

    // Every surviving entry must bind a REAL face of the NEW shape.
    const TopoDS_Shape result = bodies.get("body_1")->geom;
    for (const em::PartitionEntry* e : part.entries_for_body("body_1")) {
        const TopoDS_Shape sub = em::ElementMapPartition::shape_for_topokey(result, e->topo_key);
        check(!sub.IsNull() && sub.ShapeType() == TopAbs_FACE,
              "history: " + e->element_id + " binds a live face of the offset body");
    }
}

// The typed `inputs[]` ladder rung (Fillet discipline): `faceIds` carries an
// ElementId with no partition entry, so resolution falls to descriptor+anchor. A
// ref that resolves to nothing is NeedsRepair STATE — never a wrong bind, never Err.
void test_ladder_rung_and_needs_repair() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(box, TopAbs_FACE, faces);
    const int top_ord = face_ordinal_near(box, 5, 5, 10);
    const TopoDS_Shape top = faces(top_ord);

    {  // resolves through the ladder
        BodyStore bodies;
        bodies.create("body_1", "op_seed", box);
        em::ElementMapPartition part;
        json inputs = json::array({json{
            {"primary", {{"bodyId", "body_1"}, {"elementId", "el_top"}, {"kind", "face"}}},
            {"intent",
             {{"kind", "face"},
              {"descriptor", em::ElementMapPartition::descriptor_to_json(
                                 em::ElementMapPartition::describe(top))}}},
            {"anchor", {{"worldPoint", {5.0, 5.0, 10.0}}}}}});
        const ops::OpOutcome o = run_offset(
            bodies, part, offset_params("body_1", {"el_top"}, 2.0), "op_off", std::move(inputs));
        check(o.status == ops::OpOutcome::Status::Ok, "ladder: Ok (" + o.error_message + ")");
        check(o.needs_repair.empty(), "ladder: bound without repair");
        check_near(volume_of(bodies.get("body_1")->geom), 1200.0, 1e-6, "ladder: exact volume");
    }
    {  // no evidence at all ⇒ NeedsRepair STATE (status Ok, op not applied)
        BodyStore bodies;
        bodies.create("body_1", "op_seed", box);
        em::ElementMapPartition part;
        const ops::OpOutcome o =
            run_offset(bodies, part, offset_params("body_1", {"el_ghost"}, 2.0));
        check(o.status == ops::OpOutcome::Status::Ok, "unresolved: STATE, not Err");
        check(o.needs_repair.size() == 1, "unresolved: one NeedsRepair item");
        check(o.body_events.empty(), "unresolved: nothing published");
        check_near(volume_of(bodies.get("body_1")->geom), 1000.0, 1e-9, "unresolved: body untouched");
    }
}

// The worker is an INDEPENDENT trust boundary: a malformed typed array must be
// refused, not filtered into a different valid request. Silently dropping one
// element shortens `faceIds` while `inputs[]` keeps its length, so every remaining
// id would be paired with its neighbour's evidence (and the Total opposite would be
// read from the wrong slot entirely).
void test_malformed_wire_contract() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    const std::string top = key_near(box, 5, 5, 10);
    const int top_ord = face_ordinal_near(box, 5, 5, 10);
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(box, TopAbs_FACE, faces);
    const TopoDS_Shape top_face = faces(top_ord);

    auto seeded = [&](json params, json inputs = json()) {
        BodyStore bodies;
        bodies.create("body_1", "op_seed", box);
        em::ElementMapPartition part;
        return run_offset(bodies, part, std::move(params), "op_off", std::move(inputs));
    };
    auto refused_by_code = [&](const ops::OpOutcome& o, const char* code) {
        return o.status == ops::OpOutcome::Status::Failed && o.error_code == "OP_FAILED" &&
               !o.diagnostics.empty() && o.diagnostics.front().value("code", "") == code;
    };

    {  // (a) a non-string element in `faceIds`
        json p = offset_params("body_1", {top}, 2.0);
        p["faceIds"].push_back(42);
        const ops::OpOutcome o = seeded(p);
        check(refused_by_code(o, "OFFSET_FACE_MALFORMED_FACE_IDS"),
              "malformed faceIds element: refused by code (" + o.error_message + ")");
        check(o.body_events.empty(), "malformed faceIds element: nothing published");
    }
    {  // (b) `faceIds` present but not an array
        json p = offset_params("body_1", {top}, 2.0);
        p["faceIds"] = top;
        const ops::OpOutcome o = seeded(p);
        check(refused_by_code(o, "OFFSET_FACE_MALFORMED_FACE_IDS"),
              "non-array faceIds: refused by code (" + o.error_message + ")");
    }
    {  // (c) a non-string element in `primaryFaceIds` must NOT degrade into the
       //     misleading "V2 requires primaryFaceIds" absence message.
        json p = offset_params("body_1", {top}, 2.0);
        p["primaryFaceIds"].push_back(7);
        const ops::OpOutcome o = seeded(p);
        check(refused_by_code(o, "OFFSET_FACE_MALFORMED_FACE_IDS"),
              "malformed primaryFaceIds element: refused by code (" + o.error_message + ")");
        check(o.error_message.find("primaryFaceIds") != std::string::npos,
              "malformed primaryFaceIds element: names the offending key");
    }
    {  // (d) a typed ref whose own elementId disagrees with the id it is paired with
        json inputs = json::array({json{
            {"primary", {{"bodyId", "body_1"}, {"elementId", "el_other"}, {"kind", "face"}}},
            {"intent",
             {{"kind", "face"},
              {"descriptor", em::ElementMapPartition::descriptor_to_json(
                                 em::ElementMapPartition::describe(top_face))}}},
            {"anchor", {{"worldPoint", {5.0, 5.0, 10.0}}}}}});
        const ops::OpOutcome o =
            seeded(offset_params("body_1", {"el_top"}, 2.0), std::move(inputs));
        check(refused_by_code(o, "OFFSET_FACE_REF_MISMATCH"),
              "ref/id mismatch: refused by code (" + o.error_message + ")");
        check(o.needs_repair.empty(),
              "ref/id mismatch: a malformed record is not a repairable reference");
    }
    {  // (e) a present `inputs[]` of the wrong length
        json inputs = json::array({json{
            {"primary", {{"bodyId", "body_1"}, {"elementId", "el_top"}, {"kind", "face"}}},
            {"intent",
             {{"kind", "face"},
              {"descriptor", em::ElementMapPartition::descriptor_to_json(
                                 em::ElementMapPartition::describe(top_face))}}},
            {"anchor", {{"worldPoint", {5.0, 5.0, 10.0}}}}}});
        json p = offset_params("body_1", {"el_top", "el_second"}, 2.0);
        const ops::OpOutcome o = seeded(p, std::move(inputs));
        check(refused_by_code(o, "OFFSET_FACE_MALFORMED_FACE_IDS"),
              "inputs arity: refused by code (" + o.error_message + ")");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. `PrepareOffsetFace` (SCHEMA §7.6) — read-only, fenced, minting-free
// ═══════════════════════════════════════════════════════════════════════════

// Seed bodies into the session HEAD through the REAL transaction (no test-only
// setters — the route test_face_projection.cpp / test_preview_op.cpp take).
void seed_head(Session& session, const std::vector<std::pair<std::string, TopoDS_Shape>>& seeds) {
    session.open("doc", /*documentRevision=*/1, /*workerEpoch=*/1, "determinism");
    onecad::session::FenceOutcome fence = session.fence_and_clone(
        /*job_id=*/1, /*document_revision=*/1, /*worker_epoch=*/1,
        onecad::session::kEmptyPrefixHash);
    onecad::session::ScratchJob job;
    job.job_id = 1;
    job.plan_document_revision = 1;
    job.bodies = std::move(fence.cloned_bodies);
    job.partition = std::move(fence.cloned_partition);
    job.prepared_snapshot_id = fence.prepared_snapshot_id;
    for (const auto& [id, shape] : seeds) job.bodies.create(id, "op_seed", shape);
    session.store_prepared(std::move(job));
    session.accept_prepared(/*job_id=*/1, /*document_revision=*/1, /*worker_epoch=*/1);
}

Envelope prepare_req(json args) {
    Envelope req;
    req.id = 11;
    req.verb = "PrepareOffsetFace";
    req.args = std::move(args);
    return req;
}

json pick(const std::string& body_id, const std::string& topo_key) {
    return json{{"bodyId", body_id}, {"topoKey", topo_key}};
}

std::string refusal_code(const Envelope& e) {
    if (!e.ok.value_or(false)) {
        return "<error:" + (e.error.has_value() ? e.error->code + " " + e.error->message
                                                : std::string("?")) +
               ">";
    }
    if (!e.result.contains("refusal")) return "<no refusal key>";
    if (e.result["refusal"].is_null()) return "";
    return e.result["refusal"].value("code", std::string("<none>"));
}

// The split cylinder: picking ONE half must return BOTH halves, correctly flagged —
// this is the closure the record freezes.
void test_prepare_closure_expansion() {
    const gp_Ax2 up(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1), gp_Dir(1, 0, 0));
    const gp_Ax2 down(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1), gp_Dir(-1, 0, 0));
    const TopoDS_Shape fused =
        BRepAlgoAPI_Fuse(BRepPrimAPI_MakeCylinder(up, 10.0, 20.0, M_PI).Shape(),
                         BRepPrimAPI_MakeCylinder(down, 10.0, 20.0, M_PI).Shape())
            .Shape();
    const std::vector<int> lats = cylindrical_ordinals(fused);
    if (lats.size() != 2) {
        check(false, "prepare/splitcyl: two tangent lateral faces");
        return;
    }
    Session session;
    seed_head(session, {{"body_1", fused}});

    const Envelope ok = onecad::session::handle_prepare_offset_face(
        session, prepare_req(json{{"snapshotId", std::uint64_t{1}},
                                  {"pickedFaces", json::array({pick("body_1", of::face_topokey(lats[0]))})},
                                  {"chainTangentFaces", true},
                                  {"distanceType", "Offset"}}));
    check(ok.ok.value_or(false), "prepare/chain: ok");
    check(refusal_code(ok).empty(), "prepare/chain: no refusal (" + refusal_code(ok) + ")");
    check(ok.result.value("targetBodyId", std::string()) == "body_1", "prepare/chain: target body");
    check(ok.result["faces"].size() == 2, "prepare/chain: closure expanded to 2 faces");
    if (ok.result["faces"].size() == 2) {
        int picked = 0;
        for (const json& f : ok.result["faces"]) {
            if (f.value("picked", false)) ++picked;
            check(f.contains("descriptor") && f["descriptor"].is_object(),
                  "prepare/chain: descriptor evidence present");
            check(f.contains("anchor") && f["anchor"].contains("worldPoint") &&
                      f["anchor"].contains("surfaceUv"),
                  "prepare/chain: anchor carries worldPoint + surfaceUv");
            check(!f.contains("elementId"), "prepare/chain: NEVER mints an ElementId");
        }
        check(picked == 1, "prepare/chain: exactly the picked face is flagged");
        check(ok.result["faces"][0]["topoKey"] == of::face_topokey(lats[0]) &&
                  ok.result["faces"][1]["topoKey"] == of::face_topokey(lats[1]),
              "prepare/chain: faces are ordinal-ordered");
    }
    check(ok.result["currentDims"].contains("radius"), "prepare/chain: currentDims.radius present");
    if (ok.result["currentDims"].contains("radius")) {
        check_near(ok.result["currentDims"]["radius"].get<double>(), 10.0, 1e-9,
                   "prepare/chain: currentDims.radius == 10");
    }

    // chain OFF with a tangent neighbour outside the picks ⇒ `chainMismatch` ANSWER.
    const Envelope refused = onecad::session::handle_prepare_offset_face(
        session, prepare_req(json{{"snapshotId", std::uint64_t{1}},
                                  {"pickedFaces", json::array({pick("body_1", of::face_topokey(lats[0]))})},
                                  {"chainTangentFaces", false},
                                  {"distanceType", "Offset"}}));
    check(refused.ok.value_or(false), "prepare/chainMismatch: refusals are ok:true ANSWERS");
    check(refusal_code(refused) == "chainMismatch", "prepare/chainMismatch: code");
    check(refused.result["refusal"]["faces"].size() == 1 &&
              refused.result["refusal"]["faces"][0] == of::face_topokey(lats[1]),
          "prepare/chainMismatch: names the extra face");
}

// `Total` opposite detection is FAIL CLOSED: a clean box resolves uniquely; a stepped
// underside (the counterexample) resolves to nothing.
void test_prepare_total_opposite() {
    {
        const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 20.0, 10.0).Shape();
        Session session;
        seed_head(session, {{"body_1", box}});
        const std::string top = key_near(box, 10, 10, 10);
        const std::string bottom = key_near(box, 10, 10, 0);
        const Envelope e = onecad::session::handle_prepare_offset_face(
            session, prepare_req(json{{"snapshotId", std::uint64_t{1}},
                                      {"pickedFaces", json::array({pick("body_1", top)})},
                                      {"chainTangentFaces", false},
                                      {"distanceType", "Total"}}));
        check(e.ok.value_or(false), "prepare/total: ok");
        check(refusal_code(e).empty(), "prepare/total: no refusal (" + refusal_code(e) + ")");
        check(e.result.contains("oppositeFace"), "prepare/total: oppositeFace present");
        if (e.result.contains("oppositeFace")) {
            check(e.result["oppositeFace"].value("topoKey", std::string()) == bottom,
                  "prepare/total: opposite is the bottom face");
            check(e.result["oppositeFace"].contains("descriptor") &&
                      e.result["oppositeFace"].contains("anchor"),
                  "prepare/total: opposite carries evidence");
        }
        check(e.result["currentDims"].contains("thickness"), "prepare/total: thickness present");
        if (e.result["currentDims"].contains("thickness")) {
            check_near(e.result["currentDims"]["thickness"].get<double>(), 10.0, 1e-9,
                       "prepare/total: currentDims.thickness == 10");
        }

        // chain ON + Total is a contradiction.
        const Envelope chained = onecad::session::handle_prepare_offset_face(
            session, prepare_req(json{{"snapshotId", std::uint64_t{1}},
                                      {"pickedFaces", json::array({pick("body_1", top)})},
                                      {"chainTangentFaces", true},
                                      {"distanceType", "Total"}}));
        check(refusal_code(chained) == "chainOnTotal", "prepare/total: chainOnTotal refusal");
    }
    {  // STEPPED UNDERSIDE — the pocket ceiling backs only half the top face, and the
        //  full-depth bottom leaves a void: neither candidate passes, so FAIL CLOSED.
        const TopoDS_Shape slab = BRepPrimAPI_MakeBox(20.0, 20.0, 10.0).Shape();
        const TopoDS_Shape pocket = BRepPrimAPI_MakeBox(gp_Pnt(0, -1, -1), 10.0, 22.0, 5.0).Shape();
        const TopoDS_Shape stepped = BRepAlgoAPI_Cut(slab, pocket).Shape();
        Session session;
        seed_head(session, {{"body_1", stepped}});
        const Envelope e = onecad::session::handle_prepare_offset_face(
            session,
            prepare_req(json{{"snapshotId", std::uint64_t{1}},
                             {"pickedFaces", json::array({pick("body_1", key_near(stepped, 10, 10, 10))})},
                             {"chainTangentFaces", false},
                             {"distanceType", "Total"}}));
        check(e.ok.value_or(false), "prepare/stepped: ok");
        check(refusal_code(e) == "noUniqueOpposite",
              "prepare/stepped: noUniqueOpposite (got " + refusal_code(e) + ")");
    }
}

void test_prepare_refusals_and_fence() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    const TopoDS_Shape other = BRepPrimAPI_MakeBox(gp_Pnt(50, 0, 0), 10.0, 10.0, 10.0).Shape();
    const TopoDS_Shape sphere = BRepPrimAPI_MakeSphere(5.0).Shape();

    {  // stale snapshot: the closure is about to be FROZEN, so this verb IS fenced.
        Session session;
        seed_head(session, {{"body_1", box}});
        const Envelope e = onecad::session::handle_prepare_offset_face(
            session, prepare_req(json{{"snapshotId", std::uint64_t{999}},
                                      {"pickedFaces", json::array({pick("body_1", key_near(box, 5, 5, 10))})},
                                      {"chainTangentFaces", true},
                                      {"distanceType", "Offset"}}));
        check(!e.ok.value_or(true), "prepare/stale: refused as an ERROR");
        check(e.error.has_value() && e.error->code == "STALE_PREVIEW",
              "prepare/stale: STALE_PREVIEW code");
    }
    {  // cross-body picks
        Session session;
        seed_head(session, {{"body_1", box}, {"body_2", other}});
        const Envelope e = onecad::session::handle_prepare_offset_face(
            session, prepare_req(json{{"snapshotId", std::uint64_t{1}},
                                      {"pickedFaces",
                                       json::array({pick("body_1", key_near(box, 5, 5, 10)),
                                                    pick("body_2", key_near(other, 55, 5, 10))})},
                                      {"chainTangentFaces", true},
                                      {"distanceType", "Offset"}}));
        check(refusal_code(e) == "crossBody", "prepare/crossBody: refusal (" + refusal_code(e) + ")");
    }
    {  // a spherical face is outside the planar+cylindrical V1 scope
        Session session;
        seed_head(session, {{"body_1", sphere}});
        const Envelope e = onecad::session::handle_prepare_offset_face(
            session, prepare_req(json{{"snapshotId", std::uint64_t{1}},
                                      {"pickedFaces", json::array({pick("body_1", "f:1")})},
                                      {"chainTangentFaces", true},
                                      {"distanceType", "Offset"}}));
        check(refusal_code(e) == "unsupportedSurface",
              "prepare/sphere: unsupportedSurface (" + refusal_code(e) + ")");
    }
    {  // Radius on a planar closure
        Session session;
        seed_head(session, {{"body_1", box}});
        const Envelope e = onecad::session::handle_prepare_offset_face(
            session, prepare_req(json{{"snapshotId", std::uint64_t{1}},
                                      {"pickedFaces", json::array({pick("body_1", key_near(box, 5, 5, 10))})},
                                      {"chainTangentFaces", true},
                                      {"distanceType", "Radius"}}));
        check(refusal_code(e) == "notCylindrical",
              "prepare/radius-on-plane: notCylindrical (" + refusal_code(e) + ")");
    }
    {  // an unresolvable pick is an ERROR (a stale address), not a geometric refusal
        Session session;
        seed_head(session, {{"body_1", box}});
        const Envelope e = onecad::session::handle_prepare_offset_face(
            session, prepare_req(json{{"snapshotId", std::uint64_t{1}},
                                      {"pickedFaces", json::array({pick("body_1", "f:99")})},
                                      {"chainTangentFaces", true},
                                      {"distanceType", "Offset"}}));
        check(!e.ok.value_or(true) && e.error.has_value() && e.error->code == "REF_UNRESOLVED",
              "prepare/stale pick: REF_UNRESOLVED");
    }
}

// Identical head + identical request ⇒ BYTE-identical response (SCHEMA §7.6).
void test_prepare_determinism() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(20.0, 20.0, 10.0).Shape();
    const json args = json{{"snapshotId", std::uint64_t{1}},
                           {"pickedFaces", json::array({pick("body_1", key_near(box, 10, 10, 10))})},
                           {"chainTangentFaces", false},
                           {"distanceType", "Total"}};
    Session a;
    seed_head(a, {{"body_1", box}});
    Session b;
    seed_head(b, {{"body_1", box}});
    const Envelope ra = onecad::session::handle_prepare_offset_face(a, prepare_req(args));
    const Envelope rb = onecad::session::handle_prepare_offset_face(b, prepare_req(args));
    check(ra.ok.value_or(false) && rb.ok.value_or(false), "prepare/determinism: both ok");
    check(ra.result.dump() == rb.result.dump(),
          "prepare/determinism: byte-identical across fresh sessions");
}

}  // namespace

int main() {
    onecad::log::init_level_from_env();  // ONECAD_WORKER_LOG=debug for the op's own trace
    test_box_top_plus_minus();
    test_frustum_taper_preserved();
    test_cylinder_radius_grow();
    test_hole_sigma_convention();
    test_split_cylinder_chain();
    test_draft_pair_does_not_chain();
    test_total_on_box();
    test_traps();
    test_identity_noop();
    test_trimmed_face_sampling();
    test_history_relabels_tracked_faces();
    test_ladder_rung_and_needs_repair();
    test_malformed_wire_contract();
    test_prepare_closure_expansion();
    test_prepare_total_opposite();
    test_prepare_refusals_and_fence();
    test_prepare_determinism();
    if (g_failures == 0) std::fprintf(stderr, "test_offsetface: all checks passed\n");
    return g_failures;
}
