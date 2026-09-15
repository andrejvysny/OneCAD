# sketch-solver-truth
date: 2026-09-15
mode: derive
model: gpt-6-astra
effort: xhigh
access: grounded: worker/src/protocol/SolverLane.cpp, worker/src/sketch/Sketch.cpp, worker/src/sketch/solver/ConstraintSolver.{cpp,h}, worker/src/sketch/SketchConstraint.h, worker/third_party/planegcs/GCS.cpp, worker/tests/test_solver_residual.cpp, src/tools/sketch/autoConstrain.ts, src/ipc/mockConflicts.ts, protocol/SCHEMA.md. Transcript audit: also read worker/src/protocol/{Dispatcher,Envelope,SolverLane}.h, protocol/fixtures/sketch_solve_residual.ndjson, worker/tests/fixtures/sketch_conflicting.ndjson and the legacy oracle copy of ConstraintSolver under corpus (recorded; none carries an expected value for this derivation; no corpus/expected-values read).
packet: sha256 a36d445322e1
calls: 1 of ~3 for WP-2 (session 01a0a3ea-9d99-73b0-b46d-855c0f4270c9 recorded for followup)
verdict: n/a (derive)
verified by: Fable checked `ConstraintSolver.cpp:1790-1792` (setConvergence AND setConvergenceRedundant both take config_.tolerance = 1e-4, confirming the source correction), recomputed T5 (H/V-only rectangle, A→(22,21) ⇒ B=(80,21), D=(22,60), C unchanged; Σ movement² = 22²+21²+21²+22² = 1850), T4 (10 000·sin 0.8° = 139.62 mm; τ_ang at 80 mm = 1e-4/80 = 1.25e-6 rad), the 1 mm-at-80 mm q ≈ 1e4, and the tangency second-order counterexample (R(1−cos φ) = 10·(1−cos 1e-3) ≈ 5e-6 mm). `solverPolicyVersion` runtime value confirmed 1 (manifest.rs:158).
rejected: none. Decisions taken on Astra's open questions: (1) state vocabulary stays four tokens; residual violation ⇒ `Conflicting` + additive per-id `reasons` (`residualViolation` | `rankConflict`) and an additive `unassessed[]`; (2) τ_len = 1e-4 mm absolute; τ_ang(c) = τ_len / L_c with the LOCAL operand span as L_c (audited per getError kind — H/V are coordinate residuals in mm, so they use τ_len directly); (3) drive-only drag for DragKind::Point only (plan amendment A-12), with the T9 signed-area branch guard and the "cursor distance alone is never partial" rule; (4) the parity graph with an axis node is the shared authoring rule (`directionLock.ts`), snap intent outranks incidental inference when both arrive in one batch (a recorded product choice), committed H/V are never removed silently; (5) `solverPolicyVersion` 1 → 2 (manifest axis only, per the protocol pre-audit).

## 1. Problem restatement in your own terms; surface any misframing in one line

**Misframing:** residual failure proves an **unsatisfied returned pose**, not mathematical inconsistency; use `Conflicting` with an additive `residualViolation` reason, never `UnderConstrained` with culprits.

Labels: **P** proved mathematically; **N** numerical calculation; **S** source-cited; **I** inference/proposed policy.

Three source corrections affect the derivation:

- **S:** Runtime configuration sets both `convergence` and `convergenceRedundant` to `1e-4`. Consequently, redundancy’s `error² < convergenceRedundant` means `|error| < 0.01` in **native error units**, not the constructor’s `1e-5` threshold. `DL_tolfRedundant` remains separately configured. [Configuration](/Users/andrejvysny/workspace/OneCAD/worker/src/sketch/solver/ConstraintSolver.cpp:1786), [redundancy test](/Users/andrejvysny/workspace/OneCAD/worker/third_party/planegcs/GCS.cpp:5600).
- **S:** H/V lower to coordinate equalities. The allowlisted `SketchConstraint.h` declares abstract `getError`; it does not contain the concrete implementations. Therefore the packet’s assertion that H/V residuals are radians, and that reporting errors equal PlaneGCS errors, is unverified. [H/V lowering](/Users/andrejvysny/workspace/OneCAD/worker/third_party/planegcs/GCS.cpp:929), [reporting interface](/Users/andrejvysny/workspace/OneCAD/worker/src/sketch/SketchConstraint.h:105).
- **S:** DogLeg and LM’s inner solvers return failure on unsuccessful termination. `Converged` also arises from the outer redundant-error check and the primary/auxiliary solver used with drag drives. “DogLeg minimal-change solution” is not an established repository guarantee. [Outer solve](/Users/andrejvysny/workspace/OneCAD/worker/third_party/planegcs/GCS.cpp:1904), [DogLeg](/Users/andrejvysny/workspace/OneCAD/worker/third_party/planegcs/GCS.cpp:2268), [drag solver](/Users/andrejvysny/workspace/OneCAD/worker/third_party/planegcs/GCS.cpp:4530).

