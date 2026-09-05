// GearTool.h — the SOLID of a generated gear (SCHEMA §7.3 `Gear`).
//
// Split out of GearOp.cpp on the HoleTool precedent: the geometry here is a
// pure function of its arguments, so it is separable from the ref-resolution
// / minting / publish plumbing (which is all session state) and testable
// without a Session.
//
// PIPELINE (involute external spur, the G1 recipe):
//
//   kernel/gear sampler  ->  one tooth as ordered 2D point runs
//   replicate            ->  z teeth, each rotated by k·(2π/z)
//   edges                ->  2-point run = line, >2-point run = interpolated
//                            B-spline (kernel/geometry/BSpline.h)
//   wire -> face -> prism along the axis -> optional bore cuts
//
// EXACT REPLICATION. Each tooth's points are re-evaluated analytically at its
// own rotation angle rather than copying a transformed shape. That is
// stronger than the `TopLoc_Location` approach the plan asks for and much
// stronger than the reference's `transformGeometry` (which re-parameterises
// every copy and invites tolerance creep). The join between tooth k and k+1
// is bit-identical by construction because both sides evaluate the SAME
// expression — and the wrap-around join uses angle 0 rather than z·(2π/z),
// since `cos(2π)` is not exactly 1 in binary floating point and the wire
// would otherwise fail to close.
//
// THE ROOT LAND IS A CHORD, NOT AN ARC — matching the reference, which emits
// a 2-point run there and therefore a straight `LineSegment`. A true root
// circle arc would be marginally more correct (the sagitta is tens of
// microns on a typical gear), but parity with the reference is the stated
// goal for the ported math, and this is an approximation choice rather than a
// defect. Recorded here so it is a known, deliberate difference from an ideal
// involute gear rather than a surprise found later at a machine.

#pragma once

#include <optional>
#include <string>
#include <vector>

#include <TopoDS_Shape.hxx>
#include <gp_Ax2.hxx>

#include "kernel/gear/InvoluteMath.h"
#include "util/Cancel.h"

namespace onecad::ops::gear {

/// Everything the builder needs. Units are mm / radians; the op layer
/// converts from the wire's degrees before calling in.
struct GearBuildSpec {
  kernel::gear::InvoluteToothParams involute;
  int sampleCount = 20;   ///< spline samples per curve segment, >= 2.
  double height = 5.0;    ///< extrusion length along the axis, > 0.

  bool axleHole = false;
  double axleHoleDiameter = 10.0;

  bool offsetHole = false;
  double offsetHoleDiameter = 10.0;
  double offsetHoleOffset = 10.0;  ///< radial distance of the secondary bore from the axis.
};

/// Read-only values the op returns in its result DTO and the chip displays.
/// Never stored as parameters (SCHEMA §7.3 header rule).
struct GearComputed {
  double pitchDiameter = 0.0;
  double addendumDiameter = 0.0;
  double rootDiameter = 0.0;
  double baseDiameter = 0.0;
  double transverseModule = 0.0;
  double transversePitch = 0.0;
  double angularBacklashDeg = 0.0;
};

/// A gear parameter outside the bounds SCHEMA §7.3 sets for it
/// (kernel-hardening WP-I): the offending name, its value, and the range it
/// left. Set ALONGSIDE `error` so the op layer can attach `reasonCode`
/// `GEAR_PARAM_OUT_OF_RANGE` and `evidence {param, value, min, max}` without
/// re-parsing the message. The bounds are COST bounds — one B-spline is fitted
/// per flank and the bore cut runs against `2·teeth` such faces, so
/// `(teeth 400, sampleCount 1024)` is jointly a liveness kill while each value
/// alone is harmless.
struct GearParamBound {
  std::string param;  ///< "teeth" | "sampleCount" | "height"
  double value = 0.0;
  double min = 0.0;
  double max = 0.0;
};

struct GearBuildResult {
  bool ok = false;
  /// Populated on failure. A recoverable, human-facing reason naming the
  /// parameter at fault where one is identifiable — never a bare OCCT string.
  std::string error;
  /// Set when `error` is a BOUNDS refusal (WP-I). Absent for every other
  /// failure, including the in-range ones whose wording is unchanged.
  std::optional<GearParamBound> out_of_range;
  /// Set when the build stopped on the cancel token rather than failing.
  bool cancelled = false;
  TopoDS_Shape shape;
  GearComputed computed;
};

/// Build the gear solid positioned on `frame`: the gear's axis is
/// `frame.Direction()`, its centre is `frame.Location()`, and the body runs
/// from that point along the axis for `height`.
///
/// Every failure path is a graded refusal — an out-of-range parameter, an
/// invalid parameter combination, a sampler domain error, a spline that will
/// not interpolate, a wire that will not close, a boolean that fails. None of
/// them returns a partial or null shape as though it were a result.
///
/// `cancel` (optional) is polled ONCE PER TOOTH while the profile wire is
/// assembled — the part whose cost scales with `teeth · sampleCount`. The bore
/// boolean itself stays uninterruptible (recorded limit, SCHEMA §7.3). A
/// cancelled build returns `ok=false`, `cancelled=true` and an empty shape.
GearBuildResult build_gear_solid(const GearBuildSpec& spec, const gp_Ax2& frame,
                                 const onecad::CancelToken* cancel = nullptr);

}  // namespace onecad::ops::gear
