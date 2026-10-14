#include <cassert>
#include <cmath>
#include <cstdio>
#include <limits>
#include <numbers>
#include <GCS.h>

#include "protocol/Dispatcher.h"
#include "protocol/SolverLane.h"
#include "session/SketchStore.h"
#include "sketch/Sketch.h"
#include "sketch/SketchPose.h"
#include "sketch/WireSketch.h"
#include "sketch/constraints/Constraints.h"
#include "sketch/solver/PointBranch.h"

namespace sk = onecad::core::sketch;

using nlohmann::json;
using onecad::protocol::Envelope;

json welded_rectangle() {
    const double corners[8][2] = {{0,0},{80,0},{80,0},{80,60},{80,60},{0,60},{0,60},{0,0}};
    json entities = json::array(), constraints = json::array();
    for (int i = 0; i < 8; ++i) {
        entities.push_back({{"id", "p" + std::to_string(i)}, {"type", "Point"},
                            {"at", {corners[i][0], corners[i][1]}}});
    }
    for (int i = 0; i < 4; ++i) {
        const auto line = "L" + std::to_string(i);
        entities.push_back({{"id", line}, {"type", "Line"}, {"p0Ref", "p" + std::to_string(2*i)},
                            {"p1Ref", "p" + std::to_string(2*i+1)}});
        constraints.push_back({{"id", "w" + std::to_string(i)}, {"type", "Coincident"},
            {"entities", {"p" + std::to_string(2*i+1), "p" + std::to_string((2*i+2)%8)}}});
        constraints.push_back({{"id", "d" + std::to_string(i)},
            {"type", i%2 == 0 ? "Horizontal" : "Vertical"}, {"entities", {line}}});
    }
    return {{"sketchId", "welded"}, {"plane", {{"kind", "XY"}}},
            {"entities", entities}, {"constraints", constraints}};
}

void welded_rectangle_moves_without_neighbor_pins() {
    onecad::session::SketchStore store;
    onecad::protocol::Dispatcher dispatcher;
    onecad::protocol::SolverLane lane(store);
    lane.register_verbs(dispatcher);
    assert(dispatcher.dispatch_once(Envelope::request(1, "SketchUpsert", welded_rectangle())).ok.value_or(false));
    assert(dispatcher.dispatch_once(Envelope::request(2, "BeginGesture", {{"sketchId", "welded"},
        {"sketchRevision", 1}, {"gestureId", 5}, {"drag", {{"pointId", "p0"}}}})).ok.value_or(false));
    const auto dragged = dispatcher.dispatch_once(Envelope::request(3, "SolveDrag", {
        {"gestureId", 5}, {"seq", 1}, {"target", {22,21}}}));
    std::fprintf(stderr, "T5 status=%s positions=%s\n", dragged.result["status"].get<std::string>().c_str(),
                 dragged.result["positions"].dump().c_str());
    assert(dragged.ok.value_or(false) && dragged.result["status"] == "success");
    const auto ended = dispatcher.dispatch_once(Envelope::request(4, "EndGesture", {{"gestureId", 5}}));
    assert(ended.ok.value_or(false) && ended.result["dof"] == 4);
    assert(ended.result["state"] == "UnderConstrained");
    const double expected[8][2] = {{22,21},{80,21},{80,21},{80,60},{80,60},{22,60},{22,60},{22,21}};
    const auto stored = store.snapshot("welded");
    for (int i = 0; i < 8; ++i) {
        const auto at = stored->wire_args["entities"][i]["at"];
        assert(std::abs(at[0].get<double>() - expected[i][0]) < 1e-4);
        assert(std::abs(at[1].get<double>() - expected[i][1]) < 1e-4);
    }
}

