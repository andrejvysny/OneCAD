# Phase 4 Specification — Vertical Evidence and Executable Oracle

Duration: 3–4 weeks  
Prerequisites: Phases 0 and 1; Phase 3 policy stable for affected operations  
Priority: P1 evidence closure  
Gate name: `OPERATION-EVIDENCE-MATRIX`

## Rationale

OneCAD has many tests, but operation evidence is vertically uneven. Some operations are strong in C++ and absent from the browser; some browser flows are mock-only; some modes exist in core/worker but are not authored by the UI. The frozen corpus describes executable scenarios but is not itself fully discovered and run.

## Goals

1. Close the highest-risk operation/mode gaps.
2. Make every corpus case executable or explicitly classified.
3. Add a machine-readable operation coverage manifest.
4. Add real-worker preview/commit/reopen parity for high-risk paths.
5. Keep Playwright zero-retry and condition-based.

## Non-goals

- Quantitative source-line coverage targets.
- Replacing CTest/Rust with browser tests.
- Making mock e2e claim real kernel geometry.
- Expanding the manual release list.

## Work package 4.1 — Standalone Boolean Intersect

### C++ fixtures

Use analytic boxes:

- overlap: exact common volume,
- disjoint: chosen empty-result policy,
- containment,
- touching face/edge/vertex,
- complete tool/target identity cases,
- split/merge lifecycle where reachable.

Assert:

- BRep validity,
- solid count,
- volume and centroid,
- target/tool lifecycle,
- deterministic signatures,
- no partial mutation on failure.

### Rust real-worker

- Preview equals commit.
- Preview does not mutate head.
- Save/reopen preserves result and ids.
- Undo restores target and tool.
- Missing or duplicated body ids fail atomically.

### Frontend/Playwright

- Select two bodies.
- Choose Intersect.
- Assert preview params.
- Apply once.
- Assert row label, target/tool visibility and undo.
- Inject resolved regen failure and verify no success state.

## Work package 4.2 — Pattern and body-mirror user paths

Add Playwright specs for:

LinearPattern:

- axis, count, spacing,
- exact ghost count,
- apply, undo, re-edit,
- failure retains params.

CircularPattern:

- full and partial angle,
- exact transform positions,
- apply, undo, re-edit,
- parity with worker centroids.

MirrorBody:

- plane choice,
- reflected ghost,
- no-fuse and fuse if exposed,
- apply, undo, re-edit,
- source preservation.

These must use the modeling body Mirror tool, not sketch mirror.

## Work package 4.3 — Critical mode closure

Minimum portfolio:

- Extrude successful ToFace, ToNext, Symmetric, two-direction, Add and chosen Intersect policy.
- Revolve Add/Cut/Intersect if exposed.
- Shell multi-open-face and cross-body selection refusal.
- OffsetFace Total/Radius/Diameter commit and re-edit.
- Hole countersink commit/reopen.
- Chamfer curved/short-edge refusal atomicity.
- Draft curved-wall applied-or-refused.

## Work package 4.4 — Execute the corpus

### Required runner

A corpus test enumerates every `corpus/cases/*.json` and classifies it as:

- executable operation case,
- executable solver case,
- anti-goal identity case,
- explicitly unsupported with reason.

No silent skip.

### Assertions

- plans compile against the current schema,
- expected body events and values match,
- anti-goal naming cases auto-bind correctly or produce NeedsRepair, never wrong bind,
- timeline/rollback semantics match,
- region counts and identities match the stated oracle,
- provenance citations remain present.

### CI placement

Run fast deterministic cases in normal Rust/worker CI. Keep expensive or platform-sensitive cases in a named campaign, never as untracked manual instructions.

Relevant source: `corpus/README.md:64-108`.

## Work package 4.5 — Operation coverage manifest

### Manifest fields

```text
operation
mode
supportStatus
cppTest
rustRealWorkerTest
frontendTest
playwrightTest
corpusCase
kernelbenchSuite
ciJob
notes
```

### Generation and checks

CI compares the manifest against:

- `KnownOperation`,
- worker dispatch,
- protocol tags and mode enums,
- frontend tool registrations,
- kernelbench operation types.

Any new supported value without classification fails. Unsupported Loft/Sweep are valid classified rows.

## Work package 4.6 — Failure observability tests

For selected high-risk operations, assert that structured diagnostics are visible in the feature inspector or failure panel:

- code,
- stage,
- message,
- bounded evidence,
- recovery action.

Do not rely on a browser tooltip.

## Test quality rules

- No arbitrary sleep where a condition can be observed.
- `count()` is not the first readiness probe for mounting UI.
- Click one consequential action once; poll postcondition instead of retrying the click.
- Mock e2e asserts command and UX behavior, not OCCT geometry.
- Real-worker tests assert geometry and lifecycle.
- Every negative test proves the pre-operation state is unchanged.
- Each test has a negative control where practical.

## Acceptance gates

- All manifest rows classified.
- Boolean Intersect vertical path complete.
- Pattern and body Mirror e2e present.
- Corpus enumeration has zero unclassified files.
- Playwright full suite green on Chromium and WebKit with retries 0.
- Rust workspace runs with `ONECAD_REQUIRE_WORKER=1`.
- No increase in permanently manual checks.

## Risks

- E2E helpers can hide product failures if they retry actions.
- Corpus field names may be stale; fix through explicit versioned compilation, not by mutating old provenance.
- A manifest can become ceremonial unless generated values and positive controls are tested.

## Rollback

Coverage infrastructure is additive. Operation behavior changes belong in prior phases and should not be mixed into this test-only gate except when a red-first fixture reveals a new defect.
