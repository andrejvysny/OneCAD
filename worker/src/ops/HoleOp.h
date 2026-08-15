// HoleOp.h — machined-hole executor (WP-C T3; SCHEMA §7.3 `Hole`, 2026-08-03).
//
// New v2 op — no OneCAD-CPP analogue. Simple / counterbore / countersink are ONE
// parametric feature: the profile is a param, so switching a counterbore to a
// countersink is a param edit that keeps the record's identity and every
// downstream ref bound to it.
//
// PIPELINE
//   1. host body — `params.targetBodyId`, else the first whole-body input ref
//      (the ShellOp fallback chain).
//   2. host FACE — the `inputs[]` ref of kind `face` (the wire's second semantic
//      ref), resolved on the EXACT predecessor snapshot (Invariant 3):
//        a. partition-tracked (its ElementId was minted against this body), then
//        b. the descriptor+anchor ladder — the SAME path Fillet's edges use.
//      Resolving via NEITHER ⇒ NeedsRepair STATE (never a wrong bind, never Err).
//   3. plane + OUTWARD normal via `planar_face_plane_normal`. A non-planar
//      resolved face is a recoverable, NAMED OP_FAILED — a hole needs a seat.
//   4. `params.point` (world, frozen at authoring) is RE-PROJECTED onto the
//      resolved face and fenced at 1e-3 mm; it must also land INSIDE the face's
//      boundary. Both failures are named OP_FAILED. This is what makes the frozen
//      point safe: a face that slid within its own plane keeps the hole put, while
//      a face that moved out from under the point fails loudly instead of drilling
//      through empty space.
//   5. axis = the face's INWARD normal (−outward). Never stored — a stored axis
//      and a re-resolved face can disagree.
//   6. tool solid (`HoleTool.h`) → ONE `checked_boolean` Cut against the host.
//   7. publish MODIFIED host (id preserved, OCCT history folded into its
//      partition) — SCHEMA §7.3: "lineage = modified on targetBodyId, nothing
//      minted". A Hole never creates, never splits, never renames a body.
//      Absent `resultPolicyVersion` preserves the legacy split-host residual;
//      literal V2 requires exactly one connected solid and returns
//      `HOLE_DISJOINT_RESULT` otherwise. Future/malformed versions refuse by name.
//
// The op is preview-capable for free: `PreviewOp` and `ExecutePlan` share
// `PlanExecutor::execute_candidate_op`, so preview == commit by construction.
#pragma once

#include <string>

#include "nlohmann/json.hpp"
#include "ops/OpTypes.h"

namespace onecad::ops {

OpOutcome execute_hole(OpContext& ctx, const nlohmann::json& op, const std::string& op_id);

}  // namespace onecad::ops