void disconnected_geometry_and_curves_stay_byte_exact() {
    sk::Sketch sketch;
    const auto a = sketch.addPoint(0,0), b = sketch.addPoint(0,0);
    sketch.addCoincident(a,b);
    const auto center = sketch.addPoint(100,100);
    const auto arc_id = sketch.addArc(center,3,-.2,1);
    auto* arc = sketch.getEntityAs<sk::SketchArc>(arc_id);
    arc->startAngle() = -.2;
    const sk::SketchPose original(sketch);
    sketch.beginPointDrag(a);
    assert(sketch.solveWithDrag(a,{1,2}).success);
    assert(sketch.getEntityAs<sk::SketchPoint>(b)->x() == 1);
    const sk::SketchPose accepted(sketch);
    assert(accepted.curves == original.curves);
    assert(!sketch.solveWithDrag(a,{std::numeric_limits<double>::quiet_NaN(),2}).success);
    const sk::SketchPose refused(sketch);
    assert(refused.points == accepted.points && refused.curves == accepted.curves);
    assert(sketch.solveWithDrag(a,{3,4}).success);
    sketch.endPointDrag();
    assert(sketch.getEntityAs<sk::SketchPoint>(a)->x() == 3);
    assert(sketch.getEntityAs<sk::SketchPoint>(b)->y() == 4);
    const sk::SketchPose after(sketch);
    assert(after.curves == original.curves);
    for (const auto& [id, at] : original.points) {
        if (id != a && id != b) assert(after.points.at(id) == at);
    }
}

void real_fixed_constraints_can_hold_a_valid_drag_short_of_cursor() {
    sk::Sketch sketch;
    const auto point = sketch.addPoint(0,0);
    sketch.addFixed(point);
    assert(sketch.solve().truth.admissible());
    sketch.beginPointDrag(point);
    const auto held = sketch.solveWithDrag(point,{5,6});
    assert(held.success && held.conflictingConstraints.empty());
    assert(sketch.getEntityAs<sk::SketchPoint>(point)->x() == 0);
    sketch.endPointDrag();
    assert(sketch.solve().truth.dof == 0);
}

void point_branch_checks_the_whole_quadratic_path() {
    const double c = std::cos(std::numbers::pi/6), s = .5;
    assert(!sk::pointBranchContinues({1,0},{c,s},{-2,0},{-.5*c,-.5*s}));
    assert(sk::pointBranchContinues({1,0},{c,s},{2,0},{2*c,2*s}));
    assert(!sk::pointBranchContinues({1,0},{1,0},{1,0},{1,.01}));
    assert(!sk::pointBranchContinues({1e155,0},{0,1e150},{1e155,1e155},{1e155,1e155}));
    const double scale = std::sqrt(6.6e307);
    assert(!sk::pointBranchContinues({scale,0},{scale*c,scale*s},
                                    {-2*scale,0},{-.5*scale*c,-.5*scale*s}));
}

void ambiguous_angle_only_blocks_its_own_component() {
    sk::Sketch sketch;
    const auto a = sketch.addPoint(0,0), b = sketch.addPoint(10,0), c = sketch.addPoint(20,0);
    const auto first = sketch.addLine(a,b), second = sketch.addLine(a,c);
    sketch.addConstraint(std::make_unique<sk::constraints::AngleConstraint>(first,second,0));
    const auto unrelated = sketch.addPoint(100,100);
    assert(sketch.solveWithDrag(unrelated,{101,102}).success);
    const sk::SketchPose before(sketch);
    const auto blocked = sketch.solveWithDrag(c,{20,5});
    assert(!blocked.success && blocked.errorMessage == "branchBoundary");
    const sk::SketchPose after(sketch);
    assert(before.points == after.points && before.curves == after.curves);
}

