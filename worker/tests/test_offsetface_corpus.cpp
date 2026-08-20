// test_offsetface_corpus.cpp — WP3 C6: the ADVERSARIAL CORPUS for `op.offsetFace`
// at `resultPolicyVersion: 3`.
//
// C5 (`test_offsetface_reblend.cpp`) proved the canonical case works. This file
// exists to find the cases where it should NOT, and to pin the ones where the
// honest answer turned out to be different from the one the plan predicted.
//
// Split from `test_offsetface_reblend.cpp` (1158 lines) rather than appended: the
// combined file would have run past 1900 lines, and the two have different jobs —
// C5's file is the CONTRACT (exact volumes that must never move), this one is the
// CORPUS (what happens at the edges, measured).
//
// FOUR GROUPS:
//
//   DECOYS         the false-positive corpus. A recognizer false positive is only
//                  DESTRUCTIVE if the reconstruction differs from the original, so
//                  the target here is not "never recognize a non-fillet" — it is
//                  "either reproduce it exactly, or refuse by name". A
//                  user-authored quarter round that comes back as the same quarter
//                  round is the CORRECT outcome, not a near miss.
//   CONDITIONING   the six orders of magnitude the kernel claims to support, plus
//                  distance from the origin and imported-B-Rep tolerance.
//   PERSISTENCE    a V3 result that survives the BrepCodec byte form still has to
//                  be a blend when it comes back, because a reopened document is
//                  where the next edit starts.
//   MULTI-BLEND    the first case where the closure meets MORE THAN ONE certified
//                  blend. C5 only ever engaged one.
//
// Every case asserts an exact number or an exact code. Where the outcome was a
// characterization rather than a prediction, the measured value is printed AND
// pinned, with the reason it is the right answer written next to it.
//
// ── THE FOUR RESULTS THAT WERE NOT THE PREDICTION ───────────────────────────
// Recorded here because a corpus whose surprises are buried in its cases is not
// a corpus. Each is argued at its own test.
//
//   1. MULTI-BLEND SUPPRESS + REBLEND WORKS. A box with all four vertical edges
//      rounded (no vertex spheres, so every round certifies) engages TWO blends on
//      one push and returns the exact analytic 1165.6637061435915 with all four
//      corners still R2. The same record at V2 returns 1822.6548245743670. This
//      was open going in; it is the strongest new capability evidence in the file.
//   2. THE PARTIALLY-TRIMMED ROUND refuses one rung EARLIER than planned —
//      `_SUPPRESSION_NOT_DONE` (OCCT declines to remove the face) rather than the
//      layer-3 seed-extent postcondition. Nothing is destroyed either way.
//   3. THE VARIABLE-RADIUS BLEND never reaches the blend fork at all: OCCT's
//      approximated evolving surface is not G1 within `kTangentAngleTol`, so the
//      SHARED tangent closure excludes it and V3 is byte-identical to V2.
//   4. A 5e-3 mm PUSH ON A 0.1 mm BODY SUCCEEDS. `authoring_resolution()` is a flat
//      1e-3 mm in v1 by explicit design, so the refusal boundary does not scale
//      with the model. Asserted as the constant it is, with the v2 gate named.
//
// No framework: the process exit code IS the failure count.
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <set>
#include <string>
#include <utility>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_Copy.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRep_Tool.hxx>
#include <GC_MakeArcOfCircle.hxx>
#include <GProp_GProps.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <ShapeFix_ShapeTolerance.hxx>
#include <ShapeUpgrade_UnifySameDomain.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Vertex.hxx>
#include <gp_Ax2.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include "elementmap/ElementMapPartition.h"
#include "fillet_test_utils.h"
#include "io/BrepCodec.h"
#include "kernel/fillet/BlendReconstruction.h"
#include "kernel/fillet/BlendRecognizer.h"
#include "nlohmann/json.hpp"
#include "ops/OffsetFaceOp.h"
#include "ops/OpTypes.h"
#include "session/BodyStore.h"
#include "util/Cancel.h"

using nlohmann::json;
using onecad::session::BodyStore;
namespace ops = onecad::ops;
namespace of = onecad::ops::offsetface;
namespace kf = onecad::kernel::fillet;
namespace em = onecad::elementmap;
namespace km = onecad::kernel::elementmap;
namespace ft = onecad::tests::fillet;

namespace {

int g_failures = 0;
int g_checks = 0;

void check(bool cond, const std::string& msg) {
    ++g_checks;
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}

void check_near(double got, double want, double tol, const std::string& msg) {
    ++g_checks;
    if (!(std::abs(got - want) <= tol)) {
        std::fprintf(stderr, "FAIL: %s (got %.13g want %.13g tol %.3g)\n", msg.c_str(), got, want,
                     tol);
        ++g_failures;
    }
}

// Relative form, for the scale cases: an absolute mm³ tolerance is meaningless at
// 1e12 mm³ and vacuous at 1e-3 mm³.
void check_relative(double got, double want, double rel, const std::string& msg) {
    ++g_checks;
    const double allowed = std::abs(want) * rel;
    if (!(std::abs(got - want) <= allowed)) {
        std::fprintf(stderr, "FAIL: %s (got %.15g want %.15g rel %.1e allowed %.3g)\n", msg.c_str(),
                     got, want, rel, allowed);
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

gp_Pnt centroid_of(const TopoDS_Shape& sub) {
    GProp_GProps props;
    if (sub.ShapeType() == TopAbs_EDGE) {
        BRepGProp::LinearProperties(sub, props);
    } else {
        BRepGProp::SurfaceProperties(sub, props);
    }
    return props.CentreOfMass();
}

// Production always mints ANCHORED (`PlanExecutor` passes the ref's `anchor` echo
// straight through), and an unanchored entry scores at a reduced total weight — so
// an unanchored test measures a weaker resolver than the one that ships.
json anchor_at(const TopoDS_Shape& sub) {
    const gp_Pnt c = centroid_of(sub);
    return json{{"worldPoint", {c.X(), c.Y(), c.Z()}}};
}

int face_ordinal_near(const TopoDS_Shape& shape, double cx, double cy, double cz) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    int best = 0;
    double best_d2 = -1.0;
    for (int i = 1; i <= faces.Extent(); ++i) {
        const gp_Pnt c = centroid_of(faces(i));
        const double dx = c.X() - cx, dy = c.Y() - cy, dz = c.Z() - cz;
        const double d2 = dx * dx + dy * dy + dz * dz;
        if (best_d2 < 0.0 || d2 < best_d2) {
            best_d2 = d2;
            best = i;
        }
    }
    return best;
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
                          const std::string& op_id = "op_off") {
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const json op = {{"opType", "OffsetFace"}, {"opId", op_id}, {"params", std::move(params)}};
    return ops::execute_offset_face(ctx, op, op_id);
}

json offset_params(const std::vector<std::string>& closure_keys,
                   const std::vector<std::string>& primary_keys, double distance, int version) {
    json ids = json::array();
    for (const std::string& k : closure_keys) ids.push_back(k);
    json primary = json::array();
    for (const std::string& k : primary_keys) primary.push_back(k);
    return json{{"targetBodyId", "body_1"},
                {"faceIds", std::move(ids)},
                {"primaryFaceIds", std::move(primary)},
                {"resultPolicyVersion", version},
                {"distance", distance},
                {"distanceType", "Offset"},
                {"chainTangentFaces", true}};
}

std::string diagnostic_code(const ops::OpOutcome& o) {
    if (o.diagnostics.empty()) return {};
    return o.diagnostics.front().value("code", "");
}

std::string reason_code(const ops::OpOutcome& o) {
    if (o.diagnostics.empty()) return {};
    return o.diagnostics.front().value("reasonCode", "");
}

// The one line every characterization case prints. Kept identical everywhere so the
// ctest log can be read as a table.
void print_outcome(const char* label, const ops::OpOutcome& o) {
    std::fprintf(stderr, "  [characterized] %-28s status=%d code=%s diag=%s reason=%s msg=%s\n",
                 label, static_cast<int>(o.status), o.error_code.c_str(),
                 diagnostic_code(o).c_str(), reason_code(o).c_str(), o.error_message.c_str());
}

std::vector<std::string> keys_of(const std::vector<int>& ordinals) {
    std::vector<std::string> out;
    for (const int ord : ordinals) out.push_back(of::face_topokey(ord));
    return out;
}

void mint_faces_anchored(em::ElementMapPartition& part, const TopoDS_Shape& body) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(body, TopAbs_FACE, faces);
    for (int i = 1; i <= faces.Extent(); ++i) {
        part.mint("body_1", "el_f" + std::to_string(i), km::ElementKind::Face, faces(i), body,
                  anchor_at(faces(i)));
    }
}

std::string key_of(const em::ElementMapPartition& part, const std::string& id) {
    const em::PartitionEntry* e = part.find(id);
    return e == nullptr ? std::string("<dropped>") : e->topo_key;
}

bool topokeys_pairwise_distinct(const em::ElementMapPartition& part, std::string& collision) {
    std::set<std::string> seen;
    for (const em::PartitionEntry* e : part.entries_for_body("body_1")) {
        if (!seen.insert(e->kind == km::ElementKind::Face ? "f" + e->topo_key : e->topo_key)
                 .second) {
            collision = e->element_id + " collides on " + e->topo_key;
            return false;
        }
    }
    return true;
}

// The vertical edge whose two endpoints sit at (x, y) — the same addressing
// `test_blend_reconstruction.cpp` uses, and the reason it exists is that
// `ft::vertical_edges(...).front()` is an ORDINAL accident, not a position.
TopoDS_Edge vertical_edge_at(const TopoDS_Shape& shape, double x, double y) {
    for (const TopoDS_Edge& e : ft::vertical_edges(shape)) {
        TopoDS_Vertex a, b;
        TopExp::Vertices(e, a, b);
        const gp_Pnt p = BRep_Tool::Pnt(a);
        if (std::abs(p.X() - x) <= 1e-6 && std::abs(p.Y() - y) <= 1e-6) return e;
    }
    return {};
}

std::vector<TopoDS_Edge> shared_edges(const TopoDS_Shape& a, const TopoDS_Shape& b) {
    TopTools_IndexedMapOfShape b_edges;
    TopExp::MapShapes(b, TopAbs_EDGE, b_edges);
    std::vector<TopoDS_Edge> out;
    for (TopExp_Explorer ex(a, TopAbs_EDGE); ex.More(); ex.Next()) {
        if (b_edges.Contains(ex.Current())) out.push_back(TopoDS::Edge(ex.Current()));
    }
    return out;
}

// Face ordinals sharing at least one edge with `ordinal`, ascending. Used to
// address a decoy's SUPPORT without hard-coding a centroid: a decoy's supports are
// trimmed into shapes whose centroids are not where the naive guess puts them, and
// a fixture that quietly addresses the wrong face is a test that asserts nothing.
std::vector<int> edge_neighbours(const TopoDS_Shape& body, int ordinal) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(body, TopAbs_FACE, faces);
    std::set<int> out;
    for (int i = 1; i <= faces.Extent(); ++i) {
        if (i == ordinal) continue;
        if (!shared_edges(faces(ordinal), faces(i)).empty()) out.insert(i);
    }
    return {out.begin(), out.end()};
}

