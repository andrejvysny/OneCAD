# OneCAD Operation Correctness and Evidence Matrix

Baseline: `1c11d4958aeadea14dd8431ba78c41f14be12142`  
Legend: Strong = substantive behavioral evidence; Partial = nominal or mode-limited; Missing = no direct evidence.

## Matrix

| Operation / mode | C++ kernel | Rust real-worker | Frontend unit/contracts | Playwright | Corpus | Kernelbench | Correctness disposition |
|---|---|---|---|---|---|---|---|
| Sketch authoring and solve | Strong | Strong | Strong | Strong except named drawing gaps | Strong | Missing | Keep; exact-region work required |
| Datum-hosted sketch | N/A by design | Strong | Strong | Strong for exposed offset datum | Missing | N/A | Document-side feature, not a missing kernel op |
| Extrude Blind | Strong | Strong | Strong | Strong mock path | Strong | Missing | Keep |
| Extrude ThroughAll | Strong | Strong | Strong | Partial | Strong | Missing | Add exact host-crossing cases |
| Extrude Symmetric | Partial | Missing direct integration | Strong | Missing | Scenario exists | Missing | Add exact volume/bbox path |
| Extrude ToNext | Strong nominal | Strong nominal | Strong | Missing successful commit | Missing | Missing | Add concave/multi-face adversarial cases |
| Extrude ToFace | Strong nominal | Strong nominal | Strong | Cancel path only | Missing | Missing | Fail closed on stale promotion; add success/reopen |
| Extrude two-direction | Strong | Strong | Strong | Missing | Strong | Missing | Add browser and mixed-end-condition cases |
| Extrude draft, planar walls | Strong nominal | Strong nominal | Strong | Mock round-trip | Missing | Missing | Add analytic frustum checks |
| Extrude draft, curved walls | Missing | Missing | Missing | Missing | Missing | Missing | High-confidence risk: zero eligible planar walls; reproduce OCCT outcome, then require applied-or-refused behavior |
| Extrude Add/Cut | Strong | Strong | Strong | Partial | Strong | Missing | Keep; add body lifecycle assertions |
| Extrude Intersect | Missing direct | Missing | Frontend cannot author | Missing | Named only | Missing | Product decision + tests |
| Revolve NewBody | Strong numeric | Strong | Strong | Strong mock path | Missing | Missing | Add publication audit |
| Revolve Add/Cut/Intersect | Strong C++ | Partial Rust | Frontend omits Intersect | Missing modes | Missing | Missing | Add vertical coverage and product decision |
| Revolve body-edge axis | Thin TopoKey fixture | Missing persistent-id path | Not exposed | Missing | Missing | Missing | Dormant contract mismatch: ElementId vs TopoKey |
| Fillet constant radius | Very strong | Strong | Strong | Strong mock path | Strong | Strong, exclusive | Preserve; close body-ownership hole |
| Chamfer equal-leg | Strong | Strong | Strong | Strong mock path | Missing | Missing | Add common validation + robustness |
| Chamfer two-distance | Strong exact centroid | Partial | Strong | Strong mock path | Missing | Missing | Add curved/short-edge adversarial cases |
| Shell single face | Strong | Strong | Strong | Partial mock | Missing | Missing | Add full publication audit |
| Shell multiple faces | Partial | Partial | Partial | Missing | Missing | Missing | Add multi-open-face and cross-body selection gates |
| Boolean Union | Strong | Strong | Strong | Strong mock user path | Strong | Missing | Keep |
| Boolean Cut | Strong split path | Strong | Strong | Partial | Strong | Missing | Add full-consumption empty-result policy |
| Boolean Intersect | Missing standalone | Missing | Operation label only | Missing | Named legal only | Missing | Highest coverage gap; empty-result defect exposed |
| LinearPattern fused | Strong nominal | Strong nominal | Strong ghost/FSM | Missing | Indirect | Missing | Multi-solid/fuse policy unresolved |
| LinearPattern non-fused | Strong nominal | Missing Rust | UI cannot author | Missing | Missing | Missing | Decide one-body compound vs deterministic child bodies |
| CircularPattern full 360° | Strong nominal | Strong nominal | Strong ghost math | Missing | Missing | Missing | Add e2e and non-fused modes |
| CircularPattern partial sweep | Missing kernel assertion | Missing | Strong but uses different formula | Missing | Missing | Missing | Confirmed preview/commit mismatch |
| MirrorBody no-fuse | Strong C++ | Missing Rust | Strong ghost/FSM | Missing | Missing | Missing | UI-only supported mode needs vertical proof |
| MirrorBody fuse | Strong C++ | Strong Rust | UI cannot author | Missing | Missing | Missing | Add disjoint/touching policy and validation |
| TransformBody move/rotate | Strong | Strong | Strong | Strong mock path | Missing | Missing | Add publication audit and result classification |
| TransformBody copy/multibody | Strong | Strong | Strong | Strong mock path | Missing | Missing | Add cancellation/resource ceiling tests |
| ImportStep STEP | Strong | Strong | Strong command path | Mock only | Explicit N/A | Missing | Preserve compatibility; clarify invalid-solid policy |
| ImportStep BREP/XBF/colors | Strong | Strong | Strong | Mock only | N/A | Missing | Add post-scale validation and tie-order test |
| Hole simple blind/through | Strong analytic | Strong analytic | Strong | Partial mock | Missing | Missing | Fail closed on stale promotion and split-host result |
| Hole counterbore | Strong analytic | Strong | Strong | Commit/re-edit | Missing | Missing | Keep |
| Hole countersink | Strong analytic | Strong | Strong | Field switch only | Missing | Missing | Add browser commit/reopen path |
| OffsetFace Offset | Very strong | Strong | Strong | Partial mock | Missing | Missing | Add stale prepare fence and kernelbench |
| OffsetFace Total | Very strong | Partial | Strong | No committed mode test | Missing | Missing | Add Rust execution and browser commit |
| OffsetFace Radius | Very strong | Strong | Strong | No committed mode test | Missing | Missing | Keep; add curved robustness |
| OffsetFace Diameter | Very strong C++ | Missing execution | Strong | No committed mode test | Missing | Missing | Add vertical proof |

