// SketchHostReseat.h — PLANE RE-SEATING for a sketch attached to a solid face
// (UX-2026-09-14 WP-1 / B1; SCHEMA §7.2 `planStep.sketchPlacement`, §7.3 `Sketch`
// `hostFace`/`frameTransportVersion`, §9 the five `sketch*` op-built reasons).
//
// The accepted derivation is `docs/design/astra/sketch-host-face-transport.md`
// (A1, 2026-09-15). Everything here implements §3 (transport + epsilons +
// planarity + seat witness), §4 (degeneracy table) and §8 (interfaces) of that
// document; the numbers are ITS numbers and must not be re-derived here.
//
// The convention is PLANE RE-SEATING, not rigid following: a resolved plane
// determines neither tangential translation nor spin about its own normal
// (A1 §7 G9 — an unchanged face and a face rotated 90° about its normal supply
// IDENTICAL plane inputs), so the policy is defined on the plane alone:
//
//     d  = (p1 − o0) · n1          oc = o0 + d·n1        (project the origin)
//     qx = x0 − (x0·n1)·n1         h  = ‖qx‖             (Gram-Schmidt the X axis)
//     yc = n̂(n1 × qx)              xc = n̂(yc × n1)      (re-orthogonalise)
//
// The last step is conditioning, not convention: the subtraction cancels as h
// approaches the guard, and dividing by h amplifies the leftover ~1e−16 into an
// orthogonality error the frame validator rejects. Cross products carry no such
// amplification, and `xc` still maximises `xc·x0`.
//
// `oc` is the nearest point of the resolved plane to `o0`, and `xc` is the unit
// tangent that maximises `xc·x0` — the old X direction carried as far as the new
// plane permits. Replacing `p1` by any other point of the same plane leaves `d`
// unchanged, so tangential movement of the kernel's plane origin cannot drag the
// sketch. Every stored (u,v) is untouched, so local distances and angles survive.
//
// THREE THINGS THIS DELIBERATELY DOES NOT DO:
//   * No transport deadband. A 0.0005 mm plane displacement moves the sketch
//     0.0005 mm (A1 §3b: a positive deadband would leave the sketch behind).
//     `FrameChange::significant` is DIAGNOSTIC EVIDENCE ONLY.
//   * No automatic Gram-Schmidt fallback when `h ≤ τGS`. The fallback is
//     discontinuous at the singularity (A1 §7: projected X tends to −Z from one
//     side and +Z from the other), so publishing it would silently spin the
//     sketch. It is computed as REPAIR EVIDENCE and the frozen frame stands.
//   * No re-signing of a reversed normal toward the frozen one. A reversed host
//     normal is FOLLOWED and reported (`SKETCH_HOST_NORMAL_REVERSED`).
#ifndef ONECAD_SESSION_SKETCHHOSTRESEAT_H
#define ONECAD_SESSION_SKETCHHOSTRESEAT_H

#include <optional>
#include <string>
#include <vector>

#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>

#include "elementmap/ElementMapPartition.h"
#include "elementmap/Ladder.h"
#include "nlohmann/json.hpp"
#include "session/BodyStore.h"

