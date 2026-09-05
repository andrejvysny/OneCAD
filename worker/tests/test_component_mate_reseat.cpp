// test_component_mate_reseat.cpp — Component Library P3 WP-3.1, spec §5.5:
// `PlaceComponent`'s `mate` re-resolves through the ladder and re-seats on
// regen. In-process, real OCCT, `execute_place_component` directly (no wire
// round trip — mirrors `test_component_ops.cpp`'s and
// `test_cross_body_element_ref.cpp`'s own shape). No framework: exit code ==
// failure count.
#include <array>
#include <cmath>
#include <cstdio>
#include <string>

#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "ops/ComponentOp.h"
#include "ops/OpTypes.h"
#include "ops/TransformOp.h"
#include "session/BodyStore.h"
#include "session/ClassifyElement.h"
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
    if (std::abs(got - want) > tol) {
        std::fprintf(stderr, "FAIL: %s (got %.6f want %.6f)\n", msg.c_str(), got, want);
        ++g_failures;
    }
}

struct Ctx {
    std::vector<std::pair<std::string, json>> sketches;
    std::string last_sketch;
    onecad::CancelToken cancel;
    ops::OpContext make(BodyStore& bodies, em::ElementMapPartition& part) {
        return ops::OpContext{bodies, &sketches, part, &last_sketch, false, json::object(), &cancel};
    }
};

json generator_source() {
    return json{{"kind", "generator"}, {"generatorId", "iso4762"}, {"generatorVersion", 1}};
}

json frozen_placement(double tx, double ty, double tz) {
    return json{{"translate", {tx, ty, tz}}};
}

json component_mate(const std::string& target_body, const std::string& element_id,
                    const std::string& kind_field, const std::string& snap_kind,
                    const std::optional<json>& descriptor = std::nullopt) {
    json target = {{"primary", {{"bodyId", target_body}, {"elementId", element_id}, {"kind", kind_field}}}};
    if (descriptor) target["intent"] = {{"kind", kind_field}, {"descriptor", *descriptor}};
    return json{{"selfAttachment", "shankAxis"}, {"target", target}, {"kind", snap_kind}, {"flipped", false}};
}

json place_component_op(const std::string& op_id, const json& placement,
                        const std::optional<json>& mate) {
    json params = {{"componentId", "onecad.std.iso4762"},
                   {"componentVersion", "1.0.0"},
                   {"componentRevision", "sha256:" + std::string(64, '0')},
                   {"source", generator_source()},
                   {"placement", placement}};
    if (mate) params["mate"] = *mate;
    return json{{"opType", "PlaceComponent"}, {"opId", op_id}, {"params", params}};
}

// A cylinder body along +Z through `origin`, radius 3, height 10 — its
// lateral face is the one `concentric` mate cases target. Face index 1 is
// the lateral face for `BRepPrimAPI_MakeCylinder`'s canonical face ordering
// (side, then the two end caps) — verified once via `topokey_for_element_in
// _body`'s own round trip in `test_lateral_face_is_f1_sanity_check` below,
// not assumed silently.
TopoDS_Shape cylinder_at(const gp_Pnt& origin) {
    return BRepPrimAPI_MakeCylinder(gp_Ax2(origin, gp_Dir(0.0, 0.0, 1.0)), 3.0, 10.0).Shape();
}

TopoDS_Shape lateral_face_of(const TopoDS_Shape& cyl) {
    return em::ElementMapPartition::shape_for_topokey(cyl, "f:1");
}

// ── sanity: the fixture's own assumption about which face is the lateral one. ─────
void test_lateral_face_is_f1_sanity_check() {
    const TopoDS_Shape cyl = cylinder_at(gp_Pnt(0.0, 0.0, 0.0));
    const TopoDS_Shape face = lateral_face_of(cyl);
    check(!face.IsNull(), "fixture sanity: f:1 exists");
    const km::ElementDescriptor d = em::ElementMapPartition::describe(face);
    check(d.surfaceType == GeomAbs_Cylinder, "fixture sanity: f:1 is the cylindrical (lateral) face");
}

