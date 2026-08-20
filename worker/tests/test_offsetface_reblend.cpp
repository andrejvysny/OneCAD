// test_offsetface_reblend.cpp — WP3 C5: `op.offsetFace` at `resultPolicyVersion: 3`.
//
// THE CASE THAT ACTUALLY MATTERS is pushing a SIDE face of a filleted box, not the
// top one. The top face is C0 to a vertical fillet, so today's closure returns
// `{top}` and V2 is already correct there. On a side face `tangent_closure({side1})`
// returns `{side1, fillet, side2}`, the V2 semantic checker happily accepts the
// fillet as a cylinder that reached `R + d`, and the operation SUCCEEDS while
// inflating R2 into R4 and dragging the opposite wall along.
//
// For a 10^3 box with ONE vertical edge filleted R2, pushing one side face +2:
//
//   V0 = 1000 - 10*(4 - pi)   = 991.4159265358979    (the corner the round removes)
//   V3 = 1200 - 10*(4 - pi)   = 1191.4159265358979   only the picked face moved
//   V2 = 1440 - 10*(16 - 4pi) = 1405.6637061435916   both walls moved, corner is R4
//
// Both are asserted here. V3's number is the contract; V2's is the ZERO-DRIFT proof —
// a stored V2 record must keep producing exactly the geometry it produces today,
// because V3 is a geometry change and a silent migration would rewrite a user's part.
//
// ── DECLARED UNTESTED, with the reason ──────────────────────────────────────
// The POST-SUPPRESSION RESIDUAL-CLOSURE re-check (`OFFSET_FACE_DEPENDENT_NOT_A_BLEND`
// raised after suppression, when a G1 chain still runs out of the moving set on the
// suppressed body) has no case here, and the absence is structural rather than an
// oversight. The forward classification walk already visits every G1 neighbour in the
// input body and either traverses it into C_op or refuses by name, so the only edge
// that can newly be tangent is the sharp seed suppression exposes between the two
// supports — which requires the two supports to be near-coplanar. MEASURED, OCCT
// 8.0.1: `BRepFilletAPI_MakeFillet` THROWS on a corner of 179.25, 179.95, 179.995 and
// 179.999 degrees at both R2 and R0.5, so the fixture that would fire this rung
// cannot be built in the first place. The rung stays because it is the guard against
// unmeasured territory (Phase-0 spike fact 1: the kernel auto-propagates across a G1
// junction), not because a fixture reaches it. C6's decoy corpus is where a
// counterexample would land if one exists.
//
// `OFFSET_FACE_UNSUPPORTED_VERTEX_BLEND` is in the same position for the same kind of
// reason. It needs a certified blend whose SUPPORT also certifies, and a blend's
// supports are G1 to it by construction — so the support only certifies if it in turn
// has exactly two G1 supports. Every ordinary chain (fillet meeting a corner sphere,
// fillet of a fillet) gives the support THREE G1 neighbours, which the recognizer
// reports as `Ambiguous`, not `Recognized`. The one configuration that would work is
// two rounds meeting at exact tangency, and MEASURED: `BRepFilletAPI_MakeFillet` will
// not build R2 + R2 on the two vertical edges of a 4 mm wall (`test_blend_on_blend`
// records the outcome each run rather than asserting a prediction). The rung is kept
// because it is cheap and fail-closed.
//
// No framework: the process exit code IS the failure count.
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <set>
#include <string>
#include <utility>
#include <vector>

#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepGProp.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <Standard_Failure.hxx>
#include <gp_Ax2.hxx>
#include <gp_Vec.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRep_Tool.hxx>
#include <GC_MakeArcOfCircle.hxx>
#include <GProp_GProps.hxx>
#include <GeomAbs_CurveType.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Vertex.hxx>

#include "elementmap/ElementMapPartition.h"
#include "fillet_test_utils.h"
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
        std::fprintf(stderr, "FAIL: %s (got %.13f want %.13f tol %.1e)\n", msg.c_str(), got, want,
                     tol);
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

// PRODUCTION ALWAYS MINTS ANCHORED: `PlanExecutor` passes the ref's `anchor` echo
// straight into `ElementMapPartition::mint`. An unanchored entry scores at a REDUCED
// total weight, so a test that mints without one is not measuring the margin
// production actually gets — it is measuring a weaker resolver on the same geometry.
// Every entry in this file is anchored at the sub-shape's PRE-push world position,
// which is exactly the staleness a real edit hands the ladder.
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

// `faceIds` is the FROZEN G1 closure (what `PrepareOffsetFace` persists) and
// `primaryFaceIds` is what the user actually picked. The primary is passed
// EXPLICITLY — the closure is ordinal-sorted, so its first element is not
// necessarily the picked face, and defaulting to it would quietly test nothing.
json offset_params(const std::vector<std::string>& closure_keys,
                   const std::vector<std::string>& primary_keys, double distance, int version,
                   const char* type = "Offset", const std::string& opposite = {}) {
    json ids = json::array();
    for (const std::string& k : closure_keys) ids.push_back(k);
    json primary = json::array();
    for (const std::string& k : primary_keys) primary.push_back(k);
    json p = {{"targetBodyId", "body_1"},
              {"faceIds", std::move(ids)},
              {"primaryFaceIds", std::move(primary)},
              {"resultPolicyVersion", version},
              {"distance", distance},
              {"distanceType", type},
              {"chainTangentFaces", true}};
    if (!opposite.empty()) p["oppositeFaceId"] = opposite;
    return p;
}

std::string diagnostic_code(const ops::OpOutcome& o) {
    if (o.diagnostics.empty()) return {};
    return o.diagnostics.front().value("code", "");
}

std::string reason_code(const ops::OpOutcome& o) {
    if (o.diagnostics.empty()) return {};
    return o.diagnostics.front().value("reasonCode", "");
}

// ── the canonical fixture ───────────────────────────────────────────────────
// 10^3 box, ONE vertical edge filleted R2. `V0 = 960 + 10*pi`.
struct Fixture {
    bool ok = false;
    TopoDS_Shape body;
    int blend_ordinal = 0;
    int push_ordinal = 0;       // one of the blend's two supports
    int other_support = 0;      // the wall V2 drags along and V3 must hold still
    std::vector<int> closure;   // the frozen G1 closure of {push_ordinal}
    TopoDS_Face blend_face;
    std::vector<TopoDS_Edge> boundary_edges;  // the two straight blend/support edges
};

std::vector<TopoDS_Edge> shared_edges(const TopoDS_Shape& a, const TopoDS_Shape& b) {
    TopTools_IndexedMapOfShape b_edges;
    TopExp::MapShapes(b, TopAbs_EDGE, b_edges);
    std::vector<TopoDS_Edge> out;
    for (TopExp_Explorer ex(a, TopAbs_EDGE); ex.More(); ex.Next()) {
        if (b_edges.Contains(ex.Current())) out.push_back(TopoDS::Edge(ex.Current()));
    }
    return out;
}

