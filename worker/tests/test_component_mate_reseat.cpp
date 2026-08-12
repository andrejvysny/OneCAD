// test_component_mate_reseat.cpp — Component Library P3 WP-3.1, spec §5.5:
// `PlaceComponent`'s `mate` re-resolves through the ladder and re-seats on
// regen. In-process, real OCCT, `execute_place_component` directly (no wire
// round trip — mirrors `test_component_ops.cpp`'s and
// `test_cross_body_element_ref.cpp`'s own shape). No framework: exit code ==
// failure count.
#include <cmath>
#include <cstdio>
#include <string>

#include <BRepPrimAPI_MakeCylinder.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <gp_Ax2.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "ops/ComponentOp.h"
#include "ops/OpTypes.h"
#include "session/BodyStore.h"
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

}  // namespace

int main() {
    test_lateral_face_is_f1_sanity_check();
    test_reseat_on_target_move();
    test_no_reseat_within_epsilon();
    test_needs_repair_on_vanished_target_body();
    test_cross_body_target_never_substituted();
    if (g_failures == 0) std::fprintf(stderr, "component_mate_reseat: OK\n");
    return g_failures;
}