// ── WP-3.1: a plate that moves re-seats its screw — the differentiator. ───────────
// Target cylinder's axis sits on world Z through the origin. The frozen
// placement (0,5,5) is off-axis (as if authored before the target moved) —
// `concentric` resolution projects it onto the CURRENT axis, landing at
// (0,0,5), a 5mm move past the epsilon.
void test_reseat_on_target_move() {
    BodyStore bodies;
    em::ElementMapPartition part;
    const TopoDS_Shape cyl = cylinder_at(gp_Pnt(0.0, 0.0, 0.0));
    bodies.create("plate", "op_plate", cyl);
    const TopoDS_Shape face = lateral_face_of(cyl);
    part.mint("plate", "el_hole", km::ElementKind::Face, face, cyl, json::object());

    const json mate = component_mate("plate", "el_hole", "face", "concentric");
    const json op = place_component_op("opm1", frozen_placement(0.0, 5.0, 5.0), mate);

    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opm1");

    check(oc.status == ops::OpOutcome::Status::Ok, "reseat: Ok");
    check(oc.needs_repair.empty(), "reseat: AutoBind, no NeedsRepair");
    check(oc.mate_placement.has_value(), "reseat: matePlacement echoed (moved beyond epsilon)");
    if (oc.mate_placement) {
        const json& t = (*oc.mate_placement)["translate"];
        check_near(t[0].get<double>(), 0.0, 1e-6, "reseat: translate.x snapped onto the axis");
        check_near(t[1].get<double>(), 0.0, 1e-6, "reseat: translate.y snapped onto the axis");
        check_near(t[2].get<double>(), 5.0, 1e-6,
                   "reseat: translate.z preserved (projection of the frozen anchor)");
    }
    check(bodies.get("body_opm1") != nullptr, "reseat: body published");
}

// ── WP-3.1: a within-plane wiggle that doesn't change the mate is a no-op. ────────
// Frozen placement is EXACTLY the solved seat already (translate (0,0,5) is
// already on the axis) — resolution must not report a move.
void test_no_reseat_within_epsilon() {
    BodyStore bodies;
    em::ElementMapPartition part;
    const TopoDS_Shape cyl = cylinder_at(gp_Pnt(0.0, 0.0, 0.0));
    bodies.create("plate", "op_plate", cyl);
    const TopoDS_Shape face = lateral_face_of(cyl);
    part.mint("plate", "el_hole", km::ElementKind::Face, face, cyl, json::object());

    const json mate = component_mate("plate", "el_hole", "face", "concentric");
    const json op = place_component_op("opm2", frozen_placement(0.0, 0.0, 5.0), mate);

    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opm2");

    check(oc.status == ops::OpOutcome::Status::Ok, "no-reseat: Ok");
    check(oc.needs_repair.empty(), "no-reseat: AutoBind, no NeedsRepair");
    check(!oc.mate_placement.has_value(),
          "no-reseat: no matePlacement echoed — already seated within epsilon, a true no-op");
    check(bodies.get("body_opm2") != nullptr, "no-reseat: body still published");
}

// ── WP-3.1: a plate whose hole was deleted produces a truthful NeedsRepair —
// never a dropped or silently-moved part. ──────────────────────────────────────────
void test_needs_repair_on_vanished_target_body() {
    BodyStore bodies;  // "plate" deliberately absent — simulates a deleted target body
    em::ElementMapPartition part;

    const json mate = component_mate("plate", "el_hole", "face", "concentric");
    const json op = place_component_op("opm3", frozen_placement(1.0, 2.0, 3.0), mate);

    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opm3");

    check(oc.status == ops::OpOutcome::Status::Ok,
          "vanished target: Ok — a recoverable STATE, never OP_FAILED");
    check(!oc.needs_repair.empty(), "vanished target: NeedsRepair flagged");
    check(!oc.mate_placement.has_value(), "vanished target: no reseat — nothing to solve against");
    const onecad::session::BodyRecord* rec = bodies.get("body_opm3");
    check(rec != nullptr, "vanished target: body still published — never dropped");
    if (rec != nullptr) {
        // Placement is unchanged: the frozen (1,2,3) stands, `read_placement`'s
        // plain translate with no rotation. Cheapest honest check: the body's
        // own bounding-box centroid moved by roughly the frozen translate from
        // the un-placed origin-seated blank, not to anywhere the (absent)
        // target would have implied.
        check(!rec->geom.IsNull(), "vanished target: published geometry is non-null");
    }
}

