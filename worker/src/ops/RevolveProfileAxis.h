// RevolveProfileAxis.h — the profile-vs-axis preflight Revolve runs BEFORE any
// kernel work (SCHEMA §7.3 Revolve, "The profile must lie on ONE side of its axis").
#pragma once

#include <optional>

#include <TopoDS_Face.hxx>
#include <gp_Ax1.hxx>

#include "ops/OpTypes.h"

namespace onecad::ops {

// std::nullopt = the profile may be revolved. Otherwise a named refusal:
//   REVOLVE_PROFILE_CROSSES_AXIS — boundary material on both sides of the axis
//     beyond the on-axis tolerance (Precision::Confusion);
//   REVOLVE_PROFILE_NEAR_AXIS — some boundary edge's true closest approach to the
//     axis is off-axis (≥ Confusion) yet below the authoring resolution.
// A profile that touches the axis within Confusion is legal and unchanged.
std::optional<OpOutcome> classify_revolve_profile(const TopoDS_Face& profile, const gp_Ax1& axis);

}  // namespace onecad::ops
