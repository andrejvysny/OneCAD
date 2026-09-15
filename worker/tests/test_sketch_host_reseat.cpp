// test_sketch_host_reseat.cpp — the NUMBERS behind UX-2026-09-14 WP-1 (B1).
//
// Every vector here is taken verbatim from the accepted derivation
// `docs/design/astra/sketch-host-face-transport.md` §6 (A1, 2026-09-15). The WIRE
// CONTRACT half — a Sketch step echoing `planStep.sketchPlacement`, halting by
// name when the host vanishes, and a Cut refused as `CUT_NO_EFFECT` — lives in
// `protocol/fixtures/sketch_host_face_reseat.ndjson` (the house split: the
// contract in the fixture, the numbers here).
//
// Covered:
//   A. Frame transport — G1, G2, G4, G8, G9, the unchanged-normal verbatim rule,
//      the τGS conditioning boundary, the exact-parallel fallback, an invalid
//      (left-handed) stored basis, and the significance thresholds.
//   B. Seat classification — an annulus whose plane ORIGIN is OUT while its
//      anchor is IN (accept), and a face that shrank past its anchor (refuse).
//   C. Cut effect — A1 §6 G10 / G11 / G12 plus the representation floor (G13/G14),
//      with εV computed from the measurement error `BRepGProp::VolumeProperties`
//      actually achieved.
//   D. Forward compatibility — `frameTransportVersion`, refused by name at the
//      value's own width.
//   E. Failure policy — EVERY unresolvable host halts with the ladder's own §9
//      item, and the seat witness is the transported anchor with no exemptions.
//
// No framework: exit code == failure count.
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <limits>
#include <optional>
#include <string>
#include <utility>
#include <vector>

#include <BRepAlgoAPI_Cut.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <TopExp_Explorer.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include "session/BodyStore.h"
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <TopoDS_Wire.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Circ.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include "kernel/topology/CoplanarFacePatch.h"
#include "modeling/BooleanMode.h"
#include "nlohmann/json.hpp"
#include "ops/OpCommon.h"
#include "session/SketchHostReseat.h"

using nlohmann::json;
using onecad::session::classify_sketch_seat;
using onecad::session::FrameChange;
using onecad::session::frame_change;
using onecad::session::kSketchGramSchmidtGuard;
using onecad::session::SeatState;
using onecad::session::SketchFrame;
using onecad::session::sketch_frame_valid;
using onecad::session::transport_sketch_frame;
using onecad::session::TransportStatus;

namespace {

int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}

// A1 §6 pins the frames to 1e-9 — far above IEEE noise on these magnitudes and
// far below anything a wrong branch could produce.
constexpr double kEps = 1.0e-9;
constexpr double kPi = 3.14159265358979323846;

void check_pnt(const gp_Pnt& got, double x, double y, double z, const std::string& tag) {
    const bool ok = std::abs(got.X() - x) < kEps && std::abs(got.Y() - y) < kEps &&
                    std::abs(got.Z() - z) < kEps;
    if (!ok) {
        std::fprintf(stderr, "FAIL: %s: want (%.12f,%.12f,%.12f) got (%.12f,%.12f,%.12f)\n",
                     tag.c_str(), x, y, z, got.X(), got.Y(), got.Z());
        ++g_failures;
    }
}

void check_vec(const gp_Vec& got, double x, double y, double z, const std::string& tag) {
    const bool ok = std::abs(got.X() - x) < kEps && std::abs(got.Y() - y) < kEps &&
                    std::abs(got.Z() - z) < kEps;
    if (!ok) {
        std::fprintf(stderr, "FAIL: %s: want (%.12f,%.12f,%.12f) got (%.12f,%.12f,%.12f)\n",
                     tag.c_str(), x, y, z, got.X(), got.Y(), got.Z());
        ++g_failures;
    }
}

// The NORMATIVE non-standard XY basis (SCHEMA §7.3), at height `z`.
SketchFrame top_frame(double z) {
    SketchFrame f;
    f.origin = gp_Pnt(0.0, 0.0, z);
    f.x_axis = gp_Vec(0.0, 1.0, 0.0);
    f.y_axis = gp_Vec(-1.0, 0.0, 0.0);
    f.normal = gp_Vec(0.0, 0.0, 1.0);
    return f;
}

// ── A. Frame transport (A1 §3a / §6) ────────────────────────────────────────

void test_g1_g2_pure_normal_translation() {
    // G1: the host cap is re-planned from z=40 down to z=25. n1 == n0 exactly, so
    // the AUTHORED basis is carried verbatim and only the origin moves: d = −15.
    const SketchFrame frozen = top_frame(40.0);
    const auto g1 = transport_sketch_frame(frozen, gp_Pnt(7.0, -3.0, 25.0), gp_Vec(0, 0, 1));
    check(g1.status == TransportStatus::Ok, "G1: transports");
    check_pnt(g1.effective.origin, 0.0, 0.0, 25.0, "G1 origin");
    check_vec(g1.effective.x_axis, 0.0, 1.0, 0.0, "G1 xAxis");
    check_vec(g1.effective.y_axis, -1.0, 0.0, 0.0, "G1 yAxis");
    check_vec(g1.effective.normal, 0.0, 0.0, 1.0, "G1 normal");
    check(g1.change.moved, "G1: moved");
    check(std::abs(g1.change.translation_mm - 15.0) < kEps, "G1: translation is 15 mm");
    check(std::abs(g1.change.rotation_rad) < kEps, "G1: no rotation");
    check(!g1.normal_reversed, "G1: normal not reversed");
    // The plane POINT is deliberately off-origin: replacing p1 by any other point
    // of the same plane leaves d unchanged, so tangential drift of the kernel's
    // plane origin cannot drag the sketch (A1 §3a).

    // G2: the same F0 against a plane at z=60 ⇒ d = +20.
    const auto g2 = transport_sketch_frame(frozen, gp_Pnt(-50.0, 12.0, 60.0), gp_Vec(0, 0, 1));
    check(g2.status == TransportStatus::Ok, "G2: transports");
    check_pnt(g2.effective.origin, 0.0, 0.0, 60.0, "G2 origin");
    check_vec(g2.effective.x_axis, 0.0, 1.0, 0.0, "G2 xAxis");
    check(std::abs(g2.change.translation_mm - 20.0) < kEps, "G2: translation is 20 mm");
}