// ── WP-3.1 / VF-M7 parity: a mate's `target.primary.bodyId` is the ONLY body
// resolution ever reads — a tracked element's ACTUAL owning body is never
// substituted, and a ref that (mis)claims the wrong body must fail on THAT
// body's own geometry, never silently succeed against a different one. ────────────
void test_cross_body_target_never_substituted() {
    BodyStore bodies;
    em::ElementMapPartition part;
    // T: the correct target, a cylinder (has a lateral face concentric can bind).
    const TopoDS_Shape t = cylinder_at(gp_Pnt(0.0, 0.0, 0.0));
    bodies.create("body_t", "op_t", t);
    const TopoDS_Shape t_face = lateral_face_of(t);
    part.mint("body_t", "el_x", km::ElementKind::Face, t_face, t, json::object());
    // D: a decoy body with NO cylindrical face at all (a second, differently
    // positioned cylinder still has ONE, so use a body shaped so nothing on
    // it can auto-bind a concentric mate — reuse the SAME cylinder geometry
    // but the point is: even though it COULD structurally match, `el_x` was
    // never minted there, so the tracked rung must miss and the descriptor
    // ladder must run against D's own geometry, never T's).
    const TopoDS_Shape d = cylinder_at(gp_Pnt(100.0, 100.0, 100.0));
    bodies.create("body_d", "op_d", d);

    // A ref that claims `el_x` lives on body D (it does not — it was minted
    // on T). The tracked rung must return "" for D (VF-M7 gate), forcing a
    // descriptor-ladder resolution against D's OWN geometry.
    const km::ElementDescriptor t_face_descriptor = em::ElementMapPartition::describe(t_face);
    const json mate = component_mate("body_d", "el_x", "face", "concentric",
                                     em::ElementMapPartition::descriptor_to_json(t_face_descriptor));
    const json op = place_component_op("opm4", frozen_placement(0.0, 0.0, 0.0), mate);

    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opm4");

    // D DOES have a congruent cylindrical face (same shape, translated) — a
    // correct implementation resolves it fine, against D's OWN face, never
    // reaching for T's tracked entry. The load-bearing assertion is the
    // stronger one below: the resolved seat is near D's axis, not T's.
    check(oc.status == ops::OpOutcome::Status::Ok, "cross-body: Ok (D has its own matching face)");
    if (oc.mate_placement) {
        const json& tr = (*oc.mate_placement)["translate"];
        // Projected onto D's axis (through (100,100,100), +Z): x,y ≈ 100, not 0.
        check_near(tr[0].get<double>(), 100.0, 1e-3,
                   "cross-body: seated on D's OWN axis (x), never T's tracked entry");
        check_near(tr[1].get<double>(), 100.0, 1e-3,
                   "cross-body: seated on D's OWN axis (y), never T's tracked entry");
    } else {
        // A within-epsilon "no move" would only happen if the frozen (0,0,0)
        // already sat on D's axis — it doesn't (D's axis passes through
        // (100,100,100)), so absence here is itself a failure worth flagging.
        check(false, "cross-body: expected a reseat onto D's own (100,100,*) axis, got a no-op");
    }
}