**S, packet + source:** B2 establishes a pose-publication failure, not yet a residual-violating solved-pose reproduction. Upsert updates worker storage on success but returns no solved coordinates. Both publication and residual acceptance need correction. [Upsert](/Users/andrejvysny/workspace/OneCAD/worker/src/protocol/SolverLane.cpp:415).

## 2. Properties to guarantee: the invariants the policy must hold, each as a checkable statement

**I — proposed acceptance contract:**

| Property | Check |
|---|---|
| Honest state | `FullyConstrained`, `UnderConstrained`, or benign `OverConstrained` implies every committed constraint is readable, finite, and within its typed tolerance. |
| Honest attribution | Every `conflicting[]` member has a violated residual or belongs to a verified incompatible constraint group. |
| Pose identity | Gate, diagnosis, returned coordinates, worker storage, and document write-back identify the same pose/revision. |
| Released truth | EndGesture removes every temporary drive before its exact solve, diagnosis, and gate. |
| Constraint priority | Target distance never justifies violating a committed constraint. |
| Reference immobility | Every published locked scalar equals its gesture-start value exactly; any attempted change rejects the candidate. |
| No corruption | Failed steps restore all affected coordinates, radii, and angles. Failed finalization restores and reclassifies a retained pose. |
| Benign redundancy | A verified implied constraint, satisfied within tolerance, never creates a conflict. |
| Uncoupled stability | Scalars outside the drag’s dependency closure remain byte-identical. |
| Determinism | Canonical parameter/constraint order, deterministic work limits and tie-breaking; identical initial pose and accepted request sequence produce identical state/pose bytes. |
| Compatibility | Keep four state tokens and existing `entityStates` derivation; publish additional truth diagnostics separately. |

**P:** Non-convergence alone identifies no guilty constraint. When evaluation or diagnosis is inconclusive and no legitimate culprit exists, an error/diagnostic outcome is necessary; inventing a constraint ID cannot complete the four-state vocabulary honestly.

## 3. Predicates and epsilon derivation

### Typed residuals and scaling

**I:** Adopt an independent acceptance budget:

\[
\tau_{\rm len}=10^{-4}\ {\rm mm}.
\]

This is an **absolute physical tolerance**, matching the stated product budget. It is not inferred from PlaneGCS’s squared-error or step thresholds.

**I:** For an angular constraint, define `L_c` locally:

- One line: endpoint distance.
- Two lines: maximum of their endpoint distances.
- Endpoint tangency: maximum of the participating line span and arc radius.
- Other angular operands: require an explicit, audited characteristic-length definition.

Use the geometry being classified. Do not use the whole-sketch bounding diagonal or an unrelated longest entity.

\[
\tau_{\rm ang}(c)=\frac{\tau_{\rm len}}{L_c}\ {\rm rad},\qquad
q_c=\begin{cases}
r_c/\tau_{\rm len} & \text{length residual},\\
r_c/\tau_{\rm ang}(c) & \text{angular residual}.
\end{cases}
\]

Accept a readable, finite constraint iff `q_c ≤ 1`.

**P:** For angular deviation `δ`, endpoint lateral deviation satisfies  
`L_c |sin δ| ≤ L_c |δ|`. Thus this angular gate conservatively enforces the same positional budget. Using the larger participating span bounds either line’s endpoint effect.

**I:** Apply angular normalization only after auditing each actual `getError` implementation. H/V coordinate residuals use `τ_len`. Multi-equation constraints require an aggregation that cannot hide an individual failed equation. Internal tag-0 rules require their own integrity checks; they are absent from the public constraint list.