void test_g4_tilt() {
    // A1 §6 G4: o0=(100,30,20), x0=Y, y0=Z, n0=X; p1=(100,0,0),
    // n1=(cos10°,0,−sin10°). d = 20·sin10° = 3.472963553339 mm.
    SketchFrame frozen;
    frozen.origin = gp_Pnt(100.0, 30.0, 20.0);
    frozen.x_axis = gp_Vec(0.0, 1.0, 0.0);
    frozen.y_axis = gp_Vec(0.0, 0.0, 1.0);
    frozen.normal = gp_Vec(1.0, 0.0, 0.0);
    check(sketch_frame_valid(frozen), "G4: the stored frame is a valid RH basis");

    const double c = std::cos(10.0 * kPi / 180.0);
    const double s = std::sin(10.0 * kPi / 180.0);
    const auto r = transport_sketch_frame(frozen, gp_Pnt(100.0, 0.0, 0.0), gp_Vec(c, 0.0, -s));
    check(r.status == TransportStatus::Ok, "G4: transports");
    check_pnt(r.effective.origin, 103.420201433257, 30.0, 19.396926207859, "G4 origin");
    check_vec(r.effective.x_axis, 0.0, 1.0, 0.0, "G4 xAxis");
    check_vec(r.effective.y_axis, 0.173648177667, 0.0, 0.984807753012, "G4 yAxis");
    check_vec(r.effective.normal, 0.984807753012, 0.0, -0.173648177667, "G4 normal");
    check(!r.normal_reversed, "G4: a 10° tilt is not a reversal");
    // The displacement is d along n1: |d| = 20·sin10°.
    check(std::abs(r.change.translation_mm - 20.0 * s) < kEps,
          "G4: translation is 20·sin10° = 3.472963553 mm");
    check(std::abs(r.change.rotation_rad - 10.0 * kPi / 180.0) < kEps,
          "G4: the whole-frame rotation is exactly the 10° tilt");
    check(r.change.significant, "G4: a 10° tilt is significant");
}

void test_g8_reversed_normal() {
    // A1 §6 G8: the SAME plane, normal reversed. The resolved outward normal is
    // FOLLOWED (never re-signed toward n0) and the reversal is reported.
    const SketchFrame frozen = top_frame(40.0);
    const auto r = transport_sketch_frame(frozen, gp_Pnt(0.0, 0.0, 40.0), gp_Vec(0, 0, -1));
    check(r.status == TransportStatus::Ok, "G8: transports");
    check_pnt(r.effective.origin, 0.0, 0.0, 40.0, "G8 origin");
    check_vec(r.effective.x_axis, 0.0, 1.0, 0.0, "G8 xAxis");
    check_vec(r.effective.y_axis, 1.0, 0.0, 0.0, "G8 yAxis");
    check_vec(r.effective.normal, 0.0, 0.0, -1.0, "G8 normal");
    check(r.normal_reversed, "G8: the reversal is reported");
    check(r.change.moved, "G8: the frame moved (the basis flipped) even at zero translation");
    check(std::abs(r.change.translation_mm) < kEps, "G8: no translation");
    check(std::abs(r.change.rotation_rad - kPi) < kEps, "G8: a 180° whole-frame rotation");
}

void test_g9_spin_is_undetectable() {
    // A1 §6 G9 / §7: a face rotated 90° about its own normal supplies IDENTICAL
    // plane inputs, so plane re-seating is a fixed point there BY DEFINITION. This
    // is the policy's stated limit, pinned so nobody "fixes" it into a silent spin.
    const SketchFrame frozen = top_frame(40.0);
    const auto r = transport_sketch_frame(frozen, gp_Pnt(0.0, 0.0, 40.0), gp_Vec(0, 0, 1));
    check(r.status == TransportStatus::Ok, "G9: transports");
    check(!r.change.moved, "G9: F1 == F0 — a spin about the normal cannot be seen");
    check_vec(r.effective.x_axis, 0.0, 1.0, 0.0, "G9 xAxis unchanged");

    // IDEMPOTENCE (plan amendment A-9): transporting the result again is a fixed
    // point, which is what lets the worker recompute from the AUTHORED frame on
    // every regen without drifting.
    const auto again = transport_sketch_frame(r.effective, gp_Pnt(0.0, 0.0, 40.0), gp_Vec(0, 0, 1));
    check(!again.change.moved, "idempotence: re-transporting the effective frame moves nothing");
}

void test_legacy_basis_is_preserved() {
    // A1 §4 "Legacy basis": a stored basis that differs from today's seed
    // convention but is still orthonormal and right-handed is carried VERBATIM,
    // not rebuilt from the world axes.
    SketchFrame frozen;
    frozen.origin = gp_Pnt(0.0, 0.0, 0.0);
    frozen.x_axis = gp_Vec(1.0, 0.0, 0.0);
    frozen.y_axis = gp_Vec(0.0, 1.0, 0.0);
    frozen.normal = gp_Vec(0.0, 0.0, 1.0);
    const auto r = transport_sketch_frame(frozen, gp_Pnt(0.0, 0.0, 1.0), gp_Vec(0, 0, 1));
    check(r.status == TransportStatus::Ok, "legacy basis: transports");
    check_pnt(r.effective.origin, 0.0, 0.0, 1.0, "legacy basis origin +1 mm");
    check_vec(r.effective.x_axis, 1.0, 0.0, 0.0, "legacy basis xAxis preserved exactly");
    check_vec(r.effective.y_axis, 0.0, 1.0, 0.0, "legacy basis yAxis preserved exactly");
}