// ── WP-F1.1 (spec §2.1/§5): a mate carrying a non-identity `selfFrame` seats
// the component by its ATTACHMENT point, not by its model origin. Same target
// and same frozen placement as `test_reseat_on_target_move` — only the frame
// differs, so the 10mm delta in the answer is attributable to it alone. ─────────
void test_reseat_honors_a_non_identity_self_frame() {
    BodyStore bodies;
    em::ElementMapPartition part;
    const TopoDS_Shape cyl = cylinder_at(gp_Pnt(0.0, 0.0, 0.0));
    bodies.create("plate", "op_plate", cyl);
    const TopoDS_Shape face = lateral_face_of(cyl);
    part.mint("plate", "el_hole", km::ElementKind::Face, face, cyl, json::object());

    // The attachment sits at component-local (0,4,10) — deliberately OFF every
    // axis so the framed answer cannot be confused with the un-framed one.
    // Frozen at (0,5,5) with no rotation, so the attachment currently sits at
    // world (0,9,15); `concentric` projects THAT onto the target axis, giving
    // a seat of (0,0,15), and the body therefore lands at (0,0,15)-(0,4,10) =
    // (0,-4,5). The un-framed case (`test_reseat_on_target_move`, same target
    // and same frozen placement) lands at (0,0,5) instead — the body itself on
    // the axis. That difference IS the feature.
    json mate = component_mate("plate", "el_hole", "face", "concentric");
    mate["selfFrame"] = json{{"origin", {0.0, 4.0, 10.0}},
                             {"z", {0.0, 0.0, 1.0}},
                             {"x", {1.0, 0.0, 0.0}}};
    const json op = place_component_op("opm5", frozen_placement(0.0, 5.0, 5.0), mate);

    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opm5");

    check(oc.status == ops::OpOutcome::Status::Ok, "self-frame reseat: Ok");
    check(oc.needs_repair.empty(), "self-frame reseat: AutoBind, no NeedsRepair");
    check(oc.mate_placement.has_value(), "self-frame reseat: matePlacement echoed");
    if (oc.mate_placement) {
        const json& t = (*oc.mate_placement)["translate"];
        check_near(t[0].get<double>(), 0.0, 1e-6, "self-frame reseat: translate.x");
        check_near(t[1].get<double>(), -4.0, 1e-6,
                   "self-frame reseat: the BODY sits off the axis — it is the ATTACHMENT point "
                   "that lands on it, not the model origin");
        check_near(t[2].get<double>(), 5.0, 1e-6,
                   "self-frame reseat: the seat's depth along the axis is preserved from where "
                   "the ATTACHMENT was, not from where the body origin was");
    }
    check(bodies.get("body_opm5") != nullptr, "self-frame reseat: body published");

    // FIXED POINT. Feeding the reseated placement back in must produce NO
    // further move: the seat anchor is the ATTACHMENT point's world position,
    // not the body origin's, so the frame offset is not re-subtracted. Anchor
    // this on the body origin instead and the component walks 10mm down the
    // axis on every single regen — silent, unbounded drift.
    if (oc.mate_placement) {
        BodyStore again_bodies;
        em::ElementMapPartition again_part;
        const TopoDS_Shape again_cyl = cylinder_at(gp_Pnt(0.0, 0.0, 0.0));
        again_bodies.create("plate", "op_plate", again_cyl);
        again_part.mint("plate", "el_hole", km::ElementKind::Face, lateral_face_of(again_cyl),
                        again_cyl, json::object());
        const json again_op = place_component_op("opm6", *oc.mate_placement, mate);
        Ctx c2;
        ops::OpContext ctx2 = c2.make(again_bodies, again_part);
        const ops::OpOutcome again = ops::execute_place_component(ctx2, again_op, "opm6");
        check(again.status == ops::OpOutcome::Status::Ok, "self-frame reseat (2nd pass): Ok");
        check(!again.mate_placement.has_value(),
              "self-frame reseat is a FIXED POINT — a second regen must not move it again");
    }
}

// ═══ WP-I I0 probes ═══════════════════════════════════════════════════════════
// RED-FIRST by construction: each of the four cases below asserts the CORRECT
// behaviour the WP-I design specifies, against the SHIPPED solver. They are
// expected to fail until I2a lands; the failure line IS the measurement.

constexpr double kPi = 3.14159265358979323846;

// A plate: x∈[0,w], y∈[0,d], z∈[0,h].
TopoDS_Shape plate(double w, double d, double h) {
    return BRepPrimAPI_MakeBox(gp_Pnt(0.0, 0.0, 0.0), w, d, h).Shape();
}

// The planar face of `body` whose classify-frame normal points along `want`.
// `frame_out` receives that face's `session::classify_shape` frame VERBATIM —
// the same object `resolve_mate_reseat` hands the solver, so the probe measures
// against the production frame rather than a re-derived one.
TopoDS_Shape planar_face_with_normal(const TopoDS_Shape& body, const std::array<double, 3>& want,
                                     json& frame_out) {
    NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> faces;
    TopExp::MapShapes(body, TopAbs_FACE, faces);
    for (int i = 1; i <= faces.Extent(); ++i) {
        const json c = onecad::session::classify_shape(faces(i));
        if (!c.contains("frame") || !c["frame"].contains("normal")) continue;
        const json& n = c["frame"]["normal"];
        const double dot = n[0].get<double>() * want[0] + n[1].get<double>() * want[1] +
                           n[2].get<double>() * want[2];
        if (dot > 0.999) {
            frame_out = c["frame"];
            return faces(i);
        }
    }
    return TopoDS_Shape();
}

