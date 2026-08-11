# OneCAD Modeling Correctness Roadmap

Horizon: 3–6 months  
Priority: correctness and robustness before feature breadth  
Baseline: `1c11d4958aeadea14dd8431ba78c41f14be12142`

## Outcome

At the end of this roadmap, every supported modeling operation should have:

- an explicit publication and body-lifecycle contract,
- consistent validation appropriate to its risk,
- fail-closed semantic-reference ownership and snapshot handling,
- preview behavior that matches commit or is visibly classified as approximate,
- evidence at kernel, real-worker and user-flow layers,
- deterministic replay and persistence assertions,
- CI that is required rather than informational,
- a robustness campaign for the highest-risk OCCT operations.

This roadmap does not add Loft, Sweep, assemblies, FEM, TechDraw, CAM, addon-defined kernel operations, or production Fillet rescue algorithms.

## Sequencing principles

1. Prevent data loss and false success before improving advanced geometry.
2. Fix known preview/commit and publication defects before adding broad test matrices.
3. Enforce semantic-reference ownership and snapshot consistency before tuning the naming ladder.
4. Preserve the accepted ordinary-edit teleport residual; do not accidentally alter resolver policy.
5. Use red-first tests for every defect.
6. Separate portable semantics from same-machine digests.
7. Do not make expensive validation always-on until measured.
8. Keep user-authored parameter values canonical; do not silently clamp at lower layers.

## Roadmap overview

| Phase | Duration | Goal | Primary outputs |
|---|---:|---|---|
| 0. Safety and truth | 2–3 weeks | Stop data loss, empty-body publication, false success and stale authoring | P0 fixes, shared result classifier, atomic tests |
| 1. Semantic-reference integrity | 4–6 weeks | Make body ownership and snapshot provenance fail closed | Core ownership/fences first; repair provenance and Revolve axis as follow-ups |
| 2. Exact profile geometry | 3–5 weeks | Preserve analytic curves through region intersections and harden profile construction | Exact curve fragments, analytic fixtures, tolerance policy |
| 3. Publication policy and operation semantics | 3–4 weeks | Give every operation an explicit output-shape and lifecycle contract | Shared validator, multi-solid decisions, mode alignment |
| 4. Vertical evidence and executable oracle | 3–4 weeks | Close operation/mode test gaps and execute the corpus | Coverage manifest, Boolean/Pattern/Mirror/mode tests, corpus runner |
| 5. Robustness breadth | 4–6 weeks | Expand kernelbench beyond Fillet and complete generic metamorph/validator groundwork | Boolean and Chamfer campaigns, M3/M4 completion, m1 CI |
| 6. Required cross-platform release gates | 2–4 weeks | Make green binding and extend platform evidence | Required checks, Linux Chromium classification, Windows smoke |

For one founder, Phases 0–4 are the committed 3–5 month core. Phase 5 is the preferred stretch within a six-month window if the earlier gates land near their lower estimates. Full completion of all phases is more realistically 5–8 months. Phase 6 overlaps only with spare or external capacity; otherwise sequence its required-check stabilization after Phase 4 and treat Windows work as second-tranche scope.

## Phase 0 — Safety and truth

Specification: `phase-0-safety-and-truth.md`

### Goals

- Route Open and every replacement action through the unsaved-changes guard.
- Make successful save update frontend clean/path/title state from an authoritative backend result.
- Centralize `ApplyOperationResult` classification.
- Fix empty-result Boolean lifecycle.
- Make circular-pattern partial preview use the settled worker/protocol `angle/count` rule.
- Reproduce the curved-wall Draft risk and require applied-or-refused behavior if confirmed.
- Preserve concurrent exact region previews and restore actionable Boolean controls after failure.

### Exit gates

- Dirty Open presents Save/Discard/Cancel and Cancel preserves document, selection and active tool.
- A backend-resolved regen failure never produces a success hint in any operation/history/repair path.
- Disjoint Intersect and full-consumption Cut cannot publish an empty body.
- Partial circular-pattern ghost transforms equal committed transforms.
- Circular-profile Draft either produces proven taper or returns a named refusal.

## Phase 1 — Semantic-reference integrity

Specification: `phase-1-semantic-reference-integrity.md`

### Goals