void test_conditioning_boundary() {
    // A1 §6 "Conditioning boundary": n1 = (0, √(1−b²), b) against the top basis
    // gives h = |b| exactly. b = τGS/2 repairs; b = 2τGS transports.
    const SketchFrame frozen = top_frame(0.0);
    const auto at = [&](double b) {
        const gp_Vec n1(0.0, std::sqrt(1.0 - b * b), b);
        return transport_sketch_frame(frozen, gp_Pnt(0.0, 0.0, 0.0), n1);
    };
    const auto half = at(kSketchGramSchmidtGuard / 2.0);
    check(half.status == TransportStatus::FrameIllConditioned,
          "τGS boundary: b = τGS/2 is ill-conditioned");
    check(half.effective.origin.IsEqual(frozen.origin, 0.0) &&
              half.effective.x_axis.IsEqual(frozen.x_axis, kEps, kEps),
          "τGS boundary: the FROZEN frame stands on a repair");
    check(half.ill_conditioned_fallback.has_value(),
          "τGS boundary: the Gram-Schmidt fallback is computed as repair EVIDENCE");

    const auto twice = at(2.0 * kSketchGramSchmidtGuard);
    check(twice.status == TransportStatus::Ok, "τGS boundary: b = 2·τGS uses generic transport");

    // Exact parallel (h = 0): the fallback is x = −Z, y = −X, n = Y, and it is
    // EVIDENCE only — the published frame stays F0 (A1 §3a/§7: one fallback cannot
    // equal both one-sided limits, so auto-publishing it would silently spin).
    const auto parallel =
        transport_sketch_frame(frozen, gp_Pnt(0.0, 0.0, 0.0), gp_Vec(0.0, 1.0, 0.0));
    check(parallel.status == TransportStatus::FrameIllConditioned, "exact parallel: repairs");
    check(parallel.ill_conditioned_fallback.has_value(), "exact parallel: fallback computed");
    if (parallel.ill_conditioned_fallback) {
        check_vec(parallel.ill_conditioned_fallback->x_axis, 0.0, 0.0, -1.0, "fallback xAxis = −Z");
        check_vec(parallel.ill_conditioned_fallback->y_axis, -1.0, 0.0, 0.0, "fallback yAxis = −X");
        check_vec(parallel.ill_conditioned_fallback->normal, 0.0, 1.0, 0.0, "fallback normal = Y");
    }
    check_vec(parallel.effective.x_axis, 0.0, 1.0, 0.0, "exact parallel: F0 still published");
}

void test_conditioning_just_above_the_guard_stays_orthonormal() {
    // Astra break on WP-1. Just ABOVE τGS the Gram-Schmidt subtraction cancels
    // catastrophically: for n1 = (0, √(1−b²), b) against x0 = Y the surviving
    // component of `qx` carries ~1e−16 of absolute cancellation error, and dividing
    // by h ≈ b amplifies it — at b = 2e−8 to ~2e−9, past the validator's 1e−9
    // orthogonality bound. Before the re-orthogonalisation that reported
    // `sketchFrameInvalid`: the transport's own arithmetic blamed on a stored frame
    // that is exactly orthonormal. Every b here must transport CLEANLY.
    const SketchFrame frozen = top_frame(0.0);
    check(sketch_frame_valid(frozen), "the frozen frame under test is itself valid");
    // Every b here is ABOVE τGS = 1.4901161193847656e−08 (b = 1e−8 is below it and
    // is the ill-conditioned verdict `test_conditioning_boundary` already pins).
    for (const double b : {2.0e-8, 5.0e-8, 1.0e-7, 1.0e-6}) {
        const gp_Vec n1(0.0, std::sqrt(1.0 - b * b), b);
        const auto r = transport_sketch_frame(frozen, gp_Pnt(0.0, 0.0, 0.0), n1);
        char buf[32];
        std::snprintf(buf, sizeof(buf), "%.3g", b);
        const std::string at = std::string(" at b = ") + buf;
        check(r.status == TransportStatus::Ok, "near-guard transport succeeds" + at);
        check(sketch_frame_valid(r.effective), "near-guard result is a valid basis" + at);
        // The residuals the validator would have tripped on, pinned two decades
        // tighter than the 1e−9 it allows.
        const double ortho = std::max({std::abs(r.effective.x_axis.Dot(r.effective.y_axis)),
                                       std::abs(r.effective.x_axis.Dot(r.effective.normal)),
                                       std::abs(r.effective.y_axis.Dot(r.effective.normal))});
        const double unit = std::max({std::abs(r.effective.x_axis.Magnitude() - 1.0),
                                      std::abs(r.effective.y_axis.Magnitude() - 1.0),
                                      std::abs(r.effective.normal.Magnitude() - 1.0)});
        if (ortho > 1.0e-12 || unit > 1.0e-12) {
            std::fprintf(stderr, "FAIL: near-guard residuals at b=%.3g: ortho=%.3g unit=%.3g\n", b,
                         ortho, unit);
            ++g_failures;
        }
        // The CONVENTION is unchanged: `xc` is the unit tangent of the new plane
        // maximising `xc·x0`, so `xc·x0 = h = b` exactly here — vanishingly small
        // precisely BECAUSE the frozen X lies almost along n1. What must hold is the
        // SIGN: the re-orthogonalisation must not flip the axis a repair would then
        // be blamed for. (`|xc·x0 − b|` is pinned at 1e−12 relative to b.)
        const double along = r.effective.x_axis.Dot(frozen.x_axis);
        if (!(along > 0.0) || std::abs(along - b) > 1.0e-12 * b) {
            std::fprintf(stderr,
                         "FAIL: near-guard xc·x0 at b=%.3g: got %.17g, want %.17g (h)\n", b,
                         along, b);
            ++g_failures;
        }
    }
}

