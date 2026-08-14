// InvoluteMath.h — external involute gear tooth profile math.
//
// Ported from pygears.involute_tooth (freecad.gears v1.3.0, GPL-3.0,
// https://github.com/looooo/freecad.gears, class InvoluteTooth in
// pygears/involute_tooth.py, helpers in pygears/_functions.py). GPLv3 §13
// permits combination with this AGPL-3.0-licensed project (SCHEMA/Gear
// Generator implementation plan §11).
//
// Deliberately NO OCCT — this is the SCHEMA §7.3 `Gear` op's pure-math
// sampling layer (Gear Generator implementation plan, phase G0). It produces
// 2D profile points in the tooth's local frame (root/pitch/addendum circles
// concentric at the origin, tooth centered on the +X axis before the caller
// rotates/places N copies around the gear axis). Every quantity here is a
// closed-form or 1-D-sampled function of `InvoluteToothParams` alone — no
// session state, no OCCT handles, safe to unit-test standalone and safe to
// call from either worker lane.
//
// Angles are RADIANS throughout (matching OCCT/this file's own convention);
// the SCHEMA wire format's degrees live only at the op-layer boundary
// (worker/src/ops/gear/GearTool.*, not built yet — Gear Generator plan
// phase G1) and must convert before calling into this file.
//
// Domain failures (e.g. a shift/clearance combination with no valid tooth
// depth) are reported as an explicit `ok=false` + `error` message, never as
// a silently-NaN result — this is the project's fail-closed rule (see
// CLAUDE.md "Debugging" / the Gear Generator plan's correctness posture)
// applied to the math layer, not just the op layer.

#pragma once

#include <string>
#include <vector>

#include "kernel/gear/GearTypes.h"