- Validate all Fillet/Chamfer edges share one body before resolution.
- Require Hole, Shell and OffsetFace typed refs to match targetBodyId consistently on both Rust and worker boundaries.
- Fail closed when ToFace or Hole promotion is stale/refused.
- Add the missing post-response fence to PrepareOffsetFace.
- Carry snapshot provenance through repair candidates and invalidate caches on newer revision/events.
- Clarify or fix Revolve body-edge axis identity.

### Exit gates

- Mixed-body references are rejected before descriptor scoring.
- Stale authoring responses cannot create records.
- Cached repair candidates cannot be promoted against another snapshot.
- No threshold relaxation or resolver-policy change.

## Phase 2 — Exact profile geometry

Specification: `phase-2-exact-profile-geometry.md`

### Goals

- Replace polygon fallback for intersected analytic curves with parameterized curve fragments.
- Preserve Arc/Circle/Ellipse type and parameter ranges into BRep edges.
- Establish scale-aware region intersection and gap-repair tolerances.
- Add cancellation/resource controls to dense intersection work.

### Exit gates

- Overlapping circles produce analytic arcs and closed-form areas.
- Arc-circle and ellipse-line regions remain analytic.
- Entity-order and harmless transform metamorphs preserve region ids and BRep semantics.
- Existing simple-region ids remain byte-stable.

## Phase 3 — Publication policy and operation semantics

Specification: `phase-3-publication-policy.md`

### Goals

- Introduce one policy-driven publication validator.
- Define operation-specific allowed output shape classes and empty/multi-solid behavior.
- Resolve Pattern/Mirror/Hole multi-solid semantics with compatibility handling.
- Align frontend-exposed modes with persisted/kernel modes.
- Add cancellation and workload ceilings to long operation loops.

### Recommended policy decisions

- Boolean Cut/Intersect zero-solid result: delete target only when the product explicitly treats it as a valid complete-consumption outcome; otherwise refuse. Do not infer.
- Hole: preserve the documented legacy multi-solid residual for existing records; if reopened, version new-record behavior so a disconnected host can refuse without changing legacy replay.
- Pattern fused mode: require exactly one connected solid.
- Pattern non-fused mode: for new records, prefer deterministic child bodies per instance; preserve legacy compound behavior by version.
- Mirror fused mode: require one connected solid; no-fuse remains one mirrored body.
- Import invalid-solid handling remains a policy decision and is not silently converted from warning to failure.

### Exit gates

- Every worker operation calls the common validator or an explicitly documented import policy.
- Validation overhead is measured by operation class.
- Body lifecycle and mesh behavior are unambiguous for empty and multi-solid results.

## Phase 4 — Vertical evidence and executable oracle

Specification: `phase-4-vertical-evidence.md`

### Goals

- Close standalone Boolean Intersect coverage across C++, Rust, frontend and Playwright.
- Add LinearPattern, CircularPattern and body Mirror Playwright flows.
- Close critical mode gaps for Extrude, Revolve, Shell, Hole and OffsetFace.
- Compile every corpus case into an executable, classified test.
- Add a machine-readable operation/mode coverage manifest.

### Exit gates

- Every supported operation and meaningful mode has an explicit layer classification.
- No corpus case is inert or unclassified.
- Playwright remains zero-retry unless the user changes the settled policy.
- Real-worker tests assert preview/commit/reopen parity for high-risk modes.

## Phase 5 — Robustness breadth

Specification: `phase-5-robustness-breadth.md`

### Goals

- Complete current kernelbench M3 metamorph execution.
- Complete M4 recipe-agnostic validators.
- Add `boolean/foundation:t0` as the first non-Fillet operation family.
- Add `chamfer/foundation:t0` second.
- Add small characterization suites for Shell, OffsetFace and Hole only after publication policy is stable.
- Gate `fillet/matrix:m1` semantics and record the stable-host Linux digest.

### Exit gates

- Boolean and Chamfer run raw-OCCT versus OneCAD with replay, metamorphic and deep-audit evidence.
- T0 stays unchanged.
- Required curved-support validators no longer depend on box-specific heuristics.
- Every new baseline change has a reviewed explanation.

## Phase 6 — Required cross-platform release gates

Specification: `phase-6-required-cross-platform-gates.md`

### Goals