void test_invalid_handedness() {
    // A1 §6 "Invalid handedness": x=Y, y=X, n=Z ⇒ (x×y)·n = −1. A left-handed
    // stored basis is a repair, never silently "corrected" — correcting it would
    // mirror every stored (u,v).
    SketchFrame frozen;
    frozen.origin = gp_Pnt(0.0, 0.0, 0.0);
    frozen.x_axis = gp_Vec(0.0, 1.0, 0.0);
    frozen.y_axis = gp_Vec(1.0, 0.0, 0.0);
    frozen.normal = gp_Vec(0.0, 0.0, 1.0);
    check(!sketch_frame_valid(frozen), "invalid handedness: rejected by the validity predicate");
    const auto r = transport_sketch_frame(frozen, gp_Pnt(0.0, 0.0, 5.0), gp_Vec(0, 0, 1));
    check(r.status == TransportStatus::FrameInvalid, "invalid handedness: sketchFrameInvalid");
    check_pnt(r.effective.origin, 0.0, 0.0, 0.0, "invalid handedness: F0 stands");

    // Non-finite and non-unit are the same class.
    SketchFrame nonunit = top_frame(0.0);
    nonunit.x_axis = gp_Vec(0.0, 2.0, 0.0);
    check(!sketch_frame_valid(nonunit), "a non-unit axis is invalid");
}

void test_significance_thresholds() {
    // A1 §6: 0.000999 / 0.001 / 0.001001 mm ⇒ significance false / false / true
    // (the threshold is a STRICT `>`), and all three TRANSPORT regardless —
    // `significant` is diagnostic evidence, never a deadband.
    const SketchFrame frozen = top_frame(0.0);
    for (const auto& [d, want] : std::vector<std::pair<double, bool>>{
             {0.000999, false}, {0.001, false}, {0.001001, true}}) {
        const auto r = transport_sketch_frame(frozen, gp_Pnt(0.0, 0.0, d), gp_Vec(0, 0, 1));
        check(r.status == TransportStatus::Ok, "translation significance: transports");
        check(r.change.moved, "translation significance: a sub-epsilon shift STILL moves");
        if (r.change.significant != want) {
            std::fprintf(stderr,
                         "FAIL: translation significance at %.9f mm: want %d got %d "
                         "(measured %.17g)\n",
                         d, want ? 1 : 0, r.change.significant ? 1 : 0, r.change.translation_mm);
            ++g_failures;
        }
    }

    // 0.009999° / 0.01° / 0.010001° ⇒ false / false / true, measured on the WHOLE
    // frame (A1 §3b), so a tilt of the basis is seen even at zero translation.
    for (const auto& [deg, want] : std::vector<std::pair<double, bool>>{
             {0.009999, false}, {0.01, false}, {0.010001, true}}) {
        gp_Trsf rot;
        rot.SetRotation(gp_Ax1(frozen.origin, gp_Dir(frozen.x_axis)), deg * kPi / 180.0);
        SketchFrame tilted;
        tilted.origin = frozen.origin;
        tilted.x_axis = frozen.x_axis.Transformed(rot);
        tilted.y_axis = frozen.y_axis.Transformed(rot);
        tilted.normal = frozen.normal.Transformed(rot);
        const FrameChange change = frame_change(frozen, tilted);
        if (change.significant != want) {
            std::fprintf(stderr,
                         "FAIL: rotation significance at %.9f deg: want %d got %d "
                         "(measured %.17g rad = %.17g deg)\n",
                         deg, want ? 1 : 0, change.significant ? 1 : 0, change.rotation_rad,
                         change.rotation_rad * 180.0 / kPi);
            ++g_failures;
        }
        check(change.moved, "rotation significance: a sub-epsilon tilt STILL moves");
    }
}

// ── B. Seat classification (A1 §3d / §6) ────────────────────────────────────

TopoDS_Face annulus_face(double r_inner, double r_outer) {
    const gp_Ax2 ax(gp_Pnt(0.0, 0.0, 0.0), gp_Dir(0.0, 0.0, 1.0));
    BRepBuilderAPI_MakeWire outer(BRepBuilderAPI_MakeEdge(gp_Circ(ax, r_outer)).Edge());
    BRepBuilderAPI_MakeFace face(gp_Pln(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), outer.Wire());
    BRepBuilderAPI_MakeWire inner(BRepBuilderAPI_MakeEdge(gp_Circ(ax, r_inner)).Edge());
    face.Add(TopoDS::Wire(inner.Wire().Reversed()));
    return face.Face();
}

TopoDS_Face square_face(double half) {
    BRepBuilderAPI_MakePolygon poly(gp_Pnt(-half, -half, 0.0), gp_Pnt(half, -half, 0.0),
                                    gp_Pnt(half, half, 0.0), gp_Pnt(-half, half, 0.0), true);
    return BRepBuilderAPI_MakeFace(poly.Wire()).Face();
}