// The single face of `body` whose surface is neither a plane nor a cylinder — the
// decoy face itself, for the variable-radius and torus fixtures. 0 if not unique.
int only_freeform_face(const TopoDS_Shape& body) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(body, TopAbs_FACE, faces);
    int found = 0;
    for (int i = 1; i <= faces.Extent(); ++i) {
        if (of::surface_kind(TopoDS::Face(faces(i))) != of::SurfaceKind::Other) continue;
        if (found != 0) return 0;
        found = i;
    }
    return found;
}

// The first PLANAR neighbour of `ordinal`, ascending — the face a decoy's tangent
// chain would be entered from.
int planar_neighbour(const TopoDS_Shape& body, int ordinal) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(body, TopAbs_FACE, faces);
    for (const int n : edge_neighbours(body, ordinal)) {
        if (of::surface_kind(TopoDS::Face(faces(n))) == of::SurfaceKind::Plane) return n;
    }
    return 0;
}

// The ordinal in `result` of the planar face that is `before`'s plane displaced by
// exactly `d` along its own outward normal — the face the push was SUPPOSED to
// produce. -1 if more than one matches, so an ambiguity is loud.
int moved_plane_ordinal(const TopoDS_Shape& result, const of::PlaneInfo& before, double d) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(result, TopAbs_FACE, faces);
    int found = 0;
    for (int i = 1; i <= faces.Extent(); ++i) {
        const of::PlaneInfo after = of::plane_info(TopoDS::Face(faces(i)));
        if (!after.ok) continue;
        if (gp_Vec(before.normal).Dot(gp_Vec(after.normal)) < 1.0 - 1e-9) continue;
        const double moved = gp_Vec(before.location, after.location).Dot(gp_Vec(before.normal));
        if (std::abs(moved - d) > 1e-7) continue;
        if (found != 0) return -1;
        found = i;
    }
    return found;
}

const char* shape_type_name(const TopoDS_Shape& s) {
    switch (s.ShapeType()) {
        case TopAbs_COMPOUND: return "COMPOUND";
        case TopAbs_COMPSOLID: return "COMPSOLID";
        case TopAbs_SOLID: return "SOLID";
        case TopAbs_SHELL: return "SHELL";
        default: return "OTHER";
    }
}

// Every cylindrical face radius on the shape, ascending — the multi-blend cases need
// to say "there are still exactly N of them, all at R".
std::vector<double> cylinder_radii(const TopoDS_Shape& shape) {
    std::vector<double> out;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    for (int i = 1; i <= faces.Extent(); ++i) {
        const BRepAdaptor_Surface surf(TopoDS::Face(faces(i)));
        if (surf.GetType() == GeomAbs_Cylinder) out.push_back(surf.Cylinder().Radius());
    }
    std::sort(out.begin(), out.end());
    return out;
}

double corner_radius(const TopoDS_Shape& shape) {
    const std::vector<double> radii = cylinder_radii(shape);
    if (radii.size() != 1) return -1.0;
    return radii.front();
}

// How many faces of `shape` are certified blends PROVED at the analytic layer. The
// result-side statement "the blend is still a blend", measured rather than inferred
// from a radius.
int proved_blend_count(const TopoDS_Shape& shape) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    int n = 0;
    for (int i = 1; i <= faces.Extent(); ++i) {
        const kf::TargetedBlendRecognition t = kf::recognize_blend_at(TopoDS::Face(faces(i)), shape);
        if (t.recognized && t.proof == kf::AnalyticBlendProof::Proved) ++n;
    }
    return n;
}

const double kPi = 3.14159265358979323846;
// The material one R2 quarter round takes off a 10 mm tall corner: 10 * R^2 * (1 - pi/4).
const double kCorner = 10.0 * (4.0 - kPi);  // 8.58407346410207 mm^3

// ── the C5 canonical fixture, reused ────────────────────────────────────────
// 10^3 box, ONE vertical edge filleted R2. Rebuilt here rather than shared through a
// header: the corpus must not be able to go green because a shared fixture drifted.
struct Fixture {
    bool ok = false;
    TopoDS_Shape body;
    int blend_ordinal = 0;
    int push_ordinal = 0;
    int other_support = 0;
    std::vector<int> closure;
    TopoDS_Face blend_face;
    std::vector<TopoDS_Edge> boundary_edges;
};