namespace onecad::session {

// ── A1 §3 numerical policy ───────────────────────────────────────────────────

// Conditioning guard on the projected X axis, DIMENSIONLESS (A1 §3a):
// `sqrt(DBL_EPSILON)`. Normalising `qx` amplifies perturbations by ~1/h, so this
// separates numerical conditioning from modelling tolerance. It is NOT a measured
// bound on OCCT normal uncertainty.
inline constexpr double kSketchGramSchmidtGuard = 1.4901161193847656e-08;

// REPORTING thresholds only (A1 §3b). Same family/value as the mate re-seat
// epsilons (`ComponentOp.cpp`'s `kMateReseatTranslationEpsilonMm` /
// `kMateReseatRotationEpsilonDeg`) and kept unscaled for the same reason: they no
// longer control geometric accuracy, only whether the movement is worth calling
// significant.
inline constexpr double kSketchReseatTranslationEpsilonMm = 1.0e-3;
inline constexpr double kSketchReseatRotationEpsilonDeg = 1.0e-2;

// An orthonormal right-handed sketch frame: the authored `F0`, or a transported
// candidate `Fc`. World mm, Z-up like every other coordinate in this worker.
struct SketchFrame {
    gp_Pnt origin{0.0, 0.0, 0.0};
    gp_Vec x_axis{1.0, 0.0, 0.0};
    gp_Vec y_axis{0.0, 1.0, 0.0};
    gp_Vec normal{0.0, 0.0, 1.0};
};

// How far the candidate frame moved from the frozen one (A1 §3b). `moved` is
// EXACT — any differing component at all — and is what gates publication;
// `significant` is the reported epsilon verdict and gates nothing.
struct FrameChange {
    bool moved = false;
    bool significant = false;
    double translation_mm = 0.0;
    double rotation_rad = 0.0;
};

// Why a transport could not be published. `Ok` ⇒ `effective` is the candidate.
//
// The two failure statuses are split by WHOSE fault it is, and the split is
// load-bearing for the repair text: `FrameInvalid` is reported ONLY for inputs
// validated BEFORE any arithmetic runs, so `sketchFrameInvalid` never accuses a
// perfectly valid stored basis of being corrupt.
enum class TransportStatus {
    Ok,
    FrameInvalid,        // the FROZEN frame (or the resolved plane) is not a finite
                         // orthonormal RH basis — checked before transport
    FrameIllConditioned  // h ≤ τGS (the frozen X axis is (nearly) along n1), or the
                         // transport's own arithmetic could not produce a valid
                         // basis from otherwise valid inputs
};

struct TransportResult {
    TransportStatus status = TransportStatus::Ok;
    // The frame to publish. Equal to the frozen frame for every non-`Ok` status
    // (A1 §2 "Failure: retain the sketch and its local geometry, display F0").
    SketchFrame effective;
    FrameChange change;
    // `n0·n1 < 0` — the host came back with a reversed outward normal. Followed,
    // never re-signed, and reported as `SKETCH_HOST_NORMAL_REVERSED`.
    bool normal_reversed = false;
    // Populated ONLY for `FrameIllConditioned`: the Gram-Schmidt-against-Y frame
    // a repair could offer. Never published automatically (A1 §3a/§7).
    std::optional<SketchFrame> ill_conditioned_fallback;
};

// True iff `f` is a finite, unit, mutually orthogonal, RIGHT-HANDED basis
// (A1 §4 "Invalid frame"). Nothing is silently corrected.
bool sketch_frame_valid(const SketchFrame& f);

// A1 §3a. `plane_point` is any point of the resolved plane and `plane_normal` its
// ORIENTATION-CORRECTED normal (see `sketch_host_plane`). Pure and idempotent:
// transporting an already-transported frame onto the same plane is a fixed point.
TransportResult transport_sketch_frame(const SketchFrame& frozen, const gp_Pnt& plane_point,
                                       const gp_Vec& plane_normal);

// A1 §3b: the frame-to-frame delta of two frames, for reporting.
FrameChange frame_change(const SketchFrame& frozen, const SketchFrame& candidate);

// Where the sketch's attachment witness lands on the resolved face (A1 §3d).
enum class SeatState {
    OnFace,       // IN or ON — accept
    OffFace,      // OUT — `sketchSeatOffFace`
    Unmeasurable  // UNKNOWN / exception / no usable anchor — `sketchSeatUnmeasurable`
};

// A1 §3d. The witness is the FROZEN ANCHOR expressed in the frozen frame's own
// (u,v) and re-planted in the candidate frame — NOT the plane origin, which an
// unchanged annular face can legitimately place inside its central hole.
// Classified at the face's OWN `BRep_Tool::Tolerance`.
SeatState classify_sketch_seat(const TopoDS_Face& face, const SketchFrame& frozen,
                               const SketchFrame& candidate, const gp_Pnt& frozen_anchor);

// ── The Sketch step's host resolution ────────────────────────────────────────

// What the `Sketch` branch of `PlanExecutor::run_single_op` does with a
// `params.hostFace`. Exactly one of `sketch_placement` / `needs_repair` is
// populated on a resolved host that moved / a host that could not be re-seated;
// a resolved host that did NOT move populates neither (the authored frame is
// already correct, and SCHEMA §7.2 keeps that step's wire byte-identical).
struct SketchHostReseatResult {
    // False when `params` carried no `hostFace` at all — the caller then keeps the
    // pre-WP-1 raw-params behaviour byte for byte.
    bool attempted = false;
    // True iff the host resolved AND every gate passed. `effective` is then the
    // frame to materialise into `params.plane`.
    bool resolved = false;
    SketchFrame effective;
    FrameChange change;
    std::optional<nlohmann::json> sketch_placement;  // SCHEMA §7.2, iff `change.moved`
    std::vector<nlohmann::json> needs_repair;        // SCHEMA §9 op-built items
    std::vector<nlohmann::json> diagnostics;         // SCHEMA §7.2 info diagnostics
    // SCHEMA §7.3 `frameTransportVersion` names a value this build does not
    // implement. The caller refuses the step BY NAME (`UNSUPPORTED`, §13) rather
    // than re-seating under a different rule — moving the sketch somewhere the
    // producer never asked for is the one outcome the version field exists to
    // prevent. Nothing else is populated when this is set.
    std::optional<std::string> unsupported_transport;
};

// The `frameTransportVersion` this build implements (SCHEMA §7.3). Absent on the
// wire means 1.
inline constexpr int kSketchFrameTransportVersion = 1;

// Resolve `params.hostFace` against the scratch at THIS step and transport the
// authored frame onto it.
//
// Resolution is the mate's two-rung order (`ComponentOp.cpp::resolve_mate_reseat`):
// the tracked rung (`topokey_for_element_in_body` → `shape_for_topokey`) first,
// then `resolve_descriptor_stage`. The host gate is STRICTER than the ladder's own
// (A1 §1): an `AutoBind` is accepted only at `score ≥ kAutoBindMinScore` AND
// `margin ≥ kAutoBindMinMargin`, so the anchor-decided binds `Ladder.cpp` admits
// below margin are refused here as `ambiguous`.
//
// `hostFace` is deliberately NOT a wire `inputs[]` entry (the `mate.target` rule),
// so this never runs through `resolve_input_refs`: an unresolved host must halt
// THIS step, not zero the plan's bodies.
SketchHostReseatResult reseat_sketch_on_host(const BodyStore& bodies,
                                             const elementmap::ElementMapPartition& partition,
                                             const nlohmann::json& params,
                                             const std::string& op_id,
                                             const elementmap::LadderEditContext& edit);

// The `params.plane` object for `frame`, in the §7.3 `custom` form. Local (u,v)
// entity coordinates are never touched — only the basis they are read in.
nlohmann::json sketch_plane_params(const SketchFrame& frame);

// Read the §7.3 `plane` object back into a frame. Named kinds resolve to their
// NORMATIVE non-standard bases (SCHEMA §7.3 "Hard invariant — non-standard XY
// basis"), matching `WireSketch::parse_plane`.
SketchFrame sketch_frame_from_params(const nlohmann::json& params);

}  // namespace onecad::session

#endif  // ONECAD_SESSION_SKETCHHOSTRESEAT_H
