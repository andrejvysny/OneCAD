// WormMath.cpp — see WormMath.h for provenance and scope.

#include "kernel/gear/WormMath.h"

#include <cmath>

namespace onecad::kernel::gear {

namespace {

constexpr double kPi = 3.14159265358979323846;

/// Circle arc about the origin between two angles at a fixed radius, in
/// three-point form. Unlike the other recipes' arcs this one is defined by an
/// angle range rather than two endpoints, so the interior point is placed at
/// the exact angular midpoint — no radius averaging is needed or wanted.
Arc3 arc_by_angles(double radius, double a0, double a1) {
  Arc3 a;
  a.start = {radius * std::cos(a0), radius * std::sin(a0)};
  const double am = 0.5 * (a0 + a1);
  a.mid = {radius * std::cos(am), radius * std::sin(am)};
  a.end = {radius * std::cos(a1), radius * std::sin(a1)};
  return a;
}

}  // namespace

Point2d worm_projection(double r, double z, double module, int numThreads) {
  const double phi = 2.0 * z / module / static_cast<double>(numThreads);
  return {r * std::cos(phi), r * std::sin(phi)};
}

WormFactorsResult compute_worm_factors(const WormParams& p) {
  WormFactorsResult result;
  if (p.numThreads < 1) {
    result.error = "numThreads must be >= 1 (got " + std::to_string(p.numThreads) + ")";
    return result;
  }
  if (!(p.module > 0.0)) {
    result.error = "module must be > 0";
    return result;
  }
  if (!(p.diameter > 0.0)) {
    result.error = "diameter must be > 0";
    return result;
  }
  if (!(p.pressureAngle > 0.0) || !(p.pressureAngle < kPi / 2.0)) {
    result.error = "pressureAngle must be in (0, 90) degrees";
    return result;
  }
  if (p.sampleCount < 2) {
    result.error = "sampleCount must be >= 2 (got " + std::to_string(p.sampleCount) + ")";
    return result;
  }

  WormFactors f;
  const double m = p.module;
  const double d = p.diameter;
  const double t = static_cast<double>(p.numThreads);
  const double tanA = std::tan(p.pressureAngle);

  f.leadAngle = std::atan(m * t / d);
  f.rootRadius = (d - (2.0 + 2.0 * p.clearance) * m) / 2.0;
  f.tipRadius = (d + (2.0 + 2.0 * p.head) * m) / 2.0;

  if (!(f.rootRadius > 0.0)) {
    result.error = "root radius " + std::to_string(f.rootRadius) +
                   " mm is not positive — diameter too small for this module/clearance";
    return result;
  }
  if (!(f.tipRadius > f.rootRadius)) {
    result.error = "tip radius does not clear the root radius — check head/clearance";
    return result;
  }

  const double z_a = (2.0 + p.head + p.clearance) * m * tanA;
  const double z_b = (m * kPi - 4.0 * m * tanA) / 2.0;
  f.z0 = p.clearance * m * tanA;
  f.z1 = z_b - f.z0;
  f.z2 = f.z1 + z_a;
  f.z3 = f.z2 + z_b - 2.0 * p.head * m * tanA;
  f.z4 = f.z3 + z_a;

  // The trapezoid must not invert. `z_b` is the flat width term and goes
  // negative once tan(α) exceeds π/4 (α ≳ 38.1°), at which point upstream
  // would emit a self-crossing thread rather than refuse.
  if (!(f.z0 < f.z1 && f.z1 < f.z2 && f.z2 < f.z3 && f.z3 < f.z4)) {
    result.error =
        "thread profile stations are not increasing — pressureAngle too steep (or head/clearance "
        "too large) for this module; the trapezoid would self-cross";
    return result;
  }

  const double toPhi = 2.0 / m / t;
  f.phi0 = f.z0 * toPhi;
  f.phi1 = f.z1 * toPhi;
  f.phi2 = f.z2 * toPhi;
  f.phi3 = f.z3 * toPhi;
  f.phi4 = f.z4 * toPhi;
  f.angularPitch = 2.0 * kPi / t;

  // Upstream: beta = -(reverse_pitch*2 - 1) * (pi/2 - beta_lead), and the
  // extrusion twist is height * tan(beta) * 2/d.
  const double betaSweep =
      (p.reversePitch ? -1.0 : 1.0) * (kPi / 2.0 - f.leadAngle);
  f.helixTwistPerUnit = std::tan(betaSweep) * 2.0 / d;

  result.ok = true;
  result.factors = f;
  return result;
}

WormToothResult one_tooth_segments(const WormParams& p, const WormFactors& f) {
  WormToothResult out;

  auto sample_flank = [&](double rFrom, double rTo, double zFrom, double zTo) {
    std::vector<Point2d> pts;
    pts.reserve(static_cast<std::size_t>(p.sampleCount));
    for (int i = 0; i < p.sampleCount; ++i) {
      const double u = static_cast<double>(i) / static_cast<double>(p.sampleCount - 1);
      const double r = rFrom + (rTo - rFrom) * u;
      const double z = zFrom + (zTo - zFrom) * u;
      pts.push_back(worm_projection(r, z, p.module, p.numThreads));
    }
    return pts;
  };

  out.segments.push_back(arc_segment(arc_by_angles(f.rootRadius, f.phi0, f.phi1)));
  out.segments.push_back(
      interpolated_segment(sample_flank(f.rootRadius, f.tipRadius, f.z1, f.z2)));
  out.segments.push_back(arc_segment(arc_by_angles(f.tipRadius, f.phi2, f.phi3)));
  out.segments.push_back(
      interpolated_segment(sample_flank(f.tipRadius, f.rootRadius, f.z3, f.z4)));

  out.ok = true;
  return out;
}

}  // namespace onecad::kernel::gear
