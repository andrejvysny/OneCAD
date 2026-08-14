// LanternMath.cpp — see LanternMath.h for provenance and for why the phi_min
// solve is bracketed rather than a bare root-find.

#include "kernel/gear/LanternMath.h"

#include <cmath>

namespace onecad::kernel::gear {

namespace {

constexpr double kPi = 3.14159265358979323846;

/// Upstream's `find_phi_min` objective, with its constant positive factor
/// `r_0` dropped — same roots, better conditioned.
double phi_min_objective(double phi, double r0, double rr) {
  return phi * phi * r0 - 2.0 * phi * r0 * std::sin(phi) - 2.0 * phi * rr -
         2.0 * r0 * std::cos(phi) + 2.0 * r0 + 2.0 * rr * std::sin(phi);
}

/// d/dφ of the above, in closed form:
///   g′(φ) = 2·(1 − cos φ)·(r₀·φ − r_r)
/// Both the objective's shape argument in the header and the Newton step
/// below rest on this; it is checked against a finite difference in the unit
/// test rather than taken on faith.
double phi_min_objective_derivative(double phi, double r0, double rr) {
  return 2.0 * (1.0 - std::cos(phi)) * (r0 * phi - rr);
}

/// Solve `phi_min_objective == 0` on (r_r/r₀, ∞) — the interval on which the
/// header proves the root is unique and the spurious φ=0 root is excluded.
///
/// Safeguarded Newton: take the Newton step when it stays inside the live
/// bracket, otherwise bisect. That keeps Newton's convergence rate where the
/// function is well behaved while never leaving the bracket, so it cannot run
/// away to the degenerate root the way an unguarded solver can.
bool solve_phi_min(double r0, double rr, double& out) {
  double lo = rr / r0;  // g(lo) < 0 by the shape argument (strict minimum).
  if (!(phi_min_objective(lo, r0, rr) < 0.0)) {
    // Defensive: the shape argument says this cannot happen for rr>0, r0>0.
    return false;
  }
  // Grow an upper bound until the objective turns positive. g ~ φ²·r₀
  // dominates, so this terminates; the cap is a runaway guard, not a limit
  // any valid input approaches.
  double hi = std::max(2.0 * lo, 1e-6);
  int grow = 0;
  while (phi_min_objective(hi, r0, rr) <= 0.0) {
    hi *= 2.0;
    if (++grow > 200) return false;
  }

  double phi = 0.5 * (lo + hi);
  for (int i = 0; i < 200; ++i) {
    const double f = phi_min_objective(phi, r0, rr);
    if (f > 0.0) {
      hi = phi;
    } else {
      lo = phi;
    }
    const double d = phi_min_objective_derivative(phi, r0, rr);
    double next = (std::abs(d) > 1e-18) ? (phi - f / d) : (0.5 * (lo + hi));
    if (!(next > lo && next < hi)) next = 0.5 * (lo + hi);
    if (std::abs(next - phi) <= 1e-15 * std::max(1.0, std::abs(phi))) {
      out = next;
      return true;
    }
    phi = next;
  }
  out = phi;
  return true;
}

/// Arc through two points about the origin, three-point form. See
/// TimingMath.cpp for why the radius is averaged.
bool arc_about_origin(const Point2d& p1, const Point2d& p2, Arc3& out) {
  const double r = (norm(p1) + norm(p2)) / 2.0;
  const Point2d chordMid = scaled(add(p1, p2), 0.5);
  const double len = norm(chordMid);
  if (!(len > 1e-12)) return false;
  out.start = p1;
  out.mid = scaled(chordMid, r / len);
  out.end = p2;
  return true;
}

}  // namespace

Point2d lantern_flank_point(double phi, double r0, double rr) {
  return {r0 * (std::cos(phi) + phi * std::sin(phi)) - rr * std::sin(phi),
          r0 * (std::sin(phi) - phi * std::cos(phi)) + rr * std::cos(phi)};
}

LanternFactorsResult compute_lantern_factors(const LanternParams& p) {
  LanternFactorsResult result;
  if (p.numTeeth < 3) {
    result.error = "numTeeth must be >= 3 (got " + std::to_string(p.numTeeth) + ")";
    return result;
  }
  if (!(p.module > 0.0)) {
    result.error = "module must be > 0";
    return result;
  }
  if (!(p.rollerRadius > 0.0)) {
    result.error = "rollerRadius must be > 0 (a lantern gear meshes with rollers of finite radius)";
    return result;
  }
  if (p.sampleCount < 2) {
    result.error = "sampleCount must be >= 2 (got " + std::to_string(p.sampleCount) + ")";
    return result;
  }

  LanternFactors f;
  const double z = static_cast<double>(p.numTeeth);
  f.pitchRadius = p.module * z / 2.0;
  f.maxRadius = f.pitchRadius + p.rollerRadius + p.head * p.module;
  f.angularPitch = 2.0 * kPi / z;

  const double rmaxSq = f.maxRadius * f.maxRadius;
  const double r0Sq = f.pitchRadius * f.pitchRadius;
  if (!(rmaxSq >= r0Sq)) {
    result.error = "head " + std::to_string(p.head) +
                   " pulls the tooth tip inside the pitch circle — no real flank end exists";
    return result;
  }
  f.phiMax = (p.rollerRadius + std::sqrt(rmaxSq - r0Sq)) / f.pitchRadius;

  if (!solve_phi_min(f.pitchRadius, p.rollerRadius, f.phiMin)) {
    result.error = "could not bracket the tooth-root parameter for these lantern dimensions";
    return result;
  }
  if (!(f.phiMin < f.phiMax)) {
    result.error = "tooth root parameter " + std::to_string(f.phiMin) +
                   " is not below the tip parameter " + std::to_string(f.phiMax) +
                   " — roller radius too large for this module/tooth count";
    return result;
  }

  result.ok = true;
  result.factors = f;
  return result;
}

LanternToothResult one_tooth_segments(const LanternParams& p, const LanternFactors& f) {
  LanternToothResult out;

  // Sample the leading flank over [phiMin, phiMax].
  std::vector<Point2d> flank;
  flank.reserve(static_cast<std::size_t>(p.sampleCount));
  for (int i = 0; i < p.sampleCount; ++i) {
    const double t = static_cast<double>(i) / static_cast<double>(p.sampleCount - 1);
    const double phi = f.phiMin + (f.phiMax - f.phiMin) * t;
    flank.push_back(lantern_flank_point(phi, f.pitchRadius, p.rollerRadius));
  }

  // Trailing flank is the mirror across the x-axis (upstream's `xy1 * [1,-1]`).
  std::vector<Point2d> mirrored;
  mirrored.reserve(flank.size());
  for (const auto& q : flank) mirrored.push_back(mirror_about_x_axis(q));

  const Point2d p_1 = flank.front();
  const Point2d p_1_end = flank.back();
  const Point2d p_2 = mirrored.front();
  const Point2d p_2_end = mirrored.back();

  // Root arc passes through the deepest point of the tooth space.
  const Point2d p_12{f.pitchRadius - p.rollerRadius, 0.0};
  // Tip arc closes onto the next tooth's trailing-flank outer end.
  const Point2d p_3 = rotate(p_2_end, f.angularPitch);

  Arc3 tip;
  if (!arc_about_origin(p_1_end, p_3, tip)) {
    out.error = "degenerate tooth-tip arc";
    return out;
  }

  // Traversal order: down the mirrored flank, across the root, up the
  // leading flank, over the tip. Upstream leaves this ordering to
  // `Part.Wire`; see the header for why it is explicit here.
  std::vector<Point2d> mirroredReversed(mirrored.rbegin(), mirrored.rend());

  Arc3 root;
  root.start = p_2;
  root.mid = p_12;
  root.end = p_1;

  out.segments.push_back(interpolated_segment(std::move(mirroredReversed)));
  out.segments.push_back(arc_segment(root));
  out.segments.push_back(interpolated_segment(flank));
  out.segments.push_back(arc_segment(tip));

  // Guard the contract the header advertises rather than assuming it: the
  // mirrored run must start at the outer end and finish at the root.
  if (distance(out.segments.front().start, p_2_end) > 1e-12 ||
      distance(out.segments.front().end, p_2) > 1e-12) {
    out.error = "internal: mirrored flank orientation is wrong";
    return out;
  }

  out.ok = true;
  return out;
}

}  // namespace onecad::kernel::gear