**P:** Scaling behavior:

| Transformation | Behavior |
|---|---|
| Unit conversion by `k`, e.g. mm→m | Lengths and `τ_len` multiply by `k`; angular tolerances and every `q_c` remain unchanged. |
| Physical similarity by `s`, fixed manufacturing tolerance | Length errors multiply by `s`; angular tolerances divide by `s`. Acceptance intentionally becomes stricter for larger geometry. |
| Similarity with tolerance also multiplied by `s` | Acceptance is invariant. |
| Translation / coordinate-frame change | Acceptance is invariant mathematically, provided axes/constraints transform consistently; floating-point conditioning may change. |

**I:** Do not add an arbitrary relative tolerance to accommodate large sketches. Recenter numerical calculations and measure evaluator error. A numerical uncertainty branch is preferable to silently relaxing the physical budget.

### State table and culprits

**I:** Define:

- `V = {c : c is readable, finite, q_c > 1}`.
- `R`: verified incompatible committed groups, after removing temporary-drive influence and resolving known semantic redundancies.
- `B`: benign redundant constraints whose implication/regularity is established and whose residuals pass.
- `U`: unreadable, nonfinite, unsupported, or numerically unresolved evaluation.

A raw QR dependency is insufficient to establish either incompatibility or semantic redundancy.

| Converged? | `R` | Redundant member with error above tolerance? | Other residual above tolerance? | Result |
|---|---:|---:|---:|---|
| Either | Nonempty | Either | Either | `Conflicting`; rank-conflict reason |
| Either | Empty | Yes | Either | `Conflicting`; residual-violation reason |
| Either | Empty | No | Yes | `Conflicting`; residual-violation reason |
| Yes | Empty | No | No | `OverConstrained` if `B` nonempty; otherwise DOF classification |
| No | Empty | No | No | Returned/retained pose can still pass the gate; classify it if diagnosis is valid, while separately reporting non-convergence |
| Either | Unresolved | Unknown | Unknown | Diagnostic/error outcome; no fabricated state or culprit |

**I:** Culprits:

\[
C=\operatorname{sortUnique}\left(V\cup\operatorname{members}(R)\right).
\]

Exclude tags `0` and `−1`. Publish per-ID reasons. Unreadable constraints belong in `unassessed[]`, not automatically in `conflicting[]`.

**P:** A satisfied constraint can legitimately belong to an incompatible group: at separation 10 mm, `Distance 10` is satisfied but participates in the contradiction with `Distance 20`.

**I:** A mathematically implied Perpendicular that still has residual error indicates incomplete solving of its supporting constraints. Solve the independent system, then evaluate every authored constraint. Do not convert that into a claim that the authored relationship is contradictory.

### Drive-only drag

**I:** Drive only the selected point’s `u` and `v` toward the cursor, under tag `−1`. Keep committed constraints and reference-lock rules. Remove the rectangle and pin-everything heuristics.

Define the desired local priorities explicitly:

1. Satisfy committed constraints and preserve the selected solution branch.
2. Minimize cursor-target distance.
3. Among equivalent target solutions, minimize movement from the previous accepted pose.

The third priority is a tie-break, not additional coordinate constraints. For curve parameters, use a declared physical metric: coordinate/radius changes in mm, angle changes multiplied by a captured local radius.

**S:** PlaneGCS separates nonnegative-tag constraints from auxiliary negative-tag drives and solves the coupled case through its primary/auxiliary path. This supports priority semantics, but does not prove the three-level policy above. [Partitioning](/Users/andrejvysny/workspace/OneCAD/worker/third_party/planegcs/GCS.cpp:1740).

**P — conditional minimum-norm proof:** For a linearized system `J h = −f`, the minimum-norm step is `h = −J⁺f`. An identically zero column gives zero in that coordinate. The steepest-descent direction `−Jᵀf` also has zero there; any DogLeg combination preserves it.

This proves uncoupled stability **only for that update rule**. It does not prove the current implementation’s behavior: default Gauss–Newton steps use `FullPivLU`, equality reduction retains selected representatives, and drag invokes another algorithm. [Defaults](/Users/andrejvysny/workspace/OneCAD/worker/third_party/planegcs/GCS.cpp:489), [equality reduction](/Users/andrejvysny/workspace/OneCAD/worker/third_party/planegcs/GCS.cpp:1804).

