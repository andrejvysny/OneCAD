# Phase 3 Specification — Publication Policy and Operation Semantics

Duration: 3–4 weeks  
Prerequisites: Phase 0; Phase 1 for semantic-ref operations  
Priority: P1 systemic correctness  
Gate name: `UNIFORM-PUBLICATION-POLICY`

## Rationale

OneCAD currently has operation-specific validation ranging from Fillet and OffsetFace's deep checks to Pattern/Mirror/Transform/Revolve NewBody's null-only or no checks. The same malformed topology can be accepted or refused depending on operation order. Several operations also publish multiple solids under one BodyId while downstream tools assume one solid.

The solution is a policy-driven common validator, not an unconditional copy of the most expensive audit.

## Goals

1. Define allowed output topology and lifecycle for every supported operation and mode.
2. Introduce a common publication validator with risk tiers.
3. Resolve empty and multi-solid semantics.
4. Add workload and cancellation ceilings.
5. Align UI-exposed modes with persisted/kernel capabilities.
6. Preserve import policy as an explicit exception rather than accidental inconsistency.

## Non-goals

- Adding new operations.
- Changing topological-naming scoring.
- Enforcing an uncalibrated universal tolerance-growth threshold.
- Turning every imported marginal solid into a hard error without a product decision.

## Required per-operation contract artifact

Create a versioned, machine-readable contract row for every supported operation and mode covering all fourteen fields defined in `01-operation-correctness-matrix.md`: input shape classes; empty and multi-solid semantics; lifecycle and BodyId policy; authoritative OCCT history; ambiguity behavior; validation tier; tolerance evidence; cancellation limits; preview fidelity; persistence/upstream-edit behavior; and user recovery. Lineage/history fields that cannot be completed in this phase must be explicitly marked deferred with an owner and dependency rather than omitted.

## Publication policy model

```text
PublicationPolicy {
  allowedTopLevelShapes,
  minSolidCount,
  maxSolidCount,
  emptyResult,
  positiveVolume,
  brepValidity,
  selfInterference,
  manifoldPolicy,
  toleranceEvidence,
  toleranceCeiling,
  operationSemanticValidator,
  lifecyclePolicy
}
```

Validation returns:

- publishable result and evidence,
- recoverable refusal with stable code,
- valid lifecycle-only result such as deletion if explicitly allowed.

## Validation tiers

### Tier A — fast mandatory

- non-null,
- expected top-level shape class,
- solid count,
- finite positive volume when solids are required,
- `BRepCheck_Analyzer`,
- operation-specific empty-result classification.

### Tier B — risky topology operations

Tier A plus:

- deterministic self-interference check,
- closed-manifold edge-use check where applicable,
- tolerance distribution evidence,
- sliver/micro-edge metrics as diagnostics.

Recommended Tier B operations:

- Boolean,
- Fillet,
- Chamfer,
- Shell,
- Hole,
- OffsetFace,
- fused Pattern,
- fused Mirror.

### Tier C — gate/debug deep audit

Tier B plus full benchmark-style deep checks and expensive diagnostics. Run in:

- CTest fixtures,
- real-worker gate mode,
- kernelbench,
- optionally a debug setting.

Do not make Tier C the default interactive path until measured.

## Operation policy table

| Operation | Proposed result policy |
|---|---|
| Extrude/Revolve NewBody | exactly one valid positive-volume solid |
| Extrude/Revolve Add/Cut/Intersect | zero/one/many classified by Boolean lifecycle policy; splits deterministic |
| Standalone Boolean | explicit zero result; one modifies target; many replace target with ranked children; tool consumed only after successful classification |
| Fillet | exactly one Tier B solid plus existing semantic checks |
| Chamfer | exactly one Tier B solid plus distance/contour semantics |
| Shell | exactly one Tier B solid |
| Hole | legacy absent-version records preserve the documented one-body multi-solid residual; any new refusal policy is versioned for new records and still mints no split children |
| OffsetFace | retain existing one-solid, volume, SI and movement checks; route evidence through common result shape |
| Linear/Circular Pattern fuse | exactly one connected Tier B solid or named refusal |
| Linear/Circular Pattern non-fuse | versioned deterministic child bodies recommended for new records; legacy one-body compound preserved only under legacy version |
| Mirror no-fuse | exactly one mirrored solid |
| Mirror fuse | exactly one connected Tier B solid or named refusal |
| Transform move/copy | validate each result as one solid unless the source is explicitly classified as legacy multi-solid |
| ImportStep | each extracted solid receives Tier A evidence; invalid-solid warning/failure remains explicit import policy |

## Work package 3.1 — Common validator

### Implementation direction

