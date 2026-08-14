// TimingTMath.h — T-profile (trapezoidal) timing-belt pulley tooth math.
//
// Ported from freecad.gears v1.3.0 (GPL-3.0), `freecad/gears/timinggear_t.py`
// class `TimingGearT`. GPLv3 §13 permits combination with this AGPL-3.0
// project (Gear Generator implementation plan §11).
//
// Deliberately NO OCCT — see GearTypes.h. Angles are RADIANS; lengths are
// millimetres. The tooth is centred on +X and spans exactly one angular
// pitch.
//
// TWO DELIBERATE DIVERGENCES FROM UPSTREAM, both improvements the
// implementation plan called for:
//
//  1. CLOSED-FORM FLANK ENDPOINTS. Upstream locates the two flank endpoints
//     with `scipy.optimize.minimize` over `(|line(s)| - R)^2` — a numeric
//     minimisation of a squared distance whose true answer is just the
//     intersection of a line with a circle. This port solves that quadratic
//     directly. Besides removing the scipy dependency, it is strictly MORE
//     accurate: the squared objective is flat at its minimum, so upstream's
//     minimiser converges only to ~1e-8, and up to 5e-6 mm of error was
//     measured across this port's own test cases (recorded in
//     test_gear_math_timing_t.cpp). Root selection is `-b + sqrt(disc)` for
//     both circles, which is justified rather than fitted — see
//     `solve_flank_parameter` in the .cpp.
//
//  2. NO FILLETS HERE. Upstream inserts head/root fillets by hard-coded EDGE
//     INDEX with a pop-and-retry fallback, then slices the edge list
//     (`edges[2:-1]`). Both the index-based site selection and the extra
//     scaffolding edges it needs are exactly the upstream fragility the plan
//     refuses to port (§2.4). This layer emits the UNFILLETED four-segment
//     tooth; fillets are applied at the OCCT layer by the `Fillet2d` wrapper,
//     which identifies sites geometrically (position/tangent), on the
//     assembled closed wire where every junction has both neighbours.
//
// Fillet radii for this recipe are ABSOLUTE MILLIMETRES, not multiples of a
// module — a documented exception to the other recipes' convention (plan
// §5.5). Upstream's property help text claims "radius = head_fillet x
// module", but `TimingGearT` has no module property and passes the value
// straight through as a radius; the help text is wrong, the code is the
// contract.

#pragma once

#include <string>
#include <vector>

#include "kernel/gear/GearTypes.h"

namespace onecad::kernel::gear {

/// Schema-facing names differ from upstream's terse ones; the mapping is
/// noted per field so a reader can follow the port.
struct TimingTParams {
  double pitch = 5.0;             ///< belt pitch (mm), > 0.
  int numTeeth = 15;              ///< z, >= 3.
  double toothHeight = 1.2;       ///< radial height of the tooth (mm), > 0. Upstream `tooth_height`.
  double pitchLineOffset = 0.6;   ///< radial distance from tooth head to pitch circle (mm). Upstream `u`.
  double flankAngle = 40.0 * 3.14159265358979323846 / 180.0;  ///< total included flank angle α, radians. Upstream `alpha`.
  double backlash = 0.0;          ///< arc length at the pitch circle by which tooth thickness is reduced (mm), >= 0.
};

struct TimingTFactors {
  double pitchRadius = 0.0;   ///< r_p = pitch·z/(2π).
  double pitchDiameter = 0.0;
  double angularPitch = 0.0;  ///< upstream `gamma_0` = pitch/r_p = 2π/z.
  double halfToothAngle = 0.0;  ///< upstream `gamma_1` = gamma_0/4 − backlash/r_p.
  double tipRadius = 0.0;     ///< r_p − pitchLineOffset (tooth head).
  double rootRadius = 0.0;    ///< r_p − pitchLineOffset − toothHeight.
};

struct TimingTFactorsResult {
  bool ok = false;
  std::string error;
  TimingTFactors factors;
};

/// Validate + compute. Refuses on a non-positive root radius (tooth deeper
/// than the pulley) and on a backlash large enough to consume the whole
/// tooth thickness, rather than emitting an inside-out profile.
TimingTFactorsResult compute_timing_t_factors(const TimingTParams& params);

/// One tooth as four end-to-end continuous segments, spanning exactly one
/// angular pitch:
///   [0] line  — rising flank, root radius -> tip radius
///   [1] arc   — tooth head (centred on the axis, radius = tipRadius)
///   [2] line  — falling flank, tip radius -> root radius
///   [3] arc   — tooth space (centred on the axis, radius = rootRadius),
///               ending at the next tooth's flank start
///
/// The same two structural invariants as the GT/HTD recipe hold and are
/// asserted independently of any pinned value: chain continuity, and
/// `rotate(segments.back().end, -angularPitch) == segments.front().start`.
///
/// Refuses when the flank never reaches the root or tip circle — possible
/// for a deep tooth with a wide flank angle, where upstream's minimiser would
/// silently return a nonsense stationary point.
struct TimingTToothResult {
  bool ok = false;
  std::string error;
  std::vector<ProfileSegment> segments;
};
TimingTToothResult one_tooth_segments(const TimingTParams& params, const TimingTFactors& factors);

}  // namespace onecad::kernel::gear
