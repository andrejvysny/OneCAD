// TimingTMath.cpp — see TimingTMath.h for provenance and the two deliberate
// divergences from upstream (closed-form flank endpoints, no fillets here).

#include "kernel/gear/TimingTMath.h"

#include <cmath>

namespace onecad::kernel::gear {

namespace {

constexpr double kPi = 3.14159265358979323846;

/// Intersect the ray `p_A + d·s` (with `d` a UNIT vector) against the circle
/// of radius `R` centred at the origin, returning the parameter `s` upstream's
/// minimiser converges to.
///
/// |p_A + d·s|² = R²  with |d| = 1  gives  s² + 2·(p_A·d)·s + (|p_A|² − R²) = 0,
/// so  s = −b ± sqrt(b² − c)  with  b = p_A·d,  c = |p_A|² − R².
///
/// WHICH ROOT: always `−b + sqrt(disc)`. This is geometric, not fitted.
/// `p_A` sits at mid-tooth-height and `d` leaves it along the flank at half
/// the included flank angle, so b = |p_A|·cos(α/2) > 0 for any sane α.
///   * Tip circle (R > |p_A|): c < 0, so the roots straddle zero and
///     `−b + sqrt(disc)` is the positive one — outward along the flank.
///   * Root circle (R < |p_A|): c > 0, so both roots are negative and
///     `−b + sqrt(disc)` is the one nearer zero — a short step inward.
/// In both cases it is the intersection nearest `p_A` in the flank's own
/// sense, which is the branch a minimiser started at s = 0 descends into.
/// Verified against upstream's scipy result for every test case.
///
/// Returns false when the ray misses the circle entirely (disc < 0) — a real
/// domain failure (flank too steep / tooth too deep to reach that radius),
/// where upstream's minimiser would return a nonsense stationary point.
bool solve_flank_parameter(const Point2d& p_A, const Point2d& d, double R, double& s) {
  const double b = p_A.x * d.x + p_A.y * d.y;
  const double c = (p_A.x * p_A.x + p_A.y * p_A.y) - R * R;
  const double disc = b * b - c;
  if (!(disc >= 0.0)) return false;
  s = -b + std::sqrt(disc);
  return true;
}

/// Arc through two points about the origin, in three-point form. Same
/// construction as `_functions.arc_from_points_and_center` with centre (0,0);
/// see TimingMath.cpp for why the radius is averaged.
bool arc_about_origin(const Point2d& p1, const Point2d& p2, Arc3& out) {
  const double r = (norm(p1) + norm(p2)) / 2.0;
  const Point2d chordMid = scaled(add(p1, p2), 0.5);
  const double len = norm(chordMid);
  if (!(len > 1e-12)) return false;  // p1, p2 diametrically opposite: arc ambiguous.
  out.start = p1;
  out.mid = scaled(chordMid, r / len);
  out.end = p2;
  return true;
}

}  // namespace

TimingTFactorsResult compute_timing_t_factors(const TimingTParams& p) {
  TimingTFactorsResult result;
  if (p.numTeeth < 3) {
    result.error = "numTeeth must be >= 3 (got " + std::to_string(p.numTeeth) + ")";
    return result;
  }
  if (!(p.pitch > 0.0)) {
    result.error = "pitch must be > 0";
    return result;
  }
  if (!(p.toothHeight > 0.0)) {
    result.error = "toothHeight must be > 0";
    return result;
  }
  if (!(p.flankAngle > 0.0) || !(p.flankAngle < kPi)) {
    result.error = "flankAngle must be in (0, 180) degrees";
    return result;
  }
  if (!(p.backlash >= 0.0)) {
    result.error = "backlash must be >= 0";
    return result;
  }

  TimingTFactors f;
  const double z = static_cast<double>(p.numTeeth);
  f.pitchRadius = p.pitch * z / 2.0 / kPi;
  f.pitchDiameter = 2.0 * f.pitchRadius;
  f.angularPitch = p.pitch / f.pitchRadius;  // == 2π/z by construction.
  f.halfToothAngle = f.angularPitch / 4.0 - p.backlash / f.pitchRadius;
  f.tipRadius = f.pitchRadius - p.pitchLineOffset;
  f.rootRadius = f.tipRadius - p.toothHeight;

  if (!(f.rootRadius > 0.0)) {
    result.error = "root radius " + std::to_string(f.rootRadius) +
                   " mm is not positive — tooth is deeper than the pulley (check toothHeight / "
                   "pitchLineOffset / teeth)";
    return result;
  }
  if (!(f.halfToothAngle > 0.0)) {
    result.error = "backlash " + std::to_string(p.backlash) +
                   " mm consumes the entire tooth thickness at this pitch/tooth count";
    return result;
  }

  result.ok = true;
  result.factors = f;
  return result;
}

TimingTToothResult one_tooth_segments(const TimingTParams& p, const TimingTFactors& f) {
  TimingTToothResult out;

  const double gamma_1 = f.halfToothAngle;
  // Flank origin: mid-tooth-height on the leading tooth-thickness boundary.
  const Point2d p_A =
      scaled(Point2d{std::cos(-gamma_1), std::sin(-gamma_1)}, f.rootRadius + p.toothHeight / 2.0);
  // Unit flank direction, rotated off the radial by half the included angle.
  const Point2d d{std::cos(p.flankAngle / 2.0 - gamma_1), std::sin(p.flankAngle / 2.0 - gamma_1)};

  double s1 = 0.0;
  double s2 = 0.0;
  if (!solve_flank_parameter(p_A, d, f.rootRadius, s1)) {
    out.error = "tooth flank never reaches the root circle — flankAngle too wide for this tooth "
                "depth";
    return out;
  }
  if (!solve_flank_parameter(p_A, d, f.tipRadius, s2)) {
    out.error = "tooth flank never reaches the tip circle — flankAngle too wide for this tooth "
                "depth";
    return out;
  }

  const Point2d p_1 = add(p_A, scaled(d, s1));  // flank start, on the root circle
  const Point2d p_2 = add(p_A, scaled(d, s2));  // flank end, on the tip circle
  // Upstream mirrors the flank across the x-axis to build the trailing side.
  const Point2d p_3 = mirror_about_x_axis(p_2);
  const Point2d p_4 = mirror_about_x_axis(p_1);
  // The next tooth's flank start closes the angular pitch.
  const Point2d p_5 = rotate(p_1, f.angularPitch);

  Arc3 head;
  if (!arc_about_origin(p_2, p_3, head)) {
    out.error = "degenerate tooth-head arc";
    return out;
  }
  Arc3 space;
  if (!arc_about_origin(p_4, p_5, space)) {
    out.error = "degenerate tooth-space arc";
    return out;
  }

  out.segments = {
      line_segment(p_1, p_2),
      arc_segment(head),
      line_segment(p_3, p_4),
      arc_segment(space),
  };
  out.ok = true;
  return out;
}

}  // namespace onecad::kernel::gear
