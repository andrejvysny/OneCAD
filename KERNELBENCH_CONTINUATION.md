# Claude Code continuation: OneCAD Kernelbench KBR-0 + Fillet slice

## Continuation prompt

Continue and finish the in-progress OneCAD robustness benchmark vertical slice in this checkout. Treat this file as the task brief and current-state ledger. Start by reading `AGENTS.md`, `CURRENT_STATE.md`, `TODO.md`, and `CLAUDE.md`, then inspect the live files because the last agents were interrupted during a selector/validation refactor.

Rules:

- Preserve unrelated dirty work. Scope changes to files listed below unless a build manifest, CI, or `TODO.md` overlap is required.
- No commit, push, pull, reset, checkout, or cleanup.
- Use parallel subagents for independent work; assign explicit file ownership. Agents are not alone and must not revert others.
- Update `TODO.md` after completed gates.
- Functions under 50 lines; files under 500 lines.
- Use `apply_patch` for edits.
- Keep production `ShapeAudit`, protocol, UI, tolerance policy, `corpus/`, and `filletbench` behavior unchanged.
- Runner stdout is exactly one JSON result line. `onecad-worker` stdout remains OCW1 frames only.
- Shipping gate is OCCT 8.0.1. OCCT 7.9.3 is informational.

Repository state at handoff:

- Branch: `master`
- HEAD: `a4f8f9f`
- Worktree was already very dirty before this task. Do not sweep or reset it.
- Task files are uncommitted. No commit/push/pull was performed.
- Previous subagents were interrupted. C++ is currently not buildable; Rust unit tests pass with one warning.

## User intent and required specification

Build the first complete robustness benchmark slice:

- Public Rust executable/crate `onecad-kernelbench` as supervisor.
- Separate internal C++ executable `onecad-kernelbench-runner`.
- Differential raw OCCT (`BRepFilletAPI_MakeFillet`) versus production OneCAD `FilletBuilder`; never copy production modeling logic.
- Deterministic generation, deep audits, semantic validation, metamorphs, critical-radius search, JSONL records, and differential/report summaries.
- Fixture/schemas/presets/regressions live under `bench/robustness/`; frozen legacy `corpus/` is untouched.
- `filletbench` remains the performance microbenchmark.

### CLI and exits

```text
onecad-kernelbench run --suite fillet/foundation --preset t0 \
  --backend raw-occt|onecad|both --runner PATH --out-dir DIR

onecad-kernelbench run-case CASE.json \
  --backend raw-occt|onecad|both --runner PATH --out-dir DIR

onecad-kernelbench search-critical CASE.json \
  --param operation.radius --backend raw-occt|onecad|both \
  --runner PATH --out-dir DIR

onecad-kernelbench report RESULTS.jsonl --json SUMMARY.json
```

Runner resolution: `--runner`, `ONECAD_KERNELBENCH_RUNNER`, sibling binary, else environment error. Exit `0` pass, `1` regression, `2` CLI/schema error, `3` missing/unsupported execution environment.

### Case/result contract

- Case schema v1 requires exact `schemaVersion`, safe `caseId`, generator `{name,version,seed}` with exactly 16 lowercase hex digits, recipe/tags, constant-radius G1 fillet, semantic selector, domain, validators, metamorphs, optional search, resource/quality limits.
- Reject unknown required-block fields, non-finite/bounded-invalid numbers, oversized input, unsupported generator versions, invalid selectors, and unsafe artifact identifiers.
- Persist semantic selector evidence only: generator provenance, recipe-local anchors, surface descriptors, adjacency, topology roles. Never persist raw OCCT ordinals.
- Result schema v1 contains identity, verdict, execution/operation states, stable failure class, bounded diagnostics, input/output audits, validators, selection evidence, timing/resources, metamorph/search/replay/differential evidence, relative artifacts, `inputDigest`, and `normalizedDigest`.
- Digest includes stable structured states, quantized metrics, selection evidence, and validator outcomes. Exclude timing, RSS, paths, localized text, timestamps, and campaign comparison wrappers used only after replay comparison.
- Validate every runner record and every report input record strictly. Empty/malformed report input exits `2`.

### C++ runner/audit

- Read one bounded request from stdin; write exactly one result JSON line to stdout. Malformed request exits exactly `2` with empty stdout and schema error on stderr.
- Top-level OCCT/C++ exception boundary returns structured result.
- Capture contour, assigned radius, generated faces, partial shape, and kernel diagnostics while builders live.
- Failure artifacts: canonical case, request, stderr, input BREP, available output/partial BREP. Existing pinned BREP codec only. Total artifacts, not each file, obey the configured cumulative cap.
- Compose deep benchmark audit around unchanged production `audit_shape()`:
  - exact `BRepCheck_Analyzer`;
  - single-thread full `BOPAlgo_CheckerSI`;
  - closed-manifold edge-use check;
  - topology counts, volume, area, centroid, inertia, bounds;
  - tolerance count/max/mean/p95 for vertices/edges/faces;
  - bbox-normalized micro-edge/sliver metrics;
  - no tolerance mutation or generic production threshold.