- Verify and configure required branch checks from a dated GitHub ruleset/settings snapshot.
- Preserve the settled zero-retry baseline unless the user explicitly reopens it.
- Install runner prerequisites and classify the Linux Chromium Boolean-pick difference.
- Add Windows core/protocol/worker-supervision smoke before full Windows OCCT campaigns.
- Retain macOS packaging and WebKit as shipping gates.

### Exit gates

- A merge cannot bypass agreed correctness gates.
- Linux and macOS semantics are compared portably.
- Windows process/path/bundle assumptions have at least smoke evidence.
- Failure traces are retained with zero whole-test retries.

## Dependency graph

- Phase 0 must precede all other phases because false-success UI and empty-body publication invalidate test interpretation.
- Phase 1 precedes Phase 4 repair/e2e expansion and all further identity work.
- Phase 2 precedes robustness campaigns involving curved sketch-derived profiles.
- Phase 3 precedes broad Pattern/Mirror/Hole campaigns because output-shape semantics must be stable before baselines are recorded.
- Phase 4 can overlap the latter half of Phase 3 for unaffected operations.
- Phase 5 depends on the shared validator from Phase 3 and the coverage manifest from Phase 4.
- Phase 6 overlaps only with spare or external capacity. On the solo-founder schedule, perform small CI preparation earlier but sequence required-check stabilization after Phase 4.

## Milestones and decision gates

### Gate A — before Phase 0 Boolean implementation

Use fail-closed refusal for zero-solid Cut/Intersect in Phase 0. Any later decision to treat complete consumption as valid deletion is a separate product/protocol change with explicit lifecycle events and compatibility review.

### Gate B — after Phase 1

Confirm the current naming policy remains unchanged. Any proposal to close the ordinary-edit teleport residual is a separate design program.

### Gate C — during Phase 3

Choose non-fused Pattern output semantics and versioning. Do not ship a silent one-body-to-many-solids behavior change.

### Gate D — after Phase 4

Choose the second-tranche allocation:

- robustness breadth and cross-platform gates, recommended; or
- return to platform/addon backend work.

Do not proceed to production Fillet rescue strategies before Boolean and Chamfer robustness evidence exists.

## Recommended first work package

Start with a single gate named `MODEL-CORRECTNESS-P0`:

1. Red-first standalone Boolean disjoint Intersect and full-consumption Cut tests.
2. Fix zero-solid result handling in the shared Boolean publication path.
3. Red-first partial circular-pattern preview/commit parity test.
4. Fix the formula divergence at one normative source.
5. Red-first dirty Open test and route it through the existing guard.
6. Add the shared result classifier and migrate Pattern/Mirror first, then all remaining call sites.
7. Red-first circular-profile Draft test and refuse successful no-op draft.
8. Run the established developer-Mac and trusted-Linux gate lanes and record results before commit.

This package produces immediate user-visible risk reduction without changing the naming ladder, protocol version, or advanced Fillet implementation.

## Measurement dashboard

Track these metrics per phase:

- supported operation/mode coverage by layer,
- operations using the shared publication validator,
- number of unresolved P0/P1 findings,
- real-worker preview/commit/reopen parity cases,
- kernelbench operation families and gating cases,
- safe-refusal versus invalid-publication counts in campaigns,
- same-host replay instability,
- CI required-check pass rate without retries,
- P50/P95 operation time and validation overhead,
- no net increase from the current 12-item manual release baseline; reduce it whenever automated evidence replaces a check (`docs/qa/MANUAL_RELEASE_GATES.md`).

## Risks

- A universal deep audit can make interactive operations unusably slow. Measure and tier it.
- Changing Pattern result shape can break downstream BodyIds. Version it.
- Exact analytic curve splitting can change region signatures. Preserve simple regions and introduce an explicit region-identity migration plan for fragmented regions.
- Adding branch protection before the remaining CI tail is condition-based will block direct-to-master work unpredictably.
- Broadening kernelbench before publication semantics settle will freeze the wrong behavior.
- Treating import warnings as failures without a product decision would regress the ability to open marginal STEP files.

## Explicit non-goals

- Loft and Sweep implementation.
- Assemblies and mates.
- FEM, TechDraw or CAM modules.
- Addon-defined modeling operations.
- Production variable-radius/G2/overflow/corner Fillet algorithms.
- Relaxing topological-naming thresholds.
- Compressing or deleting the history ledgers.
