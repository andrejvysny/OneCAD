// GearOp.h — the SCHEMA §7.3 `Gear` operation (Gear Generator G1).
//
// A generated gear body. Unlike every other feature op in this directory it
// has NO HOST: it mints `body_<opId>` (decision D1) from a typed recipe block
// and a placement, contributing one `created` body event and an EMPTY
// element-map delta (nothing pre-existing is tracked, so `apply_history` is
// not involved at all).
//
// PLACEMENT is exactly one of two modes, validated both ways:
//
//   face   Hole's semantics, reused rather than reinvented — the frozen
//          world `point` is re-projected onto the resolved face each regen and
//          refused past 1e-3 mm, the gear's axis is the face's INWARD normal
//          (so the body grows into material), and the ref resolves through the
//          §10 ladder with a `NeedsRepair` (never a wrong bind) when it no
//          longer binds.
//   frame  An explicit frozen `{origin, axis, xDir}`. No refs, no
//          reprojection, no host. `xDir` fixes the angular PHASE, which is what
//          makes a pair of gears mesh.
//
// REFERENCEABILITY (SCHEMA §7.3). The element map is mint-on-demand — entries
// exist only for elements an op referenced or a user promoted — so a gear
// needs no suppression machinery to keep its teeth unreferenceable: it simply
// never mints for them. What it DOES need, and what `gear_body_op_ids` below
// exists for, is to stop a user promoting a tooth face by hand through
// `AcquireElementIds`. A tooth's identity changes with the tooth count, so
// such an id would mis-resolve after the next parameter edit — the exact
// silent-wrong-bind the ladder exists to prevent.
//
// Publication takes `single_solid_policy("Gear", TierB)` — the FULL audit
// including self-interference. A generated profile is precisely where a bad
// parameter combination yields a plausible-looking but self-intersecting
// solid, so this op does not take the lighter tier.

#pragma once

#include <set>
#include <string>

#include <nlohmann/json.hpp>

#include "ops/OpTypes.h"

namespace onecad::ops {

/// Execute one `Gear` op. Mints `body_<op_id>` on success; every failure is a
/// graded refusal (`OP_FAILED` / `GEOMETRY_INVALID` naming the parameter at
/// fault) or a `NeedsRepair` state for an unresolvable placement.
OpOutcome execute_gear(OpContext& ctx, const nlohmann::json& op, const std::string& op_id);

/// The op ids of every `Gear` operation in `plan`, i.e. the ops whose minted
/// bodies carry generated tooth geometry.
///
/// Used by `AcquireElementIds` to refuse promoting a gear's tooth face to a
/// persistent ElementId. Derived from the plan rather than stored on the body
/// because the body id IS `body_<opId>` (D1), so the mapping is total and needs
/// no extra state to survive a reopen.
std::set<std::string> gear_body_op_ids(const nlohmann::json& plan);

/// True when `body_id` was minted by a `Gear` op present in `plan`.
bool is_generated_gear_body(const nlohmann::json& plan, const std::string& body_id);

}  // namespace onecad::ops
