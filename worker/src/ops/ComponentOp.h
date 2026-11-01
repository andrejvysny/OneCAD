// ComponentOp.h — placed-component executor family (Component Library
// WP-0.2/WP-1.2; SCHEMA §7.3 `PlaceComponent`/`DetachComponent`, spec
// §3.1/§3.4). New v2 op family, no OneCAD-CPP analogue.
//
// SCOPE:
//   * `source.kind` is one of four: `generator` (built here), `embedded` and
//     `document` (a baked solid read from the wire-only `source.path` Rust
//     injects), and `profile` (an embedded planar face re-extruded to
//     `params.length`). Unknown kinds refuse with `UNSUPPORTED`.
//   * `generator` dispatches on `source.generatorId` through
//     `ComponentGenerators.h`. The catalog is ONE family (`iso4762`) by
//     decision, not by omission: the other families were removed with the
//     seed catalog's SEED_VERSION 4, and re-adding one needs a generator, a
//     table, a seed and a fixture. `source.params.thread` (M2-M12),
//     `.length` and `.thread_detail` (cosmetic/simplified/modeled) are the
//     free params; absent ones default to M6 / 20 mm / cosmetic.
//   * GENERATOR VERSION PIN: each registered family declares the one
//     `generatorVersion` it builds. A record whose version is absent or
//     differs is refused with `UNSUPPORTED` before any geometry is built,
//     because rebuilding a frozen record with different code would silently
//     change a document's geometry on regen.
//   * `PlaceComponent`'s `mate` is RE-SEATED here on every execute/regen
//     against the resolved target geometry (`ComponentMateSolver.h`); an
//     unresolvable or ambiguous mate halts with a named NeedsRepair item
//     rather than guessing. Without a mate, `placement` is used as authored.
//   * `SetComponentParams`/`ReplaceComponent` have NO C++ dispatch (they fall
//     through to `UNSUPPORTED` in `PlanExecutor.cpp`): both are in-place edits
//     of `PlaceComponentParams` at the Rust layer, not distinct wire ops (see
//     `record.rs`'s `DetachComponentParams` doc comment for why only
//     `DetachComponent` earned its own `KnownOperation` variant).
//
// PIPELINE (shared by both `execute_place_component` and
// `execute_detach_component` — see `ComponentOp.cpp`'s
// `resolve_source_and_publish`; the two ops build IDENTICAL geometry, differing
// only in what the RECORD's params carry):
//   1. validate `source.kind`; for `generator`, a non-empty `generatorId` and a
//      supported `generatorVersion`.
//   2. obtain the solid (blob read, profile extrusion, or generator). For
//      `iso4762`: build the table-driven SHCS blank: head (cylinder, dk×k) fused to
//      shank (cylinder, d×l) via a checked Union boolean, origin at the
//      head-underside seating plane (the natural mate seat for a hole);
//      then cut `thread_detail` geometry from the shank per WP-2.5.
//   3. apply `placement` (`translate`, `rotate`) — SAME normative order as
//      TransformBody: `X' = T ∘ R(center, axis, angleDeg) · X`.
//   4. publish through `single_solid_policy` (spec §9 — a component resolves
//      to exactly one solid in v1) as a NewBody (`body_<opId>`).
//
// Both ops are preview-capable for free: `PreviewOp` and `ExecutePlan` share
// `PlanExecutor::execute_candidate_op`, so preview == commit by construction.
#pragma once

#include <string>

#include "nlohmann/json.hpp"
#include "ops/OpTypes.h"

namespace onecad::ops {

OpOutcome execute_place_component(OpContext& ctx, const nlohmann::json& op,
                                  const std::string& op_id);

OpOutcome execute_detach_component(OpContext& ctx, const nlohmann::json& op,
                                   const std::string& op_id);

}  // namespace onecad::ops
