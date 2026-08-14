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

#include <string>
#include <vector>

#include <TopoDS_Shape.hxx>
#include <gp_Ax2.hxx>

#include "kernel/gear/InvoluteMath.h"

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

struct GearBuildResult {
  bool ok = false;
  /// Populated on failure. A recoverable, human-facing reason naming the
  /// parameter at fault where one is identifiable — never a bare OCCT string.
  std::string error;
  TopoDS_Shape shape;
  GearComputed computed;
};

/// Build the gear solid positioned on `frame`: the gear's axis is
/// `frame.Direction()`, its centre is `frame.Location()`, and the body runs
/// from that point along the axis for `height`.
///
/// Every failure path is a graded refusal — an invalid parameter combination,
/// a sampler domain error, a spline that will not interpolate, a wire that
/// will not close, a boolean that fails. None of them returns a partial or
/// null shape as though it were a result.
GearBuildResult build_gear_solid(const GearBuildSpec& spec, const gp_Ax2& frame);

}  // namespace onecad::ops::gear