void test_seat_annulus_origin_out_anchor_in() {
    // A1 §6 "Origin outside": an annulus r ∈ [5,10] whose PLANE ORIGIN sits in the
    // central hole, with the attachment anchor at r = 7. Classifying the ORIGIN
    // would refuse a perfectly healthy sketch; classifying the transported ANCHOR
    // accepts it. That is precisely why the witness is the anchor.
    const TopoDS_Face face = annulus_face(5.0, 10.0);
    SketchFrame frozen = top_frame(0.0);
    const gp_Pnt anchor(0.0, 7.0, 0.0);  // r = 7, inside the material ring
    check(classify_sketch_seat(face, frozen, frozen, anchor) == SeatState::OnFace,
          "annulus: the anchor at r=7 seats, even though the plane origin is in the hole");
    check(classify_sketch_seat(face, frozen, frozen, frozen.origin) == SeatState::OffFace,
          "annulus: the ORIGIN itself classifies OUT — the case the witness rule exists for");
    // …and this is exactly why SCHEMA §7.6 `exact.anchor` exists. `add_sketch_on_face`
    // freezes a point the KERNEL classified on the face, so on an annulus it is a
    // point of the material ring, never the hole the plane origin falls into. The
    // witness rule needs no exemption once the anchor is a real point of the face;
    // `test_the_seat_witness_decides_a_resolved_host` pins both verdicts end to end.
}

void test_seat_off_face_when_the_boundary_shrinks() {
    // A1 §6 "Seat outside": the boundary shrinks from [-3,3]² to [-1,1]² while the
    // anchor sits at (2,0) — 1 mm outside the survivor.
    SketchFrame frozen = top_frame(0.0);
    frozen.x_axis = gp_Vec(1.0, 0.0, 0.0);
    frozen.y_axis = gp_Vec(0.0, 1.0, 0.0);
    const gp_Pnt anchor(2.0, 0.0, 0.0);
    check(classify_sketch_seat(square_face(3.0), frozen, frozen, anchor) == SeatState::OnFace,
          "seat: the anchor is IN the original [-3,3]² face");
    check(classify_sketch_seat(square_face(1.0), frozen, frozen, anchor) == SeatState::OffFace,
          "seat: the same anchor is OUT of the shrunk [-1,1]² face");

    // The witness follows the CANDIDATE frame: the same face lifted to z = 5 with
    // the sketch re-seated onto it keeps the anchor's (u,v), so it seats again.
    SketchFrame candidate = frozen;
    candidate.origin = gp_Pnt(0.0, 0.0, 5.0);
    gp_Trsf up;
    up.SetTranslation(gp_Vec(0.0, 0.0, 5.0));
    const TopoDS_Face lifted =
        TopoDS::Face(BRepBuilderAPI_Transform(square_face(3.0), up, true).Shape());
    check(classify_sketch_seat(lifted, frozen, candidate, anchor) == SeatState::OnFace,
          "seat: the witness rides the candidate frame, not the frozen one");
}

// ── C. Cut effect (A1 §3e / §6 G10–G12) ─────────────────────────────────────

void report_cut(const char* tag, const TopoDS_Shape& target, const TopoDS_Shape& result) {
    const onecad::ops::VolumeMeasurement b = onecad::ops::measure_volume(target);
    const onecad::ops::VolumeMeasurement a = onecad::ops::measure_volume(result);
    const double u = b.relative_error * std::abs(b.volume_mm3) / (1.0 - b.relative_error) +
                     a.relative_error * std::abs(a.volume_mm3) / (1.0 - a.relative_error);
    const double scale = std::max(std::abs(b.volume_mm3), std::abs(a.volume_mm3));
    const double ulp = std::nextafter(scale, std::numeric_limits<double>::infinity()) - scale;
    std::fprintf(stderr,
                 "%s: Vbefore=%.9f (relErr %.3g) Vafter=%.9f (relErr %.3g) ΔV=%.9f U=%.9g "
                 "8ulp=%.9g εV=%.9g\n",
                 tag, b.volume_mm3, b.relative_error, a.volume_mm3, a.relative_error,
                 b.volume_mm3 - a.volume_mm3, u, 8.0 * ulp, 1.0e-9 + u + 8.0 * ulp);
}

std::string refusal_code(const std::optional<onecad::ops::OpOutcome>& refusal) {
    if (!refusal) return "<published>";
    for (const auto& d : refusal->diagnostics) {
        const std::string code = d.value("code", std::string());
        if (!code.empty()) return code;
    }
    return "<no code>";
}

