// TransformOp.h — the `TransformBody` op (SCHEMA §7.3; WP-B W0).
//
// Rigid placement of one or more bodies: translate + rotate about a FROZEN pivot,
// optionally as copies. The light multi-part "position parts for fit-check"
// primitive — parametric, ONE cumulative record per placement intent.
//
// Normative evaluation order: `X' = T ∘ R(center, axis, angleDeg) · X` — rotate
// about the pivot FIRST, then translate. (`R`-then-`T` and `T`-then-`R` differ for
// any non-central pivot, so the order is pinned by test_transform_body.)
//
// Lineage (SCHEMA §7.3):
//   * `copy: false` — the FIRST body-level op with MODIFY lineage. Each target
//     keeps its BodyId and emits `modified`; the BRepBuilderAPI_Transform history
//     is kept alive so `ElementMapPartition::apply_history` rebinds every tracked
//     element at ladder level 1 (unique image ⇒ zero descriptor scoring), and
//     `apply_placement` additionally migrates each entry's stored anchor.
//   * `copy: true` — the sources are preserved (no body event) and the copies mint
//     under the §2 N-body rule: one target ⇒ `body_<opId>`, N > 1 ⇒
//     `body_<opId>:<k>` with `k` the target's index in `params.targets`.
#pragma once

#include <string>

#include "nlohmann/json.hpp"
#include "ops/OpTypes.h"

namespace onecad::ops {

OpOutcome execute_transform_body(OpContext& ctx, const nlohmann::json& op,
                                 const std::string& op_id);

}  // namespace onecad::ops