Fixture filleted_box(double radius = 2.0) {
    Fixture f;
    const TopoDS_Shape sharp = ft::box();
    BRepFilletAPI_MakeFillet fillet(sharp);
    fillet.Add(radius, ft::vertical_edges(sharp).front());
    fillet.Build();
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

std::vector<std::string> keys_of(const std::vector<int>& ordinals) {
    std::vector<std::string> out;
    for (const int ord : ordinals) out.push_back(of::face_topokey(ord));
    return out;
}

// ── identity assertion machinery ────────────────────────────────────────────
// A bare "it relabelled" assertion cannot catch a MIS-BIND: an id that moves onto
// the WRONG face relabels just as happily as one that moves onto the right face.
// These helpers let every case below assert WHERE each id landed.

// Mint one anchored face entry per face of `body`, named `el_f<ordinal>`.
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

// Two ids on one TopoKey is a mis-bind that every per-id assertion would miss.
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

// The ordinal in `result` of the planar face that is `before`'s plane displaced by
// exactly `d` along its own outward normal — the same statement `check_semantics`
// makes, used here to name the face the push was SUPPOSED to produce.
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
        if (found != 0) return -1;  // ambiguous: the caller wants to know
        found = i;
    }
    return found;
}

// The ordinal in `shape` of the single cylindrical face of radius `radius`.
int cylinder_ordinal_at(const TopoDS_Shape& shape, double radius, double tol = 1e-9) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    int found = 0;
    for (int i = 1; i <= faces.Extent(); ++i) {
        const BRepAdaptor_Surface surf(TopoDS::Face(faces(i)));
        if (surf.GetType() != GeomAbs_Cylinder) continue;
        if (std::abs(surf.Cylinder().Radius() - radius) > tol) continue;
        if (found != 0) return -1;
        found = i;
    }
    return found;
}

// The analytic radius of the one cylindrical face of `shape`, or -1.
double corner_radius(const TopoDS_Shape& shape) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    double found = -1.0;
    for (int i = 1; i <= faces.Extent(); ++i) {
        const TopoDS_Face face = TopoDS::Face(faces(i));
        BRepAdaptor_Surface surf(face);
        if (surf.GetType() != GeomAbs_Cylinder) continue;
        if (found >= 0.0) return -2.0;  // more than one: the caller wants to know
        found = surf.Cylinder().Radius();
    }
    return found;
}

const double kPi = 3.14159265358979323846;
const double kCorner = 10.0 * (4.0 - kPi);  // 8.58407346410207 mm^3

// ═══════════════════════════════════════════════════════════════════════════
// 1. THE canonical case — side push at V3 vs V2
// ═══════════════════════════════════════════════════════════════════════════

void test_side_push_v3_preserves_the_corner() {
    const Fixture f = filleted_box();
    check(f.ok, "fixture: filleted box built and its blend recognized");
    if (!f.ok) return;
    check_near(volume_of(f.body), 1000.0 - kCorner, 1e-9, "fixture: V0 = 991.4159265358979");
    check(f.closure.size() == 3, "fixture: the frozen closure of one side wall is 3 faces");
    check(std::find(f.closure.begin(), f.closure.end(), f.blend_ordinal) != f.closure.end(),
          "fixture: the blend is inside the frozen closure (this is the V2 defect)");

    BodyStore bodies;
    bodies.create("body_1", "op_seed", f.body);
    em::ElementMapPartition part;
    const ops::OpOutcome o =
        run_offset(bodies, part,
                   offset_params(keys_of(f.closure), {of::face_topokey(f.push_ordinal)}, 2.0, 3));
    check(o.status == ops::OpOutcome::Status::Ok, "V3 side +2: Ok (" + o.error_message + ")");
    check(o.needs_repair.empty(), "V3 side +2: no NeedsRepair");
    check(o.body_events.size() == 1 && o.body_events[0].kind == "modified" &&
              o.body_events[0].body_id == "body_1",
          "V3 side +2: a single modified-in-place event on the target");
    if (!bodies.contains("body_1")) return;
    const TopoDS_Shape r = bodies.get("body_1")->geom;
    check_near(volume_of(r), 1200.0 - kCorner, 1e-6, "V3 side +2: volume 1191.4159265358979");
    check(face_count(r) == face_count(f.body), "V3 side +2: face count preserved");
    check_near(corner_radius(r), 2.0, 1e-9, "V3 side +2: the corner is STILL R2");

    // Measured on the result, not inferred from the volume: the surviving cylinder
    // must still be recognized as a two-support constant-radius blend.
    TopTools_IndexedMapOfShape rf;
    TopExp::MapShapes(r, TopAbs_FACE, rf);
    int rebuilt = 0;
    for (int i = 1; i <= rf.Extent(); ++i) {
        const kf::TargetedBlendRecognition t = kf::recognize_blend_at(TopoDS::Face(rf(i)), r);
        if (t.recognized && t.proof == kf::AnalyticBlendProof::Proved) ++rebuilt;
    }
    check(rebuilt == 1, "V3 side +2: exactly one PROVED blend on the result");
}