void test_cut_effect_vectors() {
    // A1 §6 G10: a 100×60×25 box (150 000 mm³) and a tool sitting exactly ON its
    // top cap — an IMPRINT. ΔV = 0 ⇒ |ΔV| ≤ εV ⇒ refuse.
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 100.0, 60.0, 25.0).Shape();
    {
        const TopoDS_Shape tool =
            BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 25.0), 100.0, 60.0, 10.0).Shape();
        const TopoDS_Shape result = BRepAlgoAPI_Cut(box, tool).Shape();
        report_cut("G10 imprint", box, result);
        const auto refusal = onecad::ops::cut_effect_policy(onecad::app::BooleanMode::Cut, box,
                                                           result, "Extrude", "body_g10");
        check(refusal_code(refusal) == "CUT_NO_EFFECT",
              "G10: an imprint-only Cut refuses CUT_NO_EFFECT, got " + refusal_code(refusal));
        if (refusal) {
            const json ev = refusal->diagnostics.at(0)["evidence"]["boolean"];
            check(std::abs(ev.value("volumeBefore", -1.0) - 150000.0) < 1.0e-6 &&
                      std::abs(ev.value("volumeAfter", -1.0) - 150000.0) < 1.0e-6,
                  "G10: both measured volumes are 150000 mm³");
            check(ev.value("toleranceMm3", -1.0) >= 1.0e-9,
                  "G10: εV is at least the a³ semantic floor");
        }
    }

    // A1 §6 G11: the same box with a full-footprint slab of thickness t removed —
    // ΔV = 6000·t. All three thicknesses PASS, including t = 1e-4 mm (0.6 mm³),
    // five orders below the sliver a size-scaled ε would have swallowed.
    for (const double t : {1.0e-4, 1.0e-3, 1.0e-2}) {
        const TopoDS_Shape tool =
            BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 25.0 - t), 100.0, 60.0, t + 10.0).Shape();
        const TopoDS_Shape result = BRepAlgoAPI_Cut(box, tool).Shape();
        report_cut("G11 slab", box, result);
        const auto refusal = onecad::ops::cut_effect_policy(onecad::app::BooleanMode::Cut, box,
                                                           result, "Extrude", "body_g11");
        check(!refusal.has_value(),
              "G11: a " + std::to_string(t) + " mm slab (ΔV = 6000·t) passes, got " +
                  refusal_code(refusal));
        const onecad::ops::VolumeMeasurement after = onecad::ops::measure_volume(result);
        check(std::abs((150000.0 - after.volume_mm3) - 6000.0 * t) < 1.0e-6,
              "G11: the removal really is 6000·t mm³");
    }

    // A1 §6 G12: a 10×10 pocket 0.05 mm deep — ΔV = 5 mm³.
    {
        const TopoDS_Shape tool =
            BRepPrimAPI_MakeBox(gp_Pnt(10, 10, 25.0 - 0.05), 10.0, 10.0, 10.0).Shape();
        const TopoDS_Shape result = BRepAlgoAPI_Cut(box, tool).Shape();
        report_cut("G12 pocket", box, result);
        const auto refusal = onecad::ops::cut_effect_policy(onecad::app::BooleanMode::Cut, box,
                                                           result, "Extrude", "body_g12");
        check(!refusal.has_value(), "G12: a 5 mm³ pocket passes, got " + refusal_code(refusal));
    }

    // The volume-INCREASED branch: handing the policy a LARGER result is the only
    // way a Cut can report growth, and it must refuse by its own name rather than
    // fall into `CUT_NO_EFFECT`.
    {
        const TopoDS_Shape bigger =
            BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 100.0, 60.0, 30.0).Shape();
        const auto refusal = onecad::ops::cut_effect_policy(onecad::app::BooleanMode::Cut, box,
                                                           bigger, "Extrude", "body_grow");
        check(refusal_code(refusal) == "CUT_VOLUME_INCREASED",
              "a Cut that GREW the body refuses CUT_VOLUME_INCREASED, got " +
                  refusal_code(refusal));
    }

    // THE REPRESENTATION FLOOR (Astra break on WP-1). `a³ = 1e−9 mm³` is absolute,
    // but binary64 resolution is not: at V = 1e9 mm³ (a 1 m cube) consecutive
    // doubles are 2^(29−52) ≈ 1.19e−7 mm³ apart, a hundred times `a³`. An
    // imprint-only Cut CHANGES THE TOPOLOGY, so the two integrations run over
    // different face sets and can land a few ULP apart with nothing removed — and
    // `VolumeProperties` reports relative error 0 there, so `U` does not cover it.
    {
        const TopoDS_Shape metre =
            BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 1000.0, 1000.0, 1000.0).Shape();
        const TopoDS_Shape tool =
            BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 1000.0), 1000.0, 1000.0, 100.0).Shape();
        const TopoDS_Shape result = BRepAlgoAPI_Cut(metre, tool).Shape();
        report_cut("G13 metre imprint", metre, result);
        const auto refusal = onecad::ops::cut_effect_policy(onecad::app::BooleanMode::Cut, metre,
                                                            result, "Extrude", "body_g13");
        check(refusal_code(refusal) == "CUT_NO_EFFECT",
              "G13: a 1 m cube's imprint-only Cut refuses even when ΔV is a few ULP, got " +
                  refusal_code(refusal));
        if (refusal) {
            const json ev = refusal->diagnostics.at(0)["evidence"]["boolean"];
            // 8·ulp(1e9) = 8·1.1921e−7 = 9.5367e−7 mm³, a thousand times `a³`.
            check(ev.value("toleranceMm3", -1.0) > 9.0e-7,
                  "G13: εV carries the representation floor, not just a³ + U");
        }
    }
    // …and the floor stays far below a REAL removal at the same magnitude: a
    // 1000×1000×10 plate (1e7 mm³, ulp 1.86e−9) cut 1e−4 mm deeper over its whole
    // footprint removes 100 mm³ and publishes.
    {
        const TopoDS_Shape plate =
            BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 1000.0, 1000.0, 10.0).Shape();
        const TopoDS_Shape tool =
            BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 10.0 - 1.0e-4), 1000.0, 1000.0, 100.0).Shape();
        const TopoDS_Shape result = BRepAlgoAPI_Cut(plate, tool).Shape();
        report_cut("G14 plate 1e-4", plate, result);
        const auto refusal = onecad::ops::cut_effect_policy(onecad::app::BooleanMode::Cut, plate,
                                                            result, "Extrude", "body_g14");
        check(!refusal.has_value(),
              "G14: a 100 mm³ removal from a 1e7 mm³ plate passes, got " + refusal_code(refusal));
        const onecad::ops::VolumeMeasurement after = onecad::ops::measure_volume(result);
        check(std::abs((1.0e7 - after.volume_mm3) - 100.0) < 1.0e-3,
              "G14: the removal really is 100 mm³");
    }

    // Non-Cut modes are untouched: the same identical pair is published.
    {
        const auto add = onecad::ops::cut_effect_policy(onecad::app::BooleanMode::Add, box, box,
                                                        "Extrude", "body_add");
        check(!add.has_value(), "the predicate is scoped to Cut — Add is untouched");
        const auto inter = onecad::ops::cut_effect_policy(onecad::app::BooleanMode::Intersect, box,
                                                          box, "Extrude", "body_int");
        check(!inter.has_value(), "the predicate is scoped to Cut — Intersect is untouched");
    }
}

// ── D. Forward compatibility (SCHEMA §7.3 `frameTransportVersion`, §13) ─────

