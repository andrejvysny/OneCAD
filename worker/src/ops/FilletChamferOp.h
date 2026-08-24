// FilletChamferOp.h — real OCCT Fillet / Chamfer executors (W-WP6).
//
// Ports OneCAD-CPP RegenerationEngine.cpp buildFillet (:1285-1349) and buildChamfer
// (:1351-1429):
//   * target body from the op's semantic refs (a BodyRef in the C++ engine; here
//     the edge refs' shared primary.bodyId);
//   * radius guard `< authoring_resolution() (1e-3)` → "…radius/distance too small"
//     (OP_FAILED);
//   * per-edge: fillet `Add(radius, edge)`; chamfer `Add(radius, radius, edge,
//     refFace)` (equal-leg, refFace = first ancestor face via MapShapesAndAncestors);
//   * Build + IsDone; a radius-too-large / OCCT failure → recoverable OP_FAILED /
//     GEOMETRY_INVALID (SCHEMA §8; session intact).
//
// CHAMFER MODES (SCHEMA §7.3) are discriminated by PRESENCE, never by precedence:
//   * `angleDeg` present ⇒ distance-angle, `AddDA(radius, angleDeg→rad, edge,
//     refFace)`;
//   * else `distance2` present ⇒ two-distance, `Add(radius, distance2, edge,
//     refFace)`;
//   * else equal-leg, `Add(radius, radius, edge, firstAncestorFace)`.
// Both asymmetric forms measure `radius` on the SAME deterministic reference face —
// the adjacent face with the smaller resolved face ordinal (`reference_face`) — so a
// replayed document cannot depend on which mode authored it. `angleDeg` is DEGREES on
// the wire and radians at the kernel; it is Chamfer-only and mutually exclusive with
// `distance2` (both violations are refused BY NAME, `OP_FAILED`, session intact).
// Static validation is `0 < angleDeg < 180` exclusive and deliberately loose: the true
// ceiling depends on the dihedral, so geometric feasibility stays the Build/IsDone net.
//
// EDGE RESOLUTION (SCHEMA §10 / Invariant 3): each edge is resolved through the
// resolution ladder against the EXACT predecessor snapshot (the target body BEFORE
// the fillet), using the per-edge semantic ref's descriptor + anchor evidence. An
// edge ref that no longer resolves ⇒ NeedsRepair STATE (never a wrong bind, never
// an Err) — the op does not run and the step prepares m−1. `chainTangentEdges` is
// metadata: the tangent-chain is already expanded into explicit refs upstream
// (OneCAD-CPP FilletChamferTool.cpp), so the engine iterates the given refs verbatim.
#pragma once

#include <string>

#include "nlohmann/json.hpp"
#include "ops/OpTypes.h"

namespace onecad::ops {

OpOutcome execute_fillet(OpContext& ctx, const nlohmann::json& op, const std::string& op_id);
OpOutcome execute_chamfer(OpContext& ctx, const nlohmann::json& op, const std::string& op_id);

}  // namespace onecad::ops