- Every reported successful output must unconditionally be publication-valid by deep audit, even if the case omitted a `deepAudit` validator. Invalid successful output becomes `badShape`/`auditFailed`; partial/BadShape never passes.
- Implement validator forms, including explicit `radiusTolerance`, `tangencyTolerance`, and `materialTolerance` thresholds. Constant-radius must use actual builder radius assignment evidence, not contour count alone.

### Frozen T0 suite and policy

- Frozen SplitMix64 with explicit integer-to-double conversion; no standard distributions.
- Preset seed: `6f6e656361647430`.
- 36 base cases:
  - 12 supported analytic boxes: single, disconnected, multiple edges at safe ratios;
  - 4 analytically impossible oversized boxes;
  - 8 exploratory valence-3 corners;
  - 8 exploratory valence-4 corners;
  - 4 exploratory overflow wedges.
- Translation `[1000,-2000,3000]` mm and rotation `17.137°` about input centroid around normalized `[1,2,3]` apply to supported/expected-limit bases.
- Total: 68 variants × 2 backends = 136 canonical records. Execute each twice: canonical plus replay evidence = 272 children.
- Semantic validators: assigned constant radius/generated blend evidence; applicable cylindrical radius; G1 tangency; material direction; unchanged remote analytic supports; deep audit.
- Domain gate:
  - `supported`: semantic rejection, instability, or raw-success/OneCAD-failure is red;
  - `expectedLimit`: deterministic safe refusal passes;
  - `exploratory`: refusal is characterization; crash, timeout, nondeterminism, or invalid OneCAD publication is red;
  - never gate exact topology counts or raw BREP equality.
- Differential summary separates rescued cases, supported OneCAD regressions, status differences, input mismatches, and audit-quality deltas. Do not classify exploratory raw-pass/OneCAD-refusal as a regression.

### Metamorph/search/isolation/report

- Metamorph comparison must inverse-transform output evidence, then genuinely compare normalized properties, semantic evidence, deterministic output-surface samples, and point classifications within case thresholds. Never fabricate `surfaceSamplesMatch` or `pointClassificationMatch` by aliasing other booleans. If evidence is absent, report `notRun`; final implementation should add real runner evidence and gate supported/expected-limit variants.
- Critical search: known success → deterministic growth → sweep brackets before assuming one transition → bisect locally consistent intervals → adaptive subdivision for multiple transitions → offsets `1e-2`, `1e-4`, `1e-6`, `1e-8`; stop at `maxProbes`, relative `1e-6`, or absolute `1e-6 mm`. Report intervals and `monotonicObserved`, never an exact critical radius/global monotonic claim.
- One disposable child per backend/variant/probe. Hard timeout, kill/reap/continue, bounded streams/case/artifacts, Unix limits with documented unsafe invariant. macOS must use RSS monitoring, not `RLIMIT_AS`; Windows is unsupported and returns exit `3`.
- Canonical/replay and every search probe use separate artifact directories; persisted paths are relative to out-dir. Every search record carries its actual radius inside `search.probeRadius`.
- Deterministic record ordering independent of child completion.
- Report counts by verdict/domain/generator/backend/failure; differential table; replay/metamorph stability; deduplicated transition groups; quality distributions; nearest-rank p50/p95 timing; JSON summary plus concise stderr table.

### CI

- Required OCCT 8.0.1 T0 job using pinned artifact. Upload JSONL, summary, and failures even on regression.
- Smaller OCCT 7.9.3 informational characterization in existing lane.
- Preserve all existing worker/Rust/frontend jobs and one-way 7.9.3→8.0.1 persistence gate.
- No relative performance threshold yet; record first same-host p50/p95 in `TODO.md`.

Deferred: boolean/later operations, history adapter, scale/mirror, minimizer/promotion, HTML/trends, STEP/reference ingestion, imported defects, Windows limits, advanced fillet modes.

## Scoped files already added/modified

```text
bench/robustness/**
src-tauri/crates/onecad-kernelbench/**
worker/src/benchmark/**
worker/tools/kernelbench-runner/**
src-tauri/Cargo.toml
src-tauri/Cargo.lock
worker/CMakeLists.txt
.github/workflows/ci.yml
TODO.md
```

Current added files include strict case/result/preset schemas, two regression cases, Rust CLI/suite/search/report/supervisor tests, C++ parser/generator/audit/adapters/artifacts/execution, malformed-request harness, and audit fixtures.

## What was green before the final hardening refactor

These are useful historical baselines only; rerun everything because the latest selector and validation changes invalidate them.

