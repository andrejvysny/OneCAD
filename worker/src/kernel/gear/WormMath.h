// WormMath.h — ZA worm (trapezoidal thread) cross-section math.
//
// Ported from freecad.gears v1.3.0 (GPL-3.0), `freecad/gears/wormgear.py`
// class `WormGear`. GPLv3 §13 permits combination with this AGPL-3.0 project
// (Gear Generator plan §11).
//
// Deliberately NO OCCT — see GearTypes.h. Angles are RADIANS; lengths mm.
//
// A ZA worm's thread is a straight-sided (trapezoidal) rack profile in the
// AXIAL plane, wrapped onto the cylinder: upstream's `helical_projection`
// maps an axial coordinate `z` to a rotation `φ = 2z/(m·t)` and drops the
// third coordinate, so what this file emits is the flat 2D CROSS-SECTION that
// the helical sweep later carries along the lead. The sweep itself (and hence
// the whole 3D worm) belongs to the OCCT layer — Gear Generator plan phase
// G3, which builds the `Helix` + Frenet `Sweep` wrappers. `helixTwistPerUnit`
// below is the one number that layer needs from here.
//
// AN EXACT PROPERTY WORTH KNOWING: the four z-stations that define the
// trapezoid sum so that the tooth spans EXACTLY one angular pitch,
//     φ₄ − φ₀ = 2π/t,
// identically in module, pressure angle, head and clearance — the head and
// clearance contributions cancel in pairs. That is asserted directly by the
// unit test (to 1e-15) rather than being taken on trust, because it is what
// makes the t-fold replication close into a single wire.

#pragma once

#include <string>
#include <vector>

#include "kernel/gear/GearTypes.h"

namespace onecad::kernel::gear {

struct WormParams {
  double module = 1.0;     ///< m, > 0.
  int numThreads = 3;      ///< t (thread starts). >= 1; SCHEMA caps at 10 (plan §5.5) — a policy
                           ///< limit enforced at the op layer, not a mathematical one.
  double diameter = 5.0;   ///< d, > 0.
  double pressureAngle = 20.0 * 3.14159265358979323846 / 180.0;  ///< α, radians, in (0, π/2).
  double clearance = 0.25;  ///< ×module additional root depth.
  double head = 0.0;        ///< ×module additional head length.
  int sampleCount = 10;     ///< samples per flank, >= 2.
  bool reversePitch = false;  ///< mirror the helix handedness.
};

struct WormFactors {
  double leadAngle = 0.0;   ///< β = atan(m·t/d) — upstream reports this as a computed property.
  double rootRadius = 0.0;  ///< r₁ = (d − (2 + 2·clearance)·m)/2.
  double tipRadius = 0.0;   ///< r₂ = (d + (2 + 2·head)·m)/2.
  double z0 = 0.0, z1 = 0.0, z2 = 0.0, z3 = 0.0, z4 = 0.0;      ///< axial trapezoid stations.
  double phi0 = 0.0, phi1 = 0.0, phi2 = 0.0, phi3 = 0.0, phi4 = 0.0;  ///< their wrapped angles.
  double angularPitch = 0.0;  ///< 2π/t, and (exactly) φ₄ − φ₀.
  /// Twist the G3 helical sweep must apply, PER UNIT of extrusion height:
  /// upstream passes `height · tan(β′) · 2/d`, where β′ is the lead angle's
  /// complement, negated when `reversePitch` is set. Multiply by the op's
  /// height to get the total twist angle.
  double helixTwistPerUnit = 0.0;
};

struct WormFactorsResult {
  bool ok = false;
  std::string error;
  WormFactors factors;
};

/// Validate + compute. Refuses when the bore/clearance combination drives the
/// root radius to or below zero, when the tip does not clear the root, or
/// when the pressure angle is steep enough to invert the trapezoid (the
/// axial stations must be strictly increasing — at α beyond roughly 38° the
/// flank width term `z_b` goes negative and upstream would silently emit a
/// self-crossing profile).
WormFactorsResult compute_worm_factors(const WormParams& params);

/// One thread's cross-section as four end-to-end continuous segments,
/// spanning exactly one angular pitch:
///   [0] arc          — root, radius r₁, φ₀ -> φ₁
///   [1] interpolated — rising flank, (r₁,z₁) -> (r₂,z₂)
///   [2] arc          — tip, radius r₂, φ₂ -> φ₃
///   [3] interpolated — falling flank, (r₂,z₃) -> (r₁,z₄)
///
/// Same invariants as every other recipe here: chain continuity, and
/// `rotate(segments.back().end, -2π/t) == segments.front().start`.
struct WormToothResult {
  bool ok = false;
  std::string error;
  std::vector<ProfileSegment> segments;
};
WormToothResult one_tooth_segments(const WormParams& params, const WormFactors& factors);

/// Upstream's `helical_projection` for a single station: wrap axial `z` to
/// the angle `2z/(m·t)` at radius `r`. Exposed for the oracle test.
Point2d worm_projection(double r, double z, double module, int numThreads);

}  // namespace onecad::kernel::gear
