// TimingMath.h — GT / HTD timing-belt pulley tooth profile math.
//
// Ported from freecad.gears v1.3.0 (GPL-3.0,
// https://github.com/looooo/freecad.gears), `freecad/gears/timinggear.py`
// class `TimingGear` — note this profile lives in the FEATURE layer upstream,
// not in `pygears`, so the port separates the arithmetic (here) from the
// B-rep construction (the op layer, Gear Generator plan phase G2) that
// upstream interleaves. GPLv3 §13 permits combination with this AGPL-3.0
// project (Gear Generator implementation plan §11).
//
// Deliberately NO OCCT — see GearTypes.h. Angles are RADIANS; lengths are
// millimetres.
//
// UNLIKE the involute recipe, a timing-belt tooth is built from TANGENT
// CIRCULAR ARCS ONLY — there is no sampled/splined curve and therefore no
// sample-count accuracy knob. Every arc is emitted in three-point form
// (`Arc3`), which is exactly what `GC_MakeArcOfCircle` consumes, so the G2
// wire builder is a direct translation with no approximation step. This is
// why timing pulleys are cheap relative to the involute family and why the
// implementation plan schedules them ahead of the helical-sweep risk peak.
//
// The seven profile rows are engineering-standard belt-section data (GT2/3/5/8
// and HTD 3M/5M/8M), ported as facts per the plan's §11 licensing analysis.

#pragma once

#include <string>
#include <vector>

#include "kernel/gear/GearTypes.h"

namespace onecad::kernel::gear {

/// The belt sections this recipe supports, matching upstream's `type`
/// enumeration and the SCHEMA `profile` enum (Gear Generator plan §5.5).
enum class TimingProfile {
  Gt2,
  Gt3,
  Gt5,
  Gt8,
  Htd3,
  Htd5,
  Htd8,
};

/// One row of the normative section table (upstream `TimingGear.data`).
/// `offset == 0` selects the HTD tooth construction (a symmetric four-arc
/// tooth); a non-zero `offset` selects the GT construction (six arcs, with
/// the second arc's centre displaced in x). Which branch a row takes is a
/// property of the STANDARD, not a user choice — see `one_tooth_arcs`.
struct TimingProfileData {
  double pitch = 0.0;   ///< belt pitch (mm) — tooth-to-tooth arc length at the pitch line.
  double u = 0.0;       ///< radial difference between pitch diameter and gear head.
  double h = 0.0;       ///< radial height of the tooth.
  double bigH = 0.0;    ///< upstream `H`; carried for completeness/read-out, unused by the profile math.
  double r0 = 0.0;      ///< radius of the first arc (upstream `r_12`).
  double r1 = 0.0;      ///< radius of the second arc (upstream `r_23`).
  double rs = 0.0;      ///< radius of the third arc (upstream `r_34`).
  double offset = 0.0;  ///< x-offset of the second arc's midpoint; 0 => HTD branch.
};

/// The normative table row for `profile`. Total function — every enumerator
/// has a row.
TimingProfileData timing_profile_data(TimingProfile profile);

/// Wire-name mapping for the SCHEMA `profile` enum ("gt2", "htd5", …).
std::string timing_profile_name(TimingProfile profile);

/// Parse a SCHEMA `profile` value. Returns false for an unknown name rather
/// than defaulting to a section — silently substituting a different belt
/// standard would produce a pulley that does not mesh.
bool timing_profile_from_name(const std::string& name, TimingProfile& out);

struct TimingToothParams {
  TimingProfile profile = TimingProfile::Gt2;
  int numTeeth = 15;  ///< z, >= 3 (upstream's constraint minimum).
};

/// Derived quantities. `pitchRadius` follows upstream's own convention per
/// branch and is NOT uniform between them — see the field comment; it is
/// exposed because the G2 chip surfaces it as a read-only computed value and
/// the asymmetry must be documented once, here, rather than rediscovered.
struct TimingFactors {
  TimingProfileData data;
  /// Upstream `rp`. GT branch: `pitch·z/(2π)` — the true pitch radius.
  /// HTD branch: `pitch·z/(2π) − u` — the pitch radius ALREADY reduced by
  /// `u`, because that branch never forms a separate `r5`. Callers wanting a
  /// comparable value across branches should use `pitchRadiusTrue`.
  double rp = 0.0;
  /// `pitch·z/(2π)` for both branches — the value to report to a user.
  double pitchRadiusTrue = 0.0;
  double pitchDiameter = 0.0;  ///< 2·pitchRadiusTrue.
  double outerRadius = 0.0;    ///< pitchRadiusTrue − u, the tooth-head radius.
  double halfToothAngle = 0.0;  ///< upstream `phi5` = π/z.
};

struct TimingFactorsResult {
  bool ok = false;
  std::string error;
  TimingFactors factors;
};

/// Validate + compute. Refuses (rather than returning NaN) when the tooth
/// count is below the standard's minimum or when the resulting pulley is too
/// small for the section's own tooth to fit — the fail-closed rule.
TimingFactorsResult compute_timing_factors(const TimingToothParams& params);

/// One tooth's profile as an ordered, end-to-end continuous arc chain,
/// spanning exactly one angular pitch (2π/z).
///
/// Two structural invariants hold for every profile and tooth count, and are
/// asserted by the unit test independently of any pinned oracle value:
///   1. CONTINUITY — `arcs[i].end == arcs[i+1].start`.
///   2. TOOTH WRAP — `rotate(arcs.back().end, -2π/z) == arcs.front().start`,
///      which is what makes the z-fold replication close into one wire.
/// A transcription error in the arc algebra breaks one of these long before
/// it produces a plausible-but-wrong pulley.
///
/// Arc count is 6 for GT sections and 4 for HTD sections (HTD's `r0 == r1`
/// and zero offset collapse two arc pairs). The tooth is centred on +Y.
struct TimingToothResult {
  bool ok = false;
  std::string error;
  std::vector<Arc3> arcs;
};
TimingToothResult one_tooth_arcs(const TimingToothParams& params, const TimingFactors& factors);

}  // namespace onecad::kernel::gear