**I:** Guarantee uncoupled stability structurally: exclude unrelated scalar components from the active solve. No temporary pins are needed. Preserve equality-class weights if implementing minimum movement over original parameters.

**I:** Drag outcomes:

- Target missed, valid constrained optimum reached: `success`, or `redundant`.
- Solver failure, invalid geometry, or branch-guard refusal: `partial`, restoring the pre-step pose.
- A committed conflict: `conflicting`, using committed culprits.
- Cursor distance alone never causes `partial`.

Use already-computed primary feasibility information during drag; the public per-constraint residual gate remains exact-only. Its equivalence to public residual acceptance needs verification.

**I/P — T9 branch guard:** For the ordered rays of the relevant angle, capture the sign of

\[
S=(B-A)\times(C-A).
\]

Let `L = max(|B−A|, |C−A|)`. Require the retained branch to satisfy `sign(S₀) S > τ_len L`. Check the candidate path’s signed-area quadratic, not merely its endpoint. Subdivide deterministically; if the branch cannot be continued, restore and report `partial: branchBoundary`. An initially ambiguous sign requires its own refusal.

This prevents crossing the collinear boundary; smaller cursor steps alone do not prove root preservation.

### Semantic-redundancy authoring

**P:** Introduce an axis node `X` and parity equations:

- `Horizontal(a)`: `a ⊕ X = 0`.
- `Vertical(a)`: `a ⊕ X = 1`.
- `Parallel(a,b)`: `a ⊕ b = 0`.
- `Perpendicular(a,b)`: `a ⊕ b = 1`.

For a proposed relation of parity `p`, query the graph **without that candidate**:

| Existing graph | Decision |
|---|---|
| Connected, implied parity equals `p` | Redundant: suppress candidate |
| Connected, implied parity differs from `p` | Contradictory: reject candidate with a witness path |
| Disconnected | Independent: author candidate |
| Relevant component already inconsistent | Refuse inference; report the existing contradiction |

The graph includes accepted constraints and earlier accepted constraints in the same authoring batch.

**I:** For the requested axis-locked pair, keep committed H/V and suppress the new inferred Perpendicular/Parallel. Both encode the same feasible relationship there. Never remove an existing authored H/V silently.

For simultaneous new inferences, preserve explicit snap intent before incidental coordinate inference. If provenance is unavailable, that precedence is an unresolved product input.

**S:** `autoConstrain.ts` already excludes H/V versus Perp/Parallel within its own inference path; the separate polar-snap path therefore needs a common final admission check. The mock union-find silently ignores inconsistent cycles and cannot be reused unchanged. [Inference exclusion](/Users/andrejvysny/workspace/OneCAD/src/tools/sketch/autoConstrain.ts:569), [mock parity implementation](/Users/andrejvysny/workspace/OneCAD/src/ipc/mockConflicts.ts:209).

## 4. Degeneracy classes

**I unless marked otherwise:**

| Class | Detection predicate | Required branch |
|---|---|---|
| Zero-length line | `p0 == p1`; separately, endpoint difference indistinguishable from zero within an audited arithmetic bound | Direction undefined. Reject direction-dependent evaluation; do not manufacture angle zero. |
| Insufficient angular resolution | `τ_len / L_c` becomes comparable to the relation’s angular domain | Require a separately specified angular-intent budget; do not classify the entity as topologically nonexistent. |
| Arc endpoint coupling | Either endpoint differs from `C + R(cos θ, sin θ)` beyond its positional budget | Integrity failure; restore all arc parameters and endpoints. Never blame tag 0. |
| Arc collapse | Sweep outside `[1e−3, 2π−1e−3]` rad | Refuse step, restore pose, `partial`. Radius targets clamp at 0.01 mm. **S:** [existing guards](/Users/andrejvysny/workspace/OneCAD/protocol/SCHEMA.md:3159). |
| Locked reference | Selected weld component intersects locked geometry; any candidate changes a locked scalar | Saturate at the feasible pose or refuse; preserve exact locked values. |
| Ellipse | An ellipse entity exists; or a constraint references its unsupported curve parameters | Keep documented naive DOF and omitted `entityStates`; assess supported constraints explicitly. Unsupported operands produce an error, never a zero residual. |
| Unreadable constraint | Missing operand, failed evaluation, NaN/∞ | `unassessed` diagnostic and incomplete truth result. |
| Coincident-welded handles | Distinct point IDs belong to one Coincident component | Drive one logical handle once; preserve all equality equations. Include arc ownership/coupling in dependency closure. |
| Inconsistent direction chain | Parity cycle has XOR 1 | Reject with the actual cycle/path witness, including relation constraints. |
| Singular tangency formulation | Welded tangent configuration has a vanishing tangency derivative along the weld manifold | Use a regular endpoint-tangency relation; rank loss alone is not redundancy. |
| Near-singular Jacobian | Scaled rank/pivots are numerically unresolved, or a significant residual remains with negligible feasible progress | Diagnose conditioning, restrict continuation, or refuse. Never infer contradiction from rank loss alone. |