- Worker build succeeded with explicit local provenance:
  `ONECAD_OCCT_BUILD_ID=homebrew-occt-8.0.1-20260807 scripts/build-worker.sh Release`
- Full CTest: 106/106.
- Cargo fmt/clippy/workspace tests passed against the real worker.
- T0: 136/136 canonical records passed; zero replay, metamorph, or input-digest mismatches; case/result schemas validated.
- Critical-search smoke found one interval near 16 mm and schema validated.
- `git diff --check` and CI YAML parse passed.

## Exact current state and first fixes

### 1. C++ is currently broken by interrupted semantic-selector migration

Current command/result:

```text
cmake --build worker/build --target onecad-kernelbench-runner -j4

Geometry.cpp:192: no member named 'indices' in SelectorSpec
Geometry.cpp:301: no member named 'role' in SelectorSpec
```

`Types.h` and `SelectorParser.*` use the new semantic selector, but `Geometry.cpp` still calls `select_indices()` and emits `selector.role`. Replace ordinal selection with recipe-local anchor matching plus mode/adjacency validation. Emit `selector.topology_role`. Use deterministic nearest anchor matching with explicit tolerance and unique matches; corner selectors use the common-vertex anchor to find the incident group. Do not reintroduce persisted ordinals.

Then inspect the interrupted `Execution.cpp` → `SemanticValidation.*` split. Both files currently contain overlapping validator/publication code and are both in CMake. Finish the split without duplicate definitions. `Execution.cpp` is exactly 500 lines; it must be under 500.

Already partly implemented and should be preserved:

- `FilletRun.cpp` captures assigned-radius count/max error and guards raw `Generated()` until `IsDone()`.
- `Artifacts.cpp` now uses a cumulative budget.
- `SemanticValidation.*` contains publication-valid and tolerance validator work.
- Exact malformed-request CMake harness replaced bare `WILL_FAIL`.
- Audit fixture was expanded.
- `CaseParser.cpp` quality count bounds and exception/result digest work need confirmation with tests.

### 2. Rust compiles, but hardening is incomplete

Current command/result:

```text
cd src-tauri
cargo test -p onecad-kernelbench --lib
# 20 passed; one dead_code warning for ValidatedState fields
```

Immediate fixes:

- Run `cargo fmt --all`; interrupted validation files are visibly unformatted.
- `runner.rs` is 504 lines; split below 500.
- Remove/use `ValidatedState` fields so clippy `-D warnings` passes.
- `result_validation::validate` currently checks only `caseId` and backend. Pass `&Case`; require exact generator and expected-domain equality before supervisor policy overwrites anything.
- `result_validation::persisted` incorrectly strips optional top-level `probeRadius`; final schema places radius at `search.probeRadius`. Remove top-level handling.
- `result_validation_blocks::search` currently omits required `probeRadius`; add finite positive bounded validation and exact field set.
- Supervisor synthetic artifacts must obey the configured cumulative cap while writing, not write beyond it and merely change verdict.
- `configure_memory_limit` still applies `RLIMIT_AS` on macOS. Gate pre-exec limit to Unix excluding macOS; retain macOS RSS monitoring. Windows runner resolution already returns exit `3`.
- Verify non-executable runner path returns exit `3`.
- Report strict validation exists but needs tests for empty JSONL, scalar JSON, unknown fields, and valid campaign/search records.

### 3. Semantic selector Rust generation/validation is mid-change

`case.rs` and `suite.rs` contain the new selector structs. `case_validation.rs` still validates the removed `indices` contract and must be replaced with semantic checks:

- provenance generator/recipe/feature must equal the case recipe;
- anchor arrays bounded/finite; surface descriptor adjacency is exactly two surfaces;
- mode ↔ anchor count ↔ adjacency relation consistency;
- box → `verticalEdge`; valence-3/4 → `cornerIncidentEdge`; overflow → `overflowEdge`.

Important: `add_exploratory()` currently assigns `CornerIncident` to overflow wedges, while the schema requires any corner-incident selector to use `cornerIncidentEdge`. Change overflow cases to an appropriate non-corner mode, likely `Single`, so `overflowEdge` is valid. Then JSON-schema validate the generated 36 bases/68 variants, not only Rust deserialize/validate.

Committed schema selector shape:

```json
{
  "mode": "single",
  "topologyRole": "verticalEdge",
  "provenance": {
    "generator": "fillet-foundation",
    "recipeType": "box",
    "featureIndex": 0
  },
  "anchors": [
    {"kind": "edgeMidpoint", "point": [0, 0, 6], "frame": "recipeLocal"}
  ],
  "surfaceDescriptors": [
    {"curveKind": "line", "adjacentSurfaceKinds": ["plane", "plane"]}
  ],
  "adjacency": {"relation": "single"}
}
```

Schemas/fixtures currently validate under Draft 2020-12; legacy `role/indices` and inconsistent adjacency negative probes reject.