void test_unknown_transport_version_refuses_by_name() {
    // A producer that authored against a LATER transport rule must not have its
    // sketch re-seated under this one. The step is refused by name before any
    // resolution happens, so nothing about the host matters here.
    onecad::session::BodyStore bodies;
    onecad::elementmap::ElementMapPartition partition;
    const json host = json{{"primary",
                            json{{"bodyId", "body_1"}, {"elementId", "el_top"}, {"kind", "face"}}}};
    const auto at = [&](const json& version) {
        json params = json{{"sketchId", "sk1"},
                           {"plane", json{{"kind", "XY"}}},
                           {"hostFace", host},
                           {"frameTransportVersion", version}};
        return onecad::session::reseat_sketch_on_host(bodies, partition, params, "op_7", {});
    };
    const auto future = at(json(2));
    check(future.unsupported_transport.has_value(),
          "an unimplemented frameTransportVersion is refused by name");
    check(future.needs_repair.empty() && !future.resolved,
          "the refusal carries no §9 item — it is a wire-version problem, not a binding one");
    check(at(json("1")).unsupported_transport.has_value(),
          "a NON-INTEGER version is refused too, never coerced");
    // Astra break on WP-1: read at the value's OWN width. `get<int>()` would wrap
    // 2^32+1 to 1 and re-seat a sketch authored against a policy this build has
    // never heard of — the one outcome the version field exists to prevent.
    check(at(json(4294967297LL)).unsupported_transport.has_value(),
          "an OVERSIZED version refuses by name rather than wrapping into 1");
    check(at(json(-1)).unsupported_transport.has_value(), "a NEGATIVE version is refused");
    check(!at(json(1)).unsupported_transport.has_value(),
          "version 1 is the rule this build implements");

    // Absent means 1 — every world/datum sketch and every legacy record.
    json plain = json{{"sketchId", "sk1"}, {"plane", json{{"kind", "XY"}}}};
    const auto none = onecad::session::reseat_sketch_on_host(bodies, partition, plain, "op_7", {});
    check(!none.attempted && !none.unsupported_transport.has_value(),
          "no hostFace ⇒ the pre-WP-1 path, untouched");
}

// ── E. Every failure halts ──────────────────────────────────────────────────

void test_every_unresolvable_host_halts_with_a_ladder_item() {
    // ONE FAILURE CLASS (A-1). A host the worker cannot bind halts the step with a
    // §9 item, the plan stops at m−1, and the sketch stays at its authored frame.
    //
    // WP-1 shipped a second class — an identity-CONFIDENCE shortfall degraded to an
    // info diagnostic and kept the authored frame — because the frozen anchor was
    // the face's `gp_Pln` LOCATION, routinely far outside the face it came from, so
    // a correct unambiguous host scored 0.75 and re-picking re-froze the same miss.
    // SCHEMA §7.6 `exact.anchor` now hands `add_sketch_on_face` a point the kernel
    // classified ON the face, so the shortfall means what it says and the halt is
    // clearable by re-picking.
    onecad::session::BodyStore bodies;
    bodies.create("body_1", "op_1",
                  BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 80.0, 60.0, 40.0).Shape());
    onecad::elementmap::ElementMapPartition partition;
    const json plane = json{{"kind", "custom"},
                            {"origin", json::array({0.0, 0.0, 40.0})},
                            {"xAxis", json::array({0.0, 1.0, 0.0})},
                            {"yAxis", json::array({-1.0, 0.0, 0.0})},
                            {"normal", json::array({0.0, 0.0, 1.0})}};
    const auto run = [&](const std::string& body_id, const json& intent, const json& anchor) {
        json host = json{{"primary", json{{"bodyId", body_id},
                                          {"elementId", "el_top"},
                                          {"kind", "face"}}},
                         {"anchor", anchor}};
        if (!intent.is_null()) host["intent"] = intent;
        json params =
            json{{"sketchId", "sk1"}, {"plane", plane}, {"hostFace", std::move(host)}};
        return onecad::session::reseat_sketch_on_host(bodies, partition, params, "op_9", {});
    };
    // The anchor is deliberately far off the body, so nothing can bind on it alone.
    const json stray_anchor = json{{"worldPoint", json::array({500.0, 500.0, 500.0})}};

    // (a) The body EXISTS, the ref cannot be bound confidently ⇒ HALT with the
    //     ladder's own token and its ranked candidates.
    const auto weak = run("body_1", json(), stray_anchor);
    check(weak.attempted && !weak.resolved, "unbindable host: not resolved");
    check(weak.needs_repair.size() == 1,
          "unbindable host: ONE §9 item — the confidence shortfall halts");
    if (weak.needs_repair.size() == 1) {
        const json& item = weak.needs_repair[0];
        check(item.value("refId", std::string()) == "op_9.input0",
              "unbindable host: addresses the sketch's own slot 0");
        const std::string reason = item.value("reason", std::string());
        check(reason == "low-confidence" || reason == "ambiguous",
              "unbindable host: the LADDER's own token, got " + reason);
        check(item.contains("candidates") && item["candidates"].is_array() &&
                  !item["candidates"].empty(),
              "unbindable host: the ranked candidates ride along as repair evidence");
        check(item.value("scoringVersion", -1) == onecad::elementmap::kResolverVersion,
              "unbindable host: the scoring version is the ladder's");
    }
    check(weak.diagnostics.empty(),
          "unbindable host: a halt is reported through §9, never ALSO as a diagnostic");
    check_pnt(weak.effective.origin, 0.0, 0.0, 40.0, "unbindable host keeps its authored frame");
    check(!weak.sketch_placement.has_value(), "unbindable host publishes no placement");

    // (b) The BODY is gone ⇒ halt, with the §9 item on the sketch's own slot.
    const auto gone = run("body_missing", json(), stray_anchor);
    check(gone.needs_repair.size() == 1, "a host whose BODY is absent halts with a §9 item");
    check(gone.needs_repair[0].value("refId", std::string()) == "op_9.input0",
          "and addresses the sketch's own slot 0");
    check(gone.needs_repair[0].value("reason", std::string()) == "no-candidates",
          "with the ladder's own token");
    check(gone.diagnostics.empty(),
          "a halt is not ALSO reported as a diagnostic — one channel, not two");
}