**I:** Rank analysis must use dimensionless row and parameter scaling. A numerical rank cutoff cannot be derived from the mm acceptance tolerance alone; retain/report the current cutoff until an audited replacement exists.

## 5. Algorithm sketch

**I — SketchUpsert**

1. Validate operands, dimensional error metadata, degeneracies, and direction-graph consistency.
2. Construct a canonical parameter/constraint order; preserve the input pose.
3. Solve committed constraints. For proven direction redundancies, solve an independent subset while retaining authored constraints for final evaluation.
4. Complete all solver rollback/timeout handling.
5. Diagnose and gate the actual retained pose, including redundant constraints and internal integrity.
6. Produce state, named culprits, reasoned diagnostics, and complete solved `positions`/`curves`.
7. Publish/store that same pose atomically; the Rust document must apply it before exposing its state.

**I — Gesture**

1. **Begin:** open from a certified upsert revision; capture complete rollback pose, locks, dependency closure, branch signatures, and existing `entityStates`.
2. **SolveDrag:** add only target drives; continue locally; check existing geometric/branch guards; remove drives on every exit. Publish accepted preview deltas or restore and report `partial`.
3. **End:** attempt final target if supplied; remove drives; perform an unconditional released exact solve, diagnosis, and gate.
4. If final acceptance fails, restore and classify the retained pose. Publish refusal diagnostics separately from that pose’s state.
5. Echo gesture-fixed `entityStates` as required; publish fresh released state/DOF separately.

**S:** Current EndGesture skips the released exact solve when `finalTarget` was processed and uses gesture-fixed conflict/DOF information. [EndGesture](/Users/andrejvysny/workspace/OneCAD/worker/src/protocol/SolverLane.cpp:817).

**P:** With `n` scalar parameters, `m` residual rows, and bounded operand arity:

- Residual/integrity gate: `O(m + n)`.
- Direction union-find: amortized `O(m α(n))`; canonical sorting adds `O(m log m)`.
- Dense Jacobian factorization: approximately `O(mn min(m,n))`, cubic for comparable dimensions.
- Per-step guards: linear in affected geometry; no public residual scan.

**I:** The ≤8 ms target cannot be proved from entity count. Benchmark the actual primary/auxiliary solve, continuation, and guards at approximately 300 entities. Use deterministic work limits for behavioral decisions; elapsed timing is telemetry.

## 6. Test vectors

### T1: exact rectangle and minimum-change characterization

**P:** The supplied angles and nominal “80 × 60” do not specify exact endpoint coordinates, point multiplicities, or an anchor. No unique numeric solved rectangle follows.

**N:** Interpreting the four traversal directions as `0.8°, 89.3°, 179°, 270.6°` with lengths `80,60,80,60 mm` gives closure error:

\[
\sum_i l_i(\cos\theta_i,\sin\theta_i)
=(1.365713,\;2.511979)\ {\rm mm}.
\]

Thus even that natural interpretation is not a closed rectangle.

**P:** Given actual initial corners `A₀,B₀,C₀,D₀`, the equal-weight Euclidean minimum-change H/V rectangle is:

\[
\begin{aligned}
x_L&=(A_{0x}+D_{0x})/2,&x_R&=(B_{0x}+C_{0x})/2,\\
y_B&=(A_{0y}+B_{0y})/2,&y_T&=(C_{0y}+D_{0y})/2.
\end{aligned}
\]

