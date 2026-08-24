// SolverLane.h — SCHEMA §7.4 sketch-solver-lane verbs (W-WP3b; W-WP4 rewire).
//
// Holds the live drag gestures (lane-local, unlocked) and a REFERENCE to the
// session-owned, mutex-guarded `SketchStore` (W-WP4: sketch state is
// session-owned — see Session.h / SketchStore.h). All five verbs (SketchUpsert /
// BeginGesture / SolveDrag / EndGesture / SketchRegions) run on the Dispatcher's
// SOLVER lane; the store's own mutex makes the committed-sketch handoff to the
// kernel lane (ExecutePlan regen reads) safe, while the warm PlaneGCS systems +
// gestures stay lane-local here (never crossing lanes).
//
// Gesture model: BeginGesture translates a fresh working `Sketch` from the
// stored wire, builds + diagnoses the GCS system ONCE (warm start held for the
// gesture), then each SolveDrag re-solves warm via the ported
// `Sketch::solveWithDrag` (which rebuilds the solver only when dirty — it never
// is mid-gesture). EndGesture does the final exact solve, writes the committed
// positions back into the store, and drops the gesture.
//
// SP-2: a gesture also carries WHAT the pointer grabbed (SCHEMA §7.4
// `drag.kind`). `point` is the pre-SP-2 behaviour and what an absent kind means;
// `arcEnd`, `radius` and `entityBody` each need their OWN pin set (there is no
// single set that is right for all four — see `run_step`), which is why they go
// through `Sketch::solveWithTargets` instead of `solveWithDrag`. Every kind
// reports the additive `curves` channel, because `positions` is point-only and a
// radius/angle change moves no point at all.
#pragma once

#include <cstdint>
#include <memory>
#include <optional>
#include <string>
#include <unordered_map>
#include <utility>
#include <vector>

#include "nlohmann/json.hpp"

#include "protocol/Dispatcher.h"
#include "protocol/Envelope.h"
#include "session/SketchStore.h"
#include "sketch/Sketch.h"
#include "sketch/WireSketch.h"

namespace onecad::protocol {

// SCHEMA §7.4 gesture target kinds. `Point` is what an absent `drag.kind`
// means; an UNKNOWN kind is a hard OP_FAILED and never degrades to `Point`
// (degrading would move a handle the user never grabbed).
enum class DragKind { Point, ArcEnd, Radius, EntityBody };

// One curve's reportable parameters (the §7.4 `curves` channel payload). A
// Circle carries a radius only (`has_angles` false); an Arc carries both angles
// too. An Ellipse is never collected — it is not registered with PlaneGCS, so no
// drag can move its parameters (§7.4 deviation).
struct CurveParams {
    double radius = 0.0;
    double start_angle = 0.0;
    double end_angle = 0.0;
    bool has_angles = false;
};

// Curve parameters by INTERNAL entity id (keyed to wire ids only when reported).
using CurveMap = std::unordered_map<core::sketch::EntityID, CurveParams>;

class SolverLane {
public:
    // The store is owned by the Session (shared across lanes); the lane keeps a
    // reference. It must outlive the SolverLane (main owns both).
    explicit SolverLane(session::SketchStore& store) : store_(store) {}

    // Register all five §7.4 verbs on the dispatcher's solver lane.
    void register_verbs(Dispatcher& dispatcher);

private:
    // Point position by internal id (x,y).
    using PosMap = std::unordered_map<core::sketch::EntityID, std::pair<double, double>>;

    struct Gesture {
        std::uint64_t id = 0;
        std::string sketch_id;
        std::uint64_t sketch_revision = 0;
        std::unique_ptr<core::sketch::Sketch> sketch;
        wire::WireIndex index;
        core::sketch::EntityID drag_point;
        int dof = 0;
        bool redundant = false;  // benign DOF-preserving redundancy (diagnosed at begin)
        std::vector<std::string> conflicting;  // wire constraint ids (structural)
        // SCHEMA §7.4 `entityStates`, keyed by WIRE entity id — GESTURE-FIXED
        // for the same reason `dof`/`redundant`/`conflicting` are: a drag
        // changes no constraint, and every drag step ends by invalidating the
        // PlaneGCS diagnosis this map is derived from. `std::nullopt` means the
        // sketch had nothing to report (the §7.4 ellipse rule).
        std::optional<nlohmann::json> entity_states;
        PosMap baseline;       // positions at BeginGesture
        PosMap last_reported;  // for incremental deltas
        bool last_success = true;

        // --- SP-2 §7.4 target kind -------------------------------------------
        DragKind kind = DragKind::Point;
        // The grabbed ENTITY: the arc (ArcEnd), the curve whose radius is driven
        // (Radius), or the body being translated (EntityBody). Empty for Point.
        core::sketch::EntityID drag_entity;
        // EntityBody: every point the entity owns, in `entityPointIds` handle
        // order, plus their BeginGesture pose. Targets are always derived from
        // this baseline (never incrementally), so a dropped frame costs nothing.
        std::vector<core::sketch::EntityID> body_points;
        PosMap body_baseline;
        // Pointer-down position, and the per-kind offset derived from it ONCE at
        // BeginGesture (§7.4: deriving it per step would let the gesture drift).
        std::pair<double, double> grab{0.0, 0.0};
        bool has_grab = false;
        double radius_offset = 0.0;  // |grab − center| − radius, both at Begin
        CurveMap baseline_curves;       // curves at BeginGesture (EndGesture delta)
        CurveMap last_reported_curves;  // for incremental SolveDrag deltas
    };

    // One drag step toward (tx, ty) for this gesture's kind. Shared by SolveDrag
    // and EndGesture's `commit.finalTarget` branch so the two can never diverge.
    core::sketch::SolveResult run_step(Gesture& g, double tx, double ty);

    Envelope on_upsert(const Envelope& req);
    Envelope on_begin(const Envelope& req);
    Envelope on_drag(const Envelope& req);
    Envelope on_end(const Envelope& req);
    Envelope on_regions(const Envelope& req);

    session::SketchStore& store_;  // session-owned, self-locked (see Session.h)
    std::unordered_map<std::uint64_t, Gesture> gestures_;
};

}  // namespace onecad::protocol
