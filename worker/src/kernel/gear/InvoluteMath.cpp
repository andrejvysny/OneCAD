// InvoluteMath.cpp — see InvoluteMath.h for provenance and scope.

#include "kernel/gear/InvoluteMath.h"

#include <algorithm>
#include <cmath>
#include <functional>

namespace onecad::kernel::gear {

namespace {

constexpr double kPi = 3.14159265358979323846;

std::vector<Point2d> rotateAll(const std::vector<Point2d>& pts, double angle) {
  std::vector<Point2d> out;
  out.reserve(pts.size());
  for (const auto& p : pts) out.push_back(rotate(p, angle));
  return out;
}

/// Upstream applies `reflection(0)` to a flank to build its mirror image.
std::vector<Point2d> reflectAll(const std::vector<Point2d>& pts) {
  std::vector<Point2d> out;
  out.reserve(pts.size());
  for (const auto& p : pts) out.push_back(mirror_about_x_axis(p));
  return out;
}

std::vector<Point2d> reversed(std::vector<Point2d> pts) {
  std::reverse(pts.begin(), pts.end());
  return pts;
}

std::vector<Point2d> linspace_apply(double lo, double hi, int count,
                                     const std::function<Point2d(double)>& fn) {
  std::vector<Point2d> out;
  out.reserve(static_cast<std::size_t>(count));
  if (count == 1) {
    out.push_back(fn(lo));
    return out;
  }
  for (int i = 0; i < count; ++i) {
    const double t = lo + (hi - lo) * (static_cast<double>(i) / static_cast<double>(count - 1));
    out.push_back(fn(t));
  }
  return out;
}

/// Ported from `_functions.nearestpts`. Finds the (involute-index,
/// undercut-index) pair minimizing distance between `involute[ik+1]` and
/// `undercut[jk+1]`, subject to the involute point being farther from the
/// origin than the undercut point (upstream's `re > ru` guard — without it
/// the nearest pair is often on the wrong side of the curves' near-crossing).
/// Returns `{undercut[0:jout] + [involute[iout]], involute[iout:]}` using the
/// UNSHIFTED indices, exactly as upstream (see InvoluteMath.h's
/// `one_tooth_profile` doc for why the shifted-search / unshifted-report
/// indexing is intentional, verified against the reference implementation's
/// actual output, not re-derived from the loop indices alone).
struct NearestPtsResult {
  std::vector<Point2d> undercutPrefixPlusInvolutePoint;
  std::vector<Point2d> involuteSuffix;
};

NearestPtsResult nearestpts(const std::vector<Point2d>& involute,
                             const std::vector<Point2d>& undercut) {
  std::size_t iout = 0;
  std::size_t jout = 0;
  double outmin = 1000.0;
  for (std::size_t ik = 0; ik + 1 < involute.size(); ++ik) {
    const Point2d& i = involute[ik + 1];
    for (std::size_t jk = 0; jk + 1 < undercut.size(); ++jk) {
      const Point2d& j = undercut[jk + 1];
      const double l = norm(sub(i, j));
      if (l < outmin) {
        const double re = norm(i);
        const double ru = norm(j);
        if (re > ru) {
          outmin = l;
          iout = ik;
          jout = jk;
        }
      }
    }
  }
  NearestPtsResult out;
  out.undercutPrefixPlusInvolutePoint.assign(undercut.begin(),
                                              undercut.begin() + static_cast<long>(jout));
  out.undercutPrefixPlusInvolutePoint.push_back(involute[iout]);
  out.involuteSuffix.assign(involute.begin() + static_cast<long>(iout), involute.end());
  return out;
}

}  // namespace

InvoluteFactorsResult compute_involute_factors(const InvoluteToothParams& p) {
  InvoluteFactorsResult result;
  if (p.numTeeth < 3) {
    result.error = "numTeeth must be >= 3 (got " + std::to_string(p.numTeeth) + ")";
    return result;
  }
  if (!(p.module > 0.0)) {
    result.error = "module must be > 0";
    return result;
  }
  if (!(p.pressureAngle > 0.0) || !(p.pressureAngle < kPi / 2.0)) {
    result.error = "pressureAngle must be in (0, 90) degrees";
    return result;
  }
  if (p.propertiesFromTool && std::abs(std::cos(p.helixAngle)) < 1e-9) {
    result.error = "helixAngle too close to 90 degrees for propertiesFromTool";
    return result;
  }

  InvoluteToothFactors f;
  if (p.propertiesFromTool) {
    f.pressureAngleTransverse = std::atan(std::tan(p.pressureAngle) / std::cos(p.helixAngle));
    f.transverseModule = p.module / std::cos(p.helixAngle);
  } else {
    f.pressureAngleTransverse = p.pressureAngle;
    f.transverseModule = p.module;
  }

  const double m = f.transverseModule;
  f.transversePitch = m * kPi;
  f.clearanceAbs = p.clearance * p.module;
  f.referenceDiameter = p.numTeeth * m;
  f.workingDiameter = m * p.numTeeth;
  f.addendumDiameter =
      f.workingDiameter + 2.0 * p.module + 2.0 * (p.shift + p.head) * p.module;
  f.rootDiameter = f.workingDiameter - 2.0 * p.module - 2.0 * f.clearanceAbs +
                   2.0 * p.shift * p.module;
  f.baseDiameter = f.referenceDiameter * std::cos(f.pressureAngleTransverse);
  f.angularPitch = 2.0 * kPi / static_cast<double>(p.numTeeth);

  const double da = f.addendumDiameter;
  const double df = f.rootDiameter;
  const double dg = f.baseDiameter;
  const double dw = f.workingDiameter;

  if (!(df > 0.0)) {
    result.error = "root diameter df=" + std::to_string(df) +
                    " is non-positive — reduce clearance/negative shift or increase teeth/module";
    return result;
  }
  if (da * da < df * df) {
    result.error = "addendum diameter da=" + std::to_string(da) +
                    " is smaller than root diameter df=" + std::to_string(df) +
                    " — invalid tooth depth (check shift/clearance/head)";
    return result;
  }
  if (da * da < dg * dg) {
    result.error = "addendum diameter da=" + std::to_string(da) +
                    " is smaller than base diameter dg=" + std::to_string(dg) +
                    " — pressure angle/addendum combination is invalid";
    return result;
  }
  if (dw * dw < dg * dg) {
    result.error = "base diameter dg=" + std::to_string(dg) +
                    " exceeds working diameter dw=" + std::to_string(dw) +
                    " — pressure angle out of range";
    return result;
  }

  f.undercutEnd = std::sqrt(da * da - df * df) / da;
  f.undercutRot = -df / dw *
                  std::tan(std::atan(
                      (2.0 * ((m * kPi) / 4.0 -
                              (f.clearanceAbs + p.module) * std::tan(f.pressureAngleTransverse))) /
                      df));

  f.involuteEnd = std::sqrt(da * da - dg * dg) / dg;
  const double dwSqTerm = std::sqrt(dw * dw - dg * dg) / dg;
  f.involuteRot1 = dwSqTerm - std::atan(dwSqTerm);
  // Upstream computes involute_rot2 twice, the second assignment
  // (`1/num_teeth * (...)`) overwriting the first (`m/d * (...)`); those two
  // forms are algebraically identical since d == num_teeth*m, so only the
  // final form is ported.
  f.involuteRot2 = (1.0 / static_cast<double>(p.numTeeth)) *
                    (kPi / 2.0 + 2.0 * p.shift * std::tan(f.pressureAngleTransverse));
  f.involuteRot = f.involuteRot1 + f.involuteRot2;
  f.angularBacklash = p.backlash / (f.referenceDiameter / 2.0);
  f.involuteStart = 0.0;
  if (dg <= df) {
    f.involuteStart = std::sqrt(df * df - dg * dg) / dg;
  }

  result.ok = true;
  result.factors = f;
  return result;
}

Point2d undercut_point(double psi, const InvoluteToothFactors& f) {
  const double df = f.rootDiameter;
  const double dw = f.workingDiameter;
  const double radiusTerm = std::sqrt(df * df / 4.0 + (df * df * std::tan(psi) * std::tan(psi)) / 4.0);
  const double angleTerm = psi - (df * std::tan(psi)) / dw;
  return {std::cos(angleTerm) * radiusTerm, std::sin(angleTerm) * radiusTerm};
}

Point2d involute_point(double phi, const InvoluteToothFactors& f) {
  const double r = f.baseDiameter / 2.0;
  return {r * std::cos(phi) + phi * r * std::sin(phi), r * std::sin(phi) - phi * r * std::cos(phi)};
}

std::vector<Point2d> undercut_points(const InvoluteToothFactors& f, int count) {
  auto pts = linspace_apply(0.0, f.undercutEnd, count,
                             [&f](double psi) { return undercut_point(psi, f); });
  const double angle = -f.undercutRot - f.angularPitch / 2.0 + f.angularBacklash / 2.0;
  return rotateAll(pts, angle);
}

std::vector<Point2d> involute_points(const InvoluteToothFactors& f, int count) {
  auto pts = linspace_apply(f.involuteStart, f.involuteEnd, count,
                             [&f](double phi) { return involute_point(phi, f); });
  const double angle = -f.involuteRot + f.angularBacklash / 2.0;
  return rotateAll(pts, angle);
}

ToothProfile one_tooth_profile(const InvoluteToothParams& params,
                                const InvoluteToothFactors& f, int sampleCount) {
  const std::vector<Point2d> l1 = undercut_points(f, sampleCount);
  const std::vector<Point2d> l2 = involute_points(f, sampleCount);

  std::vector<Point2d> u1;
  bool hasU1 = false;
  std::vector<Point2d> e1;

  if (params.undercut) {
    // UPSTREAM BUG, matched deliberately — see the header's "undercut" note.
    // `InvoluteTooth.points` computes `s = trimfunc(l1, l2[::-1])` and then
    // branches on `isinstance(s, ndarray)`. `trimfunc` returns either
    // `False` or a PYTHON LIST of two arrays, so that test is never true and
    // the trimmed result is always discarded: every gear the reference has
    // ever produced takes the `nearestpts` path. This port therefore calls
    // `nearestpts` unconditionally rather than porting a branch the
    // reference cannot reach.
    //
    // Found by the cross-check harness (scripts/gear-crosscheck), not by
    // reading: an earlier revision of this file honoured the intended
    // trim-on-crossing behaviour and disagreed with the reference on 224 of
    // 2673 swept cases, every one of them an `undercut=true` tooth.
    const NearestPtsResult np = nearestpts(l2, l1);
    hasU1 = true;
    u1 = np.undercutPrefixPlusInvolutePoint;
    e1 = np.involuteSuffix;
  } else {
    if (f.baseDiameter > f.rootDiameter) {
      const double n = norm(l2[0]);
      const Point2d blend = scaled(l2[0], f.rootDiameter / (2.0 * n));
      u1 = {blend, l2[0]};
      hasU1 = true;
      e1 = l2;
    } else {
      e1 = l2;
    }
  }

  const std::vector<Point2d> e2 = reversed(reflectAll(e1));

  ToothProfile profile;
  if (!hasU1) {
    profile.segments = {e1, {e1.back(), e2.front()}, e2};
  } else {
    const std::vector<Point2d> u2 = reversed(reflectAll(u1));
    profile.segments = {u1, e1, {e1.back(), e2.front()}, e2, u2};
  }
  return profile;
}

}  // namespace onecad::kernel::gear