void test_the_seat_witness_decides_a_resolved_host() {
    // A1 §3d end to end, through `reseat_sketch_on_host` rather than the bare
    // classifier. The TRACKED rung binds the host outright (score 1, margin 1), so
    // what is under test is the seat, not the identity.
    //
    // WP-1 exempted an anchor within 1e−3 mm of the frozen ORIGIN from being a
    // witness at all, because `add_sketch_on_face` froze the plane LOCATION there
    // and a healthy sketch would have reported `sketchSeatOffFace`. With an on-face
    // anchor (SCHEMA §7.6) that exemption is gone: OUT means OUT.
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(gp_Pnt(0, 0, 0), 80.0, 60.0, 40.0).Shape();
    onecad::session::BodyStore bodies;
    bodies.create("body_1", "op_1", box);
    onecad::elementmap::ElementMapPartition partition;
    // Bind `el_top` to the box's own +Z cap through the TRACKED rung.
    TopoDS_Shape cap;
    for (TopExp_Explorer it(box, TopAbs_FACE); it.More(); it.Next()) {
        gp_Pln pln;
        gp_Dir nrm;
        const TopoDS_Face f = TopoDS::Face(it.Current());
        if (!onecad::core::modeling::CoplanarFacePatch::planarFacePlaneAndNormal(f, pln, nrm)) {
            continue;
        }
        if (nrm.Z() > 0.9 && std::abs(pln.Location().Z() - 40.0) < 1.0e-9) {
            cap = f;
            break;
        }
    }
    check(!cap.IsNull(), "seat: the box's +Z cap was found");
    partition.mint("body_1", "el_top", onecad::elementmap::km::ElementKind::Face, cap, box);

    const auto plane_at = [](const json& origin) {
        return json{{"kind", "custom"},
                    {"origin", origin},
                    {"xAxis", json::array({1.0, 0.0, 0.0})},
                    {"yAxis", json::array({0.0, 1.0, 0.0})},
                    {"normal", json::array({0.0, 0.0, 1.0})}};
    };
    const auto seat_at = [&](const json& origin, const std::optional<json>& world_point) {
        json host = json{{"primary", json{{"bodyId", "body_1"},
                                          {"elementId", "el_top"},
                                          {"kind", "face"}}}};
        if (world_point) host["anchor"] = json{{"worldPoint", *world_point}};
        json params = json{
            {"sketchId", "sk1"}, {"plane", plane_at(origin)}, {"hostFace", std::move(host)}};
        return onecad::session::reseat_sketch_on_host(bodies, partition, params, "op_s", {});
    };
    const auto reason_of = [](const onecad::session::SketchHostReseatResult& r) {
        return r.needs_repair.empty() ? std::string("<no item>")
                                      : r.needs_repair[0].value("reason", std::string());
    };
    const json cap_corner = json::array({0.0, 0.0, 40.0});

    // (a) ON the cap — the descriptor centre, which is what §7.6 `exact.anchor`
    //     returns for this face and what `add_sketch_on_face` now freezes.
    const auto on = seat_at(cap_corner, json::array({40.0, 30.0, 40.0}));
    check(on.resolved && on.needs_repair.empty(),
          "seat: an ON-FACE anchor seats and the host tracks, got " + reason_of(on));

    // (b) THE LEGACY SHAPE, and the case WP-1 had to exempt: the anchor IS the
    //     frozen plane origin (that is what `add_sketch_on_face` used to freeze) and
    //     that origin lies OFF the face — 20 mm past the cap's +X boundary, exactly
    //     as an imported STEP cap's `gp_Pln` location does. WP-1 refused to treat an
    //     origin-coincident anchor as a witness at all, because with an off-face
    //     anchor everywhere the honest verdict would have condemned healthy
    //     sketches. With an on-face anchor it is honest, so it halts.
    const json off_face_origin = json::array({100.0, 30.0, 40.0});
    const auto legacy = seat_at(off_face_origin, off_face_origin);
    check(!legacy.resolved, "seat: a legacy origin-anchor off the face does not track");
    check(reason_of(legacy) == "sketchSeatOffFace",
          "seat: a legacy origin-anchor off the face refuses `sketchSeatOffFace`, got " +
              reason_of(legacy));
    check_pnt(legacy.effective.origin, 100.0, 30.0, 40.0,
              "seat: the authored frame stands on a refusal");

    // (c) NO anchor at all is not a seat either (SCHEMA §9 "no usable anchor").
    const auto anchorless = seat_at(cap_corner, std::nullopt);
    check(reason_of(anchorless) == "sketchSeatUnmeasurable",
          "seat: no witness at all is `sketchSeatUnmeasurable`, never a silent pass, got " +
              reason_of(anchorless));
}

}  // namespace

int main() {
    test_g1_g2_pure_normal_translation();
    test_g4_tilt();
    test_g8_reversed_normal();
    test_g9_spin_is_undetectable();
    test_legacy_basis_is_preserved();
    test_conditioning_boundary();
    test_conditioning_just_above_the_guard_stays_orthonormal();
    test_invalid_handedness();
    test_significance_thresholds();
    test_seat_annulus_origin_out_anchor_in();
    test_seat_off_face_when_the_boundary_shrinks();
    test_cut_effect_vectors();
    test_unknown_transport_version_refuses_by_name();
    test_every_unresolvable_host_halts_with_a_ladder_item();
    test_the_seat_witness_decides_a_resolved_host();
    if (g_failures == 0) std::fprintf(stderr, "test_sketch_host_reseat: all checks passed\n");
    return g_failures;
}