- Refactor `ShapeAuditResult` into a generic evidence object plus policy evaluation.
- Keep Fillet diagnostics compatible.
- Reuse OffsetFace evidence rather than duplicating expensive checks.
- Make tolerance evidence always available at Tier B; do not reject until a calibrated per-operation ceiling exists.
- Add positive and negative controls proving the validator really runs.

Relevant source:

- `worker/src/kernel/validation/ShapeAudit.cpp`
- `worker/src/kernel/fillet/FilletBuilder.cpp`
- `worker/src/ops/OffsetFaceOp.cpp:857-903`
- `worker/src/ops/OpCommon.cpp`

## Work package 3.2 — Pattern output semantics

### Decision required

Current protocol says both fused and non-fused Pattern produce one new body. A non-fused compound can contain several solids under one BodyId. A disjoint OCCT Fuse can do the same despite `fuseResult=true`.

Recommended new-record semantics:

- `fuseResult=true`: one connected solid; otherwise refuse with `PATTERN_DISJOINT_RESULT`.
- `fuseResult=false`: one deterministic child body per instance, including or excluding the original according to a separately explicit source-preservation rule.

Compatibility:

- Add `resultPolicyVersion` or equivalent to newly authored pattern records.
- Legacy absence executes current one-body behavior.
- Re-edit of a legacy feature must not silently add the version.
- Child BodyIds follow existing D1 `<opId>:<k>` rules and need ordinal/lineage evidence based on instance index, not geometry rank.

### Tests

- touching, overlapping and disjoint instances,
- source included exactly once,
- negative spacing/angle,
- copy ordering and save/reopen,
- downstream Fillet on a chosen child,
- undo and re-edit.

## Work package 3.3 — Mirror and Hole result semantics

Mirror:

- finite plane validation,
- no-fuse one solid,
- fuse one connected solid or refusal.

Hole:

- treat the existing disconnected-host behavior as an explicitly accepted legacy residual, not an unversioned defect fix,
- if the product reopens the decision, add a result-policy version for new records so disconnected hosts refuse with a named message while legacy replay remains unchanged,
- do not mint split children without a new protocol decision and cross-track fixtures.

## Work package 3.4 — Mode alignment

Audit and decide:

- feature Intersect for Extrude/Revolve exists in core and worker but not UI,
- Pattern `fuseResult` exists in core/worker but is hard-coded in UI,
- Mirror `fuseWithOriginal` exists in core/worker but UI hard-codes false,
- body-edge Revolve axis is typed but not correctly persistent/exposed.

For each mode choose:

- expose and test,
- explicitly unsupported in the UI with a contract test,
- remove from the supported v1 worker surface only through a compatibility-safe decision.

Do not leave silent capability mismatches.

## Work package 3.5 — Strict worker parameter readers

Replace silent defaults for malformed present fields with typed readers.

Priorities:

- Pattern count/ranges and finite vectors,
- Mirror finite plane,
- Revolve finite/ranged angle,
- Transform wrong-shaped vectors and wrong-type copy,
- Shell finite thickness,
- Boolean operation token defaulting unknown values to Union.

Rust authoring validators remain the first line; worker checks remain the independent trust boundary.

## Work package 3.6 — Cancellation and workload ceilings

- Pattern count and cumulative fuse ceiling.
- Transform target-count ceiling and per-target cancellation poll.
- BREP/XBF import read/scale cancellation where practical.
- Sketch dense-intersection ceiling from Phase 2.
- Fillet/Chamfer uninterruptible build duration metrics.

A limit is part of product semantics and must be documented and tested, not hidden in UI clamps alone.

## Performance plan

Measure before/after on representative small, medium and adversarial shapes.

Targets:

- Tier A P95 overhead under 5%.
- Tier B P95 overhead under 15% for ordinary operations.
- Anything slower must be gate-only or justified by a specific risk.
- Capture self-interference and manifold timing separately.

## Acceptance gates

- Every operation has a policy row and code path.
- Positive controls fail when validator invocation is removed.
- Existing Fillet and OffsetFace semantics remain unchanged.
- Policy-level C++ and real-worker Rust lifecycle tests cover empty/one/many solids.
- Full CTest, Rust, frontend unit/build and T0 gates remain green; new Pattern/Mirror/Hole browser flows belong to Phase 4 after policy stabilizes.
- Protocol changelog and cross-track fixtures updated for any new versioned params.

## Rollout

1. Land validator infrastructure with no behavior changes.
2. Migrate operations one family at a time.
3. Introduce versioned Pattern semantics behind new authoring only.
4. Turn warnings into failures only after fixtures and performance evidence exist.

## Rollback

Policy tables are data/configuration, but every behavior change should remain in a separate gate commit. Legacy pattern execution must remain available until migration evidence is complete.