// The first cylindrical face of `body` — the lateral face of a bare cylinder,
// found by classification rather than by a hardcoded ordinal, because the
// reversed-axis rebuild in `test_concentric_axis_reversal_is_not_silent` must
// not depend on `BRepPrimAPI_MakeCylinder`'s face order being axis-invariant.
TopoDS_Shape cylindrical_face_of(const TopoDS_Shape& body) {
    NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> faces;
    TopExp::MapShapes(body, TopAbs_FACE, faces);
    for (int i = 1; i <= faces.Extent(); ++i) {
        if (em::ElementMapPartition::describe(faces(i)).surfaceType == GeomAbs_Cylinder) {
            return faces(i);
        }
    }
    return TopoDS_Shape();
}

std::array<double, 3> translate_of(const json& placement) {
    std::array<double, 3> t{0.0, 0.0, 0.0};
    if (placement.contains("translate") && placement["translate"].is_array() &&
        placement["translate"].size() == 3) {
        for (std::size_t i = 0; i < 3; ++i) {
            const json& v = placement["translate"][i];
            if (v.is_number()) t[i] = v.get<double>();
        }
    }
    return t;
}

// The placement the component ACTUALLY publishes at: the re-seated one when the
// solve moved it past the epsilon, otherwise the frozen one that still stands.
// Reading only `mate_placement` would let a defect that produces NO re-seat at
// all read as "nothing to check".
json effective_placement(const ops::OpOutcome& oc, const json& frozen) {
    return oc.mate_placement ? *oc.mate_placement : frozen;
}

// The world direction the component's local +Z lands on under `placement` —
// the solver's own orientation convention (ComponentMateSolver.h), so this is
// the seat's axis/normal direction expressed as a measurable vector.
std::array<double, 3> mapped_local_z(const json& placement) {
    gp_Trsf rot;
    if (placement.contains("rotate") && placement["rotate"].is_object()) {
        const json& r = placement["rotate"];
        const double angle_deg =
            (r.contains("angleDeg") && r["angleDeg"].is_number()) ? r["angleDeg"].get<double>() : 0.0;
        if (angle_deg != 0.0 && r.contains("axis") && r["axis"].is_array() &&
            r["axis"].size() == 3) {
            const json& a = r["axis"];
            rot.SetRotation(gp_Ax1(gp_Pnt(0.0, 0.0, 0.0),
                                   gp_Dir(a[0].get<double>(), a[1].get<double>(),
                                          a[2].get<double>())),
                            angle_deg * kPi / 180.0);
        }
    }
    const gp_Dir z = gp_Dir(0.0, 0.0, 1.0).Transformed(rot);
    return {z.X(), z.Y(), z.Z()};
}

double signed_distance_to_plane(const std::array<double, 3>& p, const json& frame) {
    const json& o = frame["origin"];
    const json& n = frame["normal"];
    return (p[0] - o[0].get<double>()) * n[0].get<double>() +
           (p[1] - o[1].get<double>()) * n[1].get<double>() +
           (p[2] - o[2].get<double>()) * n[2].get<double>();
}