void constants_do_not_join_unknown_components_or_inactive_rows() {
    double ax=0,ay=0,bx=7,by=9,target=1,drive_x=3,drive_y=4;
    GCS::System solver;
    GCS::Point a(&ax,&ay), b(&bx,&by);
    solver.addConstraintCoordinateX(a,&target,1,true);
    solver.addConstraintCoordinateX(b,&target,2,true);
    GCS::VEC_pD all{&by,&ax,&ay,&bx};
    auto active = solver.connectedUnknowns({&ax,&ay},all);
    assert((active == GCS::VEC_pD{&ax,&ay}));
    solver.setScopedUnknowns(true);
    solver.declareUnknowns(active);
    solver.addConstraintCoordinateX(a,&drive_x,-1,true);
    solver.addConstraintCoordinateY(a,&drive_y,-1,true);
    solver.initSolution();
    assert(solver.solve() == GCS::Success);
    solver.applySolution();
    assert(ax == 1 && ay == 4 && bx == 7 && by == 9);
    solver.clearByTag(-1);
    solver.setScopedUnknowns(false);
    solver.declareUnknowns(all);
    solver.initSolution();
    const auto released_dof = solver.dofsNumber();
    GCS::System fresh;
    fresh.addConstraintCoordinateX(a,&target,1,true);
    fresh.addConstraintCoordinateX(b,&target,2,true);
    fresh.declareUnknowns(all); fresh.initSolution();
    assert(released_dof == fresh.dofsNumber());
}

void coupled_arc_fixed_preview_probe() {
    sk::Sketch sketch;
    const auto center = sketch.addPoint(0,0);
    sketch.addFixed(center);
    const auto start = sketch.addPoint(20,0), end = sketch.addPoint(0,20);
    const auto arc_id = sketch.addArc(center,20,0,std::numbers::pi/2,false,start,end);
    const auto* arc = sketch.getEntityAs<sk::SketchArc>(arc_id);
    const auto dragged = sketch.addPoint(0,20);
    sketch.addCoincident(dragged,arc->endPointId());
    assert(sketch.solve().truth.admissible());
    const sk::SketchPose before(sketch);
    const auto result = sketch.solveWithDrag(dragged,{-20,20});
    const auto* fixed = sketch.getEntityAs<sk::SketchPoint>(center);
    std::fprintf(stderr,"Point nonlinear Fixed probe success=%d center=(%.17g,%.17g) radius=%.17g angle=%.17g\n",
                 result.success,fixed->x(),fixed->y(),arc->radius(),arc->endAngle());
    assert(result.success);
    assert(fixed->x() == 0 && fixed->y() == 0);
}

void coupled_arc_refusal_restores_complete_pose() {
    sk::Sketch sketch;
    const auto center = sketch.addPoint(0,0), start = sketch.addPoint(1,0);
    const auto end = sketch.addPoint(std::cos(.1),std::sin(.1));
    const auto arc_id = sketch.addArc(center,1,0,.1,false,start,end);
    sketch.addFixed(center); sketch.addFixed(end);
    sketch.addConstraint(std::make_unique<sk::constraints::RadiusConstraint>(arc_id,1));
    assert(sketch.solve().truth.admissible());
    assert(sketch.solveWithDrag(start,{std::cos(.08),std::sin(.08)}).success);
    const sk::SketchPose accepted(sketch);
    const auto refused = sketch.solveWithDrag(start,{std::cos(.1),std::sin(.1)});
    const auto* arc = sketch.getEntityAs<sk::SketchArc>(arc_id);
    std::fprintf(stderr,"Point collapse probe success=%d radius=%.17g start=%.17g end=%.17g\n",
                 refused.success,arc->radius(),arc->startAngle(),arc->endAngle());
    assert(!refused.success);
    const sk::SketchPose restored(sketch);
    assert(restored.points == accepted.points && restored.curves == accepted.curves);
    assert(sketch.solveWithDrag(start,{std::cos(.05),std::sin(.05)}).success);
}

void positive_angle_drag_preserves_branch_and_recovers() {
    sk::Sketch sketch;
    const double c=std::cos(std::numbers::pi/6), s=.5;
    const auto a=sketch.addPoint(0,0), b=sketch.addPoint(1,0);
    const auto p=sketch.addPoint(0,0), q=sketch.addPoint(c,s);
    const auto first=sketch.addLine(a,b), second=sketch.addLine(p,q);
    sketch.addFixed(a); sketch.addFixed(p);
    sketch.addConstraint(std::make_unique<sk::constraints::AngleConstraint>(first,second,std::numbers::pi/6));
    assert(sketch.solve().truth.admissible());
    assert(sketch.solveWithDrag(b,{2,0}).success);
    const sk::SketchPose accepted(sketch);
    const auto refused=sketch.solveWithDrag(b,{-2,0});
    std::fprintf(stderr,"Positive Angle branch status=%d reason=%s\n",refused.success,refused.errorMessage.c_str());
    assert(!refused.success && refused.errorMessage == "branchBoundary");
    const sk::SketchPose restored(sketch);
    assert(accepted.points == restored.points && accepted.curves == restored.curves);
    assert(sketch.solveWithDrag(b,{3,0}).success);
}

