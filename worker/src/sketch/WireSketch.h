// WireSketch.h — translate the SCHEMA §7.3/§7.4 wire sketch (plane + entities +
// constraints, opaque string ids) into the ported C++ `Sketch` (W-WP3b).
//
// This is a NEW worker file (not a port). The wire uses inline coordinates +
// type/kind tags; the ported `Sketch` generates its OWN internal UUIDs and
// references points by id. `WireSketch::translate` bridges the two, producing a
// live `Sketch` plus a `WireIndex` that maps between wire ids / point handles
// and the internal ids so responses can be reported in wire terms.
//
// Point-handle scheme (how SolveDrag `pointId` / positions keys address points):
//   * Point   entity `p`            -> handle "p"
//   * Line    entity `l` (inline)   -> "l.p0","l.start" (start) ; "l.p1","l.end" (end)
//   * Line    entity `l` (p0Ref/p1Ref) -> aliases "l.p0"/"l.p1"/... resolve to the
//                                         referenced Point entities (whose primary
//                                         handle stays the referenced point id)
//   * Circle/Arc/Ellipse entity `c` -> "c.center"
//   * Arc     entity `a`            -> "a.start" / "a.end" (W0b)
// An Arc's endpoints are MINTED as real `SketchPoint`s at their derived
// coordinates and coupled to center+radius+angle by internal PlaneGCS arc rules
// (`ConstraintSolver::addArcRules`, tag 0 — DOF-neutral and never reported as a
// user constraint). They are therefore first-class handles: a `Coincident` can
// weld a wall to a cap (SCHEMA §7.3 `positions`) and a `SolveDrag` can address
// `"a.start"` (§7.4). This DIVERGES from OneCAD-CPP, whose arc endpoints are
// purely derived — the port's `SketchArc` keeps the angle parameterization and
// merely gained two optional point ids.
//
// Accepted wire entity shapes (superset of SCHEMA §7.3 to also accept the corpus
// author-form):  Point {at|p|pos:[x,y]} ; Line {p0,p1:[x,y]} OR {p0Ref,p1Ref:id} ;
// Circle {center:[x,y], radius} ; Arc {center:[x,y], radius, start,end:[x,y]} OR
// {center,radius,startAngle,endAngle} ; Ellipse {center:[x,y], majorR, minorR,
// rotation?}. Dimensional constraint values accept a bare number OR a
// {value, expr?} object (SCHEMA §7.3 scalar rule).
//
// An Ellipse is NOT registered with PlaneGCS (`SolverAdapter` skips it — legacy
// parity), so a sketch carrying one reports DOF from `naiveDegreesOfFreedom()`
// (SCHEMA §7.4). It is still materialized, tessellated by `LoopDetector`, and
// built as a true `Geom_Ellipse` edge by `FaceBuilder`.
#pragma once

#include <memory>
#include <string>
#include <unordered_map>
#include <vector>

#include "nlohmann/json.hpp"
#include "sketch/Sketch.h"

namespace onecad::wire {

namespace sk = onecad::core::sketch;

// Bidirectional maps between wire ids / point handles and internal ids.
struct WireIndex {
    // wire entity id -> internal entity id (points, lines, arcs, circles).
    std::unordered_map<std::string, sk::EntityID> wire_to_internal;
    // internal edge id (line/arc/circle) -> wire entity id (for region reporting).
    std::unordered_map<sk::EntityID, std::string> internal_edge_to_wire;
    // point handle (e.g. "l1.p0", "p3", "c1.center") -> internal point id.
    std::unordered_map<std::string, sk::EntityID> handle_to_point;
    // internal point id -> its primary (canonical) handle (for positions keys).
    std::unordered_map<sk::EntityID, std::string> point_to_handle;
    // internal constraint id -> wire constraint id (for reporting conflicts).
    std::unordered_map<sk::ConstraintID, std::string> internal_constraint_to_wire;

    // Resolve a point handle (with optional role token) to an internal point id.
    // Returns empty string when unresolved.
    sk::EntityID resolve_point(const std::string& entity_id, const std::string& role) const;
    // Primary handle for an internal point (empty if unknown).
    std::string handle_for(const sk::EntityID& internal_point) const;
};

struct TranslateResult {
    std::unique_ptr<sk::Sketch> sketch;
    WireIndex index;
    bool ok = false;
    std::string error;  // set when ok == false
};

// Translate a wire sketch (`args` = {plane, entities[], constraints[]}) into a
// live `Sketch`. On failure `ok` is false and `error` explains why.
TranslateResult translate(const nlohmann::json& args);

// Overwrite the coordinate fields of a stored wire `args` in place from the
// solved positions of `sketch` (used after EndGesture to keep the pre-session
// store consistent). Points, line endpoints, and circle/arc/ellipse centers +
// radii are written back; an arc echoes whichever endpoint form it arrived in
// (coord-form `start`/`end` recomputed from the angles, angle-form
// `startAngle`/`endAngle` echoed directly), and an ellipse echoes the NORMALIZED
// majorR/minorR/rotation `addEllipse` settled on.
// Silently skips anything it cannot map.
void apply_solved_positions(nlohmann::json& args, const sk::Sketch& sketch,
                            const WireIndex& index);

}  // namespace onecad::wire