Fixture filleted_box(double size = 10.0, double radius = 2.0) {
    Fixture f;
    const TopoDS_Shape sharp = BRepPrimAPI_MakeBox(size, size, size).Shape();
    BRepFilletAPI_MakeFillet fillet(sharp);
    fillet.Add(radius, ft::vertical_edges(sharp).front());
    try {
        fillet.Build();
    } catch (const Standard_Failure&) {
        return f;
    }
    if (!fillet.IsDone() || fillet.Shape().IsNull()) return f;
    f.body = fillet.Shape();

    const std::vector<kf::BlendRecognition> blends =
        kf::recognize_constant_radius_blends(f.body).recognized();
    if (blends.size() != 1 || blends.front().support_face_ordinals.size() != 2) return f;
    f.blend_ordinal = blends.front().face_ordinal;
    f.push_ordinal = blends.front().support_face_ordinals[0];
    f.other_support = blends.front().support_face_ordinals[1];

    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(f.body, TopAbs_FACE, faces);
    f.blend_face = TopoDS::Face(faces(f.blend_ordinal));
    for (const int support : blends.front().support_face_ordinals) {
        for (const TopoDS_Edge& e : shared_edges(f.blend_face, faces(support)))
            f.boundary_edges.push_back(e);
    }

    const of::ClosureResult closure = of::tangent_closure(f.body, {f.push_ordinal});
    if (!closure.ok) return f;
    f.closure = closure.ordinals;
    f.ok = true;
    return f;
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. CORE — the side push C5 did not take
// ═══════════════════════════════════════════════════════════════════════════

// C5 pins +2. The opposite sign is a different code path in exactly one respect
// that matters: suppression postcondition 8 checks the volume moved the way the
// blend's CONVEXITY demands, and the offset then moves it the other way. A sign
// error anywhere in the chain shows up here and nowhere else.
//
// ANALYTIC: pushing one wall of the 10^3 box IN by 2 leaves an 8 x 10 footprint
// with the SAME R2 corner, extruded 10.
//   V = 10 * (80 - (4 - pi)) = 800 - 10*(4 - pi) = 791.4159265358979
// (The -9 consuming push is characterized in `test_offsetface_reblend.cpp`; it is
// not repeated here.)
void test_side_push_negative_preserves_the_corner() {
    const Fixture f = filleted_box();
    check(f.ok, "side -2: fixture built and its blend recognized");
    if (!f.ok) return;
    check_near(volume_of(f.body), 1000.0 - kCorner, 1e-9, "side -2 fixture: V0 = 991.4159265358979");
    check(f.closure.size() == 3, "side -2 fixture: the frozen closure is 3 faces");

    BodyStore bodies;
    bodies.create("body_1", "op_seed", f.body);
    em::ElementMapPartition part;
    const ops::OpOutcome o =
        run_offset(bodies, part,
                   offset_params(keys_of(f.closure), {of::face_topokey(f.push_ordinal)}, -2.0, 3));
    print_outcome("side -2", o);
    check(o.status == ops::OpOutcome::Status::Ok, "side -2: Ok (" + o.error_message + ")");
    if (o.status != ops::OpOutcome::Status::Ok || !bodies.contains("body_1")) return;
    const TopoDS_Shape r = bodies.get("body_1")->geom;
    check_near(volume_of(r), 800.0 - kCorner, 1e-6, "side -2: volume 791.4159265358979");
    check(face_count(r) == face_count(f.body), "side -2: face count preserved");
    check_near(corner_radius(r), 2.0, 1e-9, "side -2: the corner is STILL R2");
    check(proved_blend_count(r) == 1, "side -2: exactly one PROVED blend on the result");
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. DECOYS — the false-positive corpus
// ═══════════════════════════════════════════════════════════════════════════

// A CHAMFER is the nearest thing to a fillet that must never be treated as one. Its
// junctions with both supports are C0, so it is not in the wall's G1 closure at all
// and the blend fork is never even consulted. Asserted EXPLICITLY — "V3 does what V2
// does, bit for bit" is the whole claim for every body with no certified blend, and
// a chamfered part is the commonest such body in real work.
void test_chamfered_box_is_v2() {
    const TopoDS_Shape sharp = ft::box();
    BRepFilletAPI_MakeChamfer chamfer(sharp);
    chamfer.Add(2.0, ft::vertical_edges(sharp).front());
    try {
        chamfer.Build();
    } catch (const Standard_Failure&) {
        check(false, "chamfer: fixture threw");
        return;
    }
    check(chamfer.IsDone() && !chamfer.Shape().IsNull(), "chamfer: fixture builds");
    if (!chamfer.IsDone() || chamfer.Shape().IsNull()) return;
    const TopoDS_Shape body = chamfer.Shape();
    // 10^3 box with a 45 deg 2 mm chamfer on one vertical edge removes a right
    // triangle of legs 2, extruded 10: 1000 - 10*2 = 980.
    check_near(volume_of(body), 980.0, 1e-9, "chamfer fixture: V0 = 980");
    check(face_count(body) == 7, "chamfer fixture: 7 faces");
    check(kf::recognize_constant_radius_blends(body).recognized().empty(),
          "chamfer: NOTHING on this body is a recognized blend");

    const int wall = face_ordinal_near(body, 10.0, 4.0, 5.0);
    const of::ClosureResult closure = of::tangent_closure(body, {wall});
    check(closure.ok && closure.ordinals.size() == 1,
          "chamfer: the wall's G1 closure is itself alone (the chamfer junction is C0)");
    const of::BlendAwareClosure classified = of::blend_aware_closure(body, {wall});
    check(classified.ok && classified.blends.empty(),
          "chamfer: the blend-aware walk certifies NOTHING");

    double volumes[2] = {0.0, 0.0};
    int faces[2] = {0, 0};
    std::string deltas[2];
    for (int i = 0; i < 2; ++i) {
        BodyStore bodies;
        bodies.create("body_1", "op_seed", body);
        em::ElementMapPartition part;
        mint_faces_anchored(part, body);
        const ops::OpOutcome o =
            run_offset(bodies, part,
                       offset_params(keys_of(closure.ordinals), {of::face_topokey(wall)}, 2.0,
                                     i == 0 ? 2 : 3));
        check(o.status == ops::OpOutcome::Status::Ok,
              std::string("chamfer v") + (i == 0 ? "2" : "3") + ": Ok (" + o.error_message + ")");
        if (!bodies.contains("body_1")) return;
        volumes[i] = volume_of(bodies.get("body_1")->geom);
        faces[i] = face_count(bodies.get("body_1")->geom);
        deltas[i] = o.delta.to_json().dump();
    }
    std::fprintf(stderr, "  [characterized] chamfer: V2=%.13f V3=%.13f faces %d/%d\n", volumes[0],
                 volumes[1], faces[0], faces[1]);
    check(volumes[0] == volumes[1], "chamfer: V3 volume BIT-IDENTICAL to V2");
    check(faces[0] == faces[1], "chamfer: identical face count at both versions");
    check(deltas[0] == deltas[1], "chamfer: byte-identical elementMapDelta at both versions");
    check_near(volumes[1], 1180.0, 1e-6, "chamfer: +2 on the wall gives 1180");
}

// ── the QUARTER-ROUND RIB ───────────────────────────────────────────────────
// A user-authored quarter round with NO fillet feature anywhere in its
// construction: an L-shaped sketch profile whose reentrant corner is a tangent arc,
// prism'd. It is geometrically indistinguishable from a concave R2 fillet, and
// `AnalyticBlend.h` is explicit that no certificate can tell them apart.
//
// THE CORRECT OUTCOME IS THAT THE OPERATION SUCCEEDS. The safety argument in this
// work package is not "never mistake a rib for a fillet" — that is undecidable from
// geometry. It is that a false positive is only destructive if the reconstruction
// DIFFERS, and a rib rebuilt at its own measured radius on its own support pair
// comes back as the same rib. Refusing here would be the wrong answer twice over:
// it would refuse a legal edit AND it would claim a distinction that does not exist.
//
// Profile (XY, prism'd 10 along Z), CCW:
//   (0,0) (10,0) (10,4) (6,4) --arc R2 about (6,6)--> (4,6) (4,10) (0,10)
// L area = 100 - 36 = 64, plus the corner fill R^2(1 - pi/4) = 4 - pi.
//   V0 = 10 * (68 - pi) = 648.5840734641021
// Pushing the y=4 wall OUT by 1 shrinks the missing corner rectangle from 6x6 to
// 6x5, so the area gains exactly 6 and the round is carried along unchanged:
//   V1 = 10 * (74 - pi) = 708.5840734641021
struct Rib {
    bool ok = false;
    TopoDS_Shape body;
    int blend_ordinal = 0;
    int push_ordinal = 0;
    int fixed_support = 0;
    std::vector<int> closure;
};

Rib quarter_round_rib() {
    Rib r;
    const double m = 6.0 - std::sqrt(2.0);  // the arc's midpoint, on the 45 deg ray
    BRepBuilderAPI_MakeWire wire;
    wire.Add(BRepBuilderAPI_MakeEdge(gp_Pnt(0, 0, 0), gp_Pnt(10, 0, 0)).Edge());
    wire.Add(BRepBuilderAPI_MakeEdge(gp_Pnt(10, 0, 0), gp_Pnt(10, 4, 0)).Edge());
    wire.Add(BRepBuilderAPI_MakeEdge(gp_Pnt(10, 4, 0), gp_Pnt(6, 4, 0)).Edge());
    wire.Add(BRepBuilderAPI_MakeEdge(
                 GC_MakeArcOfCircle(gp_Pnt(6, 4, 0), gp_Pnt(m, m, 0), gp_Pnt(4, 6, 0)).Value())
                 .Edge());
    wire.Add(BRepBuilderAPI_MakeEdge(gp_Pnt(4, 6, 0), gp_Pnt(4, 10, 0)).Edge());
    wire.Add(BRepBuilderAPI_MakeEdge(gp_Pnt(4, 10, 0), gp_Pnt(0, 10, 0)).Edge());
    wire.Add(BRepBuilderAPI_MakeEdge(gp_Pnt(0, 10, 0), gp_Pnt(0, 0, 0)).Edge());
    if (!wire.IsDone()) return r;
    const TopoDS_Shape base = BRepBuilderAPI_MakeFace(wire.Wire()).Shape();
    if (base.IsNull()) return r;
    r.body = BRepPrimAPI_MakePrism(base, gp_Vec(0, 0, 10)).Shape();
    if (r.body.IsNull()) return r;

    const std::vector<kf::BlendRecognition> blends =
        kf::recognize_constant_radius_blends(r.body).recognized();
    if (blends.size() != 1 || blends.front().support_face_ordinals.size() != 2) return r;
    r.blend_ordinal = blends.front().face_ordinal;
    // The pushed support is the y=4 wall; the other one (x=4) is held fixed.
    const int y4 = face_ordinal_near(r.body, 8.0, 4.0, 5.0);
    for (const int s : blends.front().support_face_ordinals) {
        if (s == y4) {
            r.push_ordinal = s;
        } else {
            r.fixed_support = s;
        }
    }
    if (r.push_ordinal == 0 || r.fixed_support == 0) return r;
    const of::ClosureResult closure = of::tangent_closure(r.body, {r.push_ordinal});
    if (!closure.ok) return r;
    r.closure = closure.ordinals;
    r.ok = true;
    return r;
}

void test_quarter_round_rib_is_reproduced() {
    const Rib r = quarter_round_rib();
    check(r.ok, "rib: the sketched quarter-round prism builds and recognizes as ONE blend");
    if (!r.ok) return;
    check_near(volume_of(r.body), 10.0 * (68.0 - kPi), 1e-6, "rib fixture: V0 = 648.5840734641021");
    check(face_count(r.body) == 9, "rib fixture: 9 faces");
    check_near(corner_radius(r.body), 2.0, 1e-9, "rib fixture: the authored round measures R2");

    // The certification the destructive path runs, on a face that has no fillet
    // feature behind it at all.
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(r.body, TopAbs_FACE, faces);
    const kf::BlendCertification cert =
        kf::certify_blend(TopoDS::Face(faces(r.blend_ordinal)), r.body);
    check(cert.ok, "rib: layers 1-3 CERTIFY the authored round — " + cert.reason);
    check(cert.blend.convexity == kf::BlendConvexity::Concave, "rib: the round is CONCAVE");

    BodyStore bodies;
    bodies.create("body_1", "op_seed", r.body);
    em::ElementMapPartition part;
    mint_faces_anchored(part, r.body);
    const ops::OpOutcome o =
        run_offset(bodies, part,
                   offset_params(keys_of(r.closure), {of::face_topokey(r.push_ordinal)}, 1.0, 3));
    print_outcome("quarter-round rib +1", o);
    check(o.status == ops::OpOutcome::Status::Ok, "rib: Ok (" + o.error_message + ")");
    if (o.status != ops::OpOutcome::Status::Ok || !bodies.contains("body_1")) return;
    const TopoDS_Shape result = bodies.get("body_1")->geom;
    check_near(volume_of(result), 10.0 * (74.0 - kPi), 1e-6, "rib: volume 708.5840734641021");
    check(face_count(result) == face_count(r.body), "rib: face count preserved");
    check_near(corner_radius(result), 2.0, 1e-9, "rib: the authored round comes back at R2");
    check(proved_blend_count(result) == 1, "rib: exactly one PROVED blend on the result");
    check(o.needs_repair.empty(), "rib: no NeedsRepair");
    const std::string blend_id = "el_f" + std::to_string(r.blend_ordinal);
    check(part.contains(blend_id), "rib: the round's ElementId survives the round trip");

    // NON-VACUITY: the same record at V2. MEASURED 752.1460183660 with the round
    // shrunk from R2 to R1 — V2 offsets the round along with the wall, so a +1 push
    // on a CONCAVE R2 comes back as R1. That is the destruction this whole work
    // package exists to stop, and it is what proves the numbers above came from the
    // V3 path rather than from a 3 sitting unread in the payload.
    BodyStore v2_bodies;
    v2_bodies.create("body_1", "op_seed", r.body);
    em::ElementMapPartition v2_part;
    const ops::OpOutcome v2 =
        run_offset(v2_bodies, v2_part,
                   offset_params(keys_of(r.closure), {of::face_topokey(r.push_ordinal)}, 1.0, 2));
    check(v2.status == ops::OpOutcome::Status::Ok, "rib V2: Ok (" + v2.error_message + ")");
    if (v2.status != ops::OpOutcome::Status::Ok || !v2_bodies.contains("body_1")) return;
    const TopoDS_Shape v2_shape = v2_bodies.get("body_1")->geom;
    std::fprintf(stderr, "  [characterized] rib V2: V=%.13f R=%.13f\n", volume_of(v2_shape),
                 corner_radius(v2_shape));
    check_near(corner_radius(v2_shape), 1.0, 1e-9,
               "rib V2: the authored round is SHRUNK to R1 — the destructive legacy behaviour");
    check_near(volume_of(v2_shape), 752.1460183660, 1e-6, "rib V2: unchanged legacy volume");
}

// ── the PARTIALLY-TRIMMED ROUND ─────────────────────────────────────────────
// box(10) rounded R2 on one vertical edge, then the TOP 4 mm of that round filled
// back in with a 2x2x4 block. The round now runs z in [0,6] while its two supports
// already meet SHARPLY over z in [6,10]. Suppression exposes a sharp edge spanning
// the whole 10 mm, and a rebuild seeded from it would round 4 mm the user never
// rounded. C4 pinned the KERNEL-level refusal; this pins what the OPERATION says.
TopoDS_Shape partially_trimmed_round() {
    const TopoDS_Shape sharp = ft::box();
    BRepFilletAPI_MakeFillet fillet(sharp);
    fillet.Add(2.0, vertical_edge_at(sharp, 10.0, 10.0));
    try {
        fillet.Build();
    } catch (const Standard_Failure&) {
        return {};
    }
    if (!fillet.IsDone() || fillet.Shape().IsNull()) return {};
    const TopoDS_Shape block = BRepPrimAPI_MakeBox(gp_Pnt(8.0, 8.0, 6.0), 2.0, 2.0, 4.0).Shape();
    BRepAlgoAPI_Fuse fuse(fillet.Shape(), block);
    fuse.Build();
    if (!fuse.IsDone() || fuse.Shape().IsNull()) return {};
    ShapeUpgrade_UnifySameDomain unify(fuse.Shape(), Standard_True, Standard_True, Standard_False);
    unify.Build();
    return unify.Shape();
}

void test_partially_trimmed_round() {
    const TopoDS_Shape body = partially_trimmed_round();
    check(!body.IsNull(), "partial trim: fixture builds");
    if (body.IsNull()) return;
    // 1000 minus the corner bite over the 6 mm that is actually rounded.
    check_near(volume_of(body), 1000.0 - 6.0 * (4.0 - kPi), 1e-6,
               "partial trim fixture: V0 = 994.8495559215388");
    check_near(corner_radius(body), 2.0, 1e-9, "partial trim fixture: one R2 cylindrical face");

    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(body, TopAbs_FACE, faces);
    int round_ordinal = 0;
    for (int i = 1; i <= faces.Extent(); ++i) {
        if (of::surface_kind(TopoDS::Face(faces(i))) == of::SurfaceKind::Cylinder)
            round_ordinal = i;
    }
    check(round_ordinal != 0, "partial trim: the trimmed round is addressable");
    if (round_ordinal == 0) return;
    // C4's finding, re-measured on the operation's own path: layers 1-3 ACCEPT the
    // trimmed face — its own two boundary edges are equal-length lines. The defect
    // is not visible until suppression exposes a sharp edge longer than they are.
    const kf::BlendCertification cert =
        kf::certify_blend(TopoDS::Face(faces(round_ordinal)), body);
    check(cert.ok, "partial trim: layers 1-3 CERTIFY the trimmed face — " + cert.reason);
    check_near(cert.blend.boundary_length, 6.0, 1e-6,
               "partial trim: the certified extent is the 6 mm that is actually rounded");

    const int wall = planar_neighbour(body, round_ordinal);
    check(wall != 0, "partial trim: the round has a planar support");
    if (wall == 0) return;
    const of::ClosureResult closure = of::tangent_closure(body, {wall});
    check(closure.ok, "partial trim: the wall's closure walks");
    if (!closure.ok) return;
    std::fprintf(stderr, "  [characterized] partial trim: closure = %zu faces\n",
                 closure.ordinals.size());
    BodyStore bodies;
    bodies.create("body_1", "op_seed", body);
    em::ElementMapPartition part;
    const ops::OpOutcome o = run_offset(
        bodies, part, offset_params(keys_of(closure.ordinals), {of::face_topokey(wall)}, 1.0, 3));
    print_outcome("partially-trimmed round", o);
    check(o.status == ops::OpOutcome::Status::Failed,
          "partial trim: the operation REFUSES rather than rounding 4 mm nobody rounded");
    // MEASURED, OCCT 8.0.1: the refusal arrives one rung EARLIER than layer 3's
    // seed-extent postcondition — `BRepAlgoAPI_Defeaturing` will not remove the
    // trimmed face at all, so postcondition 1 fires with the kernel's own alert
    // quoted. Pinned as the code that actually fires rather than the one the plan
    // predicted (`_SUPPRESSION_SEED_EDGE_*`), because a corpus that asserts a
    // prediction instead of the measurement is not evidence. Either way nothing is
    // destroyed, which is the property that matters.
    check(diagnostic_code(o) == "OFFSET_FACE_SUPPRESSION_NOT_DONE",
          "partial trim: named code (" + diagnostic_code(o) + ")");
    check(o.error_message.find("BOPAlgo_AlertUnableToRemoveTheFeature") != std::string::npos,
          "partial trim: the refusal quotes the kernel's own alert");
    check(o.body_events.empty(), "partial trim: nothing published");
    check(bodies.contains("body_1") && volume_of(bodies.get("body_1")->geom) == volume_of(body),
          "partial trim: body untouched");
}

// ── the VARIABLE-RADIUS BLEND ───────────────────────────────────────────────
// `BRepFilletAPI_MakeFillet::Add(R1, R2, edge)` — an EVOLVING R1 -> R3 radius.
//
// MEASURED, AND NOT WHAT THE PLAN PREDICTED. The plan expected an `Unknown`
// certificate and a named refusal. What actually happens is that the operation
// never reaches the blend fork at all: OCCT approximates the evolving surface, and
// its junctions with the two walls miss exact tangency by more than `kTangentAngleTol`
// (1e-4 rad ~ 0.006 deg), so the SHARED `tangent_closure` walk — the one V2 and
// `PrepareOffsetFace` also use — never chains across them. The wall's closure is the
// wall alone.
//
// That is the correct outcome and it is correct for a reason worth writing down:
// the blend fork is only ever consulted on faces the G1 walk actually reaches, so a
// surface too irregular to be provably tangent is excluded by the OLDEST guard in
// the op rather than the newest. V3 is therefore byte-identical to V2 here, which
// is what this case asserts.
void test_variable_radius_blend() {
    const TopoDS_Shape sharp = ft::box();
    BRepFilletAPI_MakeFillet fillet(sharp);
    fillet.Add(1.0, 3.0, ft::vertical_edges(sharp).front());
    try {
        fillet.Build();
    } catch (const Standard_Failure&) {
        check(false, "variable radius: fixture threw");
        return;
    }
    check(fillet.IsDone() && !fillet.Shape().IsNull(), "variable radius: fixture builds");
    if (!fillet.IsDone() || fillet.Shape().IsNull()) return;
    const TopoDS_Shape body = fillet.Shape();
    check(cylinder_radii(body).empty(),
          "variable radius fixture: the blend face is NOT a cylinder");

    const std::vector<kf::BlendRecognition> report =
        kf::recognize_constant_radius_blends(body).recognized();
    std::fprintf(stderr, "  [characterized] variable radius: recognized=%zu ambiguous=%d\n",
                 report.size(), static_cast<int>(kf::recognize_constant_radius_blends(body)
                                                     .has_ambiguous()));
    check(report.empty(), "variable radius: NOTHING on this body is a recognized blend");

    const int evolving = only_freeform_face(body);
    check(evolving != 0, "variable radius: the evolving-radius face is uniquely addressable");
    if (evolving == 0) return;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(body, TopAbs_FACE, faces);
    const kf::BlendCertification cert = kf::certify_blend(TopoDS::Face(faces(evolving)), body);
    std::fprintf(stderr, "  [characterized] variable radius: certify status=%d reason=%s\n",
                 static_cast<int>(cert.recognition_status), cert.reason.c_str());
    check(!cert.ok, "variable radius: the evolving face does NOT certify");
    check(cert.recognition_status == kf::BlendRecognitionStatus::NotBlend,
          "variable radius: the recognizer MEASURED it not a blend — not Ambiguous, not a guess");
    const int wall = planar_neighbour(body, evolving);
    check(wall != 0, "variable radius: the evolving face has a planar support");
    if (wall == 0) return;
    const of::ClosureResult closure = of::tangent_closure(body, {wall});
    check(closure.ok, "variable radius: the wall's closure walks");
    if (!closure.ok) return;
    check(closure.ordinals.size() == 1,
          "variable radius: the approximated surface is NOT G1 at kTangentAngleTol, so the "
          "closure is the wall alone and the blend fork is never consulted");
    const of::BlendAwareClosure classified = of::blend_aware_closure(body, {wall});
    std::fprintf(stderr,
                 "  [characterized] variable radius: bac ok=%d code=%s moving=%zu blends=%zu "
                 "fixed=%zu closure=%zu\n",
                 static_cast<int>(classified.ok), classified.code.c_str(),
                 classified.moving.size(), classified.blends.size(), classified.fixed.size(),
                 closure.ordinals.size());

    check(classified.ok && classified.blends.empty(),
          "variable radius: the blend-aware walk certifies NOTHING");

    ops::OpOutcome outcomes[2];
    double volumes[2] = {0.0, 0.0};
    std::string deltas[2];
    for (int i = 0; i < 2; ++i) {
        BodyStore bodies;
        bodies.create("body_1", "op_seed", body);
        em::ElementMapPartition part;
        mint_faces_anchored(part, body);
        outcomes[i] = run_offset(bodies, part,
                                 offset_params(keys_of(closure.ordinals),
                                               {of::face_topokey(wall)}, 1.0, i == 0 ? 2 : 3));
        if (!bodies.contains("body_1")) return;
        volumes[i] = volume_of(bodies.get("body_1")->geom);
        deltas[i] = outcomes[i].delta.to_json().dump();
    }
    print_outcome("variable radius V2", outcomes[0]);
    print_outcome("variable radius V3", outcomes[1]);
    std::fprintf(stderr, "  [characterized] variable radius: V2=%.13f V3=%.13f\n", volumes[0],
                 volumes[1]);
    check(outcomes[0].status == ops::OpOutcome::Status::Ok &&
              outcomes[1].status == ops::OpOutcome::Status::Ok,
          "variable radius: both versions succeed");
    check(volumes[0] == volumes[1], "variable radius: V3 volume BIT-IDENTICAL to V2");
    check(deltas[0] == deltas[1],
          "variable radius: byte-identical elementMapDelta at both versions");
    // MEASURED, OCCT 8.0.1. Pinned so a change in either the approximation or the
    // G1 tolerance has to break this deliberately rather than drift past it.
    check_near(volumes[1], 1189.8839669828722, 1e-6,
               "variable radius: +1 on the wall gives 1189.8839669828722 at both versions");
}

// ── the TORUS BLEND ─────────────────────────────────────────────────────────
// A boss fused to a plate and its circular junction filleted: the blend face is a
// TORUS patch, tangent to a plane on one side and a cylinder on the other, running
// the full 360 deg. `test_offsetface_reblend.cpp` pins the CONVEX rim variant; this
// is the CONCAVE junction one, which is the shape real parts actually carry.
void test_torus_blend_at_a_boss_junction() {
    const TopoDS_Shape plate = BRepPrimAPI_MakeBox(gp_Pnt(-10, -10, 0), 20.0, 20.0, 4.0).Shape();
    const TopoDS_Shape boss =
        BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0, 0, 4), gp_Dir(0, 0, 1)), 4.0, 8.0).Shape();
    BRepAlgoAPI_Fuse fuse(plate, boss);
    fuse.Build();
    check(fuse.IsDone() && !fuse.Shape().IsNull(), "torus junction: the boss fuses to the plate");
    if (!fuse.IsDone() || fuse.Shape().IsNull()) return;
    ShapeUpgrade_UnifySameDomain unify(fuse.Shape(), Standard_True, Standard_True, Standard_False);
    unify.Build();
    const TopoDS_Shape fused = unify.Shape();

    TopoDS_Edge junction;
    for (const TopoDS_Edge& e : ft::all_edges(fused)) {
        const gp_Pnt c = centroid_of(e);
        if (std::abs(c.Z() - 4.0) <= 1e-9 && std::abs(c.X()) <= 1e-6 && std::abs(c.Y()) <= 1e-6) {
            junction = e;
        }
    }
    check(!junction.IsNull(), "torus junction: the circular boss/plate edge was found");
    if (junction.IsNull()) return;
    BRepFilletAPI_MakeFillet fillet(fused);
    fillet.Add(1.5, junction);
    try {
        fillet.Build();
    } catch (const Standard_Failure&) {
        check(false, "torus junction: fixture fillet threw");
        return;
    }
    check(fillet.IsDone() && !fillet.Shape().IsNull(), "torus junction: fixture builds");
    if (!fillet.IsDone() || fillet.Shape().IsNull()) return;
    const TopoDS_Shape body = fillet.Shape();

    const int torus_ordinal = only_freeform_face(body);
    check(torus_ordinal != 0, "torus junction: the torus patch is uniquely addressable");
    if (torus_ordinal == 0) return;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(body, TopAbs_FACE, faces);
    const kf::BlendCertification cert = kf::certify_blend(TopoDS::Face(faces(torus_ordinal)), body);
    std::fprintf(stderr, "  [characterized] torus junction: certify status=%d reason=%s\n",
                 static_cast<int>(cert.recognition_status), cert.reason.c_str());
    check(!cert.ok, "torus junction: the torus patch does NOT certify");
    // The recognizer's SAMPLED layer agrees it is a blend — a torus patch has a
    // constant section radius and is G1 to both supports, so it looks exactly like
    // one. It is layer 1, the ANALYTIC certificate, that declines: `Unknown` is not
    // `Proved`, and V1 destroys nothing it cannot prove analytically. This is the
    // load-bearing half of the cylinder-only scope.
    check(cert.recognition_status == kf::BlendRecognitionStatus::Recognized,
          "torus junction: the SAMPLED layer calls it a blend (which is why layer 1 matters)");
    check(cert.reason.rfind("BLEND_NOT_PROVED", 0) == 0,
          "torus junction: layer 1 refuses it — " + cert.reason);

    const int plate_top = planar_neighbour(body, torus_ordinal);
    check(plate_top != 0, "torus junction: the plate top is the torus's planar support");
    if (plate_top == 0) return;
    const of::ClosureResult closure = of::tangent_closure(body, {plate_top});
    check(closure.ok, "torus junction: the plate top's closure walks");
    if (!closure.ok) return;
    std::fprintf(stderr, "  [characterized] torus junction: closure = %zu faces\n",
                 closure.ordinals.size());
    const of::BlendAwareClosure classified = of::blend_aware_closure(body, {plate_top});
    std::fprintf(stderr,
                 "  [characterized] torus junction: bac ok=%d code=%s moving=%zu blends=%zu "
                 "fixed=%zu msg=%s\n",
                 static_cast<int>(classified.ok), classified.code.c_str(),
                 classified.moving.size(), classified.blends.size(), classified.fixed.size(),
                 classified.message.c_str());
    check(closure.ordinals.size() == 3,
          "torus junction: the plate top chains through the torus into the boss wall — the V2 "
          "path would offset all three");
    check(!classified.ok && classified.code == "OFFSET_FACE_BLEND_SURFACE_UNSUPPORTED",
          "torus junction: the blend-aware walk stops by name (" + classified.code + ")");

    BodyStore bodies;
    bodies.create("body_1", "op_seed", body);
    em::ElementMapPartition part;
    const ops::OpOutcome o =
        run_offset(bodies, part,
                   offset_params(keys_of(closure.ordinals), {of::face_topokey(plate_top)}, 1.0, 3));
    print_outcome("torus at boss junction", o);
    check(o.status == ops::OpOutcome::Status::Failed, "torus junction: refused, never published");
    check(diagnostic_code(o) == "OFFSET_FACE_BLEND_SURFACE_UNSUPPORTED",
          "torus junction: named code (" + diagnostic_code(o) + ")");
    check(o.body_events.empty(), "torus junction: nothing published");
    check(bodies.contains("body_1") && volume_of(bodies.get("body_1")->geom) == volume_of(body),
          "torus junction: body untouched");
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. CONDITIONING — six orders of scale, distance from origin, imported tolerance
// ═══════════════════════════════════════════════════════════════════════════

