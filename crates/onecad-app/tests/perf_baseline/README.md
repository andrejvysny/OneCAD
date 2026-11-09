# Forty-feature Release replay measurements

The default `regen_baseline_forty_feature_part` test retains its original
one-shot behavior. Set `ONECAD_PERF_OUT` to opt into the controlled campaign:

```sh
scripts/build-worker.sh Release
cargo test --release -p onecad-app --test perf_baseline --no-run
perf_target_dir="$(cargo run --quiet -p xtask -- target-directory)"
cp worker/build/onecad-worker "$perf_target_dir/release/deps/onecad-worker"
ONECAD_REQUIRE_WORKER=1 \
ONECAD_WORKER_PATH="$PWD/worker/build/onecad-worker" \
ONECAD_PERF_OUT=/private/tmp/onecad-perf40-new \
  cargo test --release -p onecad-app --test perf_baseline \
  regen_baseline_forty_feature_part -- --exact --nocapture --test-threads=1
```

Release test hosts resolve only the adjacent bundled worker. The copy must match
the staged manifest embedded when the test executable was built. After packaging
rewrites or signs the staged worker, rebuild/stage the ordinary worker and rebuild
the test host before another raw-worker measurement; do not bypass integrity checks.

The output directory must be new and outside the repository. Each of three
batches uses one worker and runtime for its warm sequence. After one bootstrap
replay, each edit position (3, 21 and 39) receives five warmups and 100 measured
samples. Alternating pocket depths ensure every sample edits real geometry.
Each batch also runs 20 cold samples, each with a new worker and runtime.
Workers must retire fully before the next cold sample. OS caches are not
cleared; worker startup and fixture construction are recorded separately from
replay wall time.

`samples.jsonl` retains synced start/result records, failed or cancelled
terminals, production phase clocks, worker epoch, runtime identity and boundary
RSS observations. RSS comes from `ps` and describes the backend test-host and
worker processes; it is not peak memory or native desktop UI memory.
`metadata.json` records source, worker, configuration and fixture hashes, plus
observed power/display/hardware conditions. No system setting is changed.
The captured `.onecad` inputs preserve their original identities; the matching
fixture hash normalizes only document and derived root-definition IDs.

`summary.json` reports nearest-rank p50/p95/p99 for replay wall, planner, worker
and mesh times by batch and case. Warmups, bootstrap and failed samples do not
enter accepted distributions. The first refused/failed/cancelled sample stops
the campaign, retaining incomplete evidence. A source or worker-binary change
or test executable change during the run also makes the campaign incomplete.

Input handling, preview admission, bound-origin resolution, scene capture/upload,
GPU completion, physical presentation, peak memory and GPU memory remain
unmeasured. Compare only matching configurations and fixture hashes under
comparable recorded conditions. This harness does not establish an optimization
gain or qualify the other workloads.

## Public assembly and retained Framework measurements

The other two opt-in entry points use the same three batches, five warmups,
100 warm samples and 20 cold samples. Build both Release test executables after
staging the worker, and place that exact worker beside them as above:

```sh
cargo test --release -p onecad-app --test public_mixed_stress \
  --test framework_retention_regularization --no-run
ONECAD_REQUIRE_WORKER=1 ONECAD_WORKER_PATH="$PWD/worker/build/onecad-worker" \
ONECAD_PERF_OUT=/private/tmp/onecad-public-framework-new \
  cargo test --release -p onecad-app --test framework_retention_regularization \
  imported_retention_regularization_preserves_split_children -- --exact --nocapture --test-threads=1
ONECAD_REQUIRE_WORKER=1 ONECAD_WORKER_PATH="$PWD/worker/build/onecad-worker" \
ONECAD_PERF_OUT=/private/tmp/onecad-public-framework-new \
  cargo test --release -p onecad-app --test public_mixed_stress \
  public_mixed_ten_definitions_twenty_occurrences_cold_reopen -- --exact --nocapture --test-threads=1
```

The first public run constructs the native source recipes. Set
`ONECAD_PERF_FIXTURE=/absolute/path/to/fixture.onecad` to reuse a previously
captured initial fixture for matching comparisons; it must contain exactly
10 definitions and 20 occurrences. The warm application/runtime remains alive,
but production starts a cold auxiliary worker for each definition replay.
Each sample replays all ten definitions and captures checked CPU meshes.
Joint solving and bound-origin resolution do not apply to this unjointed fixture.

The retained Framework case measures the expected `PUBLICATION_OPEN_MANIFOLD`
refusal and verifies unchanged accepted mesh bytes and Undo. It does not qualify
the complete Framework enclosure or replace its retained reference. Its existing
30-second terminal and 0.001 mm publication contracts remain unchanged.

Both workloads share a new outside-repository output base and create distinct
workload directories. Set `ONECAD_PERF_SMOKE=1` only for harness debugging; those
results are explicitly incomplete and cannot substitute for full measurements.
Run workloads serially, without competing builds. Their provenance and percentile
artifacts retain the same limitations on peak memory, GPU and physical presentation.
