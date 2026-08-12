// ComponentOp.h — placed-component executor family (Component Library
// WP-0.2/WP-1.2; SCHEMA §7.3 `PlaceComponent`/`DetachComponent`, spec
// §3.1/§3.4). New v2 op family, no OneCAD-CPP analogue.
//
// SCOPE (deliberately reduced, not a different shape — see the
// implementation plan's phase boundaries):
//   * `source.kind` MUST be `"generator"`. `embedded` reaches the worker once
//     WP-1.3 wires the wire-only blob-path injection (mirrors ImportStep's
//     `inject_import_path`); `document` lands in P3.
//   * The generator builds an ISO 4762 socket-head cap screw solid (head
//     fused to shank), TABLE-DRIVEN as of WP-2.1 across `source.params.thread`
//     (M2–M12, BOLTS-seeded — see `ComponentOp.cpp::iso4762Table()`) and
//     `source.params.length` (free, spec §6.1). Missing params default to
//     `"M6"`/20mm, reproducing P0/P1's old hardcoded behavior exactly. WP-2.5
//     adds a third free param, `source.params.thread_detail`
//     (`cosmetic`/`simplified`/`modeled`, spec §6.4, default `cosmetic` —
//     same byte-identical-when-absent rule): `cosmetic` is the unchanged
//     head+shank blank, `simplified` cuts discrete annular grooves,
//     `modeled` cuts a true helical V-thread via `BRepOffsetAPI_MakePipeShell`
//     (flat-truncated profile, no root fillet — a deliberate kernel-
//     robustness/fidelity tradeoff, see `ComponentOp.cpp::cut_modeled_thread`'s
//     own comment). A second generator family (bearings, motors) dispatched
//     by `generatorId` is later P2 scope.
//   * `PlaceComponent`'s `mate` is READ (by Rust — see `record.rs`) but not
//     yet re-seated by the worker; a placed component always uses
//     `placement` as authored. Persistent mate re-seating on regen is the P3
//     deliverable (spec §5.5). `SetComponentParams`/`ReplaceComponent` have
//     NO C++ dispatch yet (fall through to `UNSUPPORTED` in
//     `PlanExecutor.cpp`) — both are in-place edits of `PlaceComponentParams`
//     at the Rust layer (`crate::edit::session`), not distinct wire ops (see
//     `record.rs`'s `DetachComponentParams` doc comment for why only
//     `DetachComponent` earned its own `KnownOperation` variant).
//
// PIPELINE (shared by both `execute_place_component` and
// `execute_detach_component` — see `ComponentOp.cpp`'s
// `resolve_source_and_publish`; the two ops build IDENTICAL geometry, differing
// only in what the RECORD's params carry):
//   1. validate `source.kind == "generator"` + non-empty `generatorId`.
//   2. build the table-driven SHCS blank: head (cylinder, dk×k) fused to
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