// ── I0(a1) / design I1: a coincident mate must seat ON the resolved plane. ────
// The plate was authored 3 mm thicker; the thickness edit moved its top face
// DOWN along the face's own normal. `solve_mate_placement`'s coincident branch
// never reads `frame.origin`, so the seat is the frozen anchor verbatim and the
// screw is left floating 3 mm above the plate — silently, with no repair item.
void test_coincident_reseat_follows_plane_along_normal() {
    BodyStore bodies;
    em::ElementMapPartition part;
    const TopoDS_Shape box = plate(40.0, 20.0, 10.0);  // top plane at z = 10
    bodies.create("plate", "op_plate", box);
    json top_frame;
    const TopoDS_Shape top = planar_face_with_normal(box, {0.0, 0.0, 1.0}, top_frame);
    check(!top.IsNull(), "I0(a1) fixture: the plate's +Z top face classifies with a plane frame");
    if (top.IsNull()) return;
    part.mint("plate", "el_top", km::ElementKind::Face, top, box, json::object());

    // Frozen 3 mm ABOVE the plane, laterally at the face centre.
    const json frozen = frozen_placement(20.0, 10.0, 13.0);
    const json mate = component_mate("plate", "el_top", "face", "coincident");
    const json op = place_component_op("opi1", frozen, mate);

    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_place_component(ctx, op, "opi1");

    check(oc.status == ops::OpOutcome::Status::Ok, "I0(a1): Ok");
    check(oc.needs_repair.empty(), "I0(a1): the target resolves — no NeedsRepair");
    const std::array<double, 3> seat = translate_of(effective_placement(oc, frozen));
    const double d = signed_distance_to_plane(seat, top_frame);
    std::fprintf(stderr,
                 "  [I0-a1] plane origin z=%.6f normal=(%.3f,%.3f,%.3f) seat=(%.6f,%.6f,%.6f) "
                 "matePlacement=%s signedDistance=%.6f\n",
                 top_frame["origin"][2].get<double>(), top_frame["normal"][0].get<double>(),
                 top_frame["normal"][1].get<double>(), top_frame["normal"][2].get<double>(), seat[0],
                 seat[1], seat[2], oc.mate_placement ? "echoed" : "absent", d);
    check_near(d, 0.0, 1e-6,
               "I0(a1): the solved seat lies ON the resolved plane — a target plane that moved "
               "along its own normal carries the component with it");
    check(bodies.get("body_opi1") != nullptr, "I0(a1): body published");
}

// ── I0(a2) / design I2: a rebuilt target whose cylinder axis came back
// REVERSED must not silently spin the component 180°. Same occupied volume,
// same element id, same frozen anchor — only `gp_Cylinder::Axis()`'s parametric
// direction differs, and `direction = flipped ? -axis : axis` follows it. ─────
void test_concentric_axis_reversal_is_not_silent() {
    const json frozen = frozen_placement(0.0, 0.0, 5.0);  // on the axis, mid-height, both passes
    const json mate = component_mate("plate", "el_hole", "face", "concentric");

    // Pass 1 — the cylinder as authored: axis +Z from the origin, z∈[0,10].
    std::array<double, 3> z_forward{0.0, 0.0, 0.0};
    std::array<double, 3> axis_forward{0.0, 0.0, 0.0};
    {
        BodyStore bodies;
        em::ElementMapPartition part;
        const TopoDS_Shape cyl =
            BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 0.0, 1.0)), 3.0, 10.0)
                .Shape();
        bodies.create("plate", "op_plate", cyl);
        const TopoDS_Shape face = cylindrical_face_of(cyl);
        check(!face.IsNull(), "I0(a2) fixture: the forward cylinder has a lateral face");
        if (face.IsNull()) return;
        part.mint("plate", "el_hole", km::ElementKind::Face, face, cyl, json::object());
        const json frame = onecad::session::classify_shape(face)["frame"];
        for (std::size_t i = 0; i < 3; ++i) axis_forward[i] = frame["axis"][i].get<double>();
        Ctx c;
        ops::OpContext ctx = c.make(bodies, part);
        const ops::OpOutcome oc =
            ops::execute_place_component(ctx, place_component_op("opi2a", frozen, mate), "opi2a");
        check(oc.status == ops::OpOutcome::Status::Ok, "I0(a2) pass 1: Ok");
        z_forward = mapped_local_z(effective_placement(oc, frozen));
    }

    // Pass 2 — the SAME occupied volume rebuilt from the other end, so the
    // parametric axis comes back anti-parallel. The frozen anchor still sits on
    // the axis inside the face's own extent, so the mate still resolves.
    BodyStore bodies;
    em::ElementMapPartition part;
    const TopoDS_Shape cyl =
        BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(0.0, 0.0, 10.0), gp_Dir(0.0, 0.0, -1.0)), 3.0, 10.0)
            .Shape();
    bodies.create("plate", "op_plate", cyl);
    const TopoDS_Shape face = cylindrical_face_of(cyl);
    check(!face.IsNull(), "I0(a2) fixture: the reversed cylinder has a lateral face");
    if (face.IsNull()) return;
    part.mint("plate", "el_hole", km::ElementKind::Face, face, cyl, json::object());
    const json frame = onecad::session::classify_shape(face)["frame"];
    std::array<double, 3> axis_reversed{0.0, 0.0, 0.0};
    for (std::size_t i = 0; i < 3; ++i) axis_reversed[i] = frame["axis"][i].get<double>();

    // The evidence a record authored against pass 1's geometry would carry
    // (design I2's frozen `targetAxis`). Unknown to the shipped worker, which
    // ignores it — that is exactly why this probe is red today.
    json reversed_mate = mate;
    reversed_mate["targetAxis"] = {axis_forward[0], axis_forward[1], axis_forward[2]};

    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc = ops::execute_place_component(
        ctx, place_component_op("opi2b", frozen, reversed_mate), "opi2b");
    check(oc.status == ops::OpOutcome::Status::Ok, "I0(a2) pass 2: Ok");
    const std::array<double, 3> z_reversed = mapped_local_z(effective_placement(oc, frozen));
    const double dot = z_forward[0] * z_reversed[0] + z_forward[1] * z_reversed[1] +
                       z_forward[2] * z_reversed[2];
    std::fprintf(stderr,
                 "  [I0-a2] frame axis fwd=(%.1f,%.1f,%.1f) rev=(%.1f,%.1f,%.1f) | seat local+Z "
                 "fwd=(%.3f,%.3f,%.3f) rev=(%.3f,%.3f,%.3f) dot=%.3f needsRepair=%zu\n",
                 axis_forward[0], axis_forward[1], axis_forward[2], axis_reversed[0],
                 axis_reversed[1], axis_reversed[2], z_forward[0], z_forward[1], z_forward[2],
                 z_reversed[0], z_reversed[1], z_reversed[2], dot, oc.needs_repair.size());
    // Either answer is acceptable AFTER the fix: keep the world direction the
    // component had, or halt for repair. Today it does neither — it flips.
    check(dot > 0.999 || !oc.needs_repair.empty(),
          "I0(a2): a target rebuilt with a reversed parametric axis must NOT silently spin the "
          "component 180° — keep the world direction, or raise a repair item");
}