## Required per-operation publication contract

Every operation specification must state all of the following explicitly.

1. Accepted input top-level shape classes.
2. Whether the operation requires exactly one solid.
3. Empty-result semantics.
4. Multi-solid-result semantics.
5. Body lifecycle: created, modified, deleted, split or merged.
6. Whether BodyId is preserved or deterministically minted.
7. Which OCCT builder history is authoritative.
8. What happens when history is absent, split, ambiguous or generated.
9. Post-operation validity tier.
10. Tolerance evidence and any calibrated ceiling.
11. Cancellation points and maximum uninterruptible work.
12. Preview fidelity: exact kernel, analytic exact, placement-only or none.
13. Persistence/reopen and upstream-edit behavior.
14. User-visible refusal and recovery behavior.

## Proposed invariant cases by operation

### Sketch/profile

- Two overlapping circles produce three bounded regions whose BRep boundaries remain analytic arcs and whose areas match closed-form lens/crescent values.
- Arc-circle, ellipse-line and near-tangent intersections retain analytic curve types.
- Entity-order permutation does not change region ids or face geometry.
- Gap repair above the configured small-gap policy refuses or emits a structured diagnostic; it never silently bridges 0.1 mm by default.

### Extrude

- Every end condition has exact bbox and volume fixtures.
- ToFace and ToNext must prove that the profile reaches the selected bounded face, not merely its infinite plane.
- Stale promotion leaves the tool in pick mode.
- Draft either changes at least one eligible wall and passes analytic taper checks or refuses. A circular-profile draft cannot succeed unchanged.
- NewBody must publish exactly one valid solid.

### Revolve

- NewBody validates one positive-volume solid with no self-interference.
- Body-edge axis either resolves a typed semantic ref through the element map or is removed from the supported contract.
- Intersect must be tested at C++, Rust and UI layers if exposed.

### Fillet and Chamfer

- All selected edges must name one target body before any identity or descriptor resolution.
- Mixed-body refs fail atomically and cannot fall through onto congruent target edges.
- Every refusal preserves the pre-operation body and selected refs.
- Chamfer receives the same common publication validation tier as Fillet, with operation-specific semantic checks layered above it.

### Shell

- All open faces belong to the target body before arming.
- Result is exactly one valid positive-volume solid with no self-interference.
- Multi-open-face, inward/outward policy, face disappearance and non-manifold cases are explicit.

### Boolean

- Union, Cut and Intersect each cover overlap, touching, containment, disjoint and complete-consumption cases.
- Zero-solid result has an explicit lifecycle: either delete the target or refuse. It must never publish an empty ghost body.
- Split result children are deterministically ranked and the ordinal tripwire stays effective.
- Tool consumption happens only after successful result classification.

### Patterns

- Preview and worker transforms implement the same normative angle-distribution formula, locked by cross-track fixtures.
- `fuseResult=true` either produces exactly one solid or refuses with a clear disjoint-result message.
- Non-fused output semantics are versioned and explicit. Recommended new-record policy: deterministic child bodies per instance; legacy compound records retain their frozen behavior.
- Count, coordinate and workload ceilings are validated at Rust and worker boundaries.
- Long loops poll cancellation.

### MirrorBody

- No-fuse produces one mirrored solid.
- Fuse requires one valid connected result or refuses.
- Plane parameters are finite.
- UI and persisted modes agree.

### TransformBody

- Every result passes the common publication validator.
- Malformed vectors refuse rather than default components.
- Multi-target operations remain all-or-nothing.
- Copy lineage and child ordering remain target-list-based, not geometric-rank-based.

### ImportStep

- Every published solid has an explicit invalid-shape policy.
- Unit scaling is followed by validation.
- Exact geometric tie ordering is deterministic or diagnosed.
- Hash verification responsibility is documented at both Rust and worker boundaries.
- Marginal imported geometry cannot silently enter operations whose preconditions it violates.

### Hole

- Host face body equals target body.
- A stale promotion cannot become anchor-only authoring.
- Existing records preserve the documented one-body multi-solid residual; if the decision is reopened, new-record refusal behavior is versioned and never silently changes legacy replay.
- Countersink commit/reopen and every standard table row are covered.

### OffsetFace

- The authoring closure response is accepted only if the response snapshot and current head still equal the admitted snapshot.
- Every typed primary body equals targetBodyId.
- Existing ownership, closure, volume, self-interference and semantic movement checks remain unchanged.
- Total/Radius/Diameter receive Rust execution and browser commit/re-edit tests.

## Coverage manifest recommendation

Add a machine-readable operation matrix under test infrastructure, not as a manually maintained document. The generator should compare:

- `KnownOperation` variants,
- worker dispatch,
- protocol operation tags,
- frontend registered modeling tools,
- operation and mode enums,
- declared test-layer classifications.

CI must fail when a supported operation or mode is added without an explicit classification. `Loft` and `Sweep` remain classified as unsupported, not missing tests.