void test_side_push_v2_is_unchanged() {
    const Fixture f = filleted_box();
    check(f.ok, "V2 drift: fixture built");
    if (!f.ok) return;
    BodyStore bodies;
    bodies.create("body_1", "op_seed", f.body);
    em::ElementMapPartition part;
    const ops::OpOutcome o =
        run_offset(bodies, part,
                   offset_params(keys_of(f.closure), {of::face_topokey(f.push_ordinal)}, 2.0, 2));
    check(o.status == ops::OpOutcome::Status::Ok, "V2 side +2: Ok (" + o.error_message + ")");
    if (!bodies.contains("body_1")) return;
    const TopoDS_Shape r = bodies.get("body_1")->geom;
    // MEASURED on OCCT 8.0.1 and pinned: both walls move and the corner inflates to
    // R4. This is the value a stored V2 record must keep producing forever.
    check_near(volume_of(r), 1405.6637061435916, 1e-6,
               "V2 side +2: unchanged legacy volume (both walls moved)");
    check_near(corner_radius(r), 4.0, 1e-9, "V2 side +2: the corner inflated to R4");
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. V3 == V2 wherever there is no blend to be aware of
// ═══════════════════════════════════════════════════════════════════════════

// The top cap is C0 to the vertical fillet, so the closure is `{top}` alone and B is
// empty. V3 must fall through to the V2 build and produce the identical body.
void test_top_push_matches_v2() {
    const Fixture f = filleted_box();
    check(f.ok, "top push: fixture built");
    if (!f.ok) return;
    const int top = face_ordinal_near(f.body, 5.0, 5.0, 10.0);
    const of::ClosureResult closure = of::tangent_closure(f.body, {top});
    check(closure.ok && closure.ordinals.size() == 1, "top push: the top cap closes over itself");

    double volumes[2] = {0.0, 0.0};
    int faces[2] = {0, 0};
    for (int i = 0; i < 2; ++i) {
        BodyStore bodies;
        bodies.create("body_1", "op_seed", f.body);
        em::ElementMapPartition part;
        const ops::OpOutcome o =
            run_offset(bodies, part,
                       offset_params(keys_of(closure.ordinals), {of::face_topokey(top)}, 2.0,
                                     i == 0 ? 2 : 3));
        check(o.status == ops::OpOutcome::Status::Ok,
              std::string("top push v") + (i == 0 ? "2" : "3") + ": Ok (" + o.error_message + ")");
        if (!bodies.contains("body_1")) return;
        volumes[i] = volume_of(bodies.get("body_1")->geom);
        faces[i] = face_count(bodies.get("body_1")->geom);
    }
    check(volumes[0] == volumes[1], "top push: V3 volume is BIT-IDENTICAL to V2");
    check(faces[0] == faces[1], "top push: V3 face count identical to V2");
    // The cap is not 10x10 — the R2 round takes a (4 - pi) bite out of its corner.
    check_near(volumes[1], volume_of(f.body) + 2.0 * (100.0 - (4.0 - kPi)), 1e-6,
               "top push: +2 over the rounded cap area");
}

// A body with no blend anywhere: the two versions must be indistinguishable.
void test_plain_box_matches_v2() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    const int side = face_ordinal_near(box, 10.0, 5.0, 5.0);
    double volumes[2] = {0.0, 0.0};
    int faces[2] = {0, 0};
    std::size_t relabels[2] = {0, 0};
    for (int i = 0; i < 2; ++i) {
        BodyStore bodies;
        bodies.create("body_1", "op_seed", box);
        em::ElementMapPartition part;
        TopTools_IndexedMapOfShape map;
        TopExp::MapShapes(box, TopAbs_FACE, map);
        for (int k = 1; k <= map.Extent(); ++k)
            part.mint("body_1", "el_f" + std::to_string(k), km::ElementKind::Face, map(k), box);
        const ops::OpOutcome o = run_offset(
            bodies, part,
            offset_params({of::face_topokey(side)}, {of::face_topokey(side)}, 2.0, i == 0 ? 2 : 3));
        check(o.status == ops::OpOutcome::Status::Ok,
              std::string("plain box v") + (i == 0 ? "2" : "3") + ": Ok (" + o.error_message + ")");
        check(o.delta.removed.empty(), "plain box: nothing dropped");
        if (!bodies.contains("body_1")) return;
        volumes[i] = volume_of(bodies.get("body_1")->geom);
        faces[i] = face_count(bodies.get("body_1")->geom);
        relabels[i] = o.delta.relabeled.size();
    }
    check(volumes[0] == volumes[1], "plain box: V3 volume BIT-IDENTICAL to V2 (1200)");
    check(faces[0] == faces[1] && faces[1] == 6, "plain box: 6 faces at both versions");
    check(relabels[0] == relabels[1], "plain box: identical relabel count at both versions");
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. Identity — what the wire sees (the canonical fixture will pin these)
// ═══════════════════════════════════════════════════════════════════════════

void test_identity_delta() {
    const Fixture f = filleted_box();
    check(f.ok, "identity: fixture built");
    if (!f.ok || f.boundary_edges.size() != 2) {
        check(f.boundary_edges.size() == 2, "identity: the blend has two boundary edges");
        return;
    }
    BodyStore bodies;
    bodies.create("body_1", "op_seed", f.body);
    em::ElementMapPartition part;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(f.body, TopAbs_FACE, faces);
    mint_faces_anchored(part, f.body);
    part.mint("body_1", "el_b0", km::ElementKind::Edge, f.boundary_edges[0], f.body,
              anchor_at(f.boundary_edges[0]));
    part.mint("body_1", "el_b1", km::ElementKind::Edge, f.boundary_edges[1], f.body,
              anchor_at(f.boundary_edges[1]));
    const of::PlaneInfo pushed_before = of::plane_info(TopoDS::Face(faces(f.push_ordinal)));
    check(pushed_before.ok, "identity: the pushed wall is a measurable plane");

    const ops::OpOutcome o =
        run_offset(bodies, part,
                   offset_params(keys_of(f.closure), {of::face_topokey(f.push_ordinal)}, 2.0, 3));
    check(o.status == ops::OpOutcome::Status::Ok, "identity: Ok (" + o.error_message + ")");
    if (!bodies.contains("body_1")) return;
    const TopoDS_Shape result = bodies.get("body_1")->geom;

    const std::string blend_id = "el_f" + std::to_string(f.blend_ordinal);
    const auto relabeled = [&o](const std::string& id) {
        return std::any_of(o.delta.relabeled.begin(), o.delta.relabeled.end(),
                           [&id](const em::DeltaEntry& e) { return e.element_id == id; });
    };
    const auto removed = [&o](const std::string& id) {
        return std::find(o.delta.removed.begin(), o.delta.removed.end(), id) !=
               o.delta.removed.end();
    };

    // THE SUPPRESSED-THEN-RECREATED BLEND FACE KEEPS ITS ElementId, under proof:
    // the mapping is asserted where `reblend`'s reproduction proof lives. Dropping
    // the lineage would not mean "refuse to bind" — it would mean the descriptor +
    // anchor ladder binds a cylinder of the same radius `d` away on weaker, generic
    // evidence with no blend-specific proof at all.
    check(relabeled(blend_id), "identity: the blend face RELABELS (id survives the round trip)");
    check(!removed(blend_id), "identity: the blend face id is not dropped");
    check(part.contains(blend_id), "identity: the blend face entry is still tracked");

    // Its two boundary edges are genuinely consumed by suppression. V1 lets them be
    // REMOVED on positive evidence rather than synthesizing a correspondence — an
    // edge the offset splits has no unique one. Asserted so a future silent rebind
    // is caught rather than welcomed.
    check(removed("el_b0") && removed("el_b1"),
          "identity: both blend boundary edge ids land in delta.removed");
    check(!part.contains("el_b0") && !part.contains("el_b1"),
          "identity: the consumed boundary edges are no longer tracked");

    // Every tracked face rebinds — the composed history's face bijection, seen from
    // the element map's side.
    check(o.needs_repair.empty(), "identity: no face is stranded (no NeedsRepair)");
    check(part.entries_for_body("body_1").size() == static_cast<std::size_t>(faces.Extent()),
          "identity: every input face is still tracked after the three-stage chain");

    // ── WHERE each id landed. "It relabelled" is not evidence of anything: an id
    //    that binds the WRONG face relabels exactly as happily as one that binds the
    //    right face. These are the assertions that can actually catch a mis-bind.
    const std::string pushed_id = "el_f" + std::to_string(f.push_ordinal);
    const int moved_wall = moved_plane_ordinal(result, pushed_before, 2.0);
    check(moved_wall > 0, "identity: the moved wall is uniquely identifiable in the result");
    check(key_of(part, pushed_id) == of::face_topokey(moved_wall),
          "identity: the pushed wall's id landed ON THE MOVED WALL (" +
              key_of(part, pushed_id) + " vs " + of::face_topokey(moved_wall) + ")");
    const int rebuilt = cylinder_ordinal_at(result, 2.0);
    check(rebuilt > 0, "identity: the rebuilt R2 blend is uniquely identifiable");
    check(key_of(part, blend_id) == of::face_topokey(rebuilt),
          "identity: the blend id landed ON THE REBUILT BLEND (" + key_of(part, blend_id) +
              " vs " + of::face_topokey(rebuilt) + ")");
    // The far wall was held FIXED by V3 — its id must not have followed the push.
    const std::string other_id = "el_f" + std::to_string(f.other_support);
    check(key_of(part, other_id) != of::face_topokey(moved_wall),
          "identity: the held-fixed wall's id did NOT bind the moved wall");
    std::string collision;
    check(topokeys_pairwise_distinct(part, collision),
          "identity: all surviving TopoKeys pairwise distinct (" + collision + ")");
    std::fprintf(stderr,
                 "  [characterized] identity map: pushed %s->%s (moved wall %s) | blend %s->%s "
                 "(rebuilt %s) | fixed wall %s->%s\n",
                 of::face_topokey(f.push_ordinal).c_str(), key_of(part, pushed_id).c_str(),
                 of::face_topokey(moved_wall).c_str(), of::face_topokey(f.blend_ordinal).c_str(),
                 key_of(part, blend_id).c_str(), of::face_topokey(rebuilt).c_str(),
                 of::face_topokey(f.other_support).c_str(), key_of(part, other_id).c_str());
}

// ── The cylindrical-support case ────────────────────────────────────────────
// A D-PRISM: a barrel whose outer profile is a 120-degree arc of radius 4 closed by
// three straight sides that are NOT tangent to it, so the arc meets the flats at a
// genuinely sharp 30-degree vertical edge. Rounding one of those gives a cylinder
// blend whose supports are a PLANE and a CYLINDER — the only geometry in this file
// where the resolver has to tell two cylinders apart, and therefore the only one
// where a lineage gap becomes a MIS-BIND rather than a harmless re-score.
//
// A protruding boss was the first attempt and is not usable: MEASURED, OCCT 8.0.1,
// `BRepAlgoAPI_Defeaturing` asked to remove ONLY the R1 junction fillet of a boss
// fused into a wall takes the whole boss with it (14 faces in, 8 out), which the
// suppression face-count postcondition refuses as `OFFSET_FACE_SUPPRESSION_FACE_COUNT`.
// The D-prism keeps the cylinder as part of the outer wall, where it is not a
// removable feature.
struct BossFixture {
    bool ok = false;
    TopoDS_Shape body;
    int blend_ordinal = 0;
    int push_ordinal = 0;   // the planar flat
    int boss_ordinal = 0;   // the cylindrical support
    std::vector<int> closure;
    double blend_radius = 1.0;
    double boss_radius = 4.0;
};

// INWARD. Pushing the flat OUTWARD is geometrically infeasible on this fixture and
// the refusal is honest rather than a bug: the flat sits at y = 4*sin(60) = 3.4641 and
// the arc's extreme is y = 4, so a +1 push moves the plane clear of the cylinder it is
// supposed to meet and `BRepOffset_MakeOffset` reports IsDone with a NULL shape
// (caught by `OFFSET_FACE_STAGE_NULL_SHAPE`). -1 keeps the junction on the arc.
const double kBossPush = -1.0;

BossFixture boss_in_wall() {
    BossFixture f;
    const double r = f.boss_radius;
    const double half = r * std::sin(60.0 * kPi / 180.0);  // 3.4641016...
    const double x_join = r * std::cos(60.0 * kPi / 180.0);  // 2.0
    const gp_Pnt start(x_join, -half, 0.0);
    const gp_Pnt mid(r, 0.0, 0.0);
    const gp_Pnt end(x_join, half, 0.0);
    BRepBuilderAPI_MakeWire wire;
    wire.Add(BRepBuilderAPI_MakeEdge(GC_MakeArcOfCircle(start, mid, end).Value()).Edge());
    wire.Add(BRepBuilderAPI_MakeEdge(end, gp_Pnt(-6.0, half, 0.0)).Edge());
    wire.Add(BRepBuilderAPI_MakeEdge(gp_Pnt(-6.0, half, 0.0), gp_Pnt(-6.0, -half, 0.0)).Edge());
    wire.Add(BRepBuilderAPI_MakeEdge(gp_Pnt(-6.0, -half, 0.0), start).Edge());
    if (!wire.IsDone()) return f;
    const TopoDS_Shape base = BRepBuilderAPI_MakeFace(wire.Wire()).Shape();
    if (base.IsNull()) return f;
    const TopoDS_Shape prism = BRepPrimAPI_MakePrism(base, gp_Vec(0, 0, 10)).Shape();
    if (prism.IsNull()) return f;

    // The sharp vertical junction at (2, +half) — arc meets the upper flat.
    TopoDS_Edge junction;
    for (const TopoDS_Edge& e : ft::vertical_edges(prism)) {
        TopoDS_Vertex a, b;
        TopExp::Vertices(e, a, b);
        const gp_Pnt p = BRep_Tool::Pnt(a);
        if (std::abs(p.X() - x_join) <= 1e-6 && std::abs(p.Y() - half) <= 1e-6) junction = e;
    }
    if (junction.IsNull()) return f;

    BRepFilletAPI_MakeFillet fillet(prism);
    fillet.Add(f.blend_radius, junction);
    try {
        fillet.Build();
    } catch (const Standard_Failure&) {
        return f;
    }
    if (!fillet.IsDone() || fillet.Shape().IsNull()) return f;
    f.body = fillet.Shape();

    const std::vector<kf::BlendRecognition> blends =
        kf::recognize_constant_radius_blends(f.body).recognized();
    for (const kf::BlendRecognition& b : blends) {
        if (b.support_face_ordinals.size() != 2) continue;
        if (std::abs(b.radius - f.blend_radius) > 1e-3) continue;
        f.blend_ordinal = b.face_ordinal;
        for (const int support : b.support_face_ordinals) {
            TopTools_IndexedMapOfShape faces;
            TopExp::MapShapes(f.body, TopAbs_FACE, faces);
            if (of::surface_kind(TopoDS::Face(faces(support))) == of::SurfaceKind::Cylinder) {
                f.boss_ordinal = support;
            } else {
                f.push_ordinal = support;
            }
        }
    }
    if (f.blend_ordinal == 0 || f.push_ordinal == 0 || f.boss_ordinal == 0) return f;
    const of::ClosureResult closure = of::tangent_closure(f.body, {f.push_ordinal});
    if (!closure.ok) return f;
    f.closure = closure.ordinals;
    f.ok = true;
    return f;
}

void test_cylindrical_support_identity() {
    const BossFixture f = boss_in_wall();
    check(f.ok, "boss: fixture built (plane + cylinder supported blend)");
    if (!f.ok) return;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(f.body, TopAbs_FACE, faces);
    check(of::surface_kind(TopoDS::Face(faces(f.boss_ordinal))) == of::SurfaceKind::Cylinder,
          "boss: one blend support is a CYLINDER");
    check(of::surface_kind(TopoDS::Face(faces(f.push_ordinal))) == of::SurfaceKind::Plane,
          "boss: the pushed support is a PLANE");

    BodyStore bodies;
    bodies.create("body_1", "op_seed", f.body);
    em::ElementMapPartition part;
    mint_faces_anchored(part, f.body);
    const of::PlaneInfo pushed_before = of::plane_info(TopoDS::Face(faces(f.push_ordinal)));
    const double before_volume = volume_of(f.body);

    const ops::OpOutcome o =
        run_offset(bodies, part,
                   offset_params(keys_of(f.closure), {of::face_topokey(f.push_ordinal)}, kBossPush, 3));
    std::fprintf(stderr,
                 "  [characterized] boss V3: status=%d diag=%s needsRepair=%zu msg=%s\n",
                 static_cast<int>(o.status), diagnostic_code(o).c_str(), o.needs_repair.size(),
                 o.error_message.c_str());
    check(o.status == ops::OpOutcome::Status::Ok, "boss: Ok (" + o.error_message + ")");
    check(o.needs_repair.empty(), "boss: ZERO NeedsRepair");
    if (o.status != ops::OpOutcome::Status::Ok || !bodies.contains("body_1")) return;
    const TopoDS_Shape result = bodies.get("body_1")->geom;

    check(face_count(result) == face_count(f.body), "boss: face count preserved");
    std::fprintf(stderr, "  [characterized] boss volumes: before=%.9f after=%.9f delta=%.9f\n",
                 before_volume, volume_of(result), volume_of(result) - before_volume);
    // Pushing the flat INWARD removes material, and the blend must not have been
    // inflated in the process — the R1 corner is still R1 on the result.
    check(volume_of(result) < before_volume, "boss: the inward push removed material");
    check(cylinder_ordinal_at(result, f.blend_radius) > 0,
          "boss: the corner is STILL R1 after the push");

    const int rebuilt = cylinder_ordinal_at(result, f.blend_radius);
    check(rebuilt > 0, "boss: the rebuilt R1 blend is uniquely identifiable");
    const int boss_after = cylinder_ordinal_at(result, f.boss_radius);
    check(boss_after > 0, "boss: the R3 boss survives as a distinct cylinder");
    const int moved_wall = moved_plane_ordinal(result, pushed_before, kBossPush);
    check(moved_wall > 0, "boss: the moved wall is uniquely identifiable");

    const std::string blend_id = "el_f" + std::to_string(f.blend_ordinal);
    const std::string boss_id = "el_f" + std::to_string(f.boss_ordinal);
    const std::string pushed_id = "el_f" + std::to_string(f.push_ordinal);
    check(key_of(part, pushed_id) == of::face_topokey(moved_wall),
          "boss: the pushed wall's id landed on the moved wall (" + key_of(part, pushed_id) +
              " vs " + of::face_topokey(moved_wall) + ")");
    // THE MIS-BIND THIS CASE EXISTS FOR: the rebuilt blend is an R1 cylinder and the
    // boss is an R3 cylinder. Nothing but correct lineage separates them for a
    // resolver working from descriptor + anchor alone.
    check(key_of(part, blend_id) == of::face_topokey(rebuilt),
          "boss: the blend id landed on the REBUILT BLEND, not the boss (" +
              key_of(part, blend_id) + " vs " + of::face_topokey(rebuilt) + ")");
    check(key_of(part, boss_id) == of::face_topokey(boss_after),
          "boss: the boss id stayed on the BOSS, not the rebuilt blend (" +
              key_of(part, boss_id) + " vs " + of::face_topokey(boss_after) + ")");
    std::string collision;
    check(topokeys_pairwise_distinct(part, collision),
          "boss: all surviving TopoKeys pairwise distinct (" + collision + ")");
    std::fprintf(stderr,
                 "  [characterized] boss identity map: flat %s->%s (moved %s) | blend R1 %s->%s "
                 "(rebuilt %s) | arc R4 %s->%s (arc %s)\n",
                 of::face_topokey(f.push_ordinal).c_str(), key_of(part, pushed_id).c_str(),
                 of::face_topokey(moved_wall).c_str(), of::face_topokey(f.blend_ordinal).c_str(),
                 key_of(part, blend_id).c_str(), of::face_topokey(rebuilt).c_str(),
                 of::face_topokey(f.boss_ordinal).c_str(), key_of(part, boss_id).c_str(),
                 of::face_topokey(boss_after).c_str());
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. Refusals, by name
// ═══════════════════════════════════════════════════════════════════════════

// An absolute distance describes the operative SURFACE, and with a blend in the
// closure that is not the surface that ends up moving.
void test_absolute_distance_refused() {
    const Fixture f = filleted_box();
    check(f.ok, "absolute distance: fixture built");
    if (!f.ok) return;
    const int opposite = face_ordinal_near(f.body, 5.0, 5.0, 10.0);
    BodyStore bodies;
    bodies.create("body_1", "op_seed", f.body);
    em::ElementMapPartition part;
    const ops::OpOutcome o =
        run_offset(bodies, part,
                   offset_params(keys_of(f.closure), {of::face_topokey(f.push_ordinal)}, 12.0, 3,
                                 "Total", of::face_topokey(opposite)));
    check(o.status == ops::OpOutcome::Status::Failed && o.error_code == "OP_FAILED",
          "absolute distance: refused as OP_FAILED (" + o.error_code + ")");
    check(diagnostic_code(o) == "OFFSET_FACE_BLEND_WITH_ABSOLUTE_DISTANCE",
          "absolute distance: named code (" + diagnostic_code(o) + ")");
    check(bodies.contains("body_1") && volume_of(bodies.get("body_1")->geom) == volume_of(f.body),
          "absolute distance: body untouched");
}

// `faceIds` no longer describes the body: the stored operative set is stale. V2 only
// checks that the closure does not EXCEED the set; V3 requires exact equality.
void test_closure_mismatch_refused() {
    const Fixture f = filleted_box();
    check(f.ok, "closure mismatch: fixture built");
    if (!f.ok) return;
    std::vector<int> stale = f.closure;
    stale.push_back(face_ordinal_near(f.body, 5.0, 5.0, 10.0));  // the top cap, not in the closure
    std::sort(stale.begin(), stale.end());
    stale.erase(std::unique(stale.begin(), stale.end()), stale.end());
    BodyStore bodies;
    bodies.create("body_1", "op_seed", f.body);
    em::ElementMapPartition part;
    const ops::OpOutcome o = run_offset(
        bodies, part, offset_params(keys_of(stale), {of::face_topokey(f.push_ordinal)}, 2.0, 3));
    check(o.status == ops::OpOutcome::Status::Failed,
          "closure mismatch: refused (" + o.error_message + ")");
    check(diagnostic_code(o) == "OFFSET_FACE_CLOSURE_MISMATCH",
          "closure mismatch: named code (" + diagnostic_code(o) + ")");
    check(o.error_message.find("stored-only") != std::string::npos,
          "closure mismatch: names which side the difference is on");
}

// A push that would consume the fillet. WHICH rung refuses is a characterization,
// not a prediction — it is printed and asserted as "a named V3 refusal, nothing
// published", because the honest contract is that the destructive path says no.
void test_consuming_push_refused() {
    const Fixture f = filleted_box();
    check(f.ok, "consuming push: fixture built");
    if (!f.ok) return;
    BodyStore bodies;
    bodies.create("body_1", "op_seed", f.body);
    em::ElementMapPartition part;
    const ops::OpOutcome o =
        run_offset(bodies, part,
                   offset_params(keys_of(f.closure), {of::face_topokey(f.push_ordinal)}, -9.0, 3));
    std::fprintf(stderr, "  [characterized] -9 push: status=%d code=%s diag=%s reason=%s msg=%s\n",
                 static_cast<int>(o.status), o.error_code.c_str(), diagnostic_code(o).c_str(),
                 reason_code(o).c_str(), o.error_message.c_str());
    check(o.status == ops::OpOutcome::Status::Failed, "consuming push: refused, never published");
    check(!diagnostic_code(o).empty(), "consuming push: carries a machine-readable code");
    check(o.body_events.empty(), "consuming push: no body event");
    check(bodies.contains("body_1") && volume_of(bodies.get("body_1")->geom) == volume_of(f.body),
          "consuming push: body untouched");
}

// Two rounds of DIFFERENT radii on the same wall. The rebuild is one single-radius
// fillet over every seed, so a mixed set would come back with one blend silently
// re-cut to the other's radius — refused before anything is destroyed.
void test_mixed_radii_refused() {
    const TopoDS_Shape sharp = ft::box();
    std::vector<TopoDS_Edge> at_x0;
    for (const TopoDS_Edge& e : ft::vertical_edges(sharp)) {
        TopoDS_Vertex a, b;
        TopExp::Vertices(e, a, b);
        if (std::abs(BRep_Tool::Pnt(a).X()) <= 1e-9) at_x0.push_back(e);
    }
    check(at_x0.size() == 2, "mixed radii: the x=0 wall has two vertical edges");
    if (at_x0.size() != 2) return;
    BRepFilletAPI_MakeFillet fillet(sharp);
    fillet.Add(2.0, at_x0[0]);
    fillet.Add(3.0, at_x0[1]);
    fillet.Build();
    check(fillet.IsDone() && !fillet.Shape().IsNull(), "mixed radii: R2 + R3 fixture builds");
    if (!fillet.IsDone() || fillet.Shape().IsNull()) return;
    const TopoDS_Shape body = fillet.Shape();

    const int wall = face_ordinal_near(body, 0.0, 5.0, 5.0);
    const of::ClosureResult closure = of::tangent_closure(body, {wall});
    check(closure.ok && closure.ordinals.size() == 5,
          "mixed radii: the wall's frozen closure is 5 faces (wall + 2 rounds + 2 walls)");
    BodyStore bodies;
    bodies.create("body_1", "op_seed", body);
    em::ElementMapPartition part;
    const ops::OpOutcome o = run_offset(
        bodies, part, offset_params(keys_of(closure.ordinals), {of::face_topokey(wall)}, 1.0, 3));
    check(diagnostic_code(o) == "OFFSET_FACE_MIXED_BLEND_RADII",
          "mixed radii: named code (" + diagnostic_code(o) + " / " + o.error_message + ")");
    check(o.body_events.empty(), "mixed radii: nothing published");
}

// A TORUS blend — a fillet on a circular rim. It samples as a constant-radius blend
// and is NOT an exact `GeomAbs_Cylinder`, so the analytic certificate cannot be
// obtained and V1 refuses rather than destroying a face it cannot prove.
void test_non_cylinder_blend_refused() {
    const TopoDS_Shape cyl = BRepPrimAPI_MakeCylinder(5.0, 20.0).Shape();
    TopoDS_Edge rim;
    for (const TopoDS_Edge& e : ft::all_edges(cyl)) {
        if (BRepAdaptor_Curve(e).GetType() != GeomAbs_Circle) continue;
        GProp_GProps props;
        BRepGProp::LinearProperties(e, props);
        if (props.CentreOfMass().Z() > 19.0) rim = e;
    }
    check(!rim.IsNull(), "torus blend: the top rim edge was found");
    if (rim.IsNull()) return;
    BRepFilletAPI_MakeFillet fillet(cyl);
    fillet.Add(2.0, rim);
    fillet.Build();
    check(fillet.IsDone() && !fillet.Shape().IsNull(), "torus blend: fixture builds");
    if (!fillet.IsDone() || fillet.Shape().IsNull()) return;
    const TopoDS_Shape body = fillet.Shape();

    const int lateral = face_ordinal_near(body, 5.0, 0.0, 9.0);
    const of::ClosureResult closure = of::tangent_closure(body, {lateral});
    check(closure.ok && closure.ordinals.size() >= 2,
          "torus blend: the lateral wall chains into the rim round");
    BodyStore bodies;
    bodies.create("body_1", "op_seed", body);
    em::ElementMapPartition part;
    const ops::OpOutcome o =
        run_offset(bodies, part,
                   offset_params(keys_of(closure.ordinals), {of::face_topokey(lateral)}, 1.0, 3));
    std::fprintf(stderr, "  [characterized] torus blend: status=%d diag=%s msg=%s\n",
                 static_cast<int>(o.status), diagnostic_code(o).c_str(), o.error_message.c_str());
    check(diagnostic_code(o) == "OFFSET_FACE_BLEND_SURFACE_UNSUPPORTED",
          "torus blend: named code (" + diagnostic_code(o) + ")");
    check(o.body_events.empty(), "torus blend: nothing published");
}

// Convex and concave rounds in ONE closure. The eight suppression postconditions have
// no single expected volume direction for a mixed set (one blend adds material, the
// other removes it, and they can cancel to any sign), so the classification refuses
// before anything is destroyed rather than letting postcondition 8 decide by luck.
void test_mixed_convexity_refused() {
    // An L-prism: the wall at x=4 has a CONCAVE reentrant edge at (4,4) and a CONVEX
    // outer edge at (4,10). Same radius on both, so only convexity differs.
    BRepBuilderAPI_MakePolygon poly;
    poly.Add(gp_Pnt(0, 0, 0));
    poly.Add(gp_Pnt(10, 0, 0));
    poly.Add(gp_Pnt(10, 4, 0));
    poly.Add(gp_Pnt(4, 4, 0));
    poly.Add(gp_Pnt(4, 10, 0));
    poly.Add(gp_Pnt(0, 10, 0));
    poly.Close();
    const TopoDS_Shape base = BRepBuilderAPI_MakeFace(poly.Wire()).Shape();
    const TopoDS_Shape prism = BRepPrimAPI_MakePrism(base, gp_Vec(0, 0, 10)).Shape();
    std::vector<TopoDS_Edge> corners;
    for (const TopoDS_Edge& e : ft::vertical_edges(prism)) {
        TopoDS_Vertex a, b;
        TopExp::Vertices(e, a, b);
        const gp_Pnt p = BRep_Tool::Pnt(a);
        if (std::abs(p.X() - 4.0) > 1e-6) continue;
        if (std::abs(p.Y() - 4.0) <= 1e-6 || std::abs(p.Y() - 10.0) <= 1e-6) corners.push_back(e);
    }
    check(corners.size() == 2, "mixed convexity: the x=4 wall has a concave and a convex edge");
    if (corners.size() != 2) return;
    BRepFilletAPI_MakeFillet fillet(prism);
    fillet.Add(1.0, corners[0]);
    fillet.Add(1.0, corners[1]);
    try {
        fillet.Build();
    } catch (const Standard_Failure&) {
        check(false, "mixed convexity: fixture fillet threw");
        return;
    }
    check(fillet.IsDone() && !fillet.Shape().IsNull(), "mixed convexity: fixture builds");
    if (!fillet.IsDone() || fillet.Shape().IsNull()) return;
    const TopoDS_Shape body = fillet.Shape();

    const int wall = face_ordinal_near(body, 4.0, 7.0, 5.0);
    const of::ClosureResult closure = of::tangent_closure(body, {wall});
    check(closure.ok, "mixed convexity: the wall's closure walks");
    if (!closure.ok) return;
    BodyStore bodies;
    bodies.create("body_1", "op_seed", body);
    em::ElementMapPartition part;
    const ops::OpOutcome o = run_offset(
        bodies, part, offset_params(keys_of(closure.ordinals), {of::face_topokey(wall)}, 0.5, 3));
    std::fprintf(stderr, "  [characterized] mixed convexity: diag=%s msg=%s\n",
                 diagnostic_code(o).c_str(), o.error_message.c_str());
    check(diagnostic_code(o) == "OFFSET_FACE_MIXED_BLEND_CONVEXITY",
          "mixed convexity: named code (" + diagnostic_code(o) + ")");
    check(o.body_events.empty(), "mixed convexity: nothing published");
}

// A fully-rounded box. EVERY edge fillet there is G1 to the corner sphere patches as
// well as to its two walls, so the recognizer reports `Ambiguous` ("more than two G1
// support faces") and the walk TRAVERSES it — no blend awareness, V2 behaviour.
//
// THIS IS THE DOCUMENTED V1 FALLTHROUGH, PINNED ON PURPOSE. The alternative (refusing
// an `Ambiguous` neighbour) would make V3 strictly worse than V2 on a fully-rounded
// part, for a user who may not be editing near a corner at all. C6's adversarial
// corpus is where the vertex-patch case is revisited; until then a change to this
// behaviour has to break this test first, deliberately.
void test_fully_rounded_box_falls_through_to_v2() {
    const TopoDS_Shape sharp = ft::box();
    BRepFilletAPI_MakeFillet fillet(sharp);
    for (const TopoDS_Edge& e : ft::all_edges(sharp)) fillet.Add(1.0, e);
    try {
        fillet.Build();
    } catch (const Standard_Failure&) {
        check(false, "fully rounded: fixture fillet threw");
        return;
    }
    check(fillet.IsDone() && !fillet.Shape().IsNull(), "fully rounded: fixture builds");
    if (!fillet.IsDone() || fillet.Shape().IsNull()) return;
    const TopoDS_Shape body = fillet.Shape();
    check(kf::recognize_constant_radius_blends(body).has_ambiguous(),
          "fully rounded: the recognizer reports Ambiguous somewhere on this body");

    const int wall = face_ordinal_near(body, 10.0, 5.0, 5.0);
    const of::ClosureResult closure = of::tangent_closure(body, {wall});
    check(closure.ok, "fully rounded: the wall's closure walks");
    if (!closure.ok) return;
    // The blend-aware walk must classify NOTHING here — every rounded neighbour is
    // Ambiguous, so B is empty and the record executes the V2 path.
    const of::BlendAwareClosure classified = of::blend_aware_closure(body, {wall});
    check(classified.ok, "fully rounded: the blend-aware walk does not refuse (" +
                             classified.code + " " + classified.message + ")");
    check(classified.blends.empty(),
          "fully rounded: NO blend is certified — every rounded neighbour is Ambiguous");

    // MEASURED: the closure of a wall on a fully-rounded box contains the corner
    // SPHERE patches, and V1's operative-surface scope is planar + cylindrical, so
    // BOTH versions refuse — identically, which is the point. The pin is not "V3
    // succeeds"; it is "V3 does exactly what V2 does, to the letter".
    ops::OpOutcome outcomes[2];
    double volumes[2] = {0.0, 0.0};
    for (int i = 0; i < 2; ++i) {
        BodyStore bodies;
        bodies.create("body_1", "op_seed", body);
        em::ElementMapPartition part;
        outcomes[i] = run_offset(
            bodies, part,
            offset_params(keys_of(closure.ordinals), {of::face_topokey(wall)}, 1.0, i == 0 ? 2 : 3));
        if (!bodies.contains("body_1")) return;
        volumes[i] = volume_of(bodies.get("body_1")->geom);
    }
    std::fprintf(stderr, "  [characterized] fully rounded: V2 status=%d msg=%s | V3 status=%d "
                         "msg=%s | body V2=%.9f V3=%.9f\n",
                 static_cast<int>(outcomes[0].status), outcomes[0].error_message.c_str(),
                 static_cast<int>(outcomes[1].status), outcomes[1].error_message.c_str(),
                 volumes[0], volumes[1]);
    check(outcomes[0].status == outcomes[1].status,
          "fully rounded: identical status at both versions");
    check(outcomes[0].error_code == outcomes[1].error_code,
          "fully rounded: identical error code at both versions");
    check(outcomes[0].error_message == outcomes[1].error_message,
          "fully rounded: identical message at both versions (the Ambiguous fallthrough is "
          "byte-for-byte V2)");
    check(volumes[0] == volumes[1], "fully rounded: identical resulting body at both versions");
}

// A blend whose SUPPORT is itself a certified blend. Two R2 rounds on the two vertical
// edges of a 4 mm wall meet exactly, so each round's remaining supports are the outer
// wall and THE OTHER ROUND. Characterized rather than predicted: what OCCT builds at
// the exact-tangency limit decides whether this rung is reachable at all.
void test_blend_on_blend() {
    const TopoDS_Shape sharp = BRepPrimAPI_MakeBox(4.0, 10.0, 10.0).Shape();
    std::vector<TopoDS_Edge> wall_edges;
    for (const TopoDS_Edge& e : ft::vertical_edges(sharp)) {
        TopoDS_Vertex a, b;
        TopExp::Vertices(e, a, b);
        if (std::abs(BRep_Tool::Pnt(a).Y()) <= 1e-9) wall_edges.push_back(e);
    }
    check(wall_edges.size() == 2, "blend-on-blend: the 4 mm wall has two vertical edges");
    if (wall_edges.size() != 2) return;
    BRepFilletAPI_MakeFillet fillet(sharp);
    fillet.Add(2.0, wall_edges[0]);
    fillet.Add(2.0, wall_edges[1]);
    bool built = false;
    try {
        fillet.Build();
        built = fillet.IsDone() && !fillet.Shape().IsNull();
    } catch (const Standard_Failure& failure) {
        std::fprintf(stderr, "  [characterized] blend-on-blend: fillet threw (%s)\n",
                     failure.GetMessageString() ? failure.GetMessageString() : "OCCT");
    }
    if (!built) {
        // The tangent-limit fixture is not constructible. Recorded, not papered over.
        std::fprintf(stderr, "  [characterized] blend-on-blend: fixture NOT CONSTRUCTIBLE at the "
                             "exact-tangency limit\n");
        check(true, "blend-on-blend: fixture outcome recorded");
        return;
    }
    const TopoDS_Shape body = fillet.Shape();
    const int wall = face_ordinal_near(body, 2.0, 10.0, 5.0);  // the far wall, y=10
    const of::BlendAwareClosure classified = of::blend_aware_closure(body, {wall});
    std::fprintf(stderr,
                 "  [characterized] blend-on-blend: bac ok=%d code=%s moving=%zu blends=%zu "
                 "fixed=%zu msg=%s\n",
                 static_cast<int>(classified.ok), classified.code.c_str(),
                 classified.moving.size(), classified.blends.size(), classified.fixed.size(),
                 classified.message.c_str());
    check(true, "blend-on-blend: classification outcome recorded");
}

// An unsupported version still refuses, and the message now names BOTH supported
// values instead of implying only one exists.
void test_version_gate() {
    const Fixture f = filleted_box();
    check(f.ok, "version gate: fixture built");
    if (!f.ok) return;
    BodyStore bodies;
    bodies.create("body_1", "op_seed", f.body);
    em::ElementMapPartition part;
    const ops::OpOutcome o = run_offset(
        bodies, part, offset_params(keys_of(f.closure), {of::face_topokey(f.push_ordinal)}, 2.0, 4));
    check(diagnostic_code(o) == "UNSUPPORTED_OFFSET_FACE_RESULT_POLICY_VERSION",
          "version gate: 4 is still refused by code (" + diagnostic_code(o) + ")");
    check(o.error_message.find("2, 3") != std::string::npos,
          "version gate: the refusal names both supported versions");
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. Determinism
// ═══════════════════════════════════════════════════════════════════════════

void test_determinism() {
    const Fixture f = filleted_box();
    check(f.ok, "determinism: fixture built");
    if (!f.ok) return;
    struct Run {
        double volume = 0.0;
        double radius = 0.0;
        int faces = 0;
        std::string delta;
    };
    Run runs[2];
    for (int i = 0; i < 2; ++i) {
        BodyStore bodies;
        bodies.create("body_1", "op_seed", f.body);
        em::ElementMapPartition part;
        TopTools_IndexedMapOfShape faces;
        TopExp::MapShapes(f.body, TopAbs_FACE, faces);
        for (int k = 1; k <= faces.Extent(); ++k)
            part.mint("body_1", "el_f" + std::to_string(k), km::ElementKind::Face, faces(k),
                      f.body);
        const ops::OpOutcome o = run_offset(
            bodies, part,
            offset_params(keys_of(f.closure), {of::face_topokey(f.push_ordinal)}, 2.0, 3));
        check(o.status == ops::OpOutcome::Status::Ok,
              "determinism: run Ok (" + o.error_message + ")");
        if (!bodies.contains("body_1")) return;
        const TopoDS_Shape r = bodies.get("body_1")->geom;
        runs[i].volume = volume_of(r);
        runs[i].radius = corner_radius(r);
        runs[i].faces = face_count(r);
        runs[i].delta = o.delta.to_json().dump();
    }
    check(runs[0].volume == runs[1].volume, "determinism: bit-identical volume");
    check(runs[0].radius == runs[1].radius, "determinism: bit-identical corner radius");
    check(runs[0].faces == runs[1].faces, "determinism: identical face count");
    check(runs[0].delta == runs[1].delta,
          "determinism: byte-identical elementMapDelta (ordinal-level ids)");
}

}  // namespace

int main() {
    test_side_push_v3_preserves_the_corner();
    test_side_push_v2_is_unchanged();
    test_top_push_matches_v2();
    test_plain_box_matches_v2();
    test_identity_delta();
    test_cylindrical_support_identity();
    test_absolute_distance_refused();
    test_closure_mismatch_refused();
    test_mixed_radii_refused();
    test_mixed_convexity_refused();
    test_fully_rounded_box_falls_through_to_v2();
    test_blend_on_blend();
    test_non_cylinder_blend_refused();
    test_consuming_push_refused();
    test_version_gate();
    test_determinism();
    std::fprintf(stderr, "offset-face reblend (V3): %d checks, %d failures\n", g_checks,
                 g_failures);
    return g_failures;
}