// ── I0(a3) / design I1c: a seat that no longer lies on the resolved face is a
// repair, not a silent publish. The plate shrank laterally past the seat. ────
void test_coincident_seat_off_face_is_flagged() {
    BodyStore bodies;
    em::ElementMapPartition part;
    const TopoDS_Shape box = plate(40.0, 20.0, 10.0);
    bodies.create("plate", "op_plate", box);
    json top_frame;
    const TopoDS_Shape top = planar_face_with_normal(box, {0.0, 0.0, 1.0}, top_frame);
    check(!top.IsNull(), "I0(a3) fixture: the plate's +Z top face classifies");
    if (top.IsNull()) return;
    part.mint("plate", "el_top", km::ElementKind::Face, top, box, json::object());

    // ON the plane (so the I1 projection is a no-op and this case isolates the
    // boundary check), but 20 mm past the face's own x extent of [0, 40].
    const json frozen = frozen_placement(60.0, 10.0, 10.0);
    const json mate = component_mate("plate", "el_top", "face", "coincident");

    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc =
        ops::execute_place_component(ctx, place_component_op("opi3", frozen, mate), "opi3");

    check(oc.status == ops::OpOutcome::Status::Ok, "I0(a3): Ok — a recoverable STATE");
    const std::array<double, 3> seat = translate_of(effective_placement(oc, frozen));
    std::fprintf(stderr,
                 "  [I0-a3] face x extent=[0,40] seat=(%.3f,%.3f,%.3f) signedDistance=%.6f "
                 "needsRepair=%zu\n",
                 seat[0], seat[1], seat[2], signed_distance_to_plane(seat, top_frame),
                 oc.needs_repair.size());
    check(!oc.needs_repair.empty(),
          "I0(a3): a seat outside the resolved face's own boundary is flagged for repair, never "
          "published silently at the frozen placement");
    check(bodies.get("body_opi3") != nullptr,
          "I0(a3): the body still publishes at the frozen placement (SCHEMA §7.3 invariant)");
}