// A geometric scale of the canonical fixture. Scaling the BUILT body rather than
// re-filleting at the new size keeps the topology identical, so a difference in the
// result is a difference in the PRECISION handling and nothing else.
TopoDS_Shape scaled(const TopoDS_Shape& s, double factor) {
    gp_Trsf t;
    t.SetScale(gp_Pnt(0, 0, 0), factor);
    BRepBuilderAPI_Transform transform(s, t, Standard_True);
    return transform.Shape();
}

TopoDS_Shape translated(const TopoDS_Shape& s, const gp_Vec& v) {
    gp_Trsf t;
    t.SetTranslation(v);
    BRepBuilderAPI_Transform transform(s, t, Standard_True);
    return transform.Shape();
}

// Run the canonical side push on a TRANSFORMED copy of the canonical fixture and
// report the measured volume. The closure/primary are re-derived on the transformed
// body, because the ordinals need not survive a transform.
struct ScaledRun {
    bool ok = false;
    ops::OpOutcome outcome;
    double volume = 0.0;
    double radius = 0.0;
    int faces = 0;
};

ScaledRun push_transformed(const TopoDS_Shape& body, double distance) {
    ScaledRun out;
    const std::vector<kf::BlendRecognition> blends =
        kf::recognize_constant_radius_blends(body).recognized();
    if (blends.size() != 1 || blends.front().support_face_ordinals.size() != 2) return out;
    const int push = blends.front().support_face_ordinals[0];
    const of::ClosureResult closure = of::tangent_closure(body, {push});
    if (!closure.ok) return out;

    BodyStore bodies;
    bodies.create("body_1", "op_seed", body);
    em::ElementMapPartition part;
    mint_faces_anchored(part, body);
    out.outcome = run_offset(
        bodies, part, offset_params(keys_of(closure.ordinals), {of::face_topokey(push)}, distance, 3));
    out.ok = true;
    if (bodies.contains("body_1")) {
        const TopoDS_Shape r = bodies.get("body_1")->geom;
        out.volume = volume_of(r);
        out.radius = corner_radius(r);
        out.faces = face_count(r);
    }
    return out;
}