namespace onecad::kernel::gear {

/// Input parameters for one external involute gear tooth. Field names and
/// defaults mirror `InvoluteTooth.__init__` (pygears/involute_tooth.py); the
/// SCHEMA-facing names differ in a few spots (`module` vs `m_n`, `pressureAngle`
/// vs `pressure_angle`) — see the Gear Generator implementation plan §5 table.
struct InvoluteToothParams {
  double module = 5.0;                          ///< m_n, normal module (mm), > 0.
  int numTeeth = 15;                             ///< z, >= 3.
  double pressureAngle = 20.0 * 3.14159265358979323846 / 180.0;  ///< normal α, radians, in (0, π/2).
  double clearance = 0.25;                       ///< ×m_n, dedendum clearance coefficient.
  double shift = 0.0;                            ///< profile shift coefficient x.
  double helixAngle = 0.0;                       ///< β, radians. 0 => spur (this file's callers still only build the 2D profile; the G3 sweep decides what β does with it).
  bool undercut = false;                         ///< true => trochoid root curve trimmed against the involute (upstream's fragile branch — see `one_tooth_profile`).
  double backlash = 0.0;                         ///< arc length at the pitch circle (mm), >= 0.
  double head = 0.0;                             ///< ×m_n extra addendum.
  bool propertiesFromTool = false;                ///< recompute transverse α_t, m from normal (tool) values using `helixAngle`.
};

/// Closed-form derived quantities, ported from
/// `InvoluteTooth._calc_gear_factors`. Several of these are the SCHEMA
/// `computed` read-outs (§5.1: pitchDiameter, addendumDiameter,
/// rootDiameter, transverseModule, transversePitch, angularBacklash); the
/// rest are internal sampling bounds `undercut_points`/`involute_points`
/// need.
struct InvoluteToothFactors {
  double pressureAngleTransverse = 0.0;  ///< α_t (== pressureAngle when !propertiesFromTool).
  double transverseModule = 0.0;         ///< m.
  double transversePitch = 0.0;          ///< m·π.
  double clearanceAbs = 0.0;             ///< c = clearance × module.
  double referenceDiameter = 0.0;        ///< d = z·m.
  double workingDiameter = 0.0;          ///< dw (== d here; kept distinct because other recipes, e.g. a future internal gear, redefine it).
  double addendumDiameter = 0.0;         ///< da.
  double rootDiameter = 0.0;             ///< df.
  double baseDiameter = 0.0;             ///< dg (involute base circle).
  double angularPitch = 0.0;             ///< phipart = 2π/z.
  double undercutEnd = 0.0;              ///< upper bound of the undercut sampling parameter ψ.
  double undercutRot = 0.0;
  double involuteEnd = 0.0;              ///< upper bound of the involute sampling parameter φ.
  double involuteRot1 = 0.0;
  double involuteRot2 = 0.0;
  double involuteRot = 0.0;
  double angularBacklash = 0.0;          ///< backlash / (d/2), radians.
  double involuteStart = 0.0;            ///< lower bound of φ; > 0 when the base circle is inside the root circle (dg <= df).
};

/// `compute_involute_factors` result. `ok == false` means the parameter
/// combination has no valid tooth geometry (e.g. addendum/root diameters
/// with no real undercut-sampling range) — `factors` is left default in that
/// case and must not be used.
struct InvoluteFactorsResult {
  bool ok = false;
  std::string error;
  InvoluteToothFactors factors;
};

/// Ported from `InvoluteTooth._calc_gear_factors`. Validates the domain
/// preconditions the closed-form expressions need (positive module, teeth
/// >= 3, pressure angle in range, and every `sqrt` argument used below
/// non-negative) before computing them, so a bad parameter combination is a
/// graded refusal here rather than a silent NaN three call levels away.
InvoluteFactorsResult compute_involute_factors(const InvoluteToothParams& params);

/// Parametric undercut (trochoid) curve point, PRE-rotation, for sampling
/// parameter psi in [0, factors.undercutEnd]. Ported from
/// `InvoluteTooth.undercut_function_x` / `_y`.
Point2d undercut_point(double psi, const InvoluteToothFactors& factors);

/// Parametric involute curve point, PRE-rotation, for sampling parameter phi
/// in [factors.involuteStart, factors.involuteEnd]. Ported from
/// `InvoluteTooth.involute_function_x` / `_y`.
Point2d involute_point(double phi, const InvoluteToothFactors& factors);

/// `count` points linspace-sampled over [0, undercutEnd], rotated into the
/// tooth's local frame. Ported from `InvoluteTooth.undercut_points`.
/// `count` must be >= 2.
std::vector<Point2d> undercut_points(const InvoluteToothFactors& factors, int count);

/// `count` points linspace-sampled over [involuteStart, involuteEnd],
/// rotated into the tooth's local frame. Ported from
/// `InvoluteTooth.involute_points`. `count` must be >= 2.
std::vector<Point2d> involute_points(const InvoluteToothFactors& factors, int count);

/// One full tooth profile as an ordered sequence of polyline segments,
/// concatenable end-to-end into a single flank-to-flank boundary (each
/// segment's last point equals the next segment's first point). Ported from
/// `InvoluteTooth.points()`. Shape depends on `params`:
///
///   - `!undercut`, base circle outside root circle (dg > df): 5 segments
///     `[rootBlend(2), flank(count), tip(2), mirroredFlank(count), mirroredRootBlend(2)]`.
///     `rootBlend` is a straight 2-point segment from a point on the root
///     circle to the involute's own start point — there is no trochoid
///     curve in this branch (matches upstream: undercut geometry is only
///     computed when `undercut` is requested).
///   - `!undercut`, base circle at/inside root circle (dg <= df): 3 segments
///     `[flank(count), tip(2), mirroredFlank(count)]` — the involute alone
///     reaches the root circle, no blend needed.
///   - `undercut`: 5 segments, but the first/last segment's point count
///     varies — it is the NEAREST-POINT trim of the sampled undercut and
///     involute polylines (`_functions.nearestpts`).
///
///     Upstream appears to offer a second, better path here: it computes
///     `s = trimfunc(l1, l2[::-1])` (a real curve-crossing trim) and means to
///     prefer it. That code is UNREACHABLE. `trimfunc` returns either
///     `False` or a Python LIST of two arrays, and the guard testing it is
///     `isinstance(s, ndarray)` — never true for a list. So the reference
///     discards the trim result every time and always falls back to
///     `nearestpts`; the crossing-trim branch has never run in any released
///     freecad.gears. This port matches the reachable behaviour, because
///     parity with what the reference ACTUALLY produces is the goal, and
///     silently "fixing" the bug would move every undercut tooth's root
///     blend relative to the gears users already have.
///
///     Discovered by the cross-check harness (scripts/gear-crosscheck), not
///     by reading: porting the intended behaviour disagreed with the
///     reference on 224 of 2673 swept cases. If OneCAD ever wants the
///     better geometry, it is a deliberate, documented divergence with its
///     own schema/version story — not a silent change.
///
/// `tip` is always the raw 2-point chord `[flank.back(), mirroredFlank.front()]`
/// — NOT an addendum-circle arc. Whether the wire builder that consumes this
/// (Gear Generator plan phase G1, not yet built) treats it as a chord or
/// replaces it with an arc is an open decision for that layer, not this one.
struct ToothProfile {
  std::vector<std::vector<Point2d>> segments;
};
ToothProfile one_tooth_profile(const InvoluteToothParams& params,
                                const InvoluteToothFactors& factors, int sampleCount);

}  // namespace onecad::kernel::gear
