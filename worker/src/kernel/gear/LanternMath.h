// LanternMath.h — lantern (pin/roller) gear tooth profile math.
//
// Ported from freecad.gears v1.3.0 (GPL-3.0),
// `freecad/gears/lanterngear.py` class `LanternGear`. GPLv3 §13 permits
// combination with this AGPL-3.0 project (Gear Generator plan §11).
//
// Deliberately NO OCCT — see GearTypes.h. Angles are RADIANS; lengths mm.
//
// A lantern gear meshes with a rack or chain of cylindrical rollers, so its
// flank is the involute of the pitch circle OFFSET outward by the roller
// radius — a parallel (offset) curve, not a plain involute:
//     x(φ) = r₀·(cos φ + φ·sin φ) − r_r·sin φ
//     y(φ) = r₀·(sin φ − φ·cos φ) + r_r·cos φ
// sampled over [φ_min, φ_max] and interpolated. `φ_max` is closed form;
// `φ_min` (where the offset curve meets the tooth root) needs a 1-D solve.
//
// DELIBERATE DIVERGENCE — the φ_min solve is BRACKETED, upstream's is not.
// Upstream calls `scipy.optimize.root` from the heuristic start
// `(φ_max + 4·r_r/r₀)/5` on an objective that has a SPURIOUS ROOT AT φ = 0:
// substituting φ=0 gives −2r₀ + 2r₀ = 0 identically, for every parameter set.
// Nothing in upstream prevents that solver from returning the degenerate
// root — which would collapse the whole flank to a point.
//
// This port removes the possibility rather than relying on a lucky start.
// Differentiating the objective in closed form gives
//     g′(φ) = 2·(1 − cos φ)·(r₀·φ − r_r)
// (verified numerically against a finite difference), so g decreases on
// (0, r_r/r₀), attains its minimum at φ* = r_r/r₀ with g(φ*) < 0 = g(0), and
// increases without bound after. Therefore the meaningful root is UNIQUE on
// (φ*, ∞) and φ = 0 is excluded by construction. `solve_phi_min` brackets on
// that interval and uses safeguarded Newton — guaranteed to converge to the
// right root for any valid parameters, with no magic initial guess. Agreement
// with upstream's scipy answer is ~1e-13 across the test cases.

#pragma once

#include <string>
#include <vector>

#include "kernel/gear/GearTypes.h"

namespace onecad::kernel::gear {

struct LanternParams {
  double module = 1.0;        ///< m, > 0.
  int numTeeth = 15;          ///< z, >= 3.
  double rollerRadius = 1.0;  ///< r_r, > 0 — the mating roller/pin radius. Upstream `bolt_radius`.
  double head = 0.0;          ///< ×module additional head length.
  int sampleCount = 10;       ///< samples per flank, >= 2. Upstream `num_profiles`.
};

struct LanternFactors {
  double pitchRadius = 0.0;  ///< r₀ = m·z/2.
  double maxRadius = 0.0;    ///< r_max = r₀ + r_r + head·m.
  double phiMin = 0.0;       ///< flank start parameter (bracketed solve).
  double phiMax = 0.0;       ///< flank end parameter, closed form.
  double angularPitch = 0.0;  ///< 2π/z.
};

struct LanternFactorsResult {
  bool ok = false;
  std::string error;
  LanternFactors factors;
};

/// Validate + compute, including the bracketed φ_min solve. Refuses when the
/// roller radius is non-positive (the bracket degenerates), when the head
/// makes `r_max` fall inside the pitch circle (φ_max has no real value), or
/// when the solved φ_min does not leave a usable flank span.
LanternFactorsResult compute_lantern_factors(const LanternParams& params);

/// One tooth as four end-to-end continuous segments spanning one angular
/// pitch, in traversal order:
///   [0] interpolated — trailing (mirrored) flank, outer end -> root
///   [1] arc          — tooth root, through (r₀ − r_r, 0)
///   [2] interpolated — leading flank, root -> outer end
///   [3] arc          — tooth tip about the axis, to the next tooth's start
///
/// NOTE the ordering differs from upstream, deliberately. Upstream hands
/// `Part.Wire` the edge list `[w_2, arc, w_1, l]` and relies on FreeCAD to
/// reorder and reverse edges into a connected wire. This port emits the chain
/// already correctly ordered and oriented, so the same continuity and
/// tooth-wrap invariants as every other recipe here hold and are asserted:
/// `segments[i].end == segments[i+1].start`, and
/// `rotate(segments.back().end, -2π/z) == segments.front().start`.
struct LanternToothResult {
  bool ok = false;
  std::string error;
  std::vector<ProfileSegment> segments;
};
LanternToothResult one_tooth_segments(const LanternParams& params, const LanternFactors& factors);

/// The offset-involute flank point at parameter `phi`. Exposed for the
/// oracle test, which checks sampled points against the closed form directly.
Point2d lantern_flank_point(double phi, double pitchRadius, double rollerRadius);

}  // namespace onecad::kernel::gear