Its corners are `(xL,yB),(xR,yB),(xR,yT),(xL,yT)`. Welded duplicate parameters contribute their weights to these averages.

More generally, for linear constraints `Mq=b`:

\[
q^*=q_0-W^{-1}M^T(MW^{-1}M^T)^+(Mq_0-b).
\]

This characterizes the proposed minimum-change policy, **not a guaranteed current PlaneGCS output**.

**P:** Eight distinct endpoint points, four ordinary pairwise welds, and four H/V constraints give `16−8−4 = 4 DOF` at a regular rectangle. Width and height are not preserved dimensions unless separately constrained.

### Main vectors

| Vector | Expected output and computation |
|---|---|
| **T2 — P** | Exact axis rectangle plus two implied Perpendiculars: every residual zero; `OverConstrained`, `conflicting=[]`. DOF unchanged. |
| **T3 — S/N** | Existing test restores separation 10 mm: residuals `0,10 mm`; both dimensions form the incompatible group. `Conflicting`, both IDs. At hypothetical separation 15 mm, both residuals are 5 mm. [Measured test](/Users/andrejvysny/workspace/OneCAD/worker/tests/test_solver_residual.cpp:114). |
| **T4, 10 m — N** | `τ_ang=1e−4/10000=1e−8 rad`. A `0.8°` deviation gives `10000 sin(0.8°)=139.621803 mm`: fail. An exact axis solve gives zero: pass. |
| **T4, micro — N** | For 0.5/0.3 mm sides, tolerances are `2e−4` / `3.333333e−4 rad`. Their `0.8°` deviations are `0.00698109` / `0.00418865 mm`: fail. Exact axis pose: pass. |
| **Same 1 mm deviation — N** | At 80 mm: `δ=asin(1/80)=0.012500326 rad`, `q≈10000.26`. At 10 m: `δ≈0.00010000000017 rad`, `q≈10000.00002`. Both fail consistently. |
| **T5 — P/N** | Starting `A=(0,0), B=(80,0), C=(80,60), D=(0,60)`, target A=(22,21): minimum-change solution is **A=(22,21), B=(80,21), C=(80,60), D=(22,60) mm**. Remaining free values minimize movement at `xR=80,yT=60`. Total squared corner movement `22²+21²+21²+22²=1850 mm²`. `success`; released `UnderConstrained`, DOF 4. |
| **T6 — P/S** | Move selected free point exactly to target; every other point unchanged. No pins. Existing unconstrained fast path already does this when no internal coupling exists. [Fast path](/Users/andrejvysny/workspace/OneCAD/worker/src/sketch/Sketch.cpp:1572). |
| **T7 — P/I** | Welded locked component remains at its original position. If the constrained target optimization terminates there: `success`, despite cursor lag. Numerical/guard failure: `partial`, same retained positions. The source does not determine which termination occurs for the supplied unspecified coordinates. |
| **T8 — P** | Arc center `(0,0)`, radius 10, endpoint `(10,0)`, line from `(10,0)` to `(20,0)`: arc coupling and weld hold exactly; line is radial, not tangent. Without tangency this is allowed. With tangency it must be corrected or report Tangent as violated; center-to-line distance gives `|0−10|=10 mm`. Exact drag output needs the missing target and other constraints. |
| **T9 — P/N** | `A=(0,0), B=(1,0), C=(√3/2,±1/2)` mm has unsigned angle 30° in both roots, but `S=±0.5 mm²`. Positive-root guard requires `S>1e−4 mm²`; negative candidate is refused. If Angle is signed, the negative root instead has a 60° residual—its actual semantics require verification. |
| **B3 — P, conditional** | Suppressing both implied Perpendiculars leaves nine constraints. If the unnamed eleventh original constraint is compatible and independent, released state is `UnderConstrained` with DOF reduced by its rank. Exact DOF cannot be named without it. Write-back must expose the corrected pose. Keeping the benign Perpendiculars instead requires `OverConstrained`, never `Conflicting`. |

### Boundary and degeneracy fixtures

**N/P — proposed fixtures, not executed measurements:**