// 0.01 SCALE — the small end of the supported range. The canonical 10 mm box
// becomes a 0.1 mm box with an R0.02 round.
//
// Two questions, and they are different. (a) Does a request BELOW the authoring
// resolution still refuse by name at this scale? `authoring_resolution()` is a
// CONSTANT 1e-3 mm in v1 (`GeometryPrecision.h` says so, and says that making it
// scale is a v2 gate), so the boundary does not move with the body — that is the
// documented design, and this pins it at the scale where it bites hardest. (b) Does
// a request ABOVE it still round-trip exactly?
void test_micro_scale() {
    const Fixture base = filleted_box();
    check(base.ok, "0.01 scale: the base fixture built");
    if (!base.ok) return;
    const TopoDS_Shape body = scaled(base.body, 0.01);
    check(!body.IsNull(), "0.01 scale: the scaled body builds");
    if (body.IsNull()) return;
    check_relative(volume_of(body), (1000.0 - kCorner) * 1e-6, 1e-9,
                   "0.01 scale fixture: V0 = 9.914159265358979e-4 mm^3");
    check_relative(corner_radius(body), 0.02, 1e-9, "0.01 scale fixture: the round is R0.02");

    // (a) BELOW the authoring resolution.
    const ScaledRun tiny = push_transformed(body, 5.0e-4);
    check(tiny.ok, "0.01 scale: the sub-resolution run reached the op");
    if (tiny.ok) {
        print_outcome("0.01 scale, d=5e-4", tiny.outcome);
        check(tiny.outcome.status == ops::OpOutcome::Status::Failed,
              "0.01 scale: a 5e-4 mm push is refused");
        check(diagnostic_code(tiny.outcome) == "OFFSET_FACE_CHANGE_TOO_SMALL",
              "0.01 scale: named code (" + diagnostic_code(tiny.outcome) + ")");
    }

    // 5e-3 mm is HALF the diameter of the R0.02 round on a 0.1 mm body — visually a
    // huge edit at this scale — and it SUCCEEDS, because `authoring_resolution()` is
    // 1e-3 mm flat and does not scale with the body. That is the documented v1
    // design (`GeometryPrecision.h`: "It does NOT scale with `scale_diagonal` in v1,
    // on purpose ... a candidate for v2, with its own gate"), so this asserts the
    // constant rather than a scaled boundary that does not exist yet. If v2 ever
    // makes the resolution scale-relative, THIS is the assertion that has to be
    // re-decided, deliberately.
    const ScaledRun mid = push_transformed(body, 5.0e-3);
    check(mid.ok, "0.01 scale: the 5e-3 run reached the op");
    if (mid.ok) {
        print_outcome("0.01 scale, d=5e-3", mid.outcome);
        check(mid.outcome.status == ops::OpOutcome::Status::Ok,
              "0.01 scale: 5e-3 mm is ABOVE the flat 1e-3 authoring resolution and succeeds");
    }

    // (b) The proportional push: 0.02 mm is the 0.01-scaled image of the canonical
    // +2, so the exact analytic answer is the canonical one scaled by 1e-6.
    const ScaledRun valid = push_transformed(body, 0.02);
    check(valid.ok, "0.01 scale: the proportional run reached the op");
    if (!valid.ok) return;
    print_outcome("0.01 scale, d=0.02", valid.outcome);
    std::fprintf(stderr, "  [characterized] 0.01 scale: V=%.17g R=%.17g faces=%d\n", valid.volume,
                 valid.radius, valid.faces);
    check(valid.outcome.status == ops::OpOutcome::Status::Ok,
          "0.01 scale: the proportional push is Ok (" + valid.outcome.error_message + ")");
    // MEASURED 0.0011914159265358983 against the analytic 1.1914159265358979e-3 —
    // 3e-16 relative, i.e. the round trip costs nothing at all at this scale.
    check_relative(valid.volume, (1200.0 - kCorner) * 1e-6, 1e-12,
                   "0.01 scale: volume 1.1914159265358979e-3 mm^3");
    check_relative(valid.radius, 0.02, 1e-12, "0.01 scale: the corner is STILL R0.02");
    check(valid.faces == 7, "0.01 scale: 7 faces, as at the canonical scale");
}