### 4. Campaign/search/report improvements already present; verify them

- Canonical and replay use separate directories.
- Search uses per-backend/per-radius directories and moves temporary radius into `search.probeRadius`.
- Differential checks input digests, gates input mismatch, restricts OneCAD regression to supported domain, and reports status differences.
- Digest excludes replay/metamorph/differential/search wrappers so the stored replay digest remains comparable after campaign annotations.
- Report rejects empty input, calls strict persisted-result validation, deduplicates search evidence, and uses corrected nearest-rank percentile.

Add/verify tests for all of these. Ensure artifact rebasing remains relative and schema-safe.

### 5. Metamorph evidence remains the largest functional gap

Current `campaign.rs` still fabricates:

```text
surfaceSamplesMatch = semanticEvidenceMatch
pointClassificationMatch = normalizedPropertiesMatch
```

This must not ship. Add real bounded comparison evidence from the C++ output shape, schema it, validate it, inverse-transform it in Rust, compare order-independently within the case threshold, and gate supported/expected-limit variants. A reasonable compact contract is deterministic face sample `{point,surfaceKind}` entries plus deterministic `{point,state}` solid classifications. If full evidence cannot be completed immediately, set metamorph status to `notRun` and booleans false rather than claiming pass; however, the requested final slice requires genuine evidence before completion.

## Outstanding review findings checklist

- [ ] C++ unconditional output audit publication gate proven even when case omits `deepAudit`.
- [ ] Radius/tangency/material tolerance validator thresholds implemented and tested.
- [ ] Actual constant-radius assignment evidence used.
- [ ] Semantic selector has no persisted OCCT ordinals and resolves deterministically in fresh processes.
- [ ] Overflow recipe/selector expresses intended geometry; no dead dimension branch.
- [ ] Cumulative runner and supervisor artifact caps proven.
- [ ] Quality count maximum 10,000,000 mirrored in C++.
- [ ] Exception digest uses request semantic input when geometry is unavailable; normalized digest includes structured states/evidence.
- [ ] Invalid BRep audit fixture plus supported/oversized/partial/valence/disconnected runner tests.
- [ ] Full result schema validation before trusting runner/report records.
- [ ] Supervisor recomputes domain policy; partial/bad publication cannot pass.
- [ ] Cross-backend `inputDigest` mismatch is red.
- [ ] Real metamorph samples/classification, no fabricated aliases.
- [ ] Search sweep catches hidden same-endpoint transition islands; per-probe radius/artifacts replayable.
- [ ] Replay/canonical artifacts cannot overwrite; synthetic failures retain bounded case/request/stderr.
- [ ] Exploratory raw-success/OneCAD-refusal is status difference, not regression.
- [ ] Windows/non-executable runner environment failures exit `3`.
- [ ] Report rejects empty/scalar/malformed data, deduplicates transitions, nearest-rank percentiles.
- [ ] Every task file under 500 lines and function under 50 lines.

## Verification order

Run smallest gates after each repair, then full gates:

```bash
# C++ focused
cmake --build worker/build --target onecad-kernelbench-runner test_kernelbench_audit -j4
ctest --test-dir worker/build -R 'kernelbench|fillet_builder' --output-on-failure

# Rust focused
cd src-tauri
cargo fmt --all --check
cargo clippy -p onecad-kernelbench --all-targets -- -D warnings
cargo test -p onecad-kernelbench
cd ..

# Full required gates
ONECAD_OCCT_BUILD_ID=homebrew-occt-8.0.1-20260807 scripts/build-worker.sh Release
ctest --test-dir worker/build --output-on-failure

cd src-tauri
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
ONECAD_WORKER_PATH=$PWD/../worker/build/onecad-worker \
ONECAD_REQUIRE_WORKER=1 cargo test --workspace

ONECAD_KERNELBENCH_RUNNER=$PWD/../worker/build/onecad-kernelbench-runner \
cargo run -p onecad-kernelbench -- \
  run --suite fillet/foundation --preset t0 --backend both \
  --out-dir ../worker/build/kernelbench-t0

cargo run -p onecad-kernelbench -- \
  report ../worker/build/kernelbench-t0/results.jsonl \
  --json ../worker/build/kernelbench-t0/summary.json
cd ..

git diff --check
```

Also validate every case, preset, and result JSONL record against the committed Draft 2020-12 schemas. Confirm exactly 36 bases, 68 variants, 136 canonical records, 272 child executions, identical raw/OneCAD input digests per case/variant, stable replays, and no unexpected gating failures.

Finally update the KERNELBENCH block in `TODO.md` with exact gate counts, T0 p50/p95, known characterization differences, and remaining deferred work.

## Unresolved questions

None. Make the technically safe choices above; do not weaken schema/audit/isolation claims to make the gate green.
