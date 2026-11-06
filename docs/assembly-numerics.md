# Bounded assembly numerical contract

`onecad-assembly` is a pure solver candidate. Its tests do not establish native
assembly integration, production capacity or platform acceptance (ONECAD-401,
ONECAD-530 and ONECAD-534).

## Proper rigid poses

Quaternion components are private outside the crate. `Quat::try_new` and checked
deserialization require finite components and a norm-squared error at most
`1e-12`. Zero, scaled and non-finite quaternions are refused. Rotation-vector
construction is fallible; numerical composition may normalize finite drift.
Occurrences and solver inputs validate their full rigid pose before admission.
Reflection remains separate from proper rotation and is unsupported here.

This tightens the scaffold API. It does not provide a migration for an accepted
assembly document format: that format is not yet integrated. Any later document
migration must preserve valid poses and explicitly refuse invalid legacy data.

## Constraints and limits

Local frames have oriented axes. Revolute alignment includes an oriented-axis
residual, so opposite axes cannot satisfy it merely because their cross product
is zero. Slider and revolute drives and limits are checked independently of
stored joint values. All-grounded systems still require final feasibility.

Revolute values use principal radians in `[-pi, pi]`. Limit endpoints must lie in
that interval; `-pi` and `pi` describe the same pose at the seam. Multi-turn limits
and intervals that cross the seam are unsupported and refused. Slider distances
use document millimetres. The bounded clamp/re-solve path rechecks every original
constraint and limit before returning poses; an unsatisfied coupled system is
refused rather than partially published.

A local solver stall is `NotConverged`, not proof of global inconsistency. Only
successful outcomes carry poses. Callers retain the previous accepted state on
invalid, uncertain, cancelled or failed work; application publication still
requires its session/head/worker fences.

## Rank and degrees of freedom

Perturbations rotate around each body's origin, with world-axis translation.
Finite-difference columns are scaled before numerical rank estimation. Two step
sizes (`1e-7` and `2e-7`) must agree; threshold-sensitive pivots, tiny nonzero
columns and unresolvable translation perturbations return `NumericallyUncertain`.
Both body origins and transformed joint origins are checked for representability.

These checks are conservative numerical diagnostics, not a proof of rank for
every finite input. Metamorphic regressions cover rigid, revolute and slider
systems at origins 0 and 20,000 mm, local offsets 0.001, 1 and 20,000 mm, and two
common rotations. Exact redundant and near-degenerate cases are distinct. A
`1e20`-mm origin must refuse a misleading DOF report when floating-point addition
loses the perturbation. Arbitrary scale invariance and solver capacity remain
unqualified. No measured 8 ms p95 claim is made.