// 10 m SCALE — the large end. Every length is 1000x the canonical, so every volume
// is 1e9x and the analytic answer is exact by similarity.
void test_ten_metre_scale() {
    const Fixture base = filleted_box();
    check(base.ok, "10 m scale: the base fixture built");
    if (!base.ok) return;
    const TopoDS_Shape body = scaled(base.body, 1000.0);
    check(!body.IsNull(), "10 m scale: the scaled body builds");
    if (body.IsNull()) return;
    check_relative(volume_of(body), (1000.0 - kCorner) * 1e9, 1e-12,
                   "10 m scale fixture: V0 = 9.914159265358979e11 mm^3");
    check_relative(corner_radius(body), 2000.0, 1e-12, "10 m scale fixture: the round is R2000");

    const ScaledRun run = push_transformed(body, 2000.0);
    check(run.ok, "10 m scale: the run reached the op");
    if (!run.ok) return;
    print_outcome("10 m scale, d=2000", run.outcome);
    std::fprintf(stderr, "  [characterized] 10 m scale: V=%.17g R=%.17g faces=%d\n", run.volume,
                 run.radius, run.faces);
    check(run.outcome.status == ops::OpOutcome::Status::Ok,
          "10 m scale: Ok (" + run.outcome.error_message + ")");
    // MEASURED 1191415926535.8979 against the analytic 1.1914159265358979e12 — every
    // digit a double carries. The three-stage chain costs nothing at the top of the
    // supported range either.
    check_relative(run.volume, (1200.0 - kCorner) * 1e9, 1e-12,
                   "10 m scale: volume 1.1914159265358979e12 mm^3");
    check_relative(run.radius, 2000.0, 1e-12, "10 m scale: the corner is STILL R2000");
    check(run.faces == 7, "10 m scale: 7 faces, as at the canonical scale");
}

// FAR FROM THE ORIGIN — the canonical fixture translated +1e6 mm on X, which is the
// largest coordinate magnitude `GeometryPrecision.h` claims to support. The shape is
// unchanged, so every analytic answer is unchanged; only the conditioning moves.
void test_far_from_origin() {
    const Fixture base = filleted_box();
    check(base.ok, "far from origin: the base fixture built");
    if (!base.ok) return;
    const TopoDS_Shape body = translated(base.body, gp_Vec(1.0e6, 0.0, 0.0));
    check(!body.IsNull(), "far from origin: the translated body builds");
    if (body.IsNull()) return;
    check_near(volume_of(body), 1000.0 - kCorner, 1e-6,
               "far from origin fixture: V0 is unchanged by a translation");

    const ScaledRun run = push_transformed(body, 2.0);
    check(run.ok, "far from origin: the run reached the op");
    if (!run.ok) return;
    print_outcome("far from origin, d=2", run.outcome);
    std::fprintf(stderr, "  [characterized] far from origin: V=%.17g R=%.17g faces=%d\n",
                 run.volume, run.radius, run.faces);
    check(run.outcome.status == ops::OpOutcome::Status::Ok,
          "far from origin: Ok (" + run.outcome.error_message + ")");
    // MEASURED 1191.415926535888 against the analytic 1191.4159265358979: the whole
    // conditioning cost of working a kilometre-cubed away from the origin is
    // 9.9e-12 mm^3, four orders under the 1e-8 asserted here and eleven under any
    // tolerance the op itself uses. The bound is deliberately TIGHTER than the
    // 1e-6 used at the canonical scale, because a conditioning regression would
    // show up as growth here and a loose bound would hide it.
    check_near(run.volume, 1200.0 - kCorner, 1e-8,
               "far from origin: volume 1191.4159265358979 to 1e-8 mm^3");
    check_near(run.radius, 2.0, 1e-9, "far from origin: the corner is STILL R2");
    check(run.faces == 7, "far from origin: 7 faces, as at the origin");
}