| Input | Expected |
|---|---|
| Length residual `0.00009999 / 0.00010000 / 0.00010001 mm` | Pass / pass / fail under the operational `≤` rule. |
| Angular residual on 80 mm line `0.9999 / 1 / 1.0001 × 1.25e−6 rad` | Pass / pass / fail. |
| Zero-length line `(0,0)→(0,0)` | Undefined direction; refusal diagnostic. |
| Arc radius target `0.009 mm` | Clamp to `0.01 mm`. |
| Arc sweep `0.000999 / 0.001 / 0.001001 rad` | Refuse / accept floor / accept, assuming remaining checks pass. |
| Arc endpoint displaced `0.00010001 mm` from its parametric endpoint | Internal-coupling failure; no tag-0 culprit. |
| Free points P=Q=(0,0), Coincident; drag P→(1,2) | Both become `(1,2)`; unrelated point remains unchanged. |
| Ellipse `(majorR,minorR)=(6,3) mm`, no constraints | Vacuous residual pass; naive DOF policy and omitted `entityStates`. `Radius(ellipse)` is unsupported. |
| Constraint referencing missing `ghost` | `unassessed`, no fabricated residual zero or culprit. |
| `H(a), V(b), Parallel(a,b)` | XOR contradiction `0⊕1≠0`; witness contains all three constraints. |
| Normalized Jacobian rows `(1,0),(cosφ,sinφ)`, `φ=1e−12` | Condition number approximately `2/φ=2e12`; conditioning branch, not automatic conflict. |

## 7. Counterexample search

### Strongest residual miss: tangency’s second-order error

**P/N:** For a line through an arc endpoint, rotate the line by `φ` away from tangency:

\[
r_{\rm tangent}=R(1-\cos\phi)\approx R\phi^2/2.
\]

With `R=10 mm`, `φ=0.001 rad`:

- Length residual ≈ `0.000005 mm`: passes `1e−4 mm`.
- Angular mismatch corresponds to `R sinφ≈0.01 mm`: 100 times the intended positional budget.
- Endpoint angular tolerance would be `1e−4/10=1e−5 rad`: fails.

This does **not** defeat a promise limited to the existing length residual. It defeats the stronger interpretation that all accepted tangencies have equivalent geometric accuracy.

**S/P:** The current weld-gate comment equates a vanishing gradient with redundancy. Coincidence does not imply tangency; this is a singular formulation requiring a regular endpoint-tangency check. [Weld gate](/Users/andrejvysny/workspace/OneCAD/src/tools/sketch/autoConstrain.ts:429).

### False rejection from global angular scaling

**N:** A 0.5 mm line with error `5e−5 rad` has equivalent displacement `0.000025 mm`, which passes. An unrelated 10 m entity would give global `τ_ang=1e−8 rad`, wrongly rejecting it by a factor of 5000. Local operand scaling avoids this.

### Miss from a fixed angular epsilon

**N:** `τ_ang=1e−4 rad` accepts approximately `10000 sin(1e−4)=1 mm` deviation at 10 m. The proposed local tolerance is `1e−8 rad`, so it rejects it.

### Floating-point boundary

**N, conditional:** True residual `τ_len−1e−12 mm` plus evaluator error `2e−12 mm` becomes `τ_len+1e−12 mm`: a false rejection.

**I:** No audited evaluator error bound was supplied. Therefore “never falsely rejects any converged 10 m sketch” cannot be proved. For certified truth, residual intervals straddling tolerance require refinement or `numericallyUnresolved`, not a fabricated violation.

### Geometry perceived as anchored

**P/N:** A horizontal line with `Distance=80 mm`, but no Fixed constraint, starts at `(0,0)→(80,0)`. Driving its first endpoint to `(22,21)` can translate the second to `(102,21)` while satisfying every constraint exactly.

Drive-only behavior cannot preserve an anchor that the graph does not encode. Explicit Fixed/reference locking is needed.

## 8. Implementation handoff

### C++ interfaces

**I — proposed shapes:**

