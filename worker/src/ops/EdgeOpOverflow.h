// EdgeOpOverflow.h — opt-in "Auto overflow" fallback for Fillet / Chamfer
// (SCHEMA §7.3, 2026-11-01). Runs ONLY after the native build has failed, on a
// record that carries `overflow: "auto"`; wraps kernel::fillet::build_overflow.
#pragma once

#include <optional>
#include <string>
#include <vector>

#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>

#include "kernel/fillet/EdgeContour.h"
#include "ops/OpTypes.h"

namespace onecad::ops {

// Kernel call + publication. Returns the outcome to use: the published overflow
// result, the cancelled outcome, or `native_failure` with an
// EDGE_OP_OVERFLOW_UNSUPPORTED error diagnostic appended. `std::nullopt` is never
// returned for a kernel refusal; it is reserved for "no usable target body".
std::optional<OpOutcome> try_edge_op_overflow(
    OpContext& ctx, const std::string& target_id, const std::string& op_id,
    kernel::fillet::EdgeOpMode mode, const TopoDS_Edge& seed, double value, double value2,
    const TopoDS_Face& reference_face, const OpOutcome& native_failure);

// Eligibility gate + request mapping used by the Fillet/Chamfer ops. Returns the
// outcome to use in place of `native_failure`.
OpOutcome edge_op_overflow_fallback(
    OpContext& ctx, const std::string& target_id, const std::string& op_id,
    kernel::fillet::EdgeOpMode mode, bool distance_angle, bool two_distance, double radius,
    double distance2, const std::vector<TopoDS_Edge>& seeds,
    const std::vector<TopoDS_Face>& seed_reference_faces, OpOutcome native_failure);

}  // namespace onecad::ops