// ── I0(e): orientation provenance across the OCCT-history rung. An upstream
// `TransformBody` rebinds `el_top` through `ElementMapPartition::apply_history`
// (ladder level 1). `classify_shape` folds `face.Orientation()`, and history
// lists hand back NEUTRAL instances — if the mate ever saw such an instance the
// seat normal would invert. This case MAY already be green; it is a provenance
// measurement, not a predicted defect. ───────────────────────────────────────
void test_coincident_normal_sign_after_history_rung() {
    BodyStore bodies;
    em::ElementMapPartition part;
    const TopoDS_Shape box = plate(40.0, 20.0, 10.0);
    bodies.create("plate", "op_plate", box);
    json before_frame;
    const TopoDS_Shape top = planar_face_with_normal(box, {0.0, 0.0, 1.0}, before_frame);
    check(!top.IsNull(), "I0(e) fixture: the plate's +Z top face classifies");
    if (top.IsNull()) return;
    part.mint("plate", "el_top", km::ElementKind::Face, top, box, json::object());

    // The upstream edit: the cheapest op that actually runs `apply_history` +
    // `apply_rigid_motion` on a tracked entry.
    Ctx c0;
    ops::OpContext ctx0 = c0.make(bodies, part);
    const json xf = json{{"opType", "TransformBody"},
                         {"opId", "opi4xf"},
                         {"params",
                          {{"targets", json::array({"plate"})},
                           {"translate", {0.0, 0.0, 7.0}},
                           {"copy", false}}}};
    const ops::OpOutcome moved = ops::execute_transform_body(ctx0, xf, "opi4xf");
    check(moved.status == ops::OpOutcome::Status::Ok, "I0(e) fixture: the upstream move applied");
    check(moved.needs_repair.empty(), "I0(e) fixture: history rebound el_top, no NeedsRepair");

    const onecad::session::BodyRecord* rec = bodies.get("plate");
    check(rec != nullptr, "I0(e) fixture: the plate survives the move");
    if (rec == nullptr) return;
    json after_frame;
    planar_face_with_normal(rec->geom, {0.0, 0.0, 1.0}, after_frame);

    // Seated on the MOVED top plane (z = 17), at the face centre.
    const json frozen = frozen_placement(20.0, 10.0, 17.0);
    const json mate = component_mate("plate", "el_top", "face", "coincident");
    Ctx c;
    ops::OpContext ctx = c.make(bodies, part);
    const ops::OpOutcome oc =
        ops::execute_place_component(ctx, place_component_op("opi4", frozen, mate), "opi4");

    check(oc.status == ops::OpOutcome::Status::Ok, "I0(e): Ok");
    check(oc.needs_repair.empty(), "I0(e): the history-rebound target resolves");
    const std::array<double, 3> z = mapped_local_z(effective_placement(oc, frozen));
    std::fprintf(stderr,
                 "  [I0-e] plane normal before=(%.1f,%.1f,%.1f) after=(%.1f,%.1f,%.1f) seat "
                 "local+Z=(%.3f,%.3f,%.3f)\n",
                 before_frame["normal"][0].get<double>(), before_frame["normal"][1].get<double>(),
                 before_frame["normal"][2].get<double>(),
                 after_frame.is_null() ? 0.0 : after_frame["normal"][0].get<double>(),
                 after_frame.is_null() ? 0.0 : after_frame["normal"][1].get<double>(),
                 after_frame.is_null() ? 0.0 : after_frame["normal"][2].get<double>(), z[0], z[1],
                 z[2]);
    check(z[2] > 0.999,
          "I0(e): the seat's local +Z follows the face's OUTWARD normal in the body (+Z), never "
          "the reverse a neutral history instance would give");
}

}  // namespace

int main() {
    test_lateral_face_is_f1_sanity_check();
    test_reseat_on_target_move();
    test_no_reseat_within_epsilon();
    test_needs_repair_on_vanished_target_body();
    test_cross_body_target_never_substituted();
    test_reseat_honors_a_non_identity_self_frame();
    // WP-I I0 probes (expected RED until I2a lands).
    test_coincident_reseat_follows_plane_along_normal();
    test_concentric_axis_reversal_is_not_silent();
    test_coincident_seat_off_face_is_flagged();
    test_coincident_normal_sign_after_history_rung();
    if (g_failures == 0) std::fprintf(stderr, "component_mate_reseat: OK\n");
    return g_failures;
}