// IMPORTED-STEP-STYLE TOLERANCES. A healed STEP import routinely arrives carrying
// B-Rep tolerances three orders above `Precision::Confusion()`. `ShapeFix_
// ShapeTolerance` at 1e-3 mm reproduces that on the canonical fixture without
// needing a STEP file in the tree.
//
// EITHER answer is correct here and the point is to know WHICH. The analytic
// certificate compares an axis-to-plane distance against R at semantic tolerance
// (1e-6 mm), so geometry that arrived 1e-3 mm uncertain may legitimately fail to
// prove — refusing is honest. Succeeding is also honest, because the certificate
// reads the SURFACES, which a tolerance bump does not move.
void test_inflated_tolerances() {
    const Fixture base = filleted_box();
    check(base.ok, "inflated tolerance: the base fixture built");
    if (!base.ok) return;
    TopoDS_Shape body = BRepBuilderAPI_Copy(base.body).Shape();
    ShapeFix_ShapeTolerance().SetTolerance(body, 1.0e-3);
    check_near(volume_of(body), 1000.0 - kCorner, 1e-6,
               "inflated tolerance fixture: the geometry itself is unchanged");
    std::fprintf(stderr, "  [characterized] inflated tolerance: maxTol face=%.3e\n",
                 BRep_Tool::MaxTolerance(body, TopAbs_FACE));
    check_near(BRep_Tool::MaxTolerance(body, TopAbs_FACE), 1.0e-3, 1e-12,
               "inflated tolerance fixture: the faces really do carry 1e-3 mm");

    const ScaledRun run = push_transformed(body, 2.0);
    check(run.ok, "inflated tolerance: the run reached the op");
    if (!run.ok) return;
    print_outcome("inflated tolerance, d=2", run.outcome);
    std::fprintf(stderr, "  [characterized] inflated tolerance: V=%.17g R=%.17g faces=%d\n",
                 run.volume, run.radius, run.faces);
    // MEASURED: it SUCCEEDS, at 1191.4159265358976 — 3e-13 mm^3 off the exact
    // answer, i.e. the 1e-3 mm uncertainty label costs nothing.
    //
    // WHY THAT IS THE CORRECT OUTCOME rather than a gap. Every proof layer in the
    // certification stack reads SURFACES, not tolerances: `analytic_cylinder_blend`
    // compares the blend axis against the support PLANE and the exact
    // `BRepAdaptor_Surface::Cylinder()` radius, and `ShapeFix_ShapeTolerance` moves
    // neither. A tolerance is a statement about where the trimming CURVES might be,
    // and V1's proofs deliberately do not depend on those. The tolerance is not
    // ignored either — it feeds `build_tolerance` (`max(Confusion, maxShapeTolerance,
    // kBuildToleranceFloor)` = 1e-3 here, ten times the floor) and the publication
    // ceiling, so a result that actually got sloppier WOULD be caught downstream.
    check(run.outcome.status == ops::OpOutcome::Status::Ok,
          "inflated tolerance: Ok — the proof stack reads surfaces, which a tolerance bump "
          "does not move (" + run.outcome.error_message + ")");
    check_near(run.volume, 1200.0 - kCorner, 1e-8,
               "inflated tolerance: volume 1191.4159265358979 to 1e-8 mm^3");
    check_near(run.radius, 2.0, 1e-9, "inflated tolerance: the corner is STILL R2");
    check(run.faces == 7, "inflated tolerance: 7 faces");
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. PERSISTENCE — the V3 result through the BrepCodec byte form
// ═══════════════════════════════════════════════════════════════════════════

// C5 proved in-process determinism. The question this adds is whether the RESULT is
// still a blend after it has been through the bytes a saved document actually
// stores — because a reopened document is where the NEXT edit starts, and an edit
// that can only be made before a save is not an edit the product has.
void test_brep_codec_round_trip() {
    const Fixture f = filleted_box();
    check(f.ok, "codec: fixture built");
    if (!f.ok) return;
    BodyStore bodies;
    bodies.create("body_1", "op_seed", f.body);
    em::ElementMapPartition part;
    const ops::OpOutcome o =
        run_offset(bodies, part,
                   offset_params(keys_of(f.closure), {of::face_topokey(f.push_ordinal)}, 2.0, 3));
    check(o.status == ops::OpOutcome::Status::Ok, "codec: the V3 push is Ok (" + o.error_message + ")");
    if (o.status != ops::OpOutcome::Status::Ok || !bodies.contains("body_1")) return;
    const TopoDS_Shape result = bodies.get("body_1")->geom;
    // MEASURED: the V3 pipeline publishes a COMPOUND wrapping one solid, which
    // `single_solid_policy` admits by design. The save lane
    // (`io/ExportGeometry.cpp:117-131`) flattens a body to its contained solids
    // before handing them to the codec, and this case reproduces that step rather
    // than routing around it — a test that wrote the compound directly would be
    // testing a call the product never makes.
    std::fprintf(stderr, "  [characterized] codec: published body is a %s\n",
                 shape_type_name(result));
    std::vector<TopoDS_Shape> solids;
    for (TopExp_Explorer e(result, TopAbs_SOLID); e.More(); e.Next()) solids.push_back(e.Current());
    check(solids.size() == 1, "codec: the published body flattens to exactly one solid");
    if (solids.size() != 1) return;

    std::vector<std::uint8_t> bytes;
    const std::string write_error = onecad::io::write_brep_compound(solids, bytes);
    check(write_error.empty(), "codec: write_brep_compound succeeded (" + write_error + ")");
    if (!write_error.empty()) return;
    const std::filesystem::path path =
        std::filesystem::temp_directory_path() / "onecad-offsetface-corpus-v3.brep";
    {
        std::ofstream out(path, std::ios::binary | std::ios::trunc);
        out.write(reinterpret_cast<const char*>(bytes.data()),
                  static_cast<std::streamsize>(bytes.size()));
        check(out.good(), "codec: wrote the byte form to " + path.string());
    }
    const onecad::io::BrepReadResult read = onecad::io::read_brep_solids(path.string());
    std::filesystem::remove(path);
    check(read.ok(), "codec: read_brep_solids succeeded (" + read.error + ")");
    check(read.solids.size() == 1, "codec: exactly one solid came back");
    if (!read.ok() || read.solids.size() != 1) return;
    const TopoDS_Shape reloaded = read.solids.front();

    check_near(volume_of(reloaded), 1200.0 - kCorner, 1e-6,
               "codec: the reloaded body is still 1191.4159265358979");
    check(face_count(reloaded) == face_count(result), "codec: face count survived the bytes");

    // THE POINT OF THE CASE: the rebuilt blend must still CERTIFY on the reloaded
    // body, at the same radius and the same convexity. Certification is what the
    // NEXT V3 edit will run, so anything less means the second edit refuses.
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(reloaded, TopAbs_FACE, faces);
    int certified = 0;
    kf::BlendCertification found;
    for (int i = 1; i <= faces.Extent(); ++i) {
        const kf::BlendCertification c = kf::certify_blend(TopoDS::Face(faces(i)), reloaded);
        if (!c.ok) continue;
        ++certified;
        found = c;
    }
    check(certified == 1, "codec: exactly one face of the RELOADED body certifies as a blend");
    if (certified != 1) return;
    check_near(found.blend.radius, 2.0, 1e-9, "codec: it certifies at R2");
    check(found.blend.convexity == kf::BlendConvexity::Convex, "codec: with the same convexity");
    check_near(found.blend.boundary_length, 10.0, 1e-6,
               "codec: and the same 10 mm boundary extent");
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. MULTI-BLEND — the first case where the closure meets more than one
// ═══════════════════════════════════════════════════════════════════════════

// ALL FOUR VERTICAL EDGES ROUNDED, NO TOP OR BOTTOM FILLETS. This is the sharper
// version of `test_fully_rounded_box_falls_through_to_v2`: with no vertex spheres,
// every round has EXACTLY TWO G1 supports and therefore CERTIFIES rather than
// coming back `Ambiguous`. Pushing one wall meets two certified blends of the same
// radius and the same convexity — the multi-blend suppress + reblend path, which
// C5 never engaged.
//
// ANALYTIC, if it succeeds: pushing one wall of the 10^3 box out by 2 gives a
// 12 x 10 footprint with all four corners still R2, extruded 10:
//   V = 10 * (120 - 4*(4 - pi)) = 1165.6637061435915
struct FourRounds {
    bool ok = false;
    TopoDS_Shape body;
    int push_ordinal = 0;
    std::vector<int> closure;
    std::vector<int> blend_ordinals;         // all four
    std::vector<int> engaged_blends;         // the two the pushed wall supports
    std::vector<TopoDS_Edge> engaged_edges;  // their four boundary edges
};

FourRounds four_vertical_rounds() {
    FourRounds f;
    const TopoDS_Shape sharp = ft::box();
    BRepFilletAPI_MakeFillet fillet(sharp);
    for (const TopoDS_Edge& e : ft::vertical_edges(sharp)) fillet.Add(2.0, e);
    try {
        fillet.Build();
    } catch (const Standard_Failure&) {
        return f;
    }
    if (!fillet.IsDone() || fillet.Shape().IsNull()) return f;
    f.body = fillet.Shape();
    f.push_ordinal = face_ordinal_near(f.body, 10.0, 5.0, 5.0);

    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(f.body, TopAbs_FACE, faces);
    for (const kf::BlendRecognition& b :
         kf::recognize_constant_radius_blends(f.body).recognized()) {
        f.blend_ordinals.push_back(b.face_ordinal);
        if (std::find(b.support_face_ordinals.begin(), b.support_face_ordinals.end(),
                      f.push_ordinal) == b.support_face_ordinals.end()) {
            continue;
        }
        f.engaged_blends.push_back(b.face_ordinal);
        for (const int support : b.support_face_ordinals) {
            for (const TopoDS_Edge& e : shared_edges(faces(b.face_ordinal), faces(support)))
                f.engaged_edges.push_back(e);
        }
    }
    std::sort(f.blend_ordinals.begin(), f.blend_ordinals.end());
    std::sort(f.engaged_blends.begin(), f.engaged_blends.end());

    const of::ClosureResult closure = of::tangent_closure(f.body, {f.push_ordinal});
    if (!closure.ok) return f;
    f.closure = closure.ordinals;
    f.ok = true;
    return f;
}

void test_multi_blend_engagement() {
    const FourRounds f = four_vertical_rounds();
    check(f.ok, "four rounds: fixture built");
    if (!f.ok) return;
    check_near(volume_of(f.body), 1000.0 - 4.0 * kCorner, 1e-6,
               "four rounds fixture: V0 = 965.6637061435917");
    check(f.blend_ordinals.size() == 4,
          "four rounds fixture: ALL FOUR rounds recognize (no vertex spheres, exactly 2 G1 "
          "supports each)");
    check(f.engaged_blends.size() == 2,
          "four rounds fixture: the pushed wall supports exactly TWO of them");
    check(f.engaged_edges.size() == 4,
          "four rounds fixture: those two contribute four boundary edges");
    check(f.closure.size() == 8,
          "four rounds fixture: the G1 closure runs the whole perimeter (4 walls + 4 rounds)");
    // Each of the two ENGAGED rounds certifies through layers 1-3 on its own — the
    // property `test_fully_rounded_box_falls_through_to_v2` does NOT have, because
    // there every round is G1 to a corner sphere as well and comes back Ambiguous.
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(f.body, TopAbs_FACE, faces);
    for (const int ord : f.engaged_blends) {
        const kf::BlendCertification cert = kf::certify_blend(TopoDS::Face(faces(ord)), f.body);
        check(cert.ok, "four rounds: " + of::face_topokey(ord) + " CERTIFIES — " + cert.reason);
    }

    const of::BlendAwareClosure classified = of::blend_aware_closure(f.body, {f.push_ordinal});
    std::fprintf(stderr,
                 "  [characterized] four rounds: bac ok=%d code=%s moving=%zu blends=%zu "
                 "fixed=%zu msg=%s\n",
                 static_cast<int>(classified.ok), classified.code.c_str(),
                 classified.moving.size(), classified.blends.size(), classified.fixed.size(),
                 classified.message.c_str());
    // THE CLASSIFICATION THAT MAKES V3 DIFFERENT FROM V2 HERE: one face moves, two
    // are suppressed and rebuilt, and the remaining FIVE — three walls and the two
    // far rounds — are held completely still. V2 would offset all eight.
    check(classified.ok, "four rounds: the blend-aware walk does not refuse (" + classified.code +
                             " " + classified.message + ")");
    check(classified.moving.size() == 1, "four rounds: exactly ONE face moves");
    check(classified.blends.size() == 2, "four rounds: TWO blends are certified into B");
    check(classified.fixed.size() == 5, "four rounds: five faces are held fixed");

    const of::PlaneInfo pushed_before = of::plane_info(TopoDS::Face(faces(f.push_ordinal)));
    check(pushed_before.ok, "four rounds: the pushed wall is a measurable plane");

    // Two runs: the second is the DETERMINISM probe. C5 pinned determinism on the
    // single-blend path; with two blends the suppression seed order, the reblend seed
    // order and the composed history's successor sort all become observable, so it is
    // asserted here rather than assumed to carry over.
    ops::OpOutcome outcomes[2];
    std::string deltas[2];
    double volumes[2] = {0.0, 0.0};
    TopoDS_Shape results[2];
    em::ElementMapPartition parts[2];
    for (int i = 0; i < 2; ++i) {
        BodyStore bodies;
        bodies.create("body_1", "op_seed", f.body);
        mint_faces_anchored(parts[i], f.body);
        for (std::size_t k = 0; k < f.engaged_edges.size(); ++k) {
            parts[i].mint("body_1", "el_b" + std::to_string(k), km::ElementKind::Edge,
                          f.engaged_edges[k], f.body, anchor_at(f.engaged_edges[k]));
        }
        outcomes[i] = run_offset(
            bodies, parts[i],
            offset_params(keys_of(f.closure), {of::face_topokey(f.push_ordinal)}, 2.0, 3));
        if (!bodies.contains("body_1")) return;
        results[i] = bodies.get("body_1")->geom;
        volumes[i] = volume_of(results[i]);
        deltas[i] = outcomes[i].delta.to_json().dump();
    }
    const ops::OpOutcome& o = outcomes[0];
    print_outcome("four rounds, d=+2", o);

    // THE ANSWER: multi-blend suppress + reblend SUCCEEDS. Both rounds come back at
    // R2 and the other two never moved, so the result carries four R2 cylinders.
    check(o.status == ops::OpOutcome::Status::Ok, "four rounds: Ok (" + o.error_message + ")");
    if (o.status != ops::OpOutcome::Status::Ok) return;
    const TopoDS_Shape& r = results[0];
    std::fprintf(stderr, "  [characterized] four rounds: V=%.13f faces=%d cylinders=%zu\n",
                 volume_of(r), face_count(r), cylinder_radii(r).size());
    check_near(volume_of(r), 10.0 * (120.0 - 4.0 * (4.0 - kPi)), 1e-6,
               "four rounds: volume 1165.6637061435915 — a 12x10 footprint, all four corners "
               "still R2");
    check(face_count(r) == face_count(f.body), "four rounds: face count preserved");
    const std::vector<double> radii = cylinder_radii(r);
    check(radii.size() == 4, "four rounds: FOUR cylindrical faces survive");
    if (radii.size() == 4) {
        check_near(radii.front(), 2.0, 1e-9, "four rounds: the smallest is R2");
        check_near(radii.back(), 2.0, 1e-9, "four rounds: so is the largest — none inflated");
    }
    check(proved_blend_count(r) == 4, "four rounds: all four are PROVED blends on the result");

    // NON-VACUITY, and the reason the case exists at all: the SAME record at V2 must
    // produce different geometry. If V2 and V3 agreed here, every number above would
    // be measuring the V2 path with a 3 in the payload.
    {
        BodyStore v2_bodies;
        v2_bodies.create("body_1", "op_seed", f.body);
        em::ElementMapPartition v2_part;
        const ops::OpOutcome v2 = run_offset(
            v2_bodies, v2_part,
            offset_params(keys_of(f.closure), {of::face_topokey(f.push_ordinal)}, 2.0, 2));
        print_outcome("four rounds V2, d=+2", v2);
        if (v2_bodies.contains("body_1")) {
            const TopoDS_Shape v2_shape = v2_bodies.get("body_1")->geom;
            std::fprintf(stderr,
                         "  [characterized] four rounds V2: status=%d V=%.13f cylinders=%zu\n",
                         static_cast<int>(v2.status), volume_of(v2_shape),
                         cylinder_radii(v2_shape).size());
            check(v2.status != ops::OpOutcome::Status::Ok ||
                      volume_of(v2_shape) != volume_of(r),
                  "four rounds: V2 does NOT produce the V3 geometry (the two blends are what "
                  "V3 preserves and V2 inflates)");
        }
    }

    check(volumes[0] == volumes[1], "four rounds: bit-identical volume across two runs");
    check(deltas[0] == deltas[1], "four rounds: byte-identical elementMapDelta across two runs");

    // ── IDENTITY, for the multi-blend case ──────────────────────────────────
    em::ElementMapPartition& part = parts[0];
    const auto relabeled = [&o](const std::string& id) {
        return std::any_of(o.delta.relabeled.begin(), o.delta.relabeled.end(),
                           [&id](const em::DeltaEntry& e) { return e.element_id == id; });
    };
    const auto removed = [&o](const std::string& id) {
        return std::find(o.delta.removed.begin(), o.delta.removed.end(), id) !=
               o.delta.removed.end();
    };
    check(o.needs_repair.empty(), "four rounds identity: no face is stranded (no NeedsRepair)");
    check(part.entries_for_body("body_1").size() == static_cast<std::size_t>(faces.Extent()),
          "four rounds identity: every input FACE is still tracked (the four consumed boundary "
          "edges are not)");
    // WHERE the ids landed. "It relabelled" catches nothing on its own: an id that
    // binds the WRONG face relabels exactly as happily. With FOUR R2 cylinders on the
    // result — two rebuilt, two never touched — this is the case where a lineage gap
    // would show up as a swap between them.
    const int moved_wall = moved_plane_ordinal(r, pushed_before, 2.0);
    check(moved_wall > 0, "four rounds identity: the moved wall is uniquely identifiable");
    check(key_of(part, "el_f" + std::to_string(f.push_ordinal)) == of::face_topokey(moved_wall),
          "four rounds identity: the pushed wall's id landed ON THE MOVED WALL");
    for (const int ord : f.engaged_blends) {
        const std::string id = "el_f" + std::to_string(ord);
        check(relabeled(id), "four rounds identity: " + of::face_topokey(ord) + " RELABELS");
        check(!removed(id) && part.contains(id),
              "four rounds identity: " + of::face_topokey(ord) + " is not dropped");
        check(key_of(part, id) != of::face_topokey(moved_wall),
              "four rounds identity: a rebuilt blend id did NOT bind the moved wall");
    }
    // The two rounds the push never engaged must be exactly where they were: they
    // were held FIXED, so their ids may not have followed anything.
    for (const int ord : f.blend_ordinals) {
        if (std::find(f.engaged_blends.begin(), f.engaged_blends.end(), ord) !=
            f.engaged_blends.end()) {
            continue;
        }
        const std::string id = "el_f" + std::to_string(ord);
        check(part.contains(id) && key_of(part, id) != of::face_topokey(moved_wall),
              "four rounds identity: the untouched round " + of::face_topokey(ord) +
                  " is still tracked and did not bind the moved wall");
    }
    std::fprintf(stderr,
                 "  [characterized] four rounds identity: wall %s->%s (moved %s) | blends %s->%s, "
                 "%s->%s\n",
                 of::face_topokey(f.push_ordinal).c_str(),
                 key_of(part, "el_f" + std::to_string(f.push_ordinal)).c_str(),
                 of::face_topokey(moved_wall).c_str(),
                 of::face_topokey(f.engaged_blends[0]).c_str(),
                 key_of(part, "el_f" + std::to_string(f.engaged_blends[0])).c_str(),
                 of::face_topokey(f.engaged_blends[1]).c_str(),
                 key_of(part, "el_f" + std::to_string(f.engaged_blends[1])).c_str());
    for (std::size_t k = 0; k < f.engaged_edges.size(); ++k) {
        const std::string id = "el_b" + std::to_string(k);
        check(removed(id) && !part.contains(id),
              "four rounds identity: consumed boundary edge " + id + " lands in delta.removed");
    }
    std::string collision;
    check(topokeys_pairwise_distinct(part, collision),
          "four rounds identity: all surviving TopoKeys pairwise distinct (" + collision + ")");
}

}  // namespace

int main() {
    test_side_push_negative_preserves_the_corner();
    test_chamfered_box_is_v2();
    test_quarter_round_rib_is_reproduced();
    test_partially_trimmed_round();
    test_variable_radius_blend();
    test_torus_blend_at_a_boss_junction();
    test_micro_scale();
    test_ten_metre_scale();
    test_far_from_origin();
    test_inflated_tolerances();
    test_brep_codec_round_trip();
    test_multi_blend_engagement();
    std::fprintf(stderr, "offset-face adversarial corpus (V3): %d checks, %d failures\n", g_checks,
                 g_failures);
    return g_failures;
}
