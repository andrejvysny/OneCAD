# Kernelbench CLI, result and validation contract

This historical technical reference preserves code-coupled contracts and validation procedure from [KERNELBENCH_CONTINUATION.md](https://github.com/andrejvysny/OneCAD/blob/a6d8a2dacad7b1759cd22d0f3eb64992b9c411b0/KERNELBENCH_CONTINUATION.md), master at `a6d8a2dacad7b1759cd22d0f3eb64992b9c411b0`, SHA-256 `9426804d72b7e84b49e88e8954c964a96452da79cc20312b235766423eb8faaf`. Current project work and gate status live in the [private ONECAD Plane project](https://app.plane.so/andrejvysny/projects/b7ff7739-7b47-449c-93e7-81f8d74aeaee/issues/). This extracted text does not assert present completion; newer normative protocols, schemas and accepted ADRs take precedence.

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