```cpp
enum class SolveState {
    UnderConstrained, FullyConstrained, OverConstrained, Conflicting
};

struct ResidualViolation {
    ConstraintID id;
    double residual;
    double tolerance;
    ResidualDimension dimension;
};

struct ResidualCheck {
    std::vector<ResidualViolation> violations;
    std::vector<ConstraintID> unassessed;
    std::vector<Diagnostic> diagnostics;
};

using AngularTolerance =
    std::function<double(const SketchConstraint&, const Sketch&)>;

ResidualCheck checkResiduals(
    std::span<const SketchConstraint* const> constraints,
    const Sketch& sketch,
    double tauLenMm,
    const AngularTolerance& tauAngRad);

struct TruthResult {
    std::optional<SolveState> state; // absent: unresolved/refused
    std::vector<ConstraintID> conflicting;
    std::vector<Diagnostic> diagnostics;
};

TruthResult classifyExact(
    const ResidualCheck& residuals,
    const ExactDiagnosis& releasedDiagnosis);

SolverResult solveWithDrag(
    const EntityID& draggedPoint,
    const Vec2d& target,
    GestureContext& gesture); // no pin set
```

**P:** Constraints, sketch, and tolerances alone cannot determine rank conflicts, redundancy, or DOF. The classification interface needs released diagnosis explicitly.

### Shared pure authoring rule

**I:**

```ts
type DirectionDecision =
  | { kind: "independent" }
  | { kind: "redundant"; witness: readonly string[] }
  | { kind: "contradictory"; witness: readonly string[] }
  | { kind: "invalid"; reason: string };

function classifyDirectionCandidate(
  graph: readonly DirectionConstraint[],
  candidate: DirectionConstraint,
): DirectionDecision;
```

Use one graph implementation in both authoring lanes. Include the axis node, cycle detection, deterministic witnesses, and candidate exclusion. Preserve provenance to resolve simultaneous snap/inference precedence.

### Ownership and diagnostics

**I:**

- **`ConstraintSolver.cpp`:** remove fallback pin sets; expose primary feasibility/termination; support deterministic movement policy and continuation.
- **`Sketch.cpp`:** complete-pose rollback, locked-value integrity, arc/branch guards.
- **`SolverLane.cpp`:** final-pose gate/classification and authoritative pose publication.
- **`autoConstrain.ts` / shared pure graph rule:** final direction admission.
- **Rust document handling:** apply returned positions and curve scalars atomically with state. Its implementation was outside the allowlist.
- **Range analyzer:** no role is established by the supplied sources.

Log algorithm path, termination reason, normalized worst residual, typed violations, rank evidence, policy version, and pose revision. Refusal diagnostics distinguish `residualViolation`, `unassessedConstraint`, `branchBoundary`, `internalCouplingFailure`, and `numericallyUnresolved`. Publish target lag separately from constraint blame.

### SCHEMA §7.4 changes

**I:**

1. Define `Conflicting` as named constraint failure in the reported pose or a verified incompatible group; distinguish these through additive reasons. Remove the blanket “no solution exists” implication.
2. Preserve mixed-dimension `maxResidual` as reporting-only legacy data. Add typed violation details and a dimensionless normalized maximum.
3. Add authoritative `positions` and `curves` to Upsert. Exact publication must not omit changes through preview-delta thresholds.
4. Require EndGesture’s released solve/gate and fresh final state/conflicts; retain existing `entityStates` echo semantics.
5. Define lagging feasible drag as success; guarded or non-convergent steps as partial. Temporary drives never become user culprits.
6. Unreadable constraints cannot establish a successful truth gate merely because legacy `maxResidual` skipped them.

**I:** **Bump `solverPolicyVersion`.** Acceptance decisions, redundancy interpretation, drag poses, branch selection, and final-state derivation change. This is not an additive-reporting-only revision. The schema currently documents version 1; the runtime-advertised value must be checked before assigning its successor.

## 9. Confidence and the unknowns that would change the answer

- **High:** dimensional separation, local angular scaling, parity implication rule, T5’s minimum-change coordinates, released-state requirement, and the weld/tangency counterexample.
- **Medium:** proposed state/diagnostic policy and local drag continuation design.
- **Low until validated:** byte-identical nonlinear results, ≤8 ms performance, numerical error bounds, and current solver behavior on T1/T8/T9.

**Unresolved questions:**

- Exact T1 endpoint coordinates/IDs, anchors, and B3’s eleventh constraint?
- Actual per-kind `getError` formulas, units, aggregation, and signed Angle semantics?
- Endpoint-tangency representation and required angular-intent precision?
- Required deterministic scope: same executable/platform, or across builds/platforms?
- Snap provenance for simultaneous explicit polar intent versus incidental H/V inference?

**TODO:** derivation complete; implementation and runtime tests not performed.