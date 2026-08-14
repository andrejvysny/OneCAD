// TimingMath.cpp — see TimingMath.h for provenance and scope.

#include "kernel/gear/TimingMath.h"

#include <cmath>

namespace onecad::kernel::gear {

namespace {

constexpr double kPi = 3.14159265358979323846;

/// Ported from `_functions.arc_from_points_and_center`: build the canonical
/// three-point arc form from two endpoints and a centre. The interior point
/// is placed on the bisector ray at the mean of the two endpoint radii —
/// upstream averages rather than asserting they are equal, which absorbs the
/// tiny asymmetry left by the tangency algebra.
///
/// That averaging is DEFENSIVE, not load-bearing: the endpoint radii were
/// measured to agree within 3.6e-12 mm across every section × tooth-count in
/// the test sweep, so using either endpoint's radius alone would be
/// indistinguishable at any tolerance the kernel cares about (OCCT's default
/// is 1e-7). Recorded because a mutation test that replaces the mean with
/// `distance(p1, center)` legitimately survives — that is an equivalent
/// mutant, not a hole in test_gear_math_timing.cpp.
///
/// Fails (rather than dividing by zero) when the chord midpoint coincides
/// with the centre, i.e. the endpoints are diametrically opposite and the
/// arc is ambiguous. That is a genuine degeneracy, not a tolerance issue.
bool arc_from_points_and_center(const Point2d& p1, const Point2d& p2, const Point2d& center,
                                 Arc3& out) {
  const double r = (distance(p1, center) + distance(p2, center)) / 2.0;
  const Point2d chordMid = scaled(add(p1, p2), 0.5);
  const Point2d v = sub(chordMid, center);
  const double vlen = norm(v);
  if (!(vlen > 1e-12)) return false;
  out.start = p1;
  out.mid = add(center, scaled(v, r / vlen));
  out.end = p2;
  return true;
}

}  // namespace

TimingProfileData timing_profile_data(TimingProfile profile) {
  // Normative section table, transcribed from upstream `TimingGear.data`.
  //                       pitch,      u,      h,    bigH,     r0,     r1,      rs,  offset
  switch (profile) {
    case TimingProfile::Gt2:
      return {2.0, 0.254, 0.75, 1.38, 0.555, 1.0, 0.15, 0.40};
    case TimingProfile::Gt3:
      return {3.0, 0.381, 1.14, 2.40, 0.85, 1.52, 0.25, 0.61};
    case TimingProfile::Gt5:
      return {5.0, 0.5715, 1.93, 3.81, 1.44, 2.57, 0.416, 1.03};
    case TimingProfile::Gt8:
      return {8.0, 0.9144, 3.088, 6.096, 2.304, 4.112, 0.6656, 1.648};
    case TimingProfile::Htd3:
      return {3.0, 0.381, 1.21, 2.40, 0.89, 0.89, 0.26, 0.0};
    case TimingProfile::Htd5:
      return {5.0, 0.5715, 2.06, 3.80, 1.49, 1.49, 0.43, 0.0};
    case TimingProfile::Htd8:
      return {8.0, 0.686, 3.45, 6.00, 2.46, 2.46, 0.70, 0.0};
  }
  return {};  // unreachable for a valid enumerator; keeps the compiler quiet.
}

std::string timing_profile_name(TimingProfile profile) {
  switch (profile) {
    case TimingProfile::Gt2: return "gt2";
    case TimingProfile::Gt3: return "gt3";
    case TimingProfile::Gt5: return "gt5";
    case TimingProfile::Gt8: return "gt8";
    case TimingProfile::Htd3: return "htd3";
    case TimingProfile::Htd5: return "htd5";
    case TimingProfile::Htd8: return "htd8";
  }
  return "";
}

bool timing_profile_from_name(const std::string& name, TimingProfile& out) {
  if (name == "gt2") { out = TimingProfile::Gt2; return true; }
  if (name == "gt3") { out = TimingProfile::Gt3; return true; }
  if (name == "gt5") { out = TimingProfile::Gt5; return true; }
  if (name == "gt8") { out = TimingProfile::Gt8; return true; }
  if (name == "htd3") { out = TimingProfile::Htd3; return true; }
  if (name == "htd5") { out = TimingProfile::Htd5; return true; }
  if (name == "htd8") { out = TimingProfile::Htd8; return true; }
  return false;
}

TimingFactorsResult compute_timing_factors(const TimingToothParams& params) {
  TimingFactorsResult result;
  if (params.numTeeth < 3) {
    result.error = "numTeeth must be >= 3 (got " + std::to_string(params.numTeeth) + ")";
    return result;
  }

  TimingFactors f;
  f.data = timing_profile_data(params.profile);
  const double z = static_cast<double>(params.numTeeth);
  f.pitchRadiusTrue = f.data.pitch * z / kPi / 2.0;
  f.pitchDiameter = 2.0 * f.pitchRadiusTrue;
  f.outerRadius = f.pitchRadiusTrue - f.data.u;
  f.halfToothAngle = kPi / z;
  // Upstream's `rp` differs per branch (see the field comment in the header):
  // the HTD path folds `u` in immediately, the GT path keeps it separate.
  f.rp = (f.data.offset == 0.0) ? (f.pitchRadiusTrue - f.data.u) : f.pitchRadiusTrue;

  // The tooth is cut INWARD from the head radius by `h`. If the pulley is so
  // small that the root would reach or pass the axis, the section simply does
  // not fit this tooth count — refuse rather than emit inverted arcs.
  if (!(f.outerRadius - f.data.h > 0.0)) {
    result.error = "timing profile " + timing_profile_name(params.profile) + " needs more teeth: root radius " +
                   std::to_string(f.outerRadius - f.data.h) + " mm is not positive at z=" +
                   std::to_string(params.numTeeth);
    return result;
  }

  result.ok = true;
  result.factors = f;
  return result;
}

TimingToothResult one_tooth_arcs(const TimingToothParams& params, const TimingFactors& f) {
  TimingToothResult out;
  const TimingProfileData& d = f.data;
  const double r_12 = d.r0;
  const double r_23 = d.r1;
  const double r_34 = d.rs;
  const double offset = d.offset;
  const double h = d.h;
  const double u = d.u;
  const double phi5 = f.halfToothAngle;
  // Upstream's mirror line for the tooth-space arc that closes the pitch.
  const double refAngle = -phi5 - kPi / 2.0;

  std::vector<Arc3> arcs;

  if (offset == 0.0) {
    // ---- HTD branch: four arcs, r_12 == r_23 so the flank is one arc. ----
    const double rp = f.rp;  // already has `u` folded in.
    const Point2d m_34{-(r_12 + r_34), rp - h + r_12};
    const Point2d x2{-r_12, m_34.y};
    const Point2d x4{m_34.x, m_34.y + r_34};
    const Point2d x6 = mirror_about_line(x4, refAngle);

    const Point2d xn2 = mirror_about_y_axis(x2);
    const Point2d xn4 = mirror_about_y_axis(x4);
    const Point2d mn_34 = mirror_about_y_axis(m_34);

    Arc3 a;
    if (!arc_from_points_and_center(xn4, xn2, mn_34, a)) {
      out.error = "degenerate root arc (htd)";
      return out;
    }
    arcs.push_back(a);
    // Upstream builds this one as a literal three-point arc through the
    // tooth-space bottom, not from a centre — ported as written.
    arcs.push_back(Arc3{xn2, Point2d{0.0, rp - h}, x2});
    if (!arc_from_points_and_center(x2, x4, m_34, a)) {
      out.error = "degenerate flank arc (htd)";
      return out;
    }
    arcs.push_back(a);
    if (!arc_from_points_and_center(x4, x6, Point2d{0.0, 0.0}, a)) {
      out.error = "degenerate head arc (htd)";
      return out;
    }
    arcs.push_back(a);
  } else {
    // ---- GT branch: six arcs, second arc centre displaced by `offset`. ----
    const double denomPhi12 = ((r_12 - r_23) / offset) * ((r_12 - r_23) / offset) - 1.0;
    if (!(denomPhi12 > 0.0)) {
      out.error = "timing profile " + timing_profile_name(params.profile) +
                  ": |r0-r1| must exceed offset for the flank tangency to exist";
      return out;
    }
    const double phi_12 = std::atan(std::sqrt(1.0 / denomPhi12));
    const double rp = f.rp;
    const double r4 = rp - u;
    const double r5 = r4;

    const Point2d m_12{0.0, r5 - h + r_12};
    const double tanPhi12 = std::tan(phi_12);
    if (!(std::abs(tanPhi12) > 1e-15)) {
      out.error = "timing profile flank angle degenerate";
      return out;
    }
    const Point2d m_23{offset, offset / tanPhi12 + m_12.y};
    const double m_23y = m_23.y;

    // Closed-form solve for phi4, transcribed verbatim from upstream's
    // inlined sympy result (the comment there records the original
    // `sympy.solve` call). Kept as one expression deliberately: any
    // "simplification" here is an unverifiable rewrite of a machine-derived
    // form, and the oracle test is what proves the transcription.
    const double o = offset;
    const double sqrtArg =
        -(m_23y * m_23y * m_23y * m_23y)
        - 2.0 * m_23y * m_23y * o * o
        + 2.0 * m_23y * m_23y * r5 * r5
        - 4.0 * m_23y * m_23y * r5 * r_34
        + 2.0 * m_23y * m_23y * r_23 * r_23
        + 4.0 * m_23y * m_23y * r_23 * r_34
        + 4.0 * m_23y * m_23y * r_34 * r_34
        - o * o * o * o
        + 2.0 * o * o * r5 * r5
        - 4.0 * o * o * r5 * r_34
        + 2.0 * o * o * r_23 * r_23
        + 4.0 * o * o * r_23 * r_34
        + 4.0 * o * o * r_34 * r_34
        - r5 * r5 * r5 * r5
        + 4.0 * r5 * r5 * r5 * r_34
        + 2.0 * r5 * r5 * r_23 * r_23
        + 4.0 * r5 * r5 * r_23 * r_34
        - 4.0 * r5 * r5 * r_34 * r_34
        - 4.0 * r5 * r_23 * r_23 * r_34
        - 8.0 * r5 * r_23 * r_34 * r_34
        - r_23 * r_23 * r_23 * r_23
        - 4.0 * r_23 * r_23 * r_23 * r_34
        - 4.0 * r_23 * r_23 * r_34 * r_34;
    if (!(sqrtArg >= 0.0)) {
      out.error = "timing profile " + timing_profile_name(params.profile) +
                  ": no real tangency solution at z=" + std::to_string(params.numTeeth) +
                  " (tooth does not fit this pulley)";
      return out;
    }
    const double phi4Denom = m_23y * m_23y + 2.0 * m_23y * r5 - 2.0 * m_23y * r_34 + o * o +
                             r5 * r5 - 2.0 * r5 * r_34 - r_23 * r_23 - 2.0 * r_23 * r_34;
    if (!(std::abs(phi4Denom) > 1e-15)) {
      out.error = "timing profile tangency solve is degenerate";
      return out;
    }
    const double phi4 =
        2.0 * std::atan((-2.0 * o * r5 + 2.0 * o * r_34 + std::sqrt(sqrtArg)) / phi4Denom);

    const Point2d m_34 = scaled(Point2d{-std::sin(phi4), std::cos(phi4)}, r5 - r_34);
    const Point2d x2{-r_12 * std::sin(phi_12), m_12.y - r_12 * std::cos(phi_12)};
    const Point2d x3 = add(m_34, scaled(sub(m_23, m_34), r_34 / (r_34 + r_23)));
    const Point2d x4 = scaled(Point2d{-std::sin(phi4), std::cos(phi4)}, r4);
    const Point2d x6 = mirror_about_line(x4, refAngle);

    const Point2d xn2 = mirror_about_y_axis(x2);
    const Point2d xn3 = mirror_about_y_axis(x3);
    const Point2d xn4 = mirror_about_y_axis(x4);
    const Point2d mn_34 = mirror_about_y_axis(m_34);
    const Point2d mn_23 = mirror_about_y_axis(m_23);

    const struct {
      Point2d p1, p2, c;
      const char* what;
    } spec[] = {
        {xn4, xn3, mn_34, "root arc (mirrored)"},
        {xn3, xn2, mn_23, "flank arc (mirrored)"},
        {xn2, x2, m_12, "tooth-space arc"},
        {x2, x3, m_23, "flank arc"},
        {x3, x4, m_34, "root arc"},
        {x4, x6, Point2d{0.0, 0.0}, "head arc"},
    };
    for (const auto& s : spec) {
      Arc3 a;
      if (!arc_from_points_and_center(s.p1, s.p2, s.c, a)) {
        out.error = std::string("degenerate ") + s.what + " (gt)";
        return out;
      }
      arcs.push_back(a);
    }
  }

  out.ok = true;
  out.arcs = std::move(arcs);
  return out;
}

}  // namespace onecad::kernel::gear