void wire_angle_value_is_radians_and_legacy_alias_stays_degrees() {
    const auto entities=json::array({{{"id","L"},{"type","Line"},{"p0",{0,0}},{"p1",{1,0}}},
                                    {{"id","M"},{"type","Line"},{"p0",{0,0}},{"p1",{1,1}}}});
    for (const auto& key : {"value","angleDeg","angle"}) {
        const double input=std::string(key)=="value" ? std::numbers::pi/6 : 30;
        auto translated=onecad::wire::translate({{"sketchId","angles"},{"plane",{{"kind","XY"}}},
            {"entities",entities},{"constraints",json::array({{{"id","angle"},{"type","Angle"},
                {"entities",{"L","M"}},{key,input}}})}});
        assert(translated.ok);
        const auto* angle=dynamic_cast<const sk::constraints::AngleConstraint*>(translated.sketch->getAllConstraints()[0].get());
        assert(angle && std::abs(angle->value()-std::numbers::pi/6) < 1e-15);
        if (std::string(key)=="value") assert(angle->value() == input);
    }
    const auto malformed=onecad::wire::translate({{"sketchId","angles"},{"plane",{{"kind","XY"}}},
        {"entities",entities},{"constraints",json::array({{{"id","angle"},{"type","Angle"},
            {"entities",{"L","M"}},{"value","bad"},{"angleDeg",30}}})}});
    assert(!malformed.ok);
}

void inactive_sibling_rows_cannot_erase_active_redundant_tag() {
    double ax=1,ay=2,bx=1,by=2;
    GCS::System solver;
    GCS::Point a(&ax,&ay), b(&bx,&by);
    solver.addConstraintP2PCoincident(a,b,1,true);
    solver.addConstraintEqual(&ax,&bx,2,true);
    solver.setScopedUnknowns(true);
    GCS::VEC_pD active{&ax,&bx};
    solver.declareUnknowns(active);
    solver.initSolution();
    GCS::VEC_I redundant;
    solver.getRedundant(redundant);
    assert(!redundant.empty());
}

void unrelated_collapsed_arc_does_not_block_free_point_component() {
    sk::Sketch sketch;
    const auto center=sketch.addPoint(100,100);
    const auto start=sketch.addPoint(101,100), end=sketch.addPoint(101,100);
    sketch.addArc(center,1,0,0,false,start,end);
    const auto selected=sketch.addPoint(0,0);
    const sk::SketchPose before(sketch);
    assert(sketch.solveWithDrag(selected,{3,4}).success);
    const sk::SketchPose after(sketch);
    assert(before.curves == after.curves);
    for (const auto& [id,at] : before.points) if (id != selected) assert(after.points.at(id) == at);
}

int main() {
    inactive_sibling_rows_cannot_erase_active_redundant_tag();
    unrelated_collapsed_arc_does_not_block_free_point_component();
    wire_angle_value_is_radians_and_legacy_alias_stays_degrees();
    positive_angle_drag_preserves_branch_and_recovers();
    coupled_arc_refusal_restores_complete_pose();
    coupled_arc_fixed_preview_probe();
    welded_rectangle_moves_without_neighbor_pins();
    disconnected_geometry_and_curves_stay_byte_exact();
    real_fixed_constraints_can_hold_a_valid_drag_short_of_cursor();
    point_branch_checks_the_whole_quadratic_path();
    ambiguous_angle_only_blocks_its_own_component();
    constants_do_not_join_unknown_components_or_inactive_rows();
}
